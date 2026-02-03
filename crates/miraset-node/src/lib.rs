pub mod epoch;
pub mod rpc;
pub mod state;
pub mod storage;

pub use epoch::{Epoch, EpochRewards, EpochStatus, WorkerEpochStats};
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
        let block = state.produce_block();
        tracing::info!(
            "Produced block #{} with {} txs",
            block.height,
            block.transactions.len()
        );
    }
}
