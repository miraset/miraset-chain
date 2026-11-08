use serde::{Deserialize, Serialize};

/// Transaction representing a state transfer or interaction in MIRASET L1.
///
/// `sender` is the lowercase-hex Ed25519 public key that authorizes this transaction.
/// `verify_signature` rejects any transaction whose `sender` does not match `pubkey`,
/// so account identity is the signing key itself; there is no separate address registry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transaction {
    pub sender: String,
    pub nonce: u64,
    pub recipient: String,
    pub amount: u64,
    pub fee: u64,
    pub payload: Vec<u8>,
    /// Hex-encoded Ed25519 public key of the sender. Must equal `sender`.
    #[serde(default)]
    pub pubkey: String,
    /// Hex-encoded Ed25519 signature over `canonical_sign_bytes()`.
    #[serde(default)]
    pub signature: String,
}

/// Appends a length-prefixed byte string, so adjacent variable-length fields in a
/// canonical signing payload cannot be shifted into one another (e.g. recipient
/// bytes bleeding into the following payload bytes).
fn write_length_prefixed(buf: &mut Vec<u8>, bytes: &[u8]) {
    buf.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    buf.extend_from_slice(bytes);
}

/// Minimum worker bond required to register and serve tasks (ADR-0007 / protocol/model.md: 100 MIR).
pub const MIN_WORKER_BOND: u64 = 100;

/// Base slashing penalty for worker receipt or delivery fraud (ADR-0007: 10% base).
pub const WORKER_BASE_SLASH_BPS: u64 = 1000; // 10.00%

/// Actions that a worker or governance/coordinator can execute for worker stake bonds (ADR-0007).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkerBondAction {
    /// Register or increase a worker's stake bond.
    Register {
        worker_id: String,
        bond_amount: u64,
        vram_gib: u64,
        endpoint: String,
    },
    /// Unbond stake (returns unbonded amount to worker owner account).
    Unbond { worker_id: String, amount: u64 },
    /// Slash worker bond due to verified fault or challenge (ADR-0007).
    Slash {
        worker_id: String,
        slash_amount: u64,
        reason: String,
    },
}

impl WorkerBondAction {
    pub fn to_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }
}

impl Transaction {
    pub fn new(
        sender: impl Into<String>,
        nonce: u64,
        recipient: impl Into<String>,
        amount: u64,
        fee: u64,
    ) -> Self {
        Self {
            sender: sender.into(),
            nonce,
            recipient: recipient.into(),
            amount,
            fee,
            payload: Vec::new(),
            pubkey: String::new(),
            signature: String::new(),
        }
    }

    /// Construct a worker bond transaction (register, unbond, or slash).
    pub fn new_worker_bond(
        sender: impl Into<String>,
        nonce: u64,
        fee: u64,
        action: WorkerBondAction,
    ) -> Result<Self, serde_json::Error> {
        let payload = action.to_bytes()?;
        let amount = match &action {
            WorkerBondAction::Register { bond_amount, .. } => *bond_amount,
            _ => 0,
        };
        Ok(Self {
            sender: sender.into(),
            nonce,
            recipient: "system:worker_registry".to_string(),
            amount,
            fee,
            payload,
            pubkey: String::new(),
            signature: String::new(),
        })
    }

    /// Construct a transaction that triggers epoch finalization and rollover.
    pub fn new_epoch_advance(
        sender: impl Into<String>,
        nonce: u64,
        fee: u64,
        epoch_id: u64,
    ) -> Self {
        Self {
            sender: sender.into(),
            nonce,
            recipient: "system:epoch_manager".to_string(),
            amount: 0,
            fee,
            payload: epoch_id.to_be_bytes().to_vec(),
            pubkey: String::new(),
            signature: String::new(),
        }
    }

