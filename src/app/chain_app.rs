use crate::core::types::{AccountState, BlockPayload, WorkerInfo};
use crate::mempool::Mempool;
use hotstuff_rs::app::{
    App, ProduceBlockRequest, ProduceBlockResponse, ValidateBlockRequest, ValidateBlockResponse,
};
use hotstuff_rs::block_tree::pluggables::KVStore;
use hotstuff_rs::types::data_types::{CryptoHash, Data, Datum};
use hotstuff_rs::types::update_sets::AppStateUpdates;
use sha2::{Digest, Sha256};

/// Application state machine implementing `hotstuff_rs::app::App`.
pub struct MirasetApp {
    mempool: Mempool,
}

impl MirasetApp {
    pub fn new(mempool: Mempool) -> Self {
        Self { mempool }
    }

    pub fn mempool(&self) -> &Mempool {
        &self.mempool
    }

    /// Helper key prefixes for on-chain state trie/storage.
    pub fn account_key(address: &str) -> Vec<u8> {
        let mut key = b"acc:".to_vec();
        key.extend_from_slice(address.as_bytes());
        key
    }

    pub fn worker_key(worker_id: &str) -> Vec<u8> {
        let mut key = b"worker:".to_vec();
        key.extend_from_slice(worker_id.as_bytes());
        key
    }

    pub fn task_key(task_id: &str) -> Vec<u8> {
        let mut key = b"task:".to_vec();
        key.extend_from_slice(task_id.as_bytes());
        key
    }

    /// Read account state from block tree view or default to 0.
    fn read_account<K: KVStore>(
        tree: &hotstuff_rs::block_tree::accessors::app::AppBlockTreeView<'_, K>,
        address: &str,
    ) -> AccountState {
        let key = Self::account_key(address);
        tree.app_state(&key)
            .and_then(|bytes| serde_json::from_slice::<AccountState>(&bytes).ok())
            .unwrap_or_default()
    }

    /// Read worker state from block tree view.
    fn read_worker<K: KVStore>(
        tree: &hotstuff_rs::block_tree::accessors::app::AppBlockTreeView<'_, K>,
        worker_id: &str,
    ) -> Option<WorkerInfo> {
        let key = Self::worker_key(worker_id);
        tree.app_state(&key)
            .and_then(|bytes| serde_json::from_slice::<WorkerInfo>(&bytes).ok())
    }

    /// Process payload against block tree view to compute state updates.
    fn compute_state_updates<K: KVStore>(
        tree: &hotstuff_rs::block_tree::accessors::app::AppBlockTreeView<'_, K>,
        payload: &BlockPayload,
    ) -> Option<AppStateUpdates> {
        let mut updates = AppStateUpdates::new();

        // Track running balances in-memory during block processing
        let mut local_accounts = std::collections::HashMap::<String, AccountState>::new();
        let mut local_tasks = std::collections::HashSet::<String>::new();

        for tx in &payload.transactions {
            let mut sender_acc = local_accounts
                .remove(&tx.sender)
                .unwrap_or_else(|| Self::read_account(tree, &tx.sender));

            let required = tx.amount.saturating_add(tx.fee);
            if sender_acc.balance < required {
                // Invalid transaction (insufficient funds)
                return None;
            }

            sender_acc.balance = sender_acc.balance.saturating_sub(required);
            sender_acc.nonce = sender_acc.nonce.saturating_add(1);
            local_accounts.insert(tx.sender.clone(), sender_acc);

            let mut recipient_acc = local_accounts
                .remove(&tx.recipient)
                .unwrap_or_else(|| Self::read_account(tree, &tx.recipient));
            recipient_acc.balance = recipient_acc.balance.saturating_add(tx.amount);
            local_accounts.insert(tx.recipient.clone(), recipient_acc);
        }

        // Process PoCC compute receipts:
        // - Enforce anti-replay on task_id
        // - Verify client receipt delivery confirmation
        // - Debit client account for compute cost
        // - Credit worker rewards and increment completed tasks
        for receipt in &payload.compute_receipts {
            let t_key = Self::task_key(&receipt.task_id);
            if tree.app_state(&t_key).is_some() || local_tasks.contains(&receipt.task_id) {
                // Reject block on duplicate task replay
                return None;
            }
            local_tasks.insert(receipt.task_id.clone());
            updates.insert(t_key, b"settled".to_vec());

            // If client account is specified, verify client delivery confirmation and debit cost
            if !receipt.client_address.is_empty() {
                if !receipt.client_confirmed {
                    // Client did not confirm receiving all generated output
                    return None;
                }

                if receipt.client_cost > 0 {
                    let mut client_acc = local_accounts
                        .remove(&receipt.client_address)
                        .unwrap_or_else(|| Self::read_account(tree, &receipt.client_address));

                    if client_acc.balance < receipt.client_cost {
                        // Insufficient balance to debit client
                        return None;
                    }

                    client_acc.balance = client_acc.balance.saturating_sub(receipt.client_cost);
                    local_accounts.insert(receipt.client_address.clone(), client_acc);
                }
            }

            let reward = if receipt.worker_reward > 0 {
                receipt.worker_reward
            } else {
                receipt.gas_reward
            };

            let mut worker = Self::read_worker(tree, &receipt.worker_id).unwrap_or(WorkerInfo {
                worker_id: receipt.worker_id.clone(),
                stake: 0,
                completed_tasks: 0,
                rewards_accumulated: 0,
                active: true,
            });

            worker.completed_tasks = worker.completed_tasks.saturating_add(1);
            worker.rewards_accumulated = worker.rewards_accumulated.saturating_add(reward);

            let key = Self::worker_key(&receipt.worker_id);
            let val = serde_json::to_vec(&worker).expect("worker serialization failed");
            updates.insert(key, val);
        }

        // Apply account updates (both transactions and compute receipt client debits)
        for (addr, acc) in local_accounts {
            let key = Self::account_key(&addr);
            let val = serde_json::to_vec(&acc).expect("account serialization failed");
            updates.insert(key, val);
        }

        Some(updates)
    }
}

