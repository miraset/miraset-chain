use hotstuff_rs::block_tree::accessors::public::BlockTreeCamera;
use hotstuff_rs::block_tree::pluggables::KVStore;
use hotstuff_rs::events::CommitBlockEvent;
use hotstuff_rs::networking::network::Network;
use hotstuff_rs::replica::{Replica, ReplicaSpec};
use hotstuff_rs::types::data_types::{BlockHeight, CryptoHash, Power};
use hotstuff_rs::types::update_sets::AppStateUpdates;
use hotstuff_rs::types::validator_set::{ValidatorSet, ValidatorSetState, VerifyingKey};
use std::time::UNIX_EPOCH;
use tokio::sync::broadcast;

use crate::app::MirasetApp;
use crate::core::types::{
    AccountState, BlockPayload, CommitEvent, ComputeReceipt, EpochInfo, Transaction, WorkerInfo,
    compute_state_commitment,
};
use crate::mempool::Mempool;
use crate::node::config::NodeConfig;
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
}

/// Canonical genesis application entries (accounts and epoch 0), in deterministic order.
fn genesis_app_entries(initial_accounts: &[(&str, u64)]) -> Vec<(Vec<u8>, Vec<u8>)> {
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

    let initial_epoch = EpochInfo::new(0, 0, 100_000);
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
    genesis_commitment_of(&genesis_app_entries(initial_accounts))
}

/// Initialize genesis block tree storage with the given chain ID, validators, and initial accounts.
pub fn init_genesis_with_chain_id<K: KVStore>(
    mut kv_store: K,
    chain_id: u64,
    validators: &[(VerifyingKey, u64)],
    initial_accounts: &[(&str, u64)],
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

    let genesis_entries = genesis_app_entries(initial_accounts);
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

        let app = MirasetApp::new(mempool.clone());
        let hs_config = config.to_hotstuff_config();

        let (commit_sender, _) = broadcast::channel(1024);
        let commit_tx = commit_sender.clone();
        let kv_for_event = kv_store.clone();
        let mempool_for_event = mempool.clone();

        let replica = ReplicaSpec::builder()
            .app(app)
            .network(network.clone())
            .kv_store(kv_store.clone())
            .configuration(hs_config)
            .on_commit_block(move |event: &CommitBlockEvent| {
                let block_hash_hex = hex::encode(event.block.bytes());
                let timestamp_secs = event
                    .timestamp
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);

                let camera = BlockTreeCamera::new(kv_for_event.clone());
                let snapshot = camera.snapshot();
                // Only publish finalized CommitEvent after verifying the block is durably
                // readable from the block tree snapshot.
                let (tx_count, receipt_count) = match snapshot.block_data(&event.block) {
                    Ok(Some(data)) if !data.vec().is_empty() => {
                        let datum = &data.vec()[0];
                        match BlockPayload::from_bytes(datum.bytes()) {
                            Ok(payload) => {
                                for tx in &payload.transactions {
                                    mempool_for_event.remove_tx(&tx.sender, tx.nonce);
                                }
                                for receipt in &payload.compute_receipts {
                                    mempool_for_event.remove_receipt(&receipt.task_id);
                                }
                                (payload.transactions.len(), payload.compute_receipts.len())
                            }
                            Err(_) => return,
                        }
                    }
                    Ok(Some(_)) => (0, 0),
                    _ => return,
                };

                let _ = commit_tx.send(CommitEvent {
                    block_hash: block_hash_hex,
                    timestamp_secs,
                    tx_count,
                    receipt_count,
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
        })
    }

    /// Start a new MIRASET replica node after fail-closed recovery validation.
    pub fn start(config: NodeConfig, kv_store: K, network: N, mempool: Mempool) -> Self {
        Self::try_start(config, kv_store, network, mempool)
            .expect("node startup failed recovery or config validation")
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
        let snapshot = self.block_tree_camera().snapshot();
        let hash = snapshot.highest_committed_block().ok().flatten()?;
        snapshot.block_height(&hash).ok().flatten().map(|h| h.int())
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

    /// Submit a transaction to the local mempool.
    pub fn submit_transaction(&self, tx: Transaction) -> Result<(), &'static str> {
        if self.mempool.insert_tx(tx) {
            Ok(())
        } else {
            Err("Transaction already exists in mempool")
        }
    }

    /// Submit a compute receipt to the local mempool.
    pub fn submit_receipt(&self, receipt: ComputeReceipt) -> Result<(), &'static str> {
        if self.mempool.insert_receipt(receipt) {
            Ok(())
        } else {
            Err("Receipt already exists in mempool")
        }
    }
}
