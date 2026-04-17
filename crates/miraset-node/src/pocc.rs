/// Proof of Compute Contribution (PoCC) Consensus Implementation
///
/// PoCC combines:
/// 1. Proof of Capacity - Hardware availability and uptime
/// 2. Proof of Compute - Actual inference work performed
///
/// Validators must run LLM models to participate in consensus and earn rewards.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};
use std::collections::{HashMap, BTreeMap};
use miraset_core::{Address, ObjectId, Block};
use parking_lot::RwLock;
use std::sync::Arc;

// Helper module for serializing [u8; 64] signatures
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

/// Minimum GPU VRAM required to be a validator (16 GB)
pub const MIN_VALIDATOR_VRAM_GIB: u32 = 16;

/// Minimum models required to run
pub const MIN_MODELS_REQUIRED: usize = 3;

/// Model size requirements
pub const MIN_LARGE_MODEL_SIZE: u64 = 13_000_000_000; // 13B parameters
pub const MIN_MEDIUM_MODEL_SIZE: u64 = 7_000_000_000; // 7B parameters

/// Stake required to become a validator (in smallest units)
pub const MIN_VALIDATOR_STAKE: u64 = 10_000_000_000; // 10 billion units

/// Maximum validators in the active set
pub const MAX_ACTIVE_VALIDATORS: usize = 100;

/// Minimum uptime to remain in validator set
pub const MIN_VALIDATOR_UPTIME: f64 = 0.85; // 85%

/// Block time (seconds)
pub const BLOCK_TIME_SECONDS: u64 = 5;

/// Consensus threshold (Byzantine fault tolerance: 2/3 + 1)
pub const CONSENSUS_THRESHOLD: f64 = 0.67;

