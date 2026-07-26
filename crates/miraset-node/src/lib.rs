pub mod epoch;
pub mod error;
pub mod executor;
pub mod gas;
pub mod pocc;
pub mod pocc_manager;
pub mod rpc;
pub mod state;
pub mod storage;

pub use epoch::{Epoch, EpochRewards, EpochStatus, WorkerEpochStats};
pub use executor::{ExecutionContext, ExecutionStatus, TransactionEffects};
pub use gas::{GasBudget, GasCoin, GasConfig, GasCost, GasStatus};
pub use pocc::{
    ComputeProof, GpuInfo, ModelInfo, PoccConsensus, Validator, ValidatorSet, ValidatorStatus,
};
pub use pocc_manager::PoccManager;
pub use rpc::serve_rpc;
pub use state::State;
pub use storage::Storage;

use std::time::Duration;
use tokio::time;

/// Block producer loop
pub async fn run_block_producer(state: State, interval: Duration) {
    let mut ticker = time::interval(interval);
    loop {
        ticker.tick().await;

        // D4: Auto-advance epoch status
        state.update_epoch();

        match state.produce_block() {
            Ok(block) => {
                tracing::info!(
                    "Produced block #{} with {} txs (epoch {})",
                    block.height,
                    block.transactions.len(),
                    state.get_current_epoch().id,
                );
            }
            Err(e) => {
                tracing::error!("Failed to produce block: {}", e);
            }
        }
    }
}
