#![warn(clippy::pedantic)]
#![deny(clippy::unwrap_used, clippy::expect_used)]

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
/// PoCC consensus scaffolding — not currently wired into block production.
pub use pocc::{
    ComputeProof, GpuInfo, ModelInfo, PoccConsensus, Validator, ValidatorSet, ValidatorStatus,
};
pub use pocc_manager::PoccManager;
pub use rpc::serve_rpc;
pub use state::State;
pub use storage::Storage;

use std::time::Duration;
use tokio::time;

/// Block producer loop.
///
/// # Devnet warning
/// This runs in single-author devnet mode: there is no consensus, no
/// validator set, and blocks are produced locally. Do not use this in
/// production or for multi-node deployments.
pub async fn run_block_producer(state: State, interval: Duration) {
    tracing::warn!("Starting block producer in single-author devnet mode (no consensus).");

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
                // Dispatch /jobs/accept notifications for any jobs that were
                // auto-assigned inside this block. This runs outside the state
                // lock and is fire-and-forget.
                state.dispatch_job_assignments(block.height);
            }
            Err(e) => {
                tracing::error!("Failed to produce block: {}", e);
            }
        }
    }
}
