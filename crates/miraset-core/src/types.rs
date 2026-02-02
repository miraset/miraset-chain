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

/// Transaction types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Transaction {
    Transfer {
        from: Address,
        to: Address,
        amount: u64,
        nonce: u64,
        #[serde(with = "signature_serde")]
        signature: [u8; 64],
    },
    ChatSend {
        from: Address,
        message: String,
        nonce: u64,
        #[serde(with = "signature_serde")]
        signature: [u8; 64],
    },
    WorkerRegister {
        from: Address,
        gpu_model: String,
        vram_gib: u32,
        nonce: u64,
        #[serde(with = "signature_serde")]
        signature: [u8; 64],
    },
}

impl Transaction {
    pub fn from(&self) -> &Address {
        match self {
            Self::Transfer { from, .. } => from,
            Self::ChatSend { from, .. } => from,
            Self::WorkerRegister { from, .. } => from,
        }
    }

    pub fn nonce(&self) -> u64 {
        match self {
            Self::Transfer { nonce, .. } => *nonce,
            Self::ChatSend { nonce, .. } => *nonce,
            Self::WorkerRegister { nonce, .. } => *nonce,
        }
    }

    pub fn signature(&self) -> &[u8; 64] {
        match self {
            Self::Transfer { signature, .. } => signature,
            Self::ChatSend { signature, .. } => signature,
            Self::WorkerRegister { signature, .. } => signature,
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
    WorkerRegistered {
        address: Address,
        gpu_model: String,
        vram_gib: u32,
        tx_hash: [u8; 32],
        block_height: u64,
    },
}