    /// Canonical message bytes for cryptographic signature verification.
    pub fn canonical_sign_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        write_length_prefixed(&mut bytes, self.sender.as_bytes());
        bytes.extend_from_slice(&self.nonce.to_be_bytes());
        write_length_prefixed(&mut bytes, self.recipient.as_bytes());
        bytes.extend_from_slice(&self.amount.to_be_bytes());
        bytes.extend_from_slice(&self.fee.to_be_bytes());
        write_length_prefixed(&mut bytes, &self.payload);
        bytes
    }

    /// Sign this transaction using an Ed25519 signing key. Sets `sender` and `pubkey`
    /// to the signer's hex-encoded verifying key, so the signed sender always matches
    /// the key that produced the signature.
    pub fn sign_with(&mut self, signing_key: &ed25519_dalek::SigningKey) {
        use ed25519_dalek::Signer;
        self.pubkey = hex::encode(signing_key.verifying_key().to_bytes());
        self.sender = self.pubkey.clone();
        let sign_bytes = self.canonical_sign_bytes();
        let sig = signing_key.sign(&sign_bytes);
        self.signature = hex::encode(sig.to_bytes());
    }

    /// Verify the sender's signature. Unlike `ComputeReceipt::verify_signature`, a
    /// missing signature is always rejected — transactions move funds and must always
    /// be authenticated.
    pub fn verify_signature(&self) -> Result<(), &'static str> {
        use ed25519_dalek::{Signature, Verifier, VerifyingKey};

        if self.signature.is_empty() || self.pubkey.is_empty() {
            return Err("missing transaction signature");
        }

        let pubkey_bytes = hex::decode(&self.pubkey).map_err(|_| "Invalid hex in pubkey")?;
        if pubkey_bytes.len() != 32 {
            return Err("Invalid pubkey length (must be 32 bytes)");
        }
        let mut pk_arr = [0u8; 32];
        pk_arr.copy_from_slice(&pubkey_bytes);
        let verifying_key =
            VerifyingKey::from_bytes(&pk_arr).map_err(|_| "Invalid Ed25519 verifying key")?;

        if !self.sender.eq_ignore_ascii_case(&self.pubkey) {
            return Err("sender does not match signing pubkey");
        }

        let sig_bytes = hex::decode(&self.signature).map_err(|_| "Invalid hex in signature")?;
        if sig_bytes.len() != 64 {
            return Err("Invalid signature length (must be 64 bytes)");
        }
        let mut sig_arr = [0u8; 64];
        sig_arr.copy_from_slice(&sig_bytes);
        let sig = Signature::from_bytes(&sig_arr);

        let sign_bytes = self.canonical_sign_bytes();
        verifying_key
            .verify(&sign_bytes, &sig)
            .map_err(|_| "Ed25519 transaction signature verification failed")
    }
}

/// Compute receipt for MIRASET Proof-of-Correct-Compute (PoCC).
///
/// Workflow:
/// 1. GPU Worker runs as an independent client-side program on the provider node, executes LLM
///    inference via a local engine (e.g. Ollama, vLLM), and streams outputs directly to the client.
/// 2. Once the client receives all tokens/results, delivery is confirmed.
/// 3. The Coordinator co-signs the receipt with the client cost debit and worker reward.
/// 4. The Coordinator submits the receipt to MIRASET L1 (`POST /receipt/submit`).
/// 5. Chain consensus verifies client confirmation, debits the client's escrow/account balance,
///    credits the worker, and marks the task as settled (anti-replay).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComputeReceipt {
    pub task_id: String,
    pub worker_id: String,
    pub model_hash: String,
    pub proof_hash: String,
    #[serde(default)]
    pub gas_reward: u64,
    /// Client account debited for compute usage.
    #[serde(default)]
    pub client_address: String,
    /// Permissioned Coordinator ID that co-signs the work receipt.
    #[serde(default)]
    pub coordinator_id: String,
    /// Total inference tokens generated and delivered.
    #[serde(default)]
    pub tokens_computed: u64,
    /// Amount debited from the client's balance/escrow.
    #[serde(default)]
    pub client_cost: u64,
    /// Net reward credited to the worker (defaults to gas_reward if 0).
    #[serde(default)]
    pub worker_reward: u64,
    /// Indicates that the client received all generated tokens/output.
    #[serde(default)]
    pub client_confirmed: bool,
    /// Coordinator co-signature or authentication tag (hex-encoded Ed25519 signature).
    #[serde(default)]
    pub signature: String,
    /// Optional Ed25519 hex public key of the coordinator.
    #[serde(default)]
    pub coordinator_pubkey: String,
}

