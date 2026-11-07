use hotstuff_rs::block_tree::accessors::public::BlockTreeCamera;
use hotstuff_rs::block_tree::pluggables::KVStore;
use hotstuff_rs::events::CommitBlockEvent;
use hotstuff_rs::networking::network::Network;
use hotstuff_rs::replica::{Replica, ReplicaSpec};
use hotstuff_rs::types::data_types::{CryptoHash, Power};
use hotstuff_rs::types::update_sets::AppStateUpdates;
use hotstuff_rs::types::validator_set::{ValidatorSet, ValidatorSetState, VerifyingKey};
use std::time::UNIX_EPOCH;
use tokio::sync::broadcast;

use crate::app::MirasetApp;
use crate::core::types::{
    AccountState, BlockPayload, CommitEvent, ComputeReceipt, Transaction, WorkerInfo,
};
use crate::mempool::Mempool;
use crate::node::config::NodeConfig;

/// High-level MIRASET node orchestrating consensus, state execution, mempool, and storage.
pub struct MirasetNode<K: KVStore, N: Network + 'static> {
    pub replica: Replica<K>,
    pub mempool: Mempool,
    pub kv_store: K,
    pub network: N,
    pub commit_sender: broadcast::Sender<CommitEvent>,
}

/// Initialize genesis block tree storage with the given validators and initial accounts.
pub fn init_genesis<K: KVStore>(
    kv_store: K,
    validators: &[(VerifyingKey, u64)],
    initial_accounts: &[(&str, u64)],
) {
    let mut val_set = ValidatorSet::new();
    for (vk, power) in validators {
        val_set.put(vk, Power::new(*power));
    }

    let val_set_state = ValidatorSetState::new(val_set.clone(), val_set, None, true);

    let mut app_updates = AppStateUpdates::new();
    for (address, balance) in initial_accounts {
        let acc = AccountState {
            balance: *balance,
            nonce: 0,
        };
        app_updates.insert(
            MirasetApp::account_key(address),
            acc.to_bytes()
                .expect("Serialization of genesis account failed"),
        );
    }

    Replica::initialize(kv_store, app_updates, val_set_state);
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

    /// Start a new MIRASET replica node.
    pub fn start(config: NodeConfig, kv_store: K, network: N, mempool: Mempool) -> Self {
        let app = MirasetApp::new(mempool.clone());
        let hs_config = config.to_hotstuff_config();

        let (commit_sender, _) = broadcast::channel(1024);
        let commit_tx = commit_sender.clone();
        let kv_for_event = kv_store.clone();

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
                let (tx_count, receipt_count) = match snapshot.block_data(&event.block) {
                    Ok(Some(data)) if !data.vec().is_empty() => {
                        let datum = &data.vec()[0];
                        match BlockPayload::from_bytes(datum.bytes()) {
                            Ok(payload) => {
                                (payload.transactions.len(), payload.compute_receipts.len())
                            }
                            Err(_) => (0, 0),
                        }
                    }
                    _ => (0, 0),
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

        Self {
            replica,
            mempool,
            kv_store,
            network,
            commit_sender,
        }
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
