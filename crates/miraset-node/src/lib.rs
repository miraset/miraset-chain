pub mod epoch;
pub mod executor;
pub mod gas;
pub mod move_vm;
pub mod pocc;
pub mod pocc_manager;
pub mod rpc;
pub mod state;
pub mod storage;

pub use epoch::{Epoch, EpochRewards, EpochStatus, WorkerEpochStats};
pub use executor::{ExecutionContext, TransactionEffects, ExecutionStatus};
pub use gas::{GasConfig, GasBudget, GasStatus, GasCost, GasCoin};
pub use move_vm::{MoveVMRuntime, MoveVMSession, ModuleId, FunctionId, MoveValue, MoveType, MoveObject};
pub use pocc::{PoccConsensus, Validator, ValidatorSet, ValidatorStatus, ComputeProof, GpuInfo, ModelInfo};
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

        let block = state.produce_block();
        tracing::info!(
            "Produced block #{} with {} txs (epoch {})",
            block.height,
            block.transactions.len(),
            state.get_current_epoch().id,
        );
    }
}
