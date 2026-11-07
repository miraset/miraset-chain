use serde::{Deserialize, Serialize};

/// Transaction representing a state transfer or interaction in MIRASET L1.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transaction {
    pub sender: String,
    pub nonce: u64,
    pub recipient: String,
    pub amount: u64,
    pub fee: u64,
    pub payload: Vec<u8>,
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
        }
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

        if self.signature.is_empty() && self.coordinator_pubkey.is_empty() {
            // Optional signature in test/dev mode if not configured
            return Ok(());
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
}

impl WorkerInfo {
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
}