impl ComputeReceipt {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        task_id: impl Into<String>,
        client_address: impl Into<String>,
        worker_id: impl Into<String>,
        coordinator_id: impl Into<String>,
        model_hash: impl Into<String>,
        proof_hash: impl Into<String>,
        tokens_computed: u64,
        client_cost: u64,
        worker_reward: u64,
    ) -> Self {
        Self {
            task_id: task_id.into(),
            worker_id: worker_id.into(),
            model_hash: model_hash.into(),
            proof_hash: proof_hash.into(),
            gas_reward: worker_reward,
            client_address: client_address.into(),
            coordinator_id: coordinator_id.into(),
            tokens_computed,
            client_cost,
            worker_reward,
            client_confirmed: true,
            signature: String::new(),
            coordinator_pubkey: String::new(),
        }
    }

    /// Canonical message bytes for cryptographic signature verification.
    pub fn canonical_sign_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(self.task_id.as_bytes());
        bytes.extend_from_slice(b":");
        bytes.extend_from_slice(self.client_address.as_bytes());
        bytes.extend_from_slice(b":");
        bytes.extend_from_slice(self.worker_id.as_bytes());
        bytes.extend_from_slice(b":");
        bytes.extend_from_slice(self.model_hash.as_bytes());
        bytes.extend_from_slice(b":");
        bytes.extend_from_slice(self.proof_hash.as_bytes());
        bytes.extend_from_slice(b":");
        bytes.extend_from_slice(&self.tokens_computed.to_be_bytes());
        bytes.extend_from_slice(b":");
        bytes.extend_from_slice(&self.client_cost.to_be_bytes());
        bytes.extend_from_slice(b":");
        bytes.extend_from_slice(&self.worker_reward.to_be_bytes());
        bytes
    }

    /// Sign this receipt using an Ed25519 signing key.
    pub fn sign_with(&mut self, signing_key: &ed25519_dalek::SigningKey) {
        use ed25519_dalek::Signer;
        let sign_bytes = self.canonical_sign_bytes();
        let sig = signing_key.sign(&sign_bytes);
        self.signature = hex::encode(sig.to_bytes());
        self.coordinator_pubkey = hex::encode(signing_key.verifying_key().to_bytes());
    }

    /// Verify signature if signature and coordinator_pubkey are provided.
    pub fn verify_signature(&self) -> Result<(), &'static str> {
        use ed25519_dalek::{Signature, Verifier, VerifyingKey};

        if self.signature.is_empty() || self.coordinator_pubkey.is_empty() {
            return Err("missing coordinator signature");
        }

        let pubkey_bytes = hex::decode(&self.coordinator_pubkey)
            .map_err(|_| "Invalid hex in coordinator_pubkey")?;
        if pubkey_bytes.len() != 32 {
            return Err("Invalid coordinator_pubkey length (must be 32 bytes)");
        }
        let mut pk_arr = [0u8; 32];
        pk_arr.copy_from_slice(&pubkey_bytes);
        let verifying_key =
            VerifyingKey::from_bytes(&pk_arr).map_err(|_| "Invalid Ed25519 verifying key")?;

        let sig_bytes = hex::decode(&self.signature).map_err(|_| "Invalid hex in signature")?;
        if sig_bytes.len() != 64 {
            return Err("Invalid signature length (must be 64 bytes)");
        }
        let mut sig_arr = [0u8; 64];
        sig_arr.copy_from_slice(&sig_bytes);
        let sig = Signature::from_bytes(&sig_arr);

        let sign_bytes = self.canonical_sign_bytes();
        verifying_key
            .verify(&sign_bytes, &sig)
            .map_err(|_| "Ed25519 signature verification failed")
    }
}

/// Event emitted when a block is committed (finalized by consensus).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitEvent {
    pub block_hash: String,
    pub timestamp_secs: u64,
    pub tx_count: usize,
    pub receipt_count: usize,
}