/// Validator information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Validator {
    pub address: Address,
    pub worker_id: ObjectId,
    pub stake: u64,
    pub commission_rate: f64, // 0.0 to 1.0
    pub gpu_info: GpuInfo,
    pub models: Vec<ModelInfo>,
    pub status: ValidatorStatus,
    pub uptime_score: f64,
    pub total_compute_contributed: u64, // Total tokens processed
    pub joined_epoch: u64,
    pub last_heartbeat: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuInfo {
    pub model: String,
    pub vram_total_gib: u32,
    pub vram_available_gib: u32,
    pub compute_capability: String,
    pub cuda_cores: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub name: String,
    pub size_parameters: u64,
    pub category: ModelCategory,
    pub loaded: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ModelCategory {
    Large,  // 13B+
    Medium, // 7B+
    Small,  // <7B
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ValidatorStatus {
    Active,      // Participating in consensus
    Standby,     // Qualified but not in active set
    Jailed,      // Temporarily suspended (poor performance/downtime)
    Slashed,     // Penalized for malicious behavior
    Unbonding,   // Withdrawing stake
}

impl Validator {
    pub fn new(
        address: Address,
        worker_id: ObjectId,
        stake: u64,
        gpu_info: GpuInfo,
        models: Vec<ModelInfo>,
    ) -> Result<Self> {
        // Validate minimum requirements
        if stake < MIN_VALIDATOR_STAKE {
            return Err(anyhow!("Insufficient stake: minimum {} required", MIN_VALIDATOR_STAKE));
        }

        if gpu_info.vram_total_gib < MIN_VALIDATOR_VRAM_GIB {
            return Err(anyhow!("Insufficient VRAM: minimum {} GiB required", MIN_VALIDATOR_VRAM_GIB));
        }

        if models.len() < MIN_MODELS_REQUIRED {
            return Err(anyhow!("Insufficient models: minimum {} required", MIN_MODELS_REQUIRED));
        }

        // Check model requirements
        let large_models = models.iter().filter(|m| m.category == ModelCategory::Large).count();
        let medium_models = models.iter().filter(|m| m.category == ModelCategory::Medium).count();

        if large_models < 1 || medium_models < 2 {
            return Err(anyhow!("Must run at least 1 large (13B+) and 2 medium (7B+) models"));
        }

        Ok(Self {
            address,
            worker_id,
            stake,
            commission_rate: 0.10, // Default 10% commission
            gpu_info,
            models,
            status: ValidatorStatus::Standby,
            uptime_score: 1.0,
            total_compute_contributed: 0,
            joined_epoch: 0,
            last_heartbeat: Utc::now(),
        })
    }

    /// Check if validator meets minimum requirements
    pub fn meets_requirements(&self) -> bool {
        self.stake >= MIN_VALIDATOR_STAKE
            && self.gpu_info.vram_total_gib >= MIN_VALIDATOR_VRAM_GIB
            && self.models.len() >= MIN_MODELS_REQUIRED
            && self.uptime_score >= MIN_VALIDATOR_UPTIME
            && self.status != ValidatorStatus::Slashed
    }

    /// Calculate validator weight for consensus (based on stake and performance)
    pub fn consensus_weight(&self) -> u64 {
        // Weight = stake * uptime_score
        ((self.stake as f64) * self.uptime_score) as u64
    }

    /// Update validator heartbeat
    pub fn heartbeat(&mut self, vram_available: u32) -> Result<()> {
        self.last_heartbeat = Utc::now();
        self.gpu_info.vram_available_gib = vram_available;

        // Check if validator is still healthy
        if vram_available < (MIN_VALIDATOR_VRAM_GIB / 2) {
            tracing::warn!("Validator {} has low VRAM: {} GiB", self.address, vram_available);
        }

        Ok(())
    }

    /// Record compute contribution (tokens processed)
    pub fn record_compute(&mut self, tokens: u64) {
        self.total_compute_contributed += tokens;
    }
}

/// Validator set management
#[derive(Clone)]
pub struct ValidatorSet {
    inner: Arc<RwLock<ValidatorSetInner>>,
}

struct ValidatorSetInner {
    validators: HashMap<Address, Validator>,
    active_validators: Vec<Address>,
    current_epoch: u64,
    total_stake: u64,
}

impl ValidatorSet {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(ValidatorSetInner {
                validators: HashMap::new(),
                active_validators: Vec::new(),
                current_epoch: 0,
                total_stake: 0,
            })),
        }
    }

    /// Register a new validator
    pub fn register_validator(&self, validator: Validator) -> Result<()> {
        let mut inner = self.inner.write();

        if inner.validators.contains_key(&validator.address) {
            return Err(anyhow!("Validator already registered"));
        }

        if !validator.meets_requirements() {
            return Err(anyhow!("Validator does not meet minimum requirements"));
        }

        // Store address before moving validator
        let validator_address = validator.address;
        inner.total_stake += validator.stake;
        inner.validators.insert(validator_address, validator);

        tracing::info!("Validator registered: {}", validator_address);

        Ok(())
    }

    /// Update active validator set (called at epoch transitions)
    pub fn update_active_set(&self, epoch: u64) {
        let mut inner = self.inner.write();
        inner.current_epoch = epoch;

        // Sort validators by stake (descending) and select top N
        let mut qualified: Vec<_> = inner.validators.values()
            .filter(|v| v.meets_requirements() && v.status != ValidatorStatus::Jailed)
            .collect();

        qualified.sort_by(|a, b| b.consensus_weight().cmp(&a.consensus_weight()));

        // Select top validators up to MAX_ACTIVE_VALIDATORS
        let new_active: Vec<Address> = qualified.iter()
            .take(MAX_ACTIVE_VALIDATORS)
            .map(|v| v.address)
            .collect();

        inner.active_validators = new_active.clone();

        // Update validator statuses
        for (addr, validator) in inner.validators.iter_mut() {
            if new_active.contains(addr) {
                validator.status = ValidatorStatus::Active;
            } else if validator.meets_requirements() {
                validator.status = ValidatorStatus::Standby;
            }
        }

        tracing::info!("Active validator set updated: {} validators", inner.active_validators.len());
    }

    /// Get active validators
    pub fn get_active_validators(&self) -> Vec<Validator> {
        let inner = self.inner.read();
        inner.active_validators.iter()
            .filter_map(|addr| inner.validators.get(addr).cloned())
            .collect()
    }

    /// Get validator by address
    pub fn get_validator(&self, address: &Address) -> Option<Validator> {
        self.inner.read().validators.get(address).cloned()
    }

    /// Update validator uptime score
    pub fn update_uptime(&self, address: &Address, score: f64) -> Result<()> {
        let mut inner = self.inner.write();

        let validator = inner.validators.get_mut(address)
            .ok_or_else(|| anyhow!("Validator not found"))?;

        validator.uptime_score = score;

        // Jail validator if uptime is too low
        if score < MIN_VALIDATOR_UPTIME && validator.status == ValidatorStatus::Active {
            validator.status = ValidatorStatus::Jailed;
            tracing::warn!("Validator {} jailed for low uptime: {:.2}%", address, score * 100.0);
        }

        Ok(())
    }

    /// Record compute contribution
    pub fn record_compute(&self, address: &Address, tokens: u64) -> Result<()> {
        let mut inner = self.inner.write();

        let validator = inner.validators.get_mut(address)
            .ok_or_else(|| anyhow!("Validator not found"))?;

        validator.record_compute(tokens);

        Ok(())
    }

    /// Get total stake in the system
    pub fn total_stake(&self) -> u64 {
        self.inner.read().total_stake
    }

    /// Get number of active validators
    pub fn active_count(&self) -> usize {
        self.inner.read().active_validators.len()
    }
}

