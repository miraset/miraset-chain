use crate::core::types::{
    AccountState, BlockPayload, ComputeReceipt, EpochInfo, ProtocolParams, Transaction, WorkerInfo,
};
use crate::mempool::{Mempool, ReceiptVerdict, TxVerdict};
use hotstuff_rs::app::{
    App, ProduceBlockRequest, ProduceBlockResponse, ValidateBlockRequest, ValidateBlockResponse,
};
use hotstuff_rs::block_tree::pluggables::KVStore;
use hotstuff_rs::types::data_types::{CryptoHash, Data, Datum};
use hotstuff_rs::types::update_sets::AppStateUpdates;

mod execution;
mod heartbeat;
use execution::BlockExecution;

/// Most transactions and receipts a produced block carries.
pub const MAX_BLOCK_TXS: usize = 100;
pub const MAX_BLOCK_RECEIPTS: usize = 20;
/// How far a proposed block's timestamp may run ahead of a validator's clock. Validators need
/// clocks synchronised well within this (NTP); a larger skew makes a validator refuse honest
/// proposals.
pub const MAX_FUTURE_DRIFT_MS: u64 = 10_000;
/// How far a proposed block's timestamp may lag a validator's clock. Bounds how long a leader
/// can hold chain time back (stamping parent + 1) and makes the first block carry real time,
/// so epoch timing follows the wall clock. Larger than the future bound to absorb proposal
/// delays and view timeouts.
pub const MAX_PAST_DRIFT_MS: u64 = 60_000;

