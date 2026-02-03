use crate::crypto::Address;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// Helper for serializing [u8; 64]
mod signature_serde {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(bytes: &[u8; 64], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&hex::encode(bytes))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<[u8; 64], D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let bytes = hex::decode(&s).map_err(serde::de::Error::custom)?;
        if bytes.len() != 64 {
            return Err(serde::de::Error::custom("signature must be 64 bytes"));
        }
        let mut arr = [0u8; 64];
        arr.copy_from_slice(&bytes);
        Ok(arr)
    }
}

// Helper for serializing Option<[u8; 64]>
mod opt_signature_serde {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(opt: &Option<[u8; 64]>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match opt {
            Some(bytes) => serializer.serialize_some(&hex::encode(bytes)),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<[u8; 64]>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let opt: Option<String> = Option::deserialize(deserializer)?;
        match opt {
            Some(s) => {
                let bytes = hex::decode(&s).map_err(serde::de::Error::custom)?;
                if bytes.len() != 64 {
                    return Err(serde::de::Error::custom("signature must be 64 bytes"));
                }
                let mut arr = [0u8; 64];
                arr.copy_from_slice(&bytes);
                Ok(Some(arr))
            }
            None => Ok(None),
        }
    }
}

/// Object ID - unique identifier for objects (like Sui)
pub type ObjectId = [u8; 32];

/// Object version for optimistic concurrency control
pub type Version = u64;

/// Generate a new unique ObjectId
pub fn new_object_id(seed: &[u8]) -> ObjectId {
    blake3::hash(seed).into()
}

/// Object data - polymorphic object types (Sui-like)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "object_type")]
pub enum ObjectData {
    /// Account object (for backward compatibility with balances)
    Account {
        balance: u64,
        nonce: u64,
    },
    /// Worker registration
    WorkerRegistration {
        worker_id: ObjectId,
        pubkey: Address,
        endpoints: Vec<String>,
        gpu_model: String,
        vram_total_gib: u32,
        supported_models: Vec<String>,
        stake_bond: u64,
        status: WorkerStatus,
    },
    /// Resource snapshot (VRAM availability)
    ResourceSnapshot {
        epoch_id: u64,
        worker_id: ObjectId,
        vram_avail_gib: u32,
        timestamp: DateTime<Utc>,
        #[serde(with = "signature_serde")]
        signature: [u8; 64],
    },
    /// Inference job
    InferenceJob {
        job_id: ObjectId,
        epoch_id: u64,
        requester: Address,
        model_id: String,
        max_tokens: u64,
        assigned_worker_id: Option<ObjectId>,
        fixed_price_per_token: u64,
        escrow_amount: u64,
        status: JobStatus,
        created_at: DateTime<Utc>,
    },
    /// Job result
    JobResult {
        job_id: ObjectId,
        worker_id: ObjectId,
        output_tokens: u64,
        receipt_hash: [u8; 32],
        #[serde(with = "signature_serde")]
        worker_signature: [u8; 64],
        #[serde(with = "opt_signature_serde")]
        coordinator_signature: Option<[u8; 64]>,
        completed_at: DateTime<Utc>,
    },
    /// Epoch batch settlement
    EpochBatch {
        epoch_id: u64,
        batch_root: [u8; 32],
        total_verified_tokens: u64,
        settled: bool,
        settlement_timestamp: Option<DateTime<Utc>>,
    },
    /// Receipt anchor (proof hash)
    ReceiptAnchor {
        job_id: ObjectId,
        epoch_id: u64,
        receipt_hash: [u8; 32],
        anchored_at: DateTime<Utc>,
    },
}

/// Worker status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum WorkerStatus {
    Active,
    Jailed,
    Offline,
}

/// Job status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum JobStatus {
    Created,
    Assigned,
    Running,
    Completed,
    Challenged,
    Finalized,
    Failed,
}

/// Object wrapper with metadata (Sui-like)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Object {
    pub id: ObjectId,
    pub version: Version,
    pub owner: Address,
    pub data: ObjectData,
}

impl Object {
    pub fn new(owner: Address, data: ObjectData) -> Self {
        let id = new_object_id(&bincode::serialize(&data).unwrap());
        Self {
            id,
            version: 0,
            owner,
            data,
        }
    }

    pub fn hash(&self) -> [u8; 32] {
        let bytes = bincode::serialize(self).unwrap();
        blake3::hash(&bytes).into()
    }
}

