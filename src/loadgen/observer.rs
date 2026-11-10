//! Commit observation: on each `commit_block` event from the observer node, read the committed
//! state of everything still pending and stamp what has committed with the event's arrival.
//!
//! Resolution: an item committed by a block that arrives while a read round is in flight is
//! stamped with the earlier event, so latencies can be low by up to one round; the report
//! carries the longest round so readers can bound that error.

use super::driver::task_settled;
use super::http::RpcClient;
use super::tracker::{Tracker, Window, lock};
use crate::core::types::{AccountState, CommitEvent};
use crate::rpc::ApiResponse;
use serde::Serialize;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinSet;
use tokio::time::Instant;

/// Most pending receipts of one stream checked per read round.
const RECEIPT_SCAN_LIMIT: usize = 256;
/// Consecutive unsettled receipts after which a stream's scan stops for this round.
const UNSETTLED_RUN: usize = 3;

/// One committed block as seen by the observer.
#[derive(Debug, Clone)]
pub struct BlockSeen {
    pub arrived: Instant,
    pub tx_count: usize,
    pub receipt_count: usize,
}

pub struct ObserverLog {
    pub blocks: BlockStats,
    pub max_round: Duration,
    pub total_round: Duration,
    pub rounds: usize,
    pub read_errors: usize,
}

/// Running block statistics for blocks arriving inside the measurement window (constant
/// memory: a soak sees tens of blocks per second for days).
#[derive(Debug, Clone)]
pub struct BlockStats {
    window: Window,
    last_arrival: Option<Instant>,
    blocks: usize,
    empty_blocks: usize,
    intervals: usize,
    interval_total: Duration,
    interval_max: Duration,
    txs: usize,
    txs_max: usize,
    receipts: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct BlockReport {
    pub blocks: usize,
    pub empty_blocks: usize,
    pub mean_interval_ms: Option<f64>,
    pub max_interval_ms: Option<f64>,
    pub mean_txs_per_block: f64,
    pub max_txs_per_block: usize,
    pub mean_receipts_per_block: f64,
}

impl BlockStats {
    pub fn new(window: Window) -> Self {
        Self {
            window,
            last_arrival: None,
            blocks: 0,
            empty_blocks: 0,
            intervals: 0,
            interval_total: Duration::ZERO,
            interval_max: Duration::ZERO,
            txs: 0,
            txs_max: 0,
            receipts: 0,
        }
    }

    pub fn record(&mut self, block: &BlockSeen) {
        let previous = self.last_arrival.replace(block.arrived);
        if !self.window.contains(block.arrived) {
            return;
        }
        self.blocks += 1;
        if block.tx_count + block.receipt_count == 0 {
            self.empty_blocks += 1;
        }
        self.txs += block.tx_count;
        self.txs_max = self.txs_max.max(block.tx_count);
        self.receipts += block.receipt_count;
        if let Some(previous) = previous.filter(|p| self.window.contains(*p)) {
            let interval = block.arrived.saturating_duration_since(previous);
            self.intervals += 1;
            self.interval_total += interval;
            self.interval_max = self.interval_max.max(interval);
        }
    }

    pub fn report(&self) -> BlockReport {
        let per_block = |total: usize| {
            if self.blocks == 0 {
                0.0
            } else {
                round1(total as f64 / self.blocks as f64)
            }
        };
        let has_intervals = self.intervals > 0;
        BlockReport {
            blocks: self.blocks,
            empty_blocks: self.empty_blocks,
            mean_interval_ms: has_intervals
                .then(|| round1(as_ms(self.interval_total) / self.intervals as f64)),
            max_interval_ms: has_intervals.then(|| round1(as_ms(self.interval_max))),
            mean_txs_per_block: per_block(self.txs),
            max_txs_per_block: self.txs_max,
            mean_receipts_per_block: per_block(self.receipts),
        }
    }
}

fn as_ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

pub struct Observer {
    pub client: RpcClient,
    pub node_url: String,
    pub senders: Vec<String>,
    pub tracker: Arc<Mutex<Tracker>>,
    pub window: Window,
}

impl Observer {
    /// Run until `stop` turns true or the event stream ends.
    pub async fn run(
        self,
        mut events: mpsc::UnboundedReceiver<(Instant, CommitEvent)>,
        mut stop: watch::Receiver<bool>,
    ) -> ObserverLog {
        let mut log = ObserverLog {
            blocks: BlockStats::new(self.window),
            max_round: Duration::ZERO,
            total_round: Duration::ZERO,
            rounds: 0,
            read_errors: 0,
        };
        loop {
            let first = tokio::select! {
                event = events.recv() => event,
                _ = stop.changed() => None,
            };
            let Some(first) = first else { break };
            // Coalesce a backlog: the latest arrival stamps this round (never earlier than the
            // block that committed the item).
            let mut latest = first;
            log.blocks.record(&seen(&latest));
            while let Ok(next) = events.try_recv() {
                log.blocks.record(&seen(&next));
                latest = next;
            }
            let round_start = Instant::now();
            log.read_errors += self.read_round(latest.0).await;
            let round = round_start.elapsed();
            log.max_round = log.max_round.max(round);
            log.total_round += round;
            log.rounds += 1;
            if *stop.borrow() {
                break;
            }
        }
        log
    }

