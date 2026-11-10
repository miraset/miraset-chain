//! Bookkeeping for one load run: what was offered, admitted, rejected, and when each admitted
//! item was first observed committed. Pure state; the driver and observer feed it.
//!
//! Memory stays bounded for long soaks: outcomes are counters, and only latencies of items
//! scheduled inside the measurement window are kept (4 bytes each, about 21 MB for 72 h at
//! 20 items/s). Pending queues hold only items not yet observed committed.

use super::stats::LatencySummary;
use serde::Serialize;
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;
use tokio::time::Instant;

/// Lock shared tracker state. A panicking task must not cascade into every other generator
/// through a poisoned lock; the data stays usable (each update is a single push or counter).
pub fn lock(tracker: &Mutex<Tracker>) -> MutexGuard<'_, Tracker> {
    tracker.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Transaction,
    Receipt,
}

/// Measurement window: items scheduled in `[start, end)` are reported.
#[derive(Debug, Clone, Copy)]
pub struct Window {
    pub start: Instant,
    pub end: Instant,
}

impl Window {
    pub fn contains(&self, t: Instant) -> bool {
        t >= self.start && t < self.end
    }

    fn secs(&self) -> f64 {
        self.end.saturating_duration_since(self.start).as_secs_f64()
    }
}

#[derive(Default)]
struct KindLog {
    offered: usize,
    admitted: usize,
    rejected: BTreeMap<String, usize>,
    /// Latency of items scheduled in the window, microseconds (saturating at ~71 minutes).
    latencies_us: Vec<u32>,
    /// Commits observed inside the window, whenever the item was scheduled.
    commits_in_window: usize,
}

/// An admitted item not yet observed committed. `scheduled` is when the generator was due to
/// send it, so time spent behind a slow request counts as latency (no coordinated omission).
struct Pending<K> {
    key: K,
    scheduled: Instant,
}

pub struct Tracker {
    window: Window,
    tx_pending: Vec<VecDeque<Pending<u64>>>,
    receipt_pending: Vec<VecDeque<Pending<String>>>,
    tx: KindLog,
    receipt: KindLog,
}

/// Outcome counts and latency for items scheduled inside the measurement window.
#[derive(Debug, Clone, Serialize)]
pub struct KindReport {
    /// Items with a final admission outcome (admitted or rejected).
    pub offered: usize,
    pub admitted: usize,
    pub rejected: BTreeMap<String, usize>,
    /// Admitted and observed committed (at any time before the run ended).
    pub committed: usize,
    /// Admitted and not observed committed when the run ended; excluded from `latency`.
    pub uncommitted: usize,
    /// Commits of this kind observed inside the window per second of window, including items
    /// scheduled during warm-up (throughput, not comparable one-to-one with `admitted`).
    pub committed_per_sec: f64,
    /// Scheduled send to first observed commit, for committed items scheduled in the window.
    pub latency: LatencySummary,
}

/// Running totals for periodic progress lines.
#[derive(Debug, Clone, Serialize)]
pub struct Progress {
    pub admitted: usize,
    pub rejected: usize,
    pub pending: usize,
    pub oldest_pending_secs: Option<f64>,
}

impl Tracker {
    pub fn new(window: Window, senders: usize, receipt_streams: usize) -> Self {
        Self {
            window,
            tx_pending: (0..senders).map(|_| VecDeque::new()).collect(),
            receipt_pending: (0..receipt_streams).map(|_| VecDeque::new()).collect(),
            tx: KindLog::default(),
            receipt: KindLog::default(),
        }
    }

    fn log(&mut self, kind: Kind) -> &mut KindLog {
        match kind {
            Kind::Transaction => &mut self.tx,
            Kind::Receipt => &mut self.receipt,
        }
    }

    pub fn record_rejected(&mut self, kind: Kind, scheduled: Instant, reason: impl Into<String>) {
        if !self.window.contains(scheduled) {
            return;
        }
        let log = self.log(kind);
        log.offered += 1;
        *log.rejected.entry(reason.into()).or_default() += 1;
    }

    fn record_admitted(&mut self, kind: Kind, scheduled: Instant) {
        if self.window.contains(scheduled) {
            let log = self.log(kind);
            log.offered += 1;
            log.admitted += 1;
        }
    }

