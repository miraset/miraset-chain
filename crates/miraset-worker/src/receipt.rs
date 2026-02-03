/// Canonical Receipt Payload with deterministic hashing
///
/// This implements the receipt system described in ARCHITECTURE.md:
/// - Structured receipt payload
/// - Canonical serialization (BCS-like)
/// - Deterministic hashing
/// - On-chain anchor as hash

use anyhow::Result;
use chrono::{DateTime, Utc};
use miraset_core::{Address, ObjectId};
use serde::{Deserialize, Serialize};


/// Receipt hash (32 bytes)
pub type ReceiptHash = [u8; 32];

/// Receipt payload structure (matches ARCHITECTURE.md spec)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceiptPayload {
    /// Job identifier
    pub job_id: ObjectId,

    /// Epoch when job was executed
    pub epoch_id: u64,

    /// Worker public key
    pub worker_pubkey: Address,

    /// Model identifier
    pub model_id: String,

    /// Hash of request (prompt + parameters)
    pub request_hash: [u8; 32],

    /// Hash of response stream
    pub response_stream_hash: [u8; 32],

    /// Number of output tokens
    pub output_tokens: u64,

    /// Price per token (must match epoch parameter)
    pub price_per_token: u64,

    /// Execution start time
    pub timestamp_start: DateTime<Utc>,

    /// Execution end time
    pub timestamp_end: DateTime<Utc>,

    /// Worker signature (over canonical encoding)
    #[serde(with = "signature_serde")]
    pub worker_signature: [u8; 64],

    /// Optional coordinator co-signature
    #[serde(skip_serializing_if = "Option::is_none", with = "option_signature_serde", default)]
    pub coordinator_signature: Option<[u8; 64]>,
}