/// Consolidated block payload containing transactions and PoCC receipts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct BlockPayload {
    pub transactions: Vec<Transaction>,
    pub compute_receipts: Vec<ComputeReceipt>,
}

impl BlockPayload {
    pub fn new(transactions: Vec<Transaction>, compute_receipts: Vec<ComputeReceipt>) -> Self {
        Self {
            transactions,
            compute_receipts,
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("serialization never fails")
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }
}

/// On-chain account state for balances and replay protection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AccountState {
    pub balance: u64,
    pub nonce: u64,
}

impl AccountState {
    pub fn to_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }
}

/// GPU Worker metadata in MIRASET decentralized compute network.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerInfo {
    pub worker_id: String,
    pub stake: u64,
    pub completed_tasks: u64,
    pub rewards_accumulated: u64,
    pub active: bool,
    /// Available VRAM in GiB for capacity scoring (PoCC).
    #[serde(default = "default_vram_gib")]
    pub vram_gib: u64,
    /// Historical uptime in basis points (e.g. 9850 = 98.50%) in [0, 10000].
    #[serde(default = "default_uptime_bps")]
    pub uptime_bps: u64,
    /// Accumulated verified output tokens in current epoch.
    #[serde(default)]
    pub epoch_tokens: u64,
    /// Owner/operator address holding the bond and receiving withdrawals.
    #[serde(default)]
    pub owner_address: String,
    /// Network endpoint or multiaddr for direct peer job dispatch (optional).
    #[serde(default)]
    pub endpoint: String,
}

impl WorkerInfo {
    pub fn to_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }
}

fn default_vram_gib() -> u64 {
    24
}

fn default_uptime_bps() -> u64 {
    10000
}

/// Epoch-level PoCC reward distribution parameters (ADR-0004 & REWARDS.md).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EpochRewardConfig {
    /// 60% compute pool split (basis points: 6000 / 10000).
    pub compute_split_bps: u64,
    /// 40% capacity pool split (basis points: 4000 / 10000).
    pub capacity_split_bps: u64,
    /// Minimum uptime in basis points to qualify for capacity rewards (e.g. 9000 = 90%).
    pub min_uptime_bps: u64,
    /// Saturation cap for VRAM contribution in GiB (e.g. 80 GiB).
    pub vram_cap_gib: u64,
}

impl Default for EpochRewardConfig {
    fn default() -> Self {
        Self {
            compute_split_bps: 6000,
            capacity_split_bps: 4000,
            min_uptime_bps: 9000,
            vram_cap_gib: 80,
        }
    }
}

/// Result of an epoch reward distribution calculation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EpochRewardDistribution {
    pub total_reward: u64,
    pub compute_pool: u64,
    pub capacity_pool: u64,
    pub worker_rewards: std::collections::BTreeMap<String, u64>,
}