/// Transaction types - object-centric operations
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Transaction {
    /// Transfer native tokens between accounts
    Transfer {
        from: Address,
        to: Address,
        amount: u64,
        nonce: u64,
        #[serde(with = "signature_serde")]
        signature: [u8; 64],
    },
    /// Create a new object
    CreateObject {
        creator: Address,
        data: ObjectData,
        nonce: u64,
        #[serde(with = "signature_serde")]
        signature: [u8; 64],
    },
    /// Mutate an existing object (owner-only)
    MutateObject {
        object_id: ObjectId,
        version: Version,
        new_data: ObjectData,
        owner: Address,
        nonce: u64,
        #[serde(with = "signature_serde")]
        signature: [u8; 64],
    },
    /// Transfer object ownership
    TransferObject {
        object_id: ObjectId,
        version: Version,
        from: Address,
        to: Address,
        nonce: u64,
        #[serde(with = "signature_serde")]
        signature: [u8; 64],
    },
    /// Register worker (creates WorkerRegistration object)
    RegisterWorker {
        owner: Address,
        pubkey: Address,
        endpoints: Vec<String>,
        gpu_model: String,
        vram_total_gib: u32,
        supported_models: Vec<String>,
        stake_bond: u64,
        nonce: u64,
        #[serde(with = "signature_serde")]
        signature: [u8; 64],
    },
    /// Submit resource snapshot
    SubmitResourceSnapshot {
        worker_id: ObjectId,
        epoch_id: u64,
        vram_avail_gib: u32,
        owner: Address,
        nonce: u64,
        #[serde(with = "signature_serde")]
        signature: [u8; 64],
    },
    /// Create inference job
    CreateJob {
        requester: Address,
        model_id: String,
        max_tokens: u64,
        escrow_amount: u64,
        nonce: u64,
        #[serde(with = "signature_serde")]
        signature: [u8; 64],
    },
    /// Assign job to worker
    AssignJob {
        job_id: ObjectId,
        worker_id: ObjectId,
        assigner: Address,
        nonce: u64,
        #[serde(with = "signature_serde")]
        signature: [u8; 64],
    },
    /// Submit job result
    SubmitJobResult {
        job_id: ObjectId,
        worker_id: ObjectId,
        output_tokens: u64,
        receipt_hash: [u8; 32],
        worker: Address,
        nonce: u64,
        #[serde(with = "signature_serde")]
        signature: [u8; 64],
    },
    /// Anchor receipt hash on-chain
    AnchorReceipt {
        job_id: ObjectId,
        receipt_hash: [u8; 32],
        submitter: Address,
        nonce: u64,
        #[serde(with = "signature_serde")]
        signature: [u8; 64],
    },
    /// Challenge a job result
    ChallengeJob {
        job_id: ObjectId,
        challenger: Address,
        reason: String,
        nonce: u64,
        #[serde(with = "signature_serde")]
        signature: [u8; 64],
    },
    /// Chat message (legacy support)
    ChatSend {
        from: Address,
        message: String,
        nonce: u64,
        #[serde(with = "signature_serde")]
        signature: [u8; 64],
    },
    /// Call a Move function (Sui-like programmable transaction)
    MoveCall {
        sender: Address,
        function: MoveFunction,
        type_args: Vec<String>,
        args: Vec<Vec<u8>>,
        nonce: u64,
        #[serde(with = "signature_serde")]
        signature: [u8; 64],
    },
    /// Publish Move modules
    PublishModule {
        sender: Address,
        modules: Vec<Vec<u8>>, // Compiled Move bytecode
        nonce: u64,
        #[serde(with = "signature_serde")]
        signature: [u8; 64],
    },
}

/// Move function identifier for MoveCall transactions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoveFunction {
    pub package: Vec<u8>,  // Package address (32 bytes)
    pub module: String,    // Module name
    pub function: String,  // Function name
}

impl Transaction {
    pub fn from(&self) -> &Address {
        match self {
            Self::Transfer { from, .. } => from,
            Self::CreateObject { creator, .. } => creator,
            Self::MutateObject { owner, .. } => owner,
            Self::TransferObject { from, .. } => from,
            Self::RegisterWorker { owner, .. } => owner,
            Self::SubmitResourceSnapshot { owner, .. } => owner,
            Self::CreateJob { requester, .. } => requester,
            Self::AssignJob { assigner, .. } => assigner,
            Self::SubmitJobResult { worker, .. } => worker,
            Self::AnchorReceipt { submitter, .. } => submitter,
            Self::ChallengeJob { challenger, .. } => challenger,
            Self::ChatSend { from, .. } => from,
            Self::MoveCall { sender, .. } => sender,
            Self::PublishModule { sender, .. } => sender,
        }
    }

