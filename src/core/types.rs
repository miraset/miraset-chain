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
    /// Coordinator co-signature or authentication tag.
    #[serde(default)]
    pub signature: String,
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
        }
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
