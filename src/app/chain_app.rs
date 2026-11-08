use crate::core::types::{
    AccountState, BlockPayload, MIN_WORKER_BOND, Transaction, WorkerBondAction, WorkerInfo,
};
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

    /// Pure state transition for one transaction against its sender's current account:
    /// verifies the sender's signature, enforces strict nonce sequencing (rejects replays
    /// and out-of-order submissions), and checks funds. Returns the updated sender account
    /// or a rejection reason; never mutates `sender_acc` in place.
    fn apply_transaction_to_sender(
        tx: &Transaction,
        sender_acc: &AccountState,
    ) -> Result<AccountState, &'static str> {
        tx.verify_signature()?;

        if tx.nonce != sender_acc.nonce {
            return Err("transaction nonce does not match expected account nonce");
        }

        let required = tx.amount.saturating_add(tx.fee);
        if sender_acc.balance < required {
            return Err("insufficient funds");
        }

        let mut updated = sender_acc.clone();
        updated.balance = updated.balance.saturating_sub(required);
        updated.nonce = updated
            .nonce
            .checked_add(1)
            .ok_or("account nonce overflow")?;
        Ok(updated)
    }

    /// Process payload against block tree view to compute state updates.
    fn compute_state_updates<K: KVStore>(
        tree: &hotstuff_rs::block_tree::accessors::app::AppBlockTreeView<'_, K>,
        payload: &BlockPayload,
    ) -> Option<AppStateUpdates> {
        let mut updates = AppStateUpdates::new();

        // Track running balances in-memory during block processing
        let mut local_accounts = std::collections::HashMap::<String, AccountState>::new();
        let mut local_workers = std::collections::HashMap::<String, WorkerInfo>::new();
        let mut local_tasks = std::collections::HashSet::<String>::new();

        for tx in &payload.transactions {
            let sender_acc = local_accounts
                .remove(&tx.sender)
                .unwrap_or_else(|| Self::read_account(tree, &tx.sender));

            // Reject the block on an unsigned/forged sender, a replayed or
            // out-of-order nonce, or insufficient funds.
            let updated_sender = Self::apply_transaction_to_sender(tx, &sender_acc).ok()?;
            local_accounts.insert(tx.sender.clone(), updated_sender);

            // Handle worker bond actions if recipient is system:worker_registry
            if tx.recipient == "system:worker_registry" && !tx.payload.is_empty() {
                let action: WorkerBondAction = match serde_json::from_slice(&tx.payload) {
                    Ok(a) => a,
                    Err(_) => return None, // malformed payload
                };

                match action {
                    WorkerBondAction::Register {
                        worker_id,
                        bond_amount,
                        vram_gib,
                        endpoint,
                    } => {
                        let mut worker = local_workers
                            .remove(&worker_id)
                            .or_else(|| Self::read_worker(tree, &worker_id))
                            .unwrap_or(WorkerInfo {
                                worker_id: worker_id.clone(),
                                stake: 0,
                                completed_tasks: 0,
                                rewards_accumulated: 0,
                                active: true,
                                vram_gib,
                                uptime_bps: 10000,
                                epoch_tokens: 0,
                                owner_address: tx.sender.clone(),
                                endpoint: endpoint.clone(),
                            });

                        // If already has an owner, only owner can add bond
                        if !worker.owner_address.is_empty() && worker.owner_address != tx.sender {
                            return None;
                        }

                        worker.owner_address = tx.sender.clone();
                        worker.stake = worker.stake.saturating_add(bond_amount);
                        if vram_gib > 0 {
                            worker.vram_gib = vram_gib;
                        }
                        if !endpoint.is_empty() {
                            worker.endpoint = endpoint;
                        }
                        if worker.stake >= MIN_WORKER_BOND {
                            worker.active = true;
                        }
                        local_workers.insert(worker_id, worker);
                    }
                    WorkerBondAction::Unbond { worker_id, amount } => {
                        let mut worker = local_workers
                            .remove(&worker_id)
                            .or_else(|| Self::read_worker(tree, &worker_id))?;

                        // Only owner can unbond
                        if worker.owner_address != tx.sender || worker.stake < amount {
                            return None;
                        }

                        worker.stake = worker.stake.saturating_sub(amount);
                        if worker.stake < MIN_WORKER_BOND {
                            worker.active = false;
                        }
                        local_workers.insert(worker_id, worker);

                        // Credit unbonded funds back to owner
                        let mut owner_acc = local_accounts
                            .remove(&tx.sender)
                            .unwrap_or_else(|| Self::read_account(tree, &tx.sender));
                        owner_acc.balance = owner_acc.balance.saturating_add(amount);
                        local_accounts.insert(tx.sender.clone(), owner_acc);
                    }
                    WorkerBondAction::Slash {
                        worker_id,
                        slash_amount,
                        ..
                    } => {
                        // Slashes burn worker stake
                        let mut worker = local_workers
                            .remove(&worker_id)
                            .or_else(|| Self::read_worker(tree, &worker_id))?;

                        worker.stake = worker.stake.saturating_sub(slash_amount);
                        if worker.stake < MIN_WORKER_BOND {
                            worker.active = false;
                        }
                        local_workers.insert(worker_id, worker);
                    }
                }
            } else {
                let mut recipient_acc = local_accounts
                    .remove(&tx.recipient)
                    .unwrap_or_else(|| Self::read_account(tree, &tx.recipient));
                recipient_acc.balance = recipient_acc.balance.saturating_add(tx.amount);
                local_accounts.insert(tx.recipient.clone(), recipient_acc);
            }
        }

        // Process PoCC compute receipts:
        // - Cryptographic signature verification (if signature provided)
        // - Enforce anti-replay on task_id
        // - Verify client receipt delivery confirmation
        // - Debit client account for compute cost
        // - Credit worker rewards and increment completed tasks
        for receipt in &payload.compute_receipts {
            if receipt.verify_signature().is_err() {
                // Reject block on invalid signature
                return None;
            }

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

            let mut worker = local_workers
                .remove(&receipt.worker_id)
                .or_else(|| Self::read_worker(tree, &receipt.worker_id))
                .unwrap_or(WorkerInfo {
                    worker_id: receipt.worker_id.clone(),
                    stake: 0,
                    completed_tasks: 0,
                    rewards_accumulated: 0,
                    active: true,
                    vram_gib: 24,
                    uptime_bps: 10000,
                    epoch_tokens: 0,
                    owner_address: String::new(),
                    endpoint: String::new(),
                });

            worker.completed_tasks = worker.completed_tasks.saturating_add(1);
            worker.rewards_accumulated = worker.rewards_accumulated.saturating_add(reward);
            worker.epoch_tokens = worker.epoch_tokens.saturating_add(receipt.tokens_computed);
            local_workers.insert(receipt.worker_id.clone(), worker);
        }

        // Apply worker updates
        for (w_id, worker) in local_workers {
            let key = Self::worker_key(&w_id);
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

    #[test]
    fn test_compute_receipt_signature_verification() {
        use ed25519_dalek::SigningKey;

        let signing_key = SigningKey::from_bytes(&[42u8; 32]);
        let mut receipt = ComputeReceipt::new(
            "task_crypto_1",
            "alice",
            "gpu_worker",
            "coord_hq",
            "model_70b",
            "zk_proof_hash",
            500,
            100,
            90,
        );

        // Before signing, signature is empty and must be rejected.
        assert!(receipt.verify_signature().is_err());

        // Sign with key
        receipt.sign_with(&signing_key);
        assert!(!receipt.signature.is_empty());
        assert!(!receipt.coordinator_pubkey.is_empty());

        // Valid signature verifies ok
        assert!(receipt.verify_signature().is_ok());

        // Tampering with payload fails verification
        receipt.tokens_computed = 9999;
        assert!(receipt.verify_signature().is_err());
    }

    #[test]
    fn test_apply_transaction_rejects_unsigned() {
        let tx = Transaction::new("alice_addr", 0, "bob", 100, 5);
        let sender_acc = AccountState {
            balance: 1_000,
            nonce: 0,
        };
        assert!(MirasetApp::apply_transaction_to_sender(&tx, &sender_acc).is_err());
    }

    #[test]
    fn test_apply_transaction_rejects_wrong_nonce() {
        use ed25519_dalek::SigningKey;

        let key = SigningKey::from_bytes(&[21u8; 32]);
        let addr = hex::encode(key.verifying_key().to_bytes());
        let mut tx = Transaction::new(&addr, 5, "bob", 100, 5); // account is at nonce 0
        tx.sign_with(&key);

        let sender_acc = AccountState {
            balance: 1_000,
            nonce: 0,
        };
        assert!(MirasetApp::apply_transaction_to_sender(&tx, &sender_acc).is_err());
    }

    #[test]
    fn test_apply_transaction_rejects_replay() {
        use ed25519_dalek::SigningKey;

        let key = SigningKey::from_bytes(&[23u8; 32]);
        let addr = hex::encode(key.verifying_key().to_bytes());
        let mut tx = Transaction::new(&addr, 0, "bob", 100, 5);
        tx.sign_with(&key);

        let sender_acc = AccountState {
            balance: 1_000,
            nonce: 0,
        };
        let updated = MirasetApp::apply_transaction_to_sender(&tx, &sender_acc)
            .expect("first application should succeed");
        assert_eq!(updated.nonce, 1);

        // Resubmitting the exact same (already-applied) transaction must fail
        // against the post-application account, since its nonce is now stale.
        assert!(MirasetApp::apply_transaction_to_sender(&tx, &updated).is_err());
    }

    #[test]
    fn test_apply_transaction_accepts_valid_signed_tx() {
        use ed25519_dalek::SigningKey;

        let key = SigningKey::from_bytes(&[22u8; 32]);
        let addr = hex::encode(key.verifying_key().to_bytes());
        let mut tx = Transaction::new(&addr, 0, "bob", 100, 5);
        tx.sign_with(&key);

        let sender_acc = AccountState {
            balance: 1_000,
            nonce: 0,
        };
        let updated = MirasetApp::apply_transaction_to_sender(&tx, &sender_acc)
            .expect("valid signed tx should apply");
        assert_eq!(updated.balance, 895);
        assert_eq!(updated.nonce, 1);
    }
}
