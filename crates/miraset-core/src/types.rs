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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::KeyPair;

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
    fn test_transaction_signature() {
        let kp = KeyPair::generate();
        let sig = [1u8; 64];
        let tx = Transaction::Transfer {
            from: kp.address(),
            to: kp.address(),
            amount: 100,
            nonce: 0,
            signature: sig,
        };

        assert_eq!(*tx.signature(), sig);
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

        // Same transaction should have same hash
        assert_eq!(tx1.hash(), tx2.hash());

        // Different nonce should produce different hash
        let tx3 = Transaction::Transfer {
            from: kp.address(),
            to: kp.address(),
            amount: 100,
            nonce: 1,
            signature: [0; 64],
        };

        assert_ne!(tx1.hash(), tx3.hash());
    }

    #[test]
    fn test_chat_transaction() {
        let kp = KeyPair::generate();
        let tx = Transaction::ChatSend {
            from: kp.address(),
            message: "Hello, world!".to_string(),
            nonce: 5,
            signature: [0; 64],
        };

        assert_eq!(tx.nonce(), 5);
        assert_eq!(*tx.from(), kp.address());
    }

    #[test]
    fn test_worker_register_transaction() {
        let kp = KeyPair::generate();
        let tx = Transaction::WorkerRegister {
            from: kp.address(),
            gpu_model: "NVIDIA RTX 4090".to_string(),
            vram_gib: 24,
            nonce: 0,
            signature: [0; 64],
        };

        assert_eq!(tx.nonce(), 0);
        assert_eq!(*tx.from(), kp.address());
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

        // Same block should have same hash
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_block_hash_differs() {
        let ts = Utc::now();
        let block1 = Block {
            height: 1,
            timestamp: ts,
            prev_hash: [0; 32],
            transactions: vec![],
            state_root: [0; 32],
        };

        let block2 = Block {
            height: 2,
            timestamp: ts,
            prev_hash: [0; 32],
            transactions: vec![],
            state_root: [0; 32],
        };

        // Different height should produce different hash
        assert_ne!(block1.hash(), block2.hash());
    }

    #[test]
    fn test_genesis_block() {
        let genesis = Block::genesis();

        assert_eq!(genesis.height, 0);
        assert_eq!(genesis.prev_hash, [0; 32]);
        assert_eq!(genesis.transactions.len(), 0);
        assert_eq!(genesis.state_root, [0; 32]);
    }

    #[test]
    fn test_transaction_serialization() {
        let kp = KeyPair::generate();
        let tx = Transaction::Transfer {
            from: kp.address(),
            to: kp.address(),
            amount: 1000,
            nonce: 5,
            signature: [42; 64],
        };

        // Should serialize to JSON and deserialize
        let json = serde_json::to_string(&tx).unwrap();
        let tx2: Transaction = serde_json::from_str(&json).unwrap();

        assert_eq!(tx.hash(), tx2.hash());
        assert_eq!(tx.nonce(), tx2.nonce());
    }

    #[test]
    fn test_block_serialization() {
        let block = Block::genesis();

        // Should serialize and deserialize
        let json = serde_json::to_string(&block).unwrap();
        let block2: Block = serde_json::from_str(&json).unwrap();

        assert_eq!(block.height, block2.height);
        assert_eq!(block.hash(), block2.hash());
    }

    #[test]
    fn test_event_serialization() {
        let kp = KeyPair::generate();
        let event = Event::Transferred {
            from: kp.address(),
            to: kp.address(),
            amount: 500,
            tx_hash: [1; 32],
            block_height: 10,
        };

        // Should serialize to JSON
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("Transferred"));

        // Should deserialize
        let event2: Event = serde_json::from_str(&json).unwrap();
        let json2 = serde_json::to_string(&event2).unwrap();
        assert_eq!(json, json2);
    }

    #[test]
    fn test_chat_event() {
        let kp = KeyPair::generate();
        let event = Event::ChatMessage {
            from: kp.address(),
            message: "Test message".to_string(),
            tx_hash: [2; 32],
            block_height: 5,
            timestamp: Utc::now(),
        };

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("ChatMessage"));
        assert!(json.contains("Test message"));
    }

    #[test]
    fn test_worker_registered_event() {
        let kp = KeyPair::generate();
        let event = Event::WorkerRegistered {
            address: kp.address(),
            gpu_model: "RTX 4090".to_string(),
            vram_gib: 24,
            tx_hash: [3; 32],
            block_height: 15,
        };

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("WorkerRegistered"));
        assert!(json.contains("RTX 4090"));
    }
}
