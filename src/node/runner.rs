use hotstuff_rs::block_tree::accessors::public::BlockTreeCamera;
use hotstuff_rs::block_tree::pluggables::KVStore;
use hotstuff_rs::events::{
    CollectTCEvent, CommitBlockEvent, EndSyncEvent, StartSyncEvent, StartViewEvent,
    ViewTimeoutEvent,
};
use hotstuff_rs::networking::network::Network;
use hotstuff_rs::replica::{Replica, ReplicaSpec};
use hotstuff_rs::types::data_types::{BlockHeight, CryptoHash, Power};
use hotstuff_rs::types::update_sets::AppStateUpdates;
use hotstuff_rs::types::validator_set::{ValidatorSet, ValidatorSetState, VerifyingKey};
use std::sync::Arc;
use std::time::{Duration, Instant, UNIX_EPOCH};
use tokio::sync::broadcast;

use crate::app::MirasetApp;
use crate::consensus::SafetyHalt;
use crate::core::types::{
    AccountState, BlockPayload, CommitEvent, ComputeReceipt, EpochInfo, ProtocolParams,
    Transaction, WorkerInfo, compute_state_commitment,
};
use crate::mempool::Mempool;
use crate::node::admission::{self, AdmissionError};
use crate::node::config::NodeConfig;
use crate::observability::{
    AdmissionKind, AdmissionReject, Lifecycle, NodeMetrics, NotReadyReason, ReadinessInput,
    ScrapeState, evaluate_readiness,
};
use crate::storage::{
    RecoveryMetadata, STORAGE_SCHEMA_VERSION, read_recovery_metadata, validate_recovery_state,
    write_recovery_metadata,
};

/// High-level MIRASET node orchestrating consensus, state execution, mempool, and storage.
pub struct MirasetNode<K: KVStore, N: Network + 'static> {
    pub replica: Replica<K>,
    pub mempool: Mempool,
    pub kv_store: K,
    pub network: N,
    pub commit_sender: broadcast::Sender<CommitEvent>,
    pub safety_halt: SafetyHalt,
    metrics: Arc<NodeMetrics>,
    lifecycle: Arc<Lifecycle>,
    ready_max_commit_age: Duration,
}

/// Canonical genesis application entries (accounts, protocol parameters and epoch 0), in
/// deterministic order.
fn genesis_app_entries(
    initial_accounts: &[(&str, u64)],
    params: &ProtocolParams,
) -> Vec<(Vec<u8>, Vec<u8>)> {
    let mut entries: Vec<(Vec<u8>, Vec<u8>)> = initial_accounts
        .iter()
        .map(|(address, balance)| {
            let account = AccountState {
                balance: *balance,
                nonce: 0,
            };
            let value = account
                .to_bytes()
                .expect("Serialization of genesis account failed");
            (MirasetApp::account_key(address), value)
        })
        .collect();

    entries.push((
        MirasetApp::protocol_params_key(),
        params
            .to_bytes()
            .expect("Serialization of protocol parameters failed"),
    ));
    let initial_epoch = EpochInfo::new(0, 0, params.epoch_reward_budget);
    let epoch_bytes = initial_epoch
        .to_bytes()
        .expect("Serialization of genesis epoch failed");
    entries.push((MirasetApp::current_epoch_key(), epoch_bytes.clone()));
    entries.push((MirasetApp::epoch_key(initial_epoch.epoch_id), epoch_bytes));
    entries
}

fn genesis_commitment_of(entries: &[(Vec<u8>, Vec<u8>)]) -> [u8; 32] {
    let refs: Vec<(&[u8], &[u8])> = entries
        .iter()
        .map(|(k, v)| (k.as_slice(), v.as_slice()))
        .collect();
    compute_state_commitment(&refs)
}

/// Genesis state commitment recorded in recovery metadata for the given initial accounts.
/// Nodes started from a genesis file pass this as `NodeConfig::expected_genesis_hash` so a
/// store initialized from a different genesis fails closed.
pub fn genesis_state_commitment(initial_accounts: &[(&str, u64)]) -> [u8; 32] {
    genesis_state_commitment_with_params(initial_accounts, &ProtocolParams::default())
}

