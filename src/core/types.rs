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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComputeReceipt {
    pub task_id: String,
    pub worker_id: String,
    pub model_hash: String,
    pub proof_hash: String,
    pub gas_reward: u64,
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
        let receipt = ComputeReceipt {
            task_id: "task_101".to_string(),
            worker_id: "worker_gpu_a".to_string(),
            model_hash: "hash_llama_3".to_string(),
            proof_hash: "proof_abc123".to_string(),
            gas_reward: 50,
        };

        let payload = BlockPayload::new(vec![tx], vec![receipt]);
        let bytes = payload.to_bytes();
        let decoded = BlockPayload::from_bytes(&bytes).expect("decoding failed");

        assert_eq!(payload, decoded);
    }
}