impl<K: KVStore> App<K> for MirasetApp {
    fn produce_block(&mut self, request: ProduceBlockRequest<'_, K>) -> ProduceBlockResponse {
        let (txs, receipts) = self.mempool.drain_for_block(100, 20);
        let payload = BlockPayload::new(txs, receipts);
        let payload_bytes = payload.to_bytes();

        let mut hasher = Sha256::new();
        hasher.update(&payload_bytes);
        let hash_arr: [u8; 32] = hasher.finalize().into();
        let data_hash = CryptoHash::new(hash_arr);

        let data = Data::new(vec![Datum::new(payload_bytes)]);

        let app_state_updates = Self::compute_state_updates(request.block_tree(), &payload);

        ProduceBlockResponse {
            data_hash,
            data,
            app_state_updates,
            validator_set_updates: None,
        }
    }

    fn validate_block(
        &mut self,
        request: ValidateBlockRequest<'_, '_, K>,
    ) -> ValidateBlockResponse {
        let block = request.proposed_block();

        // Extract payload from data
        if block.data.vec().is_empty() {
            return ValidateBlockResponse::Invalid;
        }

        let datum = &block.data.vec()[0];
        let payload = match BlockPayload::from_bytes(datum.bytes()) {
            Ok(p) => p,
            Err(_) => return ValidateBlockResponse::Invalid,
        };

        // Verify hash integrity
        let mut hasher = Sha256::new();
        hasher.update(datum.bytes());
        let expected_hash: [u8; 32] = hasher.finalize().into();
        if block.data_hash.bytes() != expected_hash {
            return ValidateBlockResponse::Invalid;
        }

        // Validate state transitions
        match Self::compute_state_updates(request.block_tree(), &payload) {
            Some(updates) => ValidateBlockResponse::Valid {
                app_state_updates: Some(updates),
                validator_set_updates: None,
            },
            None => ValidateBlockResponse::Invalid,
        }
    }

    fn validate_block_for_sync(
        &mut self,
        request: ValidateBlockRequest<'_, '_, K>,
    ) -> ValidateBlockResponse {
        self.validate_block(request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::{ComputeReceipt, Transaction};

    #[test]
    fn test_account_and_worker_keys() {
        let acc_k = MirasetApp::account_key("alice");
        assert_eq!(acc_k, b"acc:alice".to_vec());

        let wrk_k = MirasetApp::worker_key("gpu_01");
        assert_eq!(wrk_k, b"worker:gpu_01".to_vec());

        let task_k = MirasetApp::task_key("task_99");
        assert_eq!(task_k, b"task:task_99".to_vec());
    }

    #[test]
    fn test_app_creation_with_mempool() {
        let mempool = Mempool::new();
        mempool.insert_tx(Transaction::new("alice", 1, "bob", 100, 5));
        let receipt = ComputeReceipt::new(
            "t1",
            "client_alice",
            "gpu_worker_01",
            "coord_01",
            "model_llama",
            "proof_abc",
            150,
            20,
            18,
        );
        mempool.insert_receipt(receipt);

        let app = MirasetApp::new(mempool);
        assert_eq!(app.mempool().tx_count(), 1);
        assert_eq!(app.mempool().receipt_count(), 1);
    }
}