    /// Read committed state for every pending sender and receipt stream; returns read errors.
    async fn read_round(&self, at: Instant) -> usize {
        let (senders, streams) = {
            let tracker = lock(&self.tracker);
            (tracker.senders_with_pending(), tracker.receipt_streams())
        };
        let mut reads = JoinSet::new();
        for sender in senders {
            let client = self.client.clone();
            let url = format!("{}/account/{}", self.node_url, self.senders[sender]);
            let tracker = Arc::clone(&self.tracker);
            reads.spawn(async move {
                let account: ApiResponse<AccountState> = client.get_json(&url).await.ok()?;
                let nonce = account.data?.nonce;
                lock(&tracker).confirm_txs(sender, nonce, at);
                Some(())
            });
        }
        for stream in 0..streams {
            let client = self.client.clone();
            let base = self.node_url.clone();
            let tracker = Arc::clone(&self.tracker);
            reads.spawn(async move { settle_stream(client, base, tracker, stream, at).await });
        }
        let mut errors = 0;
        while let Some(result) = reads.join_next().await {
            if !matches!(result, Ok(Some(()))) {
                errors += 1;
            }
        }
        errors
    }
}

/// Confirm a stream's settled receipts. A node proposes a stream's receipts in task order, so
/// settled ones are found from the front; a receipt the chain refuses never settles, so the
/// scan steps over unsettled tasks and stops only after `UNSETTLED_RUN` in a row.
async fn settle_stream(
    client: RpcClient,
    base: String,
    tracker: Arc<Mutex<Tracker>>,
    stream: usize,
    at: Instant,
) -> Option<()> {
    let candidates = lock(&tracker).pending_tasks(stream, RECEIPT_SCAN_LIMIT);
    let mut unsettled_run = 0;
    for task_id in candidates {
        if task_settled(&client, &base, &task_id).await? {
            lock(&tracker).confirm_receipt(stream, &task_id, at);
            unsettled_run = 0;
        } else {
            unsettled_run += 1;
            if unsettled_run >= UNSETTLED_RUN {
                break;
            }
        }
    }
    Some(())
}

fn seen((arrived, event): &(Instant, CommitEvent)) -> BlockSeen {
    BlockSeen {
        arrived: *arrived,
        tx_count: event.tx_count,
        receipt_count: event.receipt_count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(base: Instant, ms: u64, txs: usize, receipts: usize) -> BlockSeen {
        BlockSeen {
            arrived: base + Duration::from_millis(ms),
            tx_count: txs,
            receipt_count: receipts,
        }
    }

    #[test]
    fn test_block_stats_summarise_intervals_and_contents_inside_the_window() {
        let base = Instant::now();
        let window = Window {
            start: base + Duration::from_millis(100),
            end: base + Duration::from_millis(2000),
        };
        let mut stats = BlockStats::new(window);
        for b in [
            block(base, 0, 50, 5), // before the window: not counted, no interval
            block(base, 200, 10, 2),
            block(base, 600, 0, 0),
            block(base, 1200, 20, 0),
            block(base, 2500, 99, 9), // after the window
        ] {
            stats.record(&b);
        }
        let report = stats.report();
        assert_eq!(report.blocks, 3);
        assert_eq!(report.empty_blocks, 1);
        assert_eq!(report.mean_interval_ms, Some(500.0));
        assert_eq!(report.max_interval_ms, Some(600.0));
        assert_eq!(report.mean_txs_per_block, 10.0);
        assert_eq!(report.max_txs_per_block, 20);
    }

    #[test]
    fn test_block_stats_of_no_blocks() {
        let now = Instant::now();
        let report = BlockStats::new(Window {
            start: now,
            end: now,
        })
        .report();
        assert_eq!(report.blocks, 0);
        assert_eq!(report.mean_interval_ms, None);
        assert_eq!(report.mean_txs_per_block, 0.0);
    }
}