impl EpochRewardConfig {
    /// Calculate PoCC rewards for a set of workers according to canonical ADR-0004 & REWARDS.md formulas:
    /// - ComputeReward_i = R_comp * T_i / Σ T_j
    /// - ScoreVal_i = (U_i / 10000)^2 * (min(V_i, V_cap) / V_cap)
    /// - CapacityReward_i = R_cap * ScoreVal_i / Σ ScoreVal_j
    pub fn distribute(&self, total_budget: u64, workers: &[WorkerInfo]) -> EpochRewardDistribution {
        let compute_pool = (total_budget as u128 * self.compute_split_bps as u128 / 10000) as u64;
        let capacity_pool = total_budget.saturating_sub(compute_pool);

        let mut worker_rewards = std::collections::BTreeMap::new();
        for w in workers {
            worker_rewards.insert(w.worker_id.clone(), 0u64);
        }

        // 1. Compute pool distribution: proportional to epoch_tokens
        let total_tokens: u128 = workers.iter().map(|w| w.epoch_tokens as u128).sum();
        if total_tokens > 0 && compute_pool > 0 {
            let mut compute_allocated = 0u64;
            let active_workers: Vec<_> = workers.iter().filter(|w| w.epoch_tokens > 0).collect();
            for (idx, w) in active_workers.iter().enumerate() {
                let share = if idx == active_workers.len() - 1 {
                    compute_pool.saturating_sub(compute_allocated)
                } else {
                    let s = (compute_pool as u128 * w.epoch_tokens as u128 / total_tokens) as u64;
                    compute_allocated = compute_allocated.saturating_add(s);
                    s
                };
                if let Some(r) = worker_rewards.get_mut(&w.worker_id) {
                    *r = r.saturating_add(share);
                }
            }
        }

        // 2. Capacity pool distribution:
        // ScoreVal = U^2 * (V / V_cap). Scaled with fixed point precision:
        // u_ratio = U_bps (max 10000)
        // v_norm = min(vram, cap) * 10000 / cap
        // raw_score = (u_ratio * u_ratio / 10000) * v_norm / 10000
        let mut capacity_scores = Vec::with_capacity(workers.len());
        let mut total_capacity_score: u128 = 0;

        for w in workers {
            if !w.active || w.uptime_bps < self.min_uptime_bps {
                capacity_scores.push(0u128);
                continue;
            }

            let u = w.uptime_bps as u128; // in [0, 10000]
            let u_sq = (u * u) / 10000; // in [0, 10000]

            let capped_vram = w.vram_gib.min(self.vram_cap_gib) as u128;
            let v_norm = (capped_vram * 10000) / self.vram_cap_gib as u128; // in [0, 10000]

            let score = (u_sq * v_norm) / 10000;
            total_capacity_score = total_capacity_score.saturating_add(score);
            capacity_scores.push(score);
        }

        if total_capacity_score > 0 && capacity_pool > 0 {
            let eligible_workers: Vec<(&WorkerInfo, u128)> = workers
                .iter()
                .zip(capacity_scores)
                .filter(|(_, score)| *score > 0)
                .collect();

            let mut capacity_allocated = 0u64;
            for (idx, (w, score)) in eligible_workers.iter().enumerate() {
                let share = if idx == eligible_workers.len() - 1 {
                    capacity_pool.saturating_sub(capacity_allocated)
                } else {
                    let s = (capacity_pool as u128 * *score / total_capacity_score) as u64;
                    capacity_allocated = capacity_allocated.saturating_add(s);
                    s
                };
                if let Some(r) = worker_rewards.get_mut(&w.worker_id) {
                    *r = r.saturating_add(share);
                }
            }
        }

        EpochRewardDistribution {
            total_reward: total_budget,
            compute_pool,
            capacity_pool,
            worker_rewards,
        }
    }
}

/// Status of an accounting epoch in MIRASET L1 (ADR-0003 & REWARDS.md).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EpochStatus {
    /// Active submission window (~40 mins): workers submit compute receipts.
    Active,
    /// Challenge window (~10-20 mins): receipts can be disputed or audited.
    Challenge,
    /// Finalized: PoCC reward distribution is computed and settled on-chain.
    Finalized,
}

/// Metadata and state for an on-chain 60-minute accounting epoch (ADR-0003).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EpochInfo {
    pub epoch_id: u64,
    pub status: EpochStatus,
    /// Unix timestamp when epoch began.
    pub start_timestamp: u64,
    /// Total compute tokens settled during this epoch.
    pub total_tokens: u64,
    /// Total compute receipts submitted in this epoch.
    pub total_receipts: u64,
    /// Total protocol reward budget allocated for this epoch.
    pub reward_budget: u64,
    /// Finalized distribution (if finalized).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distribution: Option<EpochRewardDistribution>,
}