/// Genesis state commitment for a genesis with explicit protocol parameters.
pub fn genesis_state_commitment_with_params(
    initial_accounts: &[(&str, u64)],
    params: &ProtocolParams,
) -> [u8; 32] {
    genesis_commitment_of(&genesis_app_entries(initial_accounts, params))
}

/// The pacemaker epoch length is a genesis parameter (TODO 8.5.2 F2): refuse a configuration
/// that disagrees with the value committed in the store's genesis state.
fn check_epoch_length_matches_genesis<K: KVStore>(
    store: &K,
    epoch_length: u32,
) -> Result<(), String> {
    let committed: ProtocolParams = store
        .committed_app_state(&MirasetApp::protocol_params_key())
        .and_then(|raw| serde_json::from_slice(&raw).ok())
        .unwrap_or_default();
    if committed.consensus_epoch_length != epoch_length {
        return Err(format!(
            "epoch_length {epoch_length} does not match genesis consensus_epoch_length {}",
            committed.consensus_epoch_length
        ));
    }
    Ok(())
}

/// Initialize genesis block tree storage with the given chain ID, validators, and initial accounts.
pub fn init_genesis_with_chain_id<K: KVStore>(
    kv_store: K,
    chain_id: u64,
    validators: &[(VerifyingKey, u64)],
    initial_accounts: &[(&str, u64)],
) {
    init_genesis_with_params(
        kv_store,
        chain_id,
        validators,
        initial_accounts,
        &ProtocolParams::default(),
    );
}

/// Initialize genesis storage with explicit protocol parameters (coordinators, governance,
/// price per token, epoch reward budget).
pub fn init_genesis_with_params<K: KVStore>(
    mut kv_store: K,
    chain_id: u64,
    validators: &[(VerifyingKey, u64)],
    initial_accounts: &[(&str, u64)],
    params: &ProtocolParams,
) {
    // Do not overwrite an already-initialized store
    if read_recovery_metadata(&kv_store).ok().flatten().is_some() {
        return;
    }

    let mut val_set = ValidatorSet::new();
    for (vk, power) in validators {
        val_set.put(vk, Power::new(*power));
    }

    let val_set_state = ValidatorSetState::new(val_set.clone(), val_set, None, true);

    let genesis_entries = genesis_app_entries(initial_accounts, params);
    let genesis_hash = genesis_commitment_of(&genesis_entries);
    let mut app_updates = AppStateUpdates::new();
    for (key, value) in genesis_entries {
        app_updates.insert(key, value);
    }
    app_updates.insert(MirasetApp::state_commitment_key(), genesis_hash.to_vec());

    Replica::initialize(kv_store.clone(), app_updates, val_set_state);

    let meta = RecoveryMetadata {
        schema_version: STORAGE_SCHEMA_VERSION,
        chain_id,
        genesis_hash,
    };
    write_recovery_metadata(&mut kv_store, &meta)
        .expect("failed to persist genesis recovery metadata");
}

/// Initialize genesis block tree storage with default devnet chain ID (1337).
pub fn init_genesis<K: KVStore>(
    kv_store: K,
    validators: &[(VerifyingKey, u64)],
    initial_accounts: &[(&str, u64)],
) {
    init_genesis_with_chain_id(kv_store, 1337, validators, initial_accounts);
}