// Signature serialization helper
mod signature_serde {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(bytes: &[u8; 64], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_bytes(bytes)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<[u8; 64], D::Error>
    where
        D: Deserializer<'de>,
    {
        let bytes: Vec<u8> = Vec::deserialize(deserializer)?;
        if bytes.len() != 64 {
            return Err(serde::de::Error::custom("signature must be 64 bytes"));
        }
        let mut arr = [0u8; 64];
        arr.copy_from_slice(&bytes);
        Ok(arr)
    }
}

// Option signature serialization helper
mod option_signature_serde {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(bytes: &Option<[u8; 64]>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match bytes {
            Some(b) => serializer.serialize_bytes(b),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<[u8; 64]>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let bytes_opt: Option<Vec<u8>> = Option::deserialize(deserializer)?;
        match bytes_opt {
            Some(bytes) => {
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

impl ReceiptPayload {
    /// Create new receipt payload
    pub fn new(
        job_id: ObjectId,
        epoch_id: u64,
        worker_pubkey: Address,
        model_id: String,
        prompt: String,
        response: Vec<String>,
        output_tokens: u64,
        timestamp_start: DateTime<Utc>,
        timestamp_end: DateTime<Utc>,
    ) -> Result<Self> {
        // Compute request hash
        let request_hash = Self::hash_request(&prompt)?;

        // Compute response stream hash
        let response_stream_hash = Self::hash_response_stream(&response)?;

        // Placeholder signature (will be set by worker)
        let worker_signature = [0u8; 64];

        Ok(Self {
            job_id,
            epoch_id,
            worker_pubkey,
            model_id,
            request_hash,
            response_stream_hash,
            output_tokens,
            price_per_token: 10, // TODO: Get from epoch config
            timestamp_start,
            timestamp_end,
            worker_signature,
            coordinator_signature: None,
        })
    }

    /// Compute canonical hash of receipt payload
    ///
    /// This hash is anchored on-chain and used for verification.
    /// The full payload is stored off-chain.
    pub fn compute_hash(&self) -> Result<ReceiptHash> {
        // Use bincode for canonical serialization
        let encoded = bincode::serialize(self)?;

        // Hash with blake3 (consistent with chain-wide choice)
        let hash: [u8; 32] = blake3::hash(&encoded).into();

        Ok(hash)
    }

    /// Hash the request (prompt + parameters)
    fn hash_request(prompt: &str) -> Result<[u8; 32]> {
        let hash: [u8; 32] = blake3::hash(prompt.as_bytes()).into();
        Ok(hash)
    }

    /// Hash the response stream
    ///
    /// This must be deterministic and reproducible.
    /// For MVP: concatenate all tokens and hash.
    fn hash_response_stream(tokens: &[String]) -> Result<[u8; 32]> {
        let combined = tokens.join("");
        let hash: [u8; 32] = blake3::hash(combined.as_bytes()).into();
        Ok(hash)
    }

    /// Verify receipt integrity
    pub fn verify(&self, expected_hash: &ReceiptHash) -> Result<bool> {
        let computed_hash = self.compute_hash()?;
        Ok(&computed_hash == expected_hash)
    }

    /// Sign the receipt (worker signature)
    pub fn sign(&mut self, keypair: &miraset_core::KeyPair) -> Result<()> {
        // Create signing payload (everything except signature)
        let mut unsigned = self.clone();
        unsigned.worker_signature = [0u8; 64];
        unsigned.coordinator_signature = None;

        let encoded = bincode::serialize(&unsigned)?;
        let hash = blake3::hash(&encoded);

        self.worker_signature = keypair.sign(hash.as_bytes());

        Ok(())
    }

    /// Add coordinator co-signature
    pub fn add_coordinator_signature(&mut self, signature: [u8; 64]) {
        self.coordinator_signature = Some(signature);
    }
}

/// Receipt anchor for on-chain storage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceiptAnchor {
    pub job_id: ObjectId,
    pub epoch_id: u64,
    pub receipt_hash: ReceiptHash,
    pub worker: Address,
    pub output_tokens: u64,
    pub timestamp: DateTime<Utc>,
}

impl ReceiptAnchor {
    /// Create from receipt payload
    pub fn from_payload(payload: &ReceiptPayload) -> Result<Self> {
        Ok(Self {
            job_id: payload.job_id,
            epoch_id: payload.epoch_id,
            receipt_hash: payload.compute_hash()?,
            worker: payload.worker_pubkey,
            output_tokens: payload.output_tokens,
            timestamp: payload.timestamp_end,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use miraset_core::KeyPair;

    #[test]
    fn test_receipt_creation() {
        let kp = KeyPair::generate();
        let receipt = ReceiptPayload::new(
            [1u8; 32],
            1,
            kp.address(),
            "llama2".to_string(),
            "Hello world".to_string(),
            vec!["Hello".to_string(), " world".to_string()],
            2,
            Utc::now(),
            Utc::now(),
        );

        assert!(receipt.is_ok());
    }

    #[test]
    fn test_receipt_hashing() {
        let kp = KeyPair::generate();
        let mut receipt = ReceiptPayload::new(
            [1u8; 32],
            1,
            kp.address(),
            "llama2".to_string(),
            "Hello world".to_string(),
            vec!["Hello".to_string(), " world".to_string()],
            2,
            Utc::now(),
            Utc::now(),
        ).unwrap();

        let hash1 = receipt.compute_hash().unwrap();
        let hash2 = receipt.compute_hash().unwrap();

        // Hash should be deterministic
        assert_eq!(hash1, hash2);

        // Sign and recompute
        receipt.sign(&kp).unwrap();
        let hash3 = receipt.compute_hash().unwrap();

        // Hash changes after signing
        assert_ne!(hash1, hash3);
    }

    #[test]
    fn test_receipt_verification() {
        let kp = KeyPair::generate();
        let receipt = ReceiptPayload::new(
            [1u8; 32],
            1,
            kp.address(),
            "llama2".to_string(),
            "Test".to_string(),
            vec!["Response".to_string()],
            1,
            Utc::now(),
            Utc::now(),
        ).unwrap();

        let hash = receipt.compute_hash().unwrap();
        assert!(receipt.verify(&hash).unwrap());

        // Wrong hash should fail
        let wrong_hash = [0u8; 32];
        assert!(!receipt.verify(&wrong_hash).unwrap());
    }

    #[test]
    fn test_canonical_serialization() {
        // Two identical receipts should produce same hash
        let kp = KeyPair::generate();
        let start = Utc::now();
        let end = Utc::now();

        let receipt1 = ReceiptPayload::new(
            [1u8; 32],
            1,
            kp.address(),
            "llama2".to_string(),
            "Test".to_string(),
            vec!["Response".to_string()],
            1,
            start,
            end,
        ).unwrap();

        let receipt2 = ReceiptPayload::new(
            [1u8; 32],
            1,
            kp.address(),
            "llama2".to_string(),
            "Test".to_string(),
            vec!["Response".to_string()],
            1,
            start,
            end,
        ).unwrap();

        assert_eq!(
            receipt1.compute_hash().unwrap(),
            receipt2.compute_hash().unwrap()
        );
    }
}