impl EpochInfo {
    pub fn new(epoch_id: u64, start_timestamp: u64, reward_budget: u64) -> Self {
        Self {
            epoch_id,
            status: EpochStatus::Active,
            start_timestamp,
            total_tokens: 0,
            total_receipts: 0,
            reward_budget,
            distribution: None,
        }
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transaction_signature_roundtrip() {
        use ed25519_dalek::SigningKey;

        let key = SigningKey::from_bytes(&[11u8; 32]);
        let mut tx = Transaction::new("placeholder", 0, "bob", 100, 5);

        // Unsigned transactions must never verify.
        assert!(tx.verify_signature().is_err());

        tx.sign_with(&key);
        assert_eq!(tx.sender, hex::encode(key.verifying_key().to_bytes()));
        assert!(tx.verify_signature().is_ok());

        // Tampering with any signed field invalidates the signature.
        tx.amount = 999;
        assert!(tx.verify_signature().is_err());
    }

    #[test]
    fn test_transaction_rejects_sender_pubkey_mismatch() {
        use ed25519_dalek::SigningKey;

        let signer = SigningKey::from_bytes(&[12u8; 32]);
        let mut tx = Transaction::new("not-the-signer", 0, "bob", 100, 5);
        tx.sign_with(&signer);
        // Force a mismatch between the claimed sender and the actual signing key.
        tx.sender = "0".repeat(64);
        assert!(tx.verify_signature().is_err());
    }

    #[test]
    fn test_block_payload_serde_roundtrip() {
        let tx = Transaction::new("alice", 1, "bob", 500, 10);
        let receipt = ComputeReceipt::new(
            "task_101",
            "client_bob",
            "worker_gpu_a",
            "coordinator_main",
            "hash_llama_3",
            "proof_abc123",
            100,
            25,
            20,
        );

        let payload = BlockPayload::new(vec![tx], vec![receipt]);
        let bytes = payload.to_bytes();
        let decoded = BlockPayload::from_bytes(&bytes).expect("decoding failed");

        assert_eq!(payload, decoded);
    }

    #[test]
    fn test_epoch_reward_distribution_60_40() {
        let config = EpochRewardConfig::default(); // 60% compute, 40% capacity

        let w1 = WorkerInfo {
            worker_id: "worker_a".to_string(),
            stake: 1000,
            completed_tasks: 10,
            rewards_accumulated: 0,
            active: true,
            vram_gib: 40,
            uptime_bps: 9900, // 99%
            epoch_tokens: 120_000,
            owner_address: "owner_a".to_string(),
            endpoint: "10.0.0.1:8000".to_string(),
        };

        let w2 = WorkerInfo {
            worker_id: "worker_b".to_string(),
            stake: 2000,
            completed_tasks: 8,
            rewards_accumulated: 0,
            active: true,
            vram_gib: 80,
            uptime_bps: 9500, // 95%
            epoch_tokens: 80_000,
            owner_address: "owner_b".to_string(),
            endpoint: "10.0.0.2:8000".to_string(),
        };

        let total_budget = 3000; // 1800 compute, 1200 capacity
        let dist = config.distribute(total_budget, &[w1, w2]);

        assert_eq!(dist.compute_pool, 1800);
        assert_eq!(dist.capacity_pool, 1200);

        // Compute share: total 200k. w1 has 120k (60%), w2 has 80k (40%).
        // w1 compute = 1800 * 60% = 1080.
        // w2 compute = 1800 * 40% = 720.
        let reward_a = dist.worker_rewards.get("worker_a").copied().unwrap();
        let reward_b = dist.worker_rewards.get("worker_b").copied().unwrap();

        assert!(reward_a > 1080, "worker_a should get compute + capacity");
        assert!(reward_b > 720, "worker_b should get compute + capacity");
        assert_eq!(reward_a + reward_b, total_budget);
    }

    #[test]
    fn test_worker_bond_action_serde() {
        let action = WorkerBondAction::Register {
            worker_id: "worker_1".to_string(),
            bond_amount: 500,
            vram_gib: 48,
            endpoint: "127.0.0.1:9000".to_string(),
        };

        let bytes = action.to_bytes().unwrap();
        let decoded = WorkerBondAction::from_bytes(&bytes).unwrap();
        assert_eq!(action, decoded);

        let unbond = WorkerBondAction::Unbond {
            worker_id: "worker_1".to_string(),
            amount: 200,
        };
        let u_bytes = unbond.to_bytes().unwrap();
        let u_decoded = WorkerBondAction::from_bytes(&u_bytes).unwrap();
        assert_eq!(unbond, u_decoded);
    }
}