    pub fn record_admitted_tx(&mut self, sender: usize, nonce: u64, scheduled: Instant) {
        self.record_admitted(Kind::Transaction, scheduled);
        self.tx_pending[sender].push_back(Pending {
            key: nonce,
            scheduled,
        });
    }

    pub fn record_admitted_receipt(&mut self, stream: usize, task_id: String, scheduled: Instant) {
        self.record_admitted(Kind::Receipt, scheduled);
        self.receipt_pending[stream].push_back(Pending {
            key: task_id,
            scheduled,
        });
    }

    fn record_commit(&mut self, kind: Kind, scheduled: Instant, at: Instant) {
        let window = self.window;
        let log = self.log(kind);
        if window.contains(at) {
            log.commits_in_window += 1;
        }
        if window.contains(scheduled) {
            let micros = at.saturating_duration_since(scheduled).as_micros();
            log.latencies_us
                .push(u32::try_from(micros).unwrap_or(u32::MAX));
        }
    }

    /// Senders with admitted transfers not yet observed committed.
    pub fn senders_with_pending(&self) -> Vec<usize> {
        (0..self.tx_pending.len())
            .filter(|i| !self.tx_pending[*i].is_empty())
            .collect()
    }

    /// The sender's committed account nonce is `committed_nonce`: every pending transfer with a
    /// lower nonce is committed, observed at `at`. (A sender's transfers can only commit in
    /// nonce order, so a stuck nonce really does hold back the later ones.)
    pub fn confirm_txs(&mut self, sender: usize, committed_nonce: u64, at: Instant) {
        while let Some(front) = self.tx_pending[sender].front() {
            if front.key >= committed_nonce {
                break;
            }
            let scheduled = front.scheduled;
            self.tx_pending[sender].pop_front();
            self.record_commit(Kind::Transaction, scheduled, at);
        }
    }

    pub fn receipt_streams(&self) -> usize {
        self.receipt_pending.len()
    }

    /// Up to `limit` oldest unsettled tasks of a stream, oldest first.
    pub fn pending_tasks(&self, stream: usize, limit: usize) -> Vec<String> {
        self.receipt_pending[stream]
            .iter()
            .take(limit)
            .map(|p| p.key.clone())
            .collect()
    }

    /// Task `task_id` of a stream is settled, observed at `at`. Receipts commit independently
    /// (a receipt the chain refuses must not hide the ones after it), so any position matches.
    pub fn confirm_receipt(&mut self, stream: usize, task_id: &str, at: Instant) {
        let queue = &mut self.receipt_pending[stream];
        let Some(position) = queue.iter().position(|p| p.key == task_id) else {
            return;
        };
        if let Some(pending) = queue.remove(position) {
            self.record_commit(Kind::Receipt, pending.scheduled, at);
        }
    }

    pub fn pending(&self) -> usize {
        self.tx_pending.iter().map(VecDeque::len).sum::<usize>()
            + self
                .receipt_pending
                .iter()
                .map(VecDeque::len)
                .sum::<usize>()
    }

    fn oldest_pending(&self) -> Option<Instant> {
        let txs = self
            .tx_pending
            .iter()
            .filter_map(|q| q.front().map(|p| p.scheduled));
        let receipts = self
            .receipt_pending
            .iter()
            .flat_map(|q| q.iter().map(|p| p.scheduled));
        txs.chain(receipts).min()
    }

    pub fn progress(&self, now: Instant) -> Progress {
        let rejected = |log: &KindLog| log.rejected.values().sum::<usize>();
        Progress {
            admitted: self.tx.admitted + self.receipt.admitted,
            rejected: rejected(&self.tx) + rejected(&self.receipt),
            pending: self.pending(),
            oldest_pending_secs: self
                .oldest_pending()
                .map(|t| round1(now.saturating_duration_since(t).as_secs_f64())),
        }
    }

    pub fn report(&self, kind: Kind) -> KindReport {
        let log = match kind {
            Kind::Transaction => &self.tx,
            Kind::Receipt => &self.receipt,
        };
        let latencies: Vec<Duration> = log
            .latencies_us
            .iter()
            .map(|us| Duration::from_micros(u64::from(*us)))
            .collect();
        KindReport {
            offered: log.offered,
            admitted: log.admitted,
            rejected: log.rejected.clone(),
            committed: latencies.len(),
            uncommitted: log.admitted.saturating_sub(latencies.len()),
            committed_per_sec: per_sec(log.commits_in_window, self.window.secs()),
            latency: LatencySummary::from_samples(&latencies),
        }
    }
}