    pub fn nonce(&self) -> u64 {
        match self {
            Self::Transfer { nonce, .. } => *nonce,
            Self::CreateObject { nonce, .. } => *nonce,
            Self::MutateObject { nonce, .. } => *nonce,
            Self::TransferObject { nonce, .. } => *nonce,
            Self::RegisterWorker { nonce, .. } => *nonce,
            Self::SubmitResourceSnapshot { nonce, .. } => *nonce,
            Self::CreateJob { nonce, .. } => *nonce,
            Self::AssignJob { nonce, .. } => *nonce,
            Self::SubmitJobResult { nonce, .. } => *nonce,
            Self::AnchorReceipt { nonce, .. } => *nonce,
            Self::ChallengeJob { nonce, .. } => *nonce,
            Self::ChatSend { nonce, .. } => *nonce,
            Self::MoveCall { nonce, .. } => *nonce,
            Self::PublishModule { nonce, .. } => *nonce,
        }
    }

    pub fn signature(&self) -> &[u8; 64] {
        match self {
            Self::Transfer { signature, .. } => signature,
            Self::CreateObject { signature, .. } => signature,
            Self::MutateObject { signature, .. } => signature,
            Self::TransferObject { signature, .. } => signature,
            Self::RegisterWorker { signature, .. } => signature,
            Self::SubmitResourceSnapshot { signature, .. } => signature,
            Self::CreateJob { signature, .. } => signature,
            Self::AssignJob { signature, .. } => signature,
            Self::SubmitJobResult { signature, .. } => signature,
            Self::AnchorReceipt { signature, .. } => signature,
            Self::ChallengeJob { signature, .. } => signature,
            Self::ChatSend { signature, .. } => signature,
            Self::MoveCall { signature, .. } => signature,
            Self::PublishModule { signature, .. } => signature,
        }
    }

    pub fn hash(&self) -> [u8; 32] {
        let bytes = bincode::serialize(self).unwrap();
        blake3::hash(&bytes).into()
    }
}

/// Block
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Block {
    pub height: u64,
    pub timestamp: DateTime<Utc>,
    pub prev_hash: [u8; 32],
    pub transactions: Vec<Transaction>,
    pub state_root: [u8; 32],
}

impl Block {
    pub fn hash(&self) -> [u8; 32] {
        let bytes = bincode::serialize(self).unwrap();
        blake3::hash(&bytes).into()
    }

    pub fn genesis() -> Self {
        Self {
            height: 0,
            timestamp: Utc::now(),
            prev_hash: [0; 32],
            transactions: vec![],
            state_root: [0; 32],
        }
    }
}

/// Events emitted by transactions
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event_type")]
pub enum Event {
    // Legacy events
    Transferred {
        from: Address,
        to: Address,
        amount: u64,
        tx_hash: [u8; 32],
        block_height: u64,
    },
    ChatMessage {
        from: Address,
        message: String,
        tx_hash: [u8; 32],
        block_height: u64,
        timestamp: DateTime<Utc>,
    },

    // Object-centric events
    ObjectCreated {
        object_id: ObjectId,
        owner: Address,
        object_type: String,
        tx_hash: [u8; 32],
        block_height: u64,
    },
    ObjectMutated {
        object_id: ObjectId,
        version: Version,
        owner: Address,
        tx_hash: [u8; 32],
        block_height: u64,
    },
    ObjectTransferred {
        object_id: ObjectId,
        from: Address,
        to: Address,
        tx_hash: [u8; 32],
        block_height: u64,
    },