impl<K: KVStore, N: Network + 'static> MirasetNode<K, N> {
    /// Initialize genesis block tree storage with the given validators and initial accounts.
    pub fn init_genesis(
        kv_store: K,
        validators: &[(VerifyingKey, u64)],
        initial_accounts: &[(&str, u64)],
    ) {
        init_genesis(kv_store, validators, initial_accounts);
    }

    /// Fallible node startup that validates `NodeConfig` and fail-closed storage recovery
    /// metadata before participating in consensus (PRD R05).
    pub fn try_start(
        config: NodeConfig,
        kv_store: K,
        network: N,
        mempool: Mempool,
    ) -> Result<Self, String> {
        config.validate().map_err(|e| e.to_string())?;
        validate_recovery_state(&kv_store, config.chain_id, config.expected_genesis_hash)?;
        check_epoch_length_matches_genesis(&kv_store, config.epoch_length)?;

        let safety_halt = config.safety_halt.clone();
        let app = MirasetApp::new(mempool.clone())
            .with_empty_block_interval(config.effective_empty_block_interval());
        let hs_config = config.to_hotstuff_config();

        let (commit_sender, _) = broadcast::channel(1024);
        let commit_tx = commit_sender.clone();
        let kv_for_event = kv_store.clone();
        let mempool_for_event = mempool.clone();
        let metrics = Arc::new(NodeMetrics::default());
        let metrics_for_event = Arc::clone(&metrics);
        let metrics_for_sync_start = Arc::clone(&metrics);
        let metrics_for_sync_end = Arc::clone(&metrics);
        let metrics_for_view = Arc::clone(&metrics);
        let metrics_for_timeout = Arc::clone(&metrics);
        let metrics_for_tc = Arc::clone(&metrics);
        let ready_max_commit_age = config.ready_max_commit_age;

        let replica = ReplicaSpec::builder()
            .app(app)
            .network(network.clone())
            .kv_store(kv_store.clone())
            .configuration(hs_config)
            // Events reach handlers in publication order on one bus thread, so commits made
            // inside a sync session are seen between its start and end.
            .on_start_sync(move |_: &StartSyncEvent| metrics_for_sync_start.record_sync_start())
            .on_end_sync(move |event: &EndSyncEvent| {
                metrics_for_sync_end.record_sync_end(event.blocks_synced)
            })
            .on_start_view(move |event: &StartViewEvent| {
                metrics_for_view.record_view_started();
                metrics_for_view.runtime().record_view(event.view.int());
            })
            .on_view_timeout(move |_: &ViewTimeoutEvent| {
                metrics_for_timeout.runtime().record_view_timeout()
            })
            .on_collect_tc(move |_: &CollectTCEvent| {
                metrics_for_tc.runtime().record_timeout_certificate()
            })
            .on_commit_block(move |event: &CommitBlockEvent| {
                // Timed before any work in this handler (I2); sync commits do not form intervals.
                metrics_for_event
                    .runtime()
                    .record_commit_time(Instant::now(), !metrics_for_event.is_syncing());
                let block_hash_hex = hex::encode(event.block.bytes());
                let timestamp_secs = event
                    .timestamp
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);

                let camera = BlockTreeCamera::new(kv_for_event.clone());
                let snapshot = camera.snapshot();
                // The block is committed whatever its payload, so readiness sees it first.
                let height = match snapshot.block_height(&event.block) {
                    Ok(Some(height)) => Some(height.int()),
                    other => {
                        tracing::warn!(
                            block = %block_hash_hex,
                            "committed block height unreadable ({other:?}); recorded without height"
                        );
                        None
                    }
                };
                metrics_for_event.record_commit(height);
                // Only publish finalized CommitEvent after verifying the block is durably
                // readable from the block tree snapshot.
                let (tx_count, receipt_count) = match snapshot.block_data(&event.block) {
                    Ok(Some(data)) if !data.vec().is_empty() => {
                        let datum = &data.vec()[0];
                        match BlockPayload::from_bytes(datum.bytes()) {
                            Ok(payload) => {
                                // Every node commits every block, so this also clears copies
                                // of a nonce or task that were admitted on this node but
                                // committed through another one.
                                for tx in &payload.transactions {
                                    mempool_for_event.prune_sender_up_to_nonce(
                                        &tx.sender,
                                        tx.nonce.saturating_add(1),
                                    );
                                }
                                for receipt in &payload.compute_receipts {
                                    mempool_for_event.remove_receipt(&receipt.task_id);
                                }
                                metrics_for_event.runtime().record_block(
                                    payload.transactions.len(),
                                    payload.compute_receipts.len(),
                                    datum.bytes().len(),
                                );
                                (payload.transactions.len(), payload.compute_receipts.len())
                            }
                            Err(_) => {
                                metrics_for_event.runtime().record_unreadable_block();
                                return;
                            }
                        }
                    }
                    Ok(Some(_)) => {
                        metrics_for_event.runtime().record_block(0, 0, 0);
                        (0, 0)
                    }
                    _ => {
                        metrics_for_event.runtime().record_unreadable_block();
                        return;
                    }
                };

                let _ = commit_tx.send(CommitEvent {
                    block_hash: block_hash_hex,
                    timestamp_secs,
                    tx_count,
                    receipt_count,
                    height,
                });
            })
            .build()
            .start();

        Ok(Self {
            replica,
            mempool,
            kv_store,
            network,
            commit_sender,
            safety_halt,
            metrics,
            lifecycle: Arc::new(Lifecycle::default()),
            ready_max_commit_age,
        })
    }

    /// Start a new MIRASET replica node after fail-closed recovery validation.
    pub fn start(config: NodeConfig, kv_store: K, network: N, mempool: Mempool) -> Self {
        Self::try_start(config, kv_store, network, mempool)
            .expect("node startup failed recovery or config validation")
    }

    /// Consensus safety halt shared with the guarded network and chain store.
    pub fn safety_halt(&self) -> &SafetyHalt {
        &self.safety_halt
    }

    /// Process metrics (TODO 8.4.4).
    pub fn metrics(&self) -> &Arc<NodeMetrics> {
        &self.metrics
    }

    /// Drain state shared by admission, probes and shutdown (TODO 8.4.4).
    pub fn lifecycle(&self) -> &Lifecycle {
        &self.lifecycle
    }

    /// Readiness to finalize: no safety halt, not draining, not syncing, and a recent commit
    /// through consensus.
    pub fn readiness(&self) -> Result<(), NotReadyReason> {
        evaluate_readiness(ReadinessInput {
            halted: self.safety_halt.is_halted(),
            draining: self.lifecycle.is_draining(),
            syncing: self.metrics.is_syncing(),
            last_commit_age: self.metrics.last_consensus_commit_age(),
            max_commit_age: self.ready_max_commit_age,
        })
    }

    /// How fresh reads from this node's committed state are: the time since its last commit
    /// through consensus (absent since start or since a block-sync session began) and, if the
    /// node may be behind the chain, why — halted, syncing, or no consensus commit recently.
    /// Commits made by block sync do not count: until the node commits through consensus again
    /// it cannot tell that it has reached the tip. The age is read once, so both answers agree.
    pub fn freshness(&self) -> (Option<Duration>, Option<NotReadyReason>) {
        let age = self.metrics.last_consensus_commit_age();
        let stale = evaluate_readiness(ReadinessInput {
            halted: self.safety_halt.is_halted(),
            draining: false,
            syncing: self.metrics.is_syncing(),
            last_commit_age: age,
            max_commit_age: self.ready_max_commit_age,
        })
        .err();
        (age, stale)
    }

    /// Metrics in Prometheus text format, with gauges sampled now.
    pub fn render_metrics(&self) -> String {
        self.metrics.render(ScrapeState {
            mempool_pending_txs: self.mempool.tx_count(),
            mempool_pending_receipts: self.mempool.receipt_count(),
            halted: self.safety_halt.is_halted(),
            draining: self.lifecycle.is_draining(),
            ready: self.readiness().is_ok(),
        })
    }

    /// Subscribe to committed block events.
    pub fn subscribe_commits(&self) -> broadcast::Receiver<CommitEvent> {
        self.commit_sender.subscribe()
    }

    /// Returns a reference to the node's mempool.
    pub fn mempool(&self) -> &Mempool {
        &self.mempool
    }

    /// Returns a camera into the block tree for state queries.
    pub fn block_tree_camera(&self) -> &BlockTreeCamera<K> {
        self.replica.block_tree_camera()
    }

    /// Read an account state from the committed state.
    pub fn get_account(&self, address: &str) -> Option<AccountState> {
        let snapshot = self.block_tree_camera().snapshot();
        let raw = snapshot.committed_app_state(&MirasetApp::account_key(address))?;
        AccountState::from_bytes(&raw).ok()
    }

    /// Read a registered worker's state from the committed state.
    /// Remaining amount `coordinator` may charge `client` through receipts (committed).
    pub fn allowance(&self, client: &str, coordinator: &str) -> u64 {
        let snapshot = self.block_tree_camera().snapshot();
        snapshot
            .committed_app_state(&MirasetApp::allowance_key(client, coordinator))
            .and_then(|raw| raw.try_into().ok().map(u64::from_be_bytes))
            .unwrap_or(0)
    }

    /// Timestamp (Unix ms) of the highest committed block; 0 before the first.
    pub fn committed_time_ms(&self) -> u64 {
        let snapshot = self.block_tree_camera().snapshot();
        snapshot
            .committed_app_state(&MirasetApp::chain_time_key())
            .and_then(|raw| raw.try_into().ok().map(u64::from_be_bytes))
            .unwrap_or(0)
    }

    /// Whether the current epoch has lasted `epoch_seconds` of committed block time.
    pub fn epoch_may_close(&self) -> bool {
        let Some(epoch) = self.get_current_epoch() else {
            return false;
        };
        let end = epoch
            .start_timestamp
            .saturating_add(self.protocol_params().epoch_seconds);
        self.committed_time_ms() / 1000 >= end
    }

    /// Genesis protocol parameters from committed state (defaults if absent).
    pub fn protocol_params(&self) -> ProtocolParams {
        let snapshot = self.block_tree_camera().snapshot();
        snapshot
            .committed_app_state(&MirasetApp::protocol_params_key())
            .and_then(|raw| serde_json::from_slice(&raw).ok())
            .unwrap_or_default()
    }

    pub fn get_worker(&self, worker_id: &str) -> Option<WorkerInfo> {
        let snapshot = self.block_tree_camera().snapshot();
        let raw = snapshot.committed_app_state(&MirasetApp::worker_key(worker_id))?;
        WorkerInfo::from_bytes(&raw).ok()
    }

    /// Read the currently active accounting epoch from the committed state.
    pub fn get_current_epoch(&self) -> Option<EpochInfo> {
        let snapshot = self.block_tree_camera().snapshot();
        let raw = snapshot.committed_app_state(&MirasetApp::current_epoch_key())?;
        EpochInfo::from_bytes(&raw).ok()
    }

    /// Read a specific historical or active epoch from the committed state.
    pub fn get_epoch(&self, epoch_id: u64) -> Option<EpochInfo> {
        let snapshot = self.block_tree_camera().snapshot();
        let raw = snapshot.committed_app_state(&MirasetApp::epoch_key(epoch_id))?;
        EpochInfo::from_bytes(&raw).ok()
    }

    /// Check if a compute task has been settled on-chain.
    pub fn is_task_settled(&self, task_id: &str) -> bool {
        let snapshot = self.block_tree_camera().snapshot();
        snapshot
            .committed_app_state(&MirasetApp::task_key(task_id))
            .is_some()
    }

    /// Read the hash of the highest committed block.
    pub fn highest_committed_block(&self) -> Option<CryptoHash> {
        self.block_tree_camera()
            .snapshot()
            .highest_committed_block()
            .ok()
            .flatten()
    }

    /// Height of the highest committed block, if any block has been committed.
    pub fn highest_committed_height(&self) -> Option<u64> {
        self.highest_committed().map(|(_, height)| height)
    }

    /// Hash and height of the highest committed block, read from one snapshot so the two
    /// always describe the same block.
    pub fn highest_committed(&self) -> Option<(CryptoHash, u64)> {
        let snapshot = self.block_tree_camera().snapshot();
        let hash = snapshot.highest_committed_block().ok().flatten()?;
        let height = snapshot.block_height(&hash).ok().flatten()?.int();
        Some((hash, height))
    }

    /// Hash of the committed block at `height`; `None` above the highest committed height.
    pub fn committed_block_at_height(&self, height: u64) -> Option<CryptoHash> {
        if height > self.highest_committed_height()? {
            return None;
        }
        self.block_tree_camera()
            .snapshot()
            .block_at_height(BlockHeight::new(height))
            .ok()
            .flatten()
    }

    /// Check a transaction against committed state, then admit it to the local mempool.
    pub fn submit_transaction(&self, tx: Transaction) -> Result<(), AdmissionError> {
        self.admit(AdmissionKind::Transaction, || {
            if self.mempool.contains_tx(&tx.sender, tx.nonce) {
                return Err(AdmissionError::Duplicate);
            }
            let committed = self.get_account(&tx.sender).unwrap_or(AccountState {
                balance: 0,
                nonce: 0,
            });
            let next_nonce = self
                .mempool
                .last_pending_nonce(&tx.sender)
                .and_then(|last| last.checked_add(1))
                .map_or(committed.nonce, |next| next.max(committed.nonce));
            let min_fee = self.mempool.limits().min_fee;
            admission::check_transaction(&tx, &committed, next_nonce, min_fee)?;
            admission::check_system_transaction(
                &tx,
                &self.protocol_params(),
                self.epoch_may_close(),
            )?;
            let (sender, nonce) = (tx.sender.clone(), tx.nonce);
            self.mempool.insert_tx(tx)?;
            // A block may have committed this nonce between the check and the insert, after the
            // commit hook pruned the pool; do not leave a stale transaction behind.
            if self.get_account(&sender).is_some_and(|a| a.nonce > nonce) {
                self.mempool
                    .prune_sender_up_to_nonce(&sender, nonce.saturating_add(1));
                return Err(AdmissionError::Replay("nonce already used"));
            }
            Ok(())
        })
    }

    /// Check a compute receipt against committed state, then admit it to the local mempool.
    pub fn submit_receipt(&self, receipt: ComputeReceipt) -> Result<(), AdmissionError> {
        self.admit(AdmissionKind::Receipt, || {
            if self.mempool.contains_receipt(&receipt.task_id) {
                return Err(AdmissionError::Duplicate);
            }
            let params = self.protocol_params();
            let worker = self.get_worker(&receipt.worker_id);
            let client = (!receipt.client_address.is_empty())
                .then(|| self.get_account(&receipt.client_address))
                .flatten();
            let ctx = admission::ReceiptContext {
                params: &params,
                task_settled: self.is_task_settled(&receipt.task_id),
                worker: worker.as_ref(),
                client: client.as_ref(),
                allowance: self.allowance(&receipt.client_address, &receipt.coordinator_pubkey),
            };
            admission::check_receipt(&receipt, &ctx)?;
            let task_id = receipt.task_id.clone();
            self.mempool.insert_receipt(receipt)?;
            // Same race as for transactions: the task may have settled meanwhile.
            if self.is_task_settled(&task_id) {
                self.mempool.remove_receipt(&task_id);
                return Err(AdmissionError::Replay("task already settled"));
            }
            Ok(())
        })
    }

    /// Refuse while draining, otherwise check and insert; count the outcome.
    fn admit(
        &self,
        kind: AdmissionKind,
        check_and_insert: impl FnOnce() -> Result<(), AdmissionError>,
    ) -> Result<(), AdmissionError> {
        let outcome = if self.lifecycle.is_draining() {
            Err(AdmissionError::Draining)
        } else {
            check_and_insert()
        };
        match outcome {
            Ok(()) => self.metrics.record_admitted(kind),
            Err(err) => self
                .metrics
                .record_rejected(kind, AdmissionReject::from(err)),
        }
        outcome
    }
}