fn per_sec(count: usize, secs: f64) -> f64 {
    if secs <= 0.0 {
        return 0.0;
    }
    ((count as f64 / secs) * 100.0).round() / 100.0
}

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(base: Instant, ms: u64) -> Instant {
        base + Duration::from_millis(ms)
    }

    fn window(base: Instant, start_ms: u64, end_ms: u64) -> Window {
        Window {
            start: at(base, start_ms),
            end: at(base, end_ms),
        }
    }

    #[test]
    fn test_committed_nonce_confirms_only_lower_pending_nonces() {
        let base = Instant::now();
        let mut tracker = Tracker::new(window(base, 0, 1000), 1, 0);
        for nonce in 0..3 {
            tracker.record_admitted_tx(0, nonce, at(base, nonce * 10));
        }
        tracker.confirm_txs(0, 2, at(base, 500));
        assert_eq!(tracker.pending(), 1);
        assert_eq!(tracker.senders_with_pending(), vec![0]);

        let report = tracker.report(Kind::Transaction);
        assert_eq!(report.admitted, 3);
        assert_eq!(report.committed, 2);
        assert_eq!(report.uncommitted, 1);
        assert_eq!(report.latency.max_ms, Some(500.0));
        assert_eq!(report.latency.p50_ms, Some(490.0));
    }

    #[test]
    fn test_a_stuck_receipt_does_not_hide_later_settled_ones() {
        let base = Instant::now();
        let mut tracker = Tracker::new(window(base, 0, 1000), 0, 1);
        for id in ["a", "b", "c"] {
            tracker.record_admitted_receipt(0, id.into(), base);
        }
        tracker.confirm_receipt(0, "b", at(base, 100));
        tracker.confirm_receipt(0, "b", at(base, 200)); // already confirmed: no second sample
        tracker.confirm_receipt(0, "unknown", at(base, 200));
        assert_eq!(tracker.pending(), 2);
        assert_eq!(
            tracker.pending_tasks(0, 5),
            vec!["a".to_string(), "c".to_string()]
        );
        assert_eq!(tracker.pending_tasks(0, 1), vec!["a".to_string()]);
        assert_eq!(tracker.report(Kind::Receipt).committed, 1);
    }

    #[test]
    fn test_report_counts_only_items_scheduled_inside_the_window() {
        let base = Instant::now();
        let mut tracker = Tracker::new(window(base, 1000, 3000), 1, 0);
        tracker.record_admitted_tx(0, 0, at(base, 0)); // warm-up
        tracker.record_admitted_tx(0, 1, at(base, 2000));
        tracker.record_rejected(Kind::Transaction, at(base, 2100), "status 503");
        tracker.record_rejected(Kind::Transaction, at(base, 100), "status 503"); // warm-up
        tracker.confirm_txs(0, 2, at(base, 2500));

        let report = tracker.report(Kind::Transaction);
        assert_eq!(report.offered, 2);
        assert_eq!(report.admitted, 1);
        assert_eq!(report.committed, 1);
        assert_eq!(report.latency.count, 1);
        assert_eq!(report.rejected.get("status 503"), Some(&1));
        // Both commits (including the warm-up item) were observed inside the 2 s window.
        assert_eq!(report.committed_per_sec, 1.0);
    }

    #[test]
    fn test_progress_reports_the_oldest_pending_item() {
        let base = Instant::now();
        let mut tracker = Tracker::new(window(base, 0, 10_000), 1, 1);
        tracker.record_admitted_tx(0, 0, at(base, 300));
        tracker.record_admitted_receipt(0, "a".into(), at(base, 100));
        tracker.record_rejected(Kind::Receipt, at(base, 200), "status 503");
        let progress = tracker.progress(at(base, 1100));
        assert_eq!(progress.admitted, 2);
        assert_eq!(progress.rejected, 1);
        assert_eq!(progress.pending, 2);
        assert_eq!(progress.oldest_pending_secs, Some(1.0));
    }
}