    // PoCC events
    WorkerRegistered {
        worker_id: ObjectId,
        owner: Address,
        gpu_model: String,
        vram_gib: u32,
        tx_hash: [u8; 32],
        block_height: u64,
    },
    ResourceSnapshotSubmitted {
        worker_id: ObjectId,
        epoch_id: u64,
        vram_avail_gib: u32,
        tx_hash: [u8; 32],
        block_height: u64,
    },
    JobCreated {
        job_id: ObjectId,
        requester: Address,
        model_id: String,
        max_tokens: u64,
        escrow_amount: u64,
        tx_hash: [u8; 32],
        block_height: u64,
    },
    JobAssigned {
        job_id: ObjectId,
        worker_id: ObjectId,
        tx_hash: [u8; 32],
        block_height: u64,
    },
    JobCompleted {
        job_id: ObjectId,
        worker_id: ObjectId,
        output_tokens: u64,
        tx_hash: [u8; 32],
        block_height: u64,
    },
    ReceiptAnchored {
        job_id: ObjectId,
        receipt_hash: [u8; 32],
        tx_hash: [u8; 32],
        block_height: u64,
    },
    JobChallenged {
        job_id: ObjectId,
        challenger: Address,
        reason: String,
        tx_hash: [u8; 32],
        block_height: u64,
    },
    EpochSettled {
        epoch_id: u64,
        total_verified_tokens: u64,
        total_workers: u64,
        total_jobs: u64,
        tx_hash: [u8; 32],
        block_height: u64,
    },
    RewardsDistributed {
        epoch_id: u64,
        recipient: Address,
        capacity_reward: u64,
        compute_reward: u64,
        tx_hash: [u8; 32],
        block_height: u64,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::KeyPair;

    #[test]
    fn test_object_id_generation() {
        let id1 = new_object_id(b"test1");
        let id2 = new_object_id(b"test2");
        let id3 = new_object_id(b"test1");

        assert_ne!(id1, id2);
        assert_eq!(id1, id3); // Same input = same ID
    }

    #[test]
    fn test_object_creation() {
        let kp = KeyPair::generate();
        let data = ObjectData::Account {
            balance: 1000,
            nonce: 0,
        };

        let obj = Object::new(kp.address(), data);
        assert_eq!(obj.version, 0);
        assert_eq!(obj.owner, kp.address());
    }

    #[test]
    fn test_transaction_from() {
        let kp = KeyPair::generate();
        let tx = Transaction::Transfer {
            from: kp.address(),
            to: kp.address(),
            amount: 100,
            nonce: 0,
            signature: [0; 64],
        };

        assert_eq!(*tx.from(), kp.address());
    }

    #[test]
    fn test_transaction_nonce() {
        let kp = KeyPair::generate();
        let tx = Transaction::Transfer {
            from: kp.address(),
            to: kp.address(),
            amount: 100,
            nonce: 42,
            signature: [0; 64],
        };

        assert_eq!(tx.nonce(), 42);
    }

    #[test]
    fn test_register_worker_transaction() {
        let kp = KeyPair::generate();
        let tx = Transaction::RegisterWorker {
            owner: kp.address(),
            pubkey: kp.address(),
            endpoints: vec!["http://localhost:8080".to_string()],
            gpu_model: "NVIDIA RTX 4090".to_string(),
            vram_total_gib: 24,
            supported_models: vec!["llama-3-8b".to_string()],
            stake_bond: 1000,
            nonce: 0,
            signature: [0; 64],
        };

        assert_eq!(tx.nonce(), 0);
        assert_eq!(*tx.from(), kp.address());
    }

    #[test]
    fn test_create_job_transaction() {
        let kp = KeyPair::generate();
        let tx = Transaction::CreateJob {
            requester: kp.address(),
            model_id: "llama-3-8b".to_string(),
            max_tokens: 1000,
            escrow_amount: 10000,
            nonce: 5,
            signature: [0; 64],
        };

        assert_eq!(tx.nonce(), 5);
        assert_eq!(*tx.from(), kp.address());
    }

    #[test]
    fn test_transaction_hash() {
        let kp = KeyPair::generate();
        let tx1 = Transaction::Transfer {
            from: kp.address(),
            to: kp.address(),
            amount: 100,
            nonce: 0,
            signature: [0; 64],
        };

        let tx2 = Transaction::Transfer {
            from: kp.address(),
            to: kp.address(),
            amount: 100,
            nonce: 0,
            signature: [0; 64],
        };

        assert_eq!(tx1.hash(), tx2.hash());
    }

    #[test]
    fn test_block_hash() {
        let block1 = Block {
            height: 1,
            timestamp: Utc::now(),
            prev_hash: [0; 32],
            transactions: vec![],
            state_root: [0; 32],
        };

        let hash1 = block1.hash();
        let hash2 = block1.hash();

        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_genesis_block() {
        let genesis = Block::genesis();

        assert_eq!(genesis.height, 0);
        assert_eq!(genesis.prev_hash, [0; 32]);
        assert_eq!(genesis.transactions.len(), 0);
    }

    #[test]
    fn test_worker_status() {
        let status = WorkerStatus::Active;
        let json = serde_json::to_string(&status).unwrap();
        let status2: WorkerStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(status, status2);
    }

    #[test]
    fn test_job_status() {
        let status = JobStatus::Running;
        let json = serde_json::to_string(&status).unwrap();
        let status2: JobStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(status, status2);
    }
}