/// PoCC Consensus Engine
/// PoCC Consensus Engine
pub struct PoccConsensus {
    pub validator_set: ValidatorSet,
    block_proposer_index: Arc<RwLock<usize>>,
    epoch: Arc<RwLock<u64>>,
}

impl PoccConsensus {
    pub fn new(validator_set: ValidatorSet) -> Self {
        Self {
            validator_set,
            block_proposer_index: Arc::new(RwLock::new(0)),
            epoch: Arc::new(RwLock::new(0)),
        }
    }

    /// Select next block proposer (round-robin weighted by stake)
    pub fn select_proposer(&self) -> Result<Address> {
        let active_validators = self.validator_set.get_active_validators();

        if active_validators.is_empty() {
            return Err(anyhow!("No active validators"));
        }

        let mut index = self.block_proposer_index.write();
        *index = (*index + 1) % active_validators.len();

        Ok(active_validators[*index].address)
    }

    /// Verify block proposal (simplified - in production would check signatures, etc.)
    pub fn verify_proposal(&self, block: &Block, proposer: &Address) -> Result<bool> {
        // Check if proposer is an active validator
        let validator = self.validator_set.get_validator(proposer)
            .ok_or_else(|| anyhow!("Proposer is not a registered validator"))?;

        if validator.status != ValidatorStatus::Active {
            return Err(anyhow!("Proposer is not in active validator set"));
        }

        // Verify block is properly formed
        if block.transactions.is_empty() {
            tracing::warn!("Block has no transactions");
        }

        // TODO: Verify block signatures, merkle roots, etc.

        Ok(true)
    }

    /// Collect votes from validators (Byzantine fault tolerant)
    pub fn collect_votes(&self, block_hash: &[u8; 32]) -> Result<ConsensusVote> {
        let active_validators = self.validator_set.get_active_validators();
        let total_stake: u64 = active_validators.iter().map(|v| v.stake).sum();
        let threshold_stake = ((total_stake as f64) * CONSENSUS_THRESHOLD) as u64;

        // In production, this would collect actual votes from validators
        // For now, simulate unanimous approval
        let approve_stake = total_stake;

        let vote = ConsensusVote {
            block_hash: *block_hash,
            approve_stake,
            reject_stake: 0,
            total_stake,
            threshold_stake,
            approved: approve_stake >= threshold_stake,
        };

        Ok(vote)
    }

    /// Advance to next epoch
    pub fn advance_epoch(&self) -> u64 {
        let mut epoch = self.epoch.write();
        *epoch += 1;
        let new_epoch = *epoch;

        // Update active validator set for new epoch
        self.validator_set.update_active_set(new_epoch);

        tracing::info!("Advanced to epoch {}", new_epoch);

        new_epoch
    }

    /// Get current epoch
    pub fn current_epoch(&self) -> u64 {
        *self.epoch.read()
    }
}

/// Consensus vote result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsensusVote {
    pub block_hash: [u8; 32],
    pub approve_stake: u64,
    pub reject_stake: u64,
    pub total_stake: u64,
    pub threshold_stake: u64,
    pub approved: bool,
}

/// Compute contribution proof (for PoCC)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComputeProof {
    pub validator: Address,
    pub job_id: ObjectId,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub model_used: String,
    pub timestamp: DateTime<Utc>,
    pub result_hash: [u8; 32],
    #[serde(with = "signature_serde")]
    pub signature: [u8; 64],
}