/// Whether a proposal's timestamp is acceptable against this validator's clock (live proposals
/// only; block sync accepts history). Monotonicity against the parent is checked by execution.
pub fn timestamp_near_clock(timestamp_ms: u64, clock_ms: u64) -> bool {
    timestamp_ms <= clock_ms.saturating_add(MAX_FUTURE_DRIFT_MS)
        && timestamp_ms >= clock_ms.saturating_sub(MAX_PAST_DRIFT_MS)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// Candidates that fail execution before block production stops trying more of a kind.
const MAX_FAILED_CANDIDATES: usize = 256;

/// Application state machine implementing `hotstuff_rs::app::App`.
pub struct MirasetApp {
    mempool: Mempool,
    empty_block_interval: std::time::Duration,
    min_block_interval: std::time::Duration,
}

impl MirasetApp {
    pub fn new(mempool: Mempool) -> Self {
        Self {
            mempool,
            empty_block_interval: std::time::Duration::ZERO,
            min_block_interval: std::time::Duration::ZERO,
        }
    }

    /// Propose an empty block at most once per `interval` of block time (zero: every view).
    /// See `chain_app/heartbeat.rs`. The leader waits inside `produce_block` for up to
    /// `interval`, so callers must keep it well below the view time; the node passes
    /// `NodeConfig::effective_empty_block_interval` (at most half of `max_view_time`).
    pub fn with_empty_block_interval(mut self, interval: std::time::Duration) -> Self {
        self.empty_block_interval = interval;
        self
    }

    /// Propose any block (empty or not) at most once per `interval` of block time; zero turns
    /// the limit off. Same view-time bound as [`Self::with_empty_block_interval`].
    pub fn with_min_block_interval(mut self, interval: std::time::Duration) -> Self {
        self.min_block_interval = interval;
        self
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

    pub fn worker_index_key(worker_id: &str) -> Vec<u8> {
        let mut key = b"workers:idx:".to_vec();
        key.extend_from_slice(worker_id.as_bytes());
        key
    }

    pub fn read_all_registered_workers<K: KVStore>(
        tree: &hotstuff_rs::block_tree::accessors::app::AppBlockTreeView<'_, K>,
    ) -> Vec<WorkerInfo> {
        let key = b"workers:all_ids".to_vec();
        let ids: Vec<String> = tree
            .app_state(&key)
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        let mut workers = Vec::new();
        for id in ids {
            if let Some(w) = Self::read_worker(tree, &id) {
                workers.push(w);
            }
        }
        workers
    }

    fn add_worker_id(worker_ids: &mut Vec<String>, worker_id: &str) -> bool {
        if worker_ids.iter().any(|id| id == worker_id) {
            return false;
        }

        worker_ids.push(worker_id.to_string());
        true
    }

    /// Timestamp (Unix ms) of the latest block on this branch (TODO 8.5.1).
    pub fn chain_time_key() -> Vec<u8> {
        b"chain:time_ms".to_vec()
    }

    pub fn read_chain_time_ms<K: KVStore>(
        tree: &hotstuff_rs::block_tree::accessors::app::AppBlockTreeView<'_, K>,
    ) -> u64 {
        tree.app_state(&Self::chain_time_key())
            .and_then(|bytes| bytes.try_into().ok().map(u64::from_be_bytes))
            .unwrap_or(0)
    }

    /// Remaining amount `coordinator` may charge `client` (TODO 8.5.1).
    pub fn allowance_key(client: &str, coordinator: &str) -> Vec<u8> {
        format!("allowance:{client}:{}", coordinator.to_ascii_lowercase()).into_bytes()
    }

    pub fn read_allowance<K: KVStore>(
        tree: &hotstuff_rs::block_tree::accessors::app::AppBlockTreeView<'_, K>,
        client: &str,
        coordinator: &str,
    ) -> u64 {
        tree.app_state(&Self::allowance_key(client, coordinator))
            .and_then(|bytes| bytes.try_into().ok().map(u64::from_be_bytes))
            .unwrap_or(0)
    }

    /// Genesis protocol parameters (TODO 8.5.1).
    pub fn protocol_params_key() -> Vec<u8> {
        b"params:protocol".to_vec()
    }

    /// Protocol parameters from state; defaults (no coordinators, no governance) if absent.
    pub fn read_protocol_params<K: KVStore>(
        tree: &hotstuff_rs::block_tree::accessors::app::AppBlockTreeView<'_, K>,
    ) -> ProtocolParams {
        tree.app_state(&Self::protocol_params_key())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn current_epoch_key() -> Vec<u8> {
        b"epoch:current".to_vec()
    }

    pub fn epoch_key(epoch_id: u64) -> Vec<u8> {
        let mut key = b"epoch:info:".to_vec();
        key.extend_from_slice(&epoch_id.to_be_bytes());
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

    /// Read an epoch state from block tree view.
    pub fn read_epoch<K: KVStore>(
        tree: &hotstuff_rs::block_tree::accessors::app::AppBlockTreeView<'_, K>,
        epoch_id: u64,
    ) -> Option<EpochInfo> {
        let key = Self::epoch_key(epoch_id);
        tree.app_state(&key)
            .and_then(|bytes| serde_json::from_slice::<EpochInfo>(&bytes).ok())
    }

    /// Read the currently active epoch from block tree view.
    pub fn read_current_epoch<K: KVStore>(
        tree: &hotstuff_rs::block_tree::accessors::app::AppBlockTreeView<'_, K>,
    ) -> EpochInfo {
        let key = Self::current_epoch_key();
        tree.app_state(&key)
            .and_then(|bytes| serde_json::from_slice::<EpochInfo>(&bytes).ok())
            .unwrap_or_else(|| EpochInfo::new(0, 0, 100_000))
    }

    /// Pure state transition for one transaction against its sender's current account:
    /// verifies the sender's signature, enforces strict nonce sequencing (rejects replays
    /// and out-of-order submissions), and checks funds using checked arithmetic.
    /// Returns the updated sender account or a rejection reason; never mutates `sender_acc` in place.
    fn apply_transaction_to_sender(
        tx: &Transaction,
        sender_acc: &AccountState,
    ) -> Result<AccountState, &'static str> {
        tx.verify_signature()?;
        // One key, one account: the sender address is the key in lowercase hex (TODO 8.5.1;
        // signature checks accept either case).
        if tx.sender.bytes().any(|b| b.is_ascii_uppercase()) {
            return Err("sender must be lowercase hex");
        }

        if tx.nonce != sender_acc.nonce {
            return Err("transaction nonce does not match expected account nonce");
        }

        let required = tx
            .amount
            .checked_add(tx.fee)
            .ok_or("transaction amount + fee overflow")?;
        let next_balance = sender_acc
            .balance
            .checked_sub(required)
            .ok_or("insufficient funds")?;
        let next_nonce = sender_acc
            .nonce
            .checked_add(1)
            .ok_or("account nonce overflow")?;

        Ok(AccountState {
            balance: next_balance,
            nonce: next_nonce,
        })
    }

    pub fn state_commitment_key() -> Vec<u8> {
        b"state:commitment_root".to_vec()
    }

    /// Process payload against block tree view to compute deterministic state updates
    /// and the resulting domain-separated state commitment root (PRD R04). Any invalid
    /// transaction or receipt rejects the whole payload.
    fn compute_state_updates_with_commitment<K: KVStore>(
        tree: &hotstuff_rs::block_tree::accessors::app::AppBlockTreeView<'_, K>,
        payload: &BlockPayload,
    ) -> Option<(AppStateUpdates, [u8; 32])> {
        let mut execution = BlockExecution::new(tree);
        execution.set_block_time(tree, payload.timestamp_ms)?;
        for tx in &payload.transactions {
            execution.apply_transaction(tree, tx)?;
        }
        for receipt in &payload.compute_receipts {
            execution.apply_receipt(tree, receipt)?;
        }
        execution.finish()
    }
}

/// Keep `item` if it applies on top of `execution`; otherwise leave `execution` unchanged.
/// One application per candidate (PRD R04, TODO 8.5.3 finding b).
fn try_apply(
    execution: &mut BlockExecution,
    apply: impl FnOnce(&mut BlockExecution) -> Option<()>,
) -> bool {
    let mut trial = execution.clone();
    if apply(&mut trial).is_some() {
        *execution = trial;
        true
    } else {
        false
    }
}

impl<K: KVStore> App<K> for MirasetApp {
    fn produce_block(&mut self, request: ProduceBlockRequest<'_, K>) -> ProduceBlockResponse {
        // Greedy selection: a candidate is kept if it applies on top of the ones kept before it
        // (transactions first, then receipts, as execution orders them). The pool offers sender
        // chains by head fee, each in nonce order; nonce mismatches are decided without
        // executing. A candidate that fails execution is evicted from the pool (with its
        // sender's later nonces), so it cannot fail again in the next block; the failures per
        // block are bounded, and each block makes progress through them. Eviction is local to
        // this node's pool: a transaction that only failed on this fork's state is dropped and
        // must be resubmitted.
        heartbeat::wait_for_min_block_interval(&request, self.min_block_interval);
        heartbeat::wait_for_work_or_heartbeat(&request, &self.mempool, self.empty_block_interval);
        let tree = request.block_tree();
        // Block time: the local clock, but always after the parent block's (TODO 8.5.1).
        let timestamp_ms = now_ms().max(Self::read_chain_time_ms(tree).saturating_add(1));
        let mut execution = BlockExecution::new(tree);
        execution
            .set_block_time(tree, timestamp_ms)
            .expect("a timestamp after the parent's is always accepted");
        let mut failures = 0usize;
        let selected_txs: Vec<Transaction> = self.mempool.select_txs(MAX_BLOCK_TXS, |tx| {
            let expected = execution.account_nonce(tree, &tx.sender);
            if tx.nonce < expected {
                return TxVerdict::Skip;
            }
            if tx.nonce > expected {
                return TxVerdict::DropSender;
            }
            if try_apply(&mut execution, |e| e.apply_transaction(tree, tx)) {
                TxVerdict::Included
            } else {
                failures += 1;
                if failures >= MAX_FAILED_CANDIDATES {
                    TxVerdict::Stop
                } else {
                    TxVerdict::Evict
                }
            }
        });
        failures = 0;
        let selected_receipts: Vec<ComputeReceipt> =
            self.mempool.select_receipts(MAX_BLOCK_RECEIPTS, |receipt| {
                if try_apply(&mut execution, |e| e.apply_receipt(tree, receipt)) {
                    ReceiptVerdict::Included
                } else {
                    failures += 1;
                    if failures >= MAX_FAILED_CANDIDATES {
                        ReceiptVerdict::Stop
                    } else {
                        ReceiptVerdict::Evict
                    }
                }
            });

        // `finish` is the same code validators run after applying the payload, so the state
        // root and updates match what `validate_block` will compute.
        let mut payload =
            BlockPayload::new(selected_txs, selected_receipts).with_timestamp_ms(timestamp_ms);
        let app_state_updates = match execution.finish() {
            Some((updates, state_root)) => {
                payload.state_root = state_root;
                Some(updates)
            }
            None => {
                payload = BlockPayload::default().with_timestamp_ms(timestamp_ms);
                let (updates, state_root) =
                    Self::compute_state_updates_with_commitment(tree, &payload)
                        .expect("empty block state commitment must succeed");
                payload.state_root = state_root;
                Some(updates)
            }
        };

        let payload_bytes = payload.to_bytes();
        let hash_arr = payload
            .commitment_hash()
            .expect("canonical block commitment hash failed");
        let data_hash = CryptoHash::new(hash_arr);
        let data = Data::new(vec![Datum::new(payload_bytes)]);

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
        Self::validate(request, Some(now_ms()))
    }

    /// Blocks fetched by block sync are history: their time is checked against the parent's,
    /// not against this node's clock.
    fn validate_block_for_sync(
        &mut self,
        request: ValidateBlockRequest<'_, '_, K>,
    ) -> ValidateBlockResponse {
        Self::validate(request, None)
    }
}

impl MirasetApp {
    /// Validate a proposed block; with `clock_ms`, also refuse a timestamp more than
    /// `MAX_FUTURE_DRIFT_MS` ahead of that clock (a proposer cannot pull epoch closes forward).
    fn validate<K: KVStore>(
        request: ValidateBlockRequest<'_, '_, K>,
        clock_ms: Option<u64>,
    ) -> ValidateBlockResponse {
        let block = request.proposed_block();

        // Extract payload from data (must contain exactly one canonical BlockPayload datum)
        if block.data.vec().len() != 1 {
            return ValidateBlockResponse::Invalid;
        }

        let datum = &block.data.vec()[0];
        let payload = match BlockPayload::from_bytes(datum.bytes()) {
            Ok(p) => p,
            Err(_) => return ValidateBlockResponse::Invalid,
        };

        // Reject any noncanonical representation
        let Ok(canonical_bytes) = payload.to_canonical_bytes() else {
            return ValidateBlockResponse::Invalid;
        };
        if canonical_bytes.as_slice() != datum.bytes() {
            return ValidateBlockResponse::Invalid;
        }

        // Verify domain-separated commitment hash integrity
        let Ok(expected_hash) = payload.commitment_hash() else {
            return ValidateBlockResponse::Invalid;
        };
        if block.data_hash.bytes() != expected_hash {
            return ValidateBlockResponse::Invalid;
        }

        // Validate state transitions and deterministic state commitment root
        if let Some(now) = clock_ms
            && !timestamp_near_clock(payload.timestamp_ms, now)
        {
            tracing::warn!(
                block_timestamp_ms = payload.timestamp_ms,
                local_clock_ms = now,
                "refusing proposal: block timestamp too far from this validator's clock \
                 (check NTP)"
            );
            return ValidateBlockResponse::Invalid;
        }

        match Self::compute_state_updates_with_commitment(request.block_tree(), &payload) {
            Some((updates, computed_state_root)) if computed_state_root == payload.state_root => {
                ValidateBlockResponse::Valid {
                    app_state_updates: Some(updates),
                    validator_set_updates: None,
                }
            }
            _ => ValidateBlockResponse::Invalid,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::EpochRewardConfig;
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
    fn test_worker_index_accumulates_multiple_registrations() {
        let mut worker_ids = Vec::new();

        assert!(MirasetApp::add_worker_id(&mut worker_ids, "worker_a"));
        assert!(MirasetApp::add_worker_id(&mut worker_ids, "worker_b"));
        assert!(!MirasetApp::add_worker_id(&mut worker_ids, "worker_a"));

        assert_eq!(worker_ids, ["worker_a", "worker_b"]);
    }

    #[test]
    fn test_app_creation_with_mempool() {
        let mempool = Mempool::new();
        mempool
            .insert_tx(Transaction::new("alice", 1, "bob", 100, 5))
            .unwrap();
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
        mempool.insert_receipt(receipt).unwrap();

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
            "receipt_anchor",
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

    #[test]
    fn test_apply_transaction_rejects_arithmetic_overflow() {
        use ed25519_dalek::SigningKey;

        let key = SigningKey::from_bytes(&[24u8; 32]);
        let addr = hex::encode(key.verifying_key().to_bytes());
        // amount + fee overflows u64::MAX
        let mut tx = Transaction::new(&addr, 0, "bob", u64::MAX, 10);
        tx.sign_with(&key);

        let sender_acc = AccountState {
            balance: u64::MAX,
            nonce: 0,
        };
        assert!(MirasetApp::apply_transaction_to_sender(&tx, &sender_acc).is_err());
        assert_eq!(sender_acc.balance, u64::MAX);
        assert_eq!(sender_acc.nonce, 0);
    }

    #[test]
    fn test_reward_distribution_is_deterministic_across_worker_order() {
        let cfg = EpochRewardConfig::default();
        let w1 = WorkerInfo {
            worker_id: "worker_alpha".to_string(),
            stake: 500,
            completed_tasks: 1,
            rewards_accumulated: 0,
            active: true,
            vram_gib: 48,
            uptime_bps: 9800,
            epoch_tokens: 333,
            owner_address: "o1".to_string(),
            endpoint: String::new(),
            unbonding: 0,
        };
        let w2 = WorkerInfo {
            worker_id: "worker_beta".to_string(),
            stake: 500,
            completed_tasks: 1,
            rewards_accumulated: 0,
            active: true,
            vram_gib: 80,
            uptime_bps: 9600,
            epoch_tokens: 333,
            owner_address: "o2".to_string(),
            endpoint: String::new(),
            unbonding: 0,
        };
        let w3 = WorkerInfo {
            worker_id: "worker_gamma".to_string(),
            stake: 500,
            completed_tasks: 1,
            rewards_accumulated: 0,
            active: true,
            vram_gib: 24,
            uptime_bps: 9900,
            epoch_tokens: 333,
            owner_address: "o3".to_string(),
            endpoint: String::new(),
            unbonding: 0,
        };

        let d1 = cfg
            .try_distribute(10_000, &[w1.clone(), w2.clone(), w3.clone()])
            .unwrap();
        let d2 = cfg.try_distribute(10_000, &[w3, w1, w2]).unwrap();
        assert_eq!(d1, d2);
    }

    #[test]
    fn test_live_proposals_must_be_stamped_near_the_validators_clock() {
        let now = 1_800_000_000_000;
        assert!(timestamp_near_clock(now, now));
        assert!(timestamp_near_clock(now + MAX_FUTURE_DRIFT_MS, now));
        assert!(!timestamp_near_clock(now + MAX_FUTURE_DRIFT_MS + 1, now));
        assert!(timestamp_near_clock(now - MAX_PAST_DRIFT_MS, now));
        assert!(!timestamp_near_clock(now - MAX_PAST_DRIFT_MS - 1, now));
        // A first block stamped near zero (to start epoch 0 in 1970) is refused.
        assert!(!timestamp_near_clock(1, now));
    }
}
