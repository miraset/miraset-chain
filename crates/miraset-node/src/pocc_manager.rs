/// PoCC Manager - Integrates Proof of Compute Contribution
use anyhow::{anyhow, Result};
use crate::pocc::{PoccConsensus, Validator, ValidatorSet, ComputeProof, GpuInfo, ModelInfo};
use crate::state::State;
use miraset_core::{Address, Block, Transaction, ObjectId};
use std::sync::Arc;
pub struct PoccManager {
    consensus: Arc<PoccConsensus>,
    state: State,
}
impl PoccManager {
    pub fn new(state: State) -> Self {
        let validator_set = ValidatorSet::new();
        let consensus = PoccConsensus::new(validator_set);
        Self {
            consensus: Arc::new(consensus),
            state,
        }
    }
    pub fn register_validator(
        &self,
        address: Address,
        worker_id: ObjectId,
        stake: u64,
        gpu_info: GpuInfo,
        models: Vec<ModelInfo>,
    ) -> Result<()> {
        let validator = Validator::new(address, worker_id, stake, gpu_info, models)?;
        self.consensus.validator_set.register_validator(validator)?;
        Ok(())
    }
}