impl ComputeProof {
    /// Verify proof is valid
    pub fn verify(&self, validator_pubkey: &Address) -> bool {
        // Serialize proof data for verification
        let mut proof_data = Vec::new();
        proof_data.extend_from_slice(self.job_id.as_ref());
        proof_data.extend_from_slice(&self.input_tokens.to_le_bytes());
        proof_data.extend_from_slice(&self.output_tokens.to_le_bytes());
        proof_data.extend_from_slice(self.model_used.as_bytes());
        proof_data.extend_from_slice(&self.result_hash);

        // Verify signature
        miraset_core::verify_signature(validator_pubkey, &proof_data, &self.signature)
    }

    /// Calculate contribution score
    pub fn contribution_score(&self) -> u64 {
        // Score based on total tokens processed
        self.input_tokens + self.output_tokens
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use miraset_core::KeyPair;

    #[test]
    fn test_validator_creation() {
        let kp = KeyPair::generate();
        let gpu_info = GpuInfo {
            model: "NVIDIA RTX 4090".to_string(),
            vram_total_gib: 24,
            vram_available_gib: 20,
            compute_capability: "8.9".to_string(),
            cuda_cores: Some(16384),
        };

        let models = vec![
            ModelInfo {
                name: "llama-2-13b".to_string(),
                size_parameters: 13_000_000_000,
                category: ModelCategory::Large,
                loaded: true,
            },
            ModelInfo {
                name: "mistral-7b".to_string(),
                size_parameters: 7_000_000_000,
                category: ModelCategory::Medium,
                loaded: true,
            },
            ModelInfo {
                name: "phi-2".to_string(),
                size_parameters: 7_000_000_000,
                category: ModelCategory::Medium,
                loaded: true,
            },
        ];

        let validator = Validator::new(
            kp.address(),
            [1u8; 32],
            MIN_VALIDATOR_STAKE,
            gpu_info,
            models,
        );

        assert!(validator.is_ok());
        let v = validator.unwrap();
        assert!(v.meets_requirements());
    }

    #[test]
    fn test_insufficient_stake() {
        let kp = KeyPair::generate();
        let gpu_info = GpuInfo {
            model: "NVIDIA RTX 4090".to_string(),
            vram_total_gib: 24,
            vram_available_gib: 20,
            compute_capability: "8.9".to_string(),
            cuda_cores: Some(16384),
        };

        let models = vec![
            ModelInfo {
                name: "llama-2-13b".to_string(),
                size_parameters: 13_000_000_000,
                category: ModelCategory::Large,
                loaded: true,
            },
        ];

        let validator = Validator::new(
            kp.address(),
            [1u8; 32],
            1000, // Too low
            gpu_info,
            models,
        );

        assert!(validator.is_err());
    }

    #[test]
    fn test_validator_set() {
        let set = ValidatorSet::new();

        let kp = KeyPair::generate();
        let gpu_info = GpuInfo {
            model: "NVIDIA RTX 4090".to_string(),
            vram_total_gib: 24,
            vram_available_gib: 20,
            compute_capability: "8.9".to_string(),
            cuda_cores: Some(16384),
        };

        let models = vec![
            ModelInfo {
                name: "llama-2-13b".to_string(),
                size_parameters: 13_000_000_000,
                category: ModelCategory::Large,
                loaded: true,
            },
            ModelInfo {
                name: "mistral-7b".to_string(),
                size_parameters: 7_000_000_000,
                category: ModelCategory::Medium,
                loaded: true,
            },
            ModelInfo {
                name: "phi-2".to_string(),
                size_parameters: 7_000_000_000,
                category: ModelCategory::Medium,
                loaded: true,
            },
        ];

        let validator = Validator::new(
            kp.address(),
            [1u8; 32],
            MIN_VALIDATOR_STAKE,
            gpu_info,
            models,
        ).unwrap();

        let result = set.register_validator(validator);
        assert!(result.is_ok());

        // Update active set
        set.update_active_set(1);

        assert_eq!(set.active_count(), 1);
        let active = set.get_active_validators();
        assert_eq!(active[0].address, kp.address());
    }
}
