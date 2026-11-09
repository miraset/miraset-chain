//! Node metrics in Prometheus text exposition format (TODO 8.4.4, PRD R12).
//!
//! The metric set is fixed and no label takes a value from requests, peers or chain data, so
//! cardinality is bounded by construction. Counters are process-local and reset on restart.

use std::fmt::Write as _;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Rejection reasons for transaction and receipt admission. Fixed set of label values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionReject {
    Duplicate,
    Draining,
}

impl AdmissionReject {
    const ALL: [AdmissionReject; 2] = [AdmissionReject::Duplicate, AdmissionReject::Draining];

    fn as_str(self) -> &'static str {
        match self {
            AdmissionReject::Duplicate => "duplicate",
            AdmissionReject::Draining => "draining",
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// What is admitted: a transaction or a compute receipt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionKind {
    Transaction,
    Receipt,
}

impl AdmissionKind {
    const ALL: [AdmissionKind; 2] = [AdmissionKind::Transaction, AdmissionKind::Receipt];

    fn as_str(self) -> &'static str {
        match self {
            AdmissionKind::Transaction => "transaction",
            AdmissionKind::Receipt => "receipt",
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

#[derive(Debug, Default)]
struct AdmissionCounters {
    admitted: AtomicU64,
    rejected: [AtomicU64; AdmissionReject::ALL.len()],
}

/// Process-wide node metrics updated by the commit hook, RPC admission and the transport.
#[derive(Debug)]
pub struct NodeMetrics {
    started: Instant,
    commits_total: AtomicU64,
    committed_height: AtomicU64,
    /// Nanoseconds since `started` at the last commit, plus one; zero means no commit yet.
    last_commit_offset_ns: AtomicU64,
    admission: [AdmissionCounters; AdmissionKind::ALL.len()],
    p2p_dropped: OnceLock<std::sync::Arc<AtomicU64>>,
}

impl Default for NodeMetrics {
    fn default() -> Self {
        Self {
            started: Instant::now(),
            commits_total: AtomicU64::new(0),
            committed_height: AtomicU64::new(0),
            last_commit_offset_ns: AtomicU64::new(0),
            admission: Default::default(),
            p2p_dropped: OnceLock::new(),
        }
    }
}

/// Values computed at scrape time from node state rather than counted here.
#[derive(Debug, Clone, Copy, Default)]
pub struct ScrapeState {
    pub mempool_pending_txs: usize,
    pub mempool_pending_receipts: usize,
    pub halted: bool,
    pub draining: bool,
    pub ready: bool,
}

impl NodeMetrics {
    /// Record a committed block. `height` is `None` when it could not be read; the commit still
    /// counts for readiness, because the block is final either way.
    pub fn record_commit(&self, height: Option<u64>) {
        self.commits_total.fetch_add(1, Ordering::Relaxed);
        if let Some(height) = height {
            self.committed_height.fetch_max(height, Ordering::Relaxed);
        }
        let offset = u64::try_from(self.started.elapsed().as_nanos()).unwrap_or(u64::MAX - 1);
        self.last_commit_offset_ns
            .store(offset.saturating_add(1), Ordering::Release);
    }

    /// Time since the last commit this process observed; `None` before the first.
    pub fn last_commit_age(&self) -> Option<Duration> {
        match self.last_commit_offset_ns.load(Ordering::Acquire) {
            0 => None,
            stored => {
                let at = Duration::from_nanos(stored - 1);
                Some(self.started.elapsed().saturating_sub(at))
            }
        }
    }

    pub fn committed_height(&self) -> u64 {
        self.committed_height.load(Ordering::Relaxed)
    }

    pub fn record_admitted(&self, kind: AdmissionKind) {
        self.admission[kind.index()]
            .admitted
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_rejected(&self, kind: AdmissionKind, reason: AdmissionReject) {
        self.admission[kind.index()].rejected[reason.index()].fetch_add(1, Ordering::Relaxed);
    }

    /// Export the transport's dropped-message counter. Only the first call takes effect.
    pub fn attach_p2p_dropped(&self, counter: std::sync::Arc<AtomicU64>) {
        let _ = self.p2p_dropped.set(counter);
    }

    /// Render all metrics in Prometheus text exposition format 0.0.4.
    pub fn render(&self, scrape: ScrapeState) -> String {
        let mut out = String::with_capacity(2048);
        let counter = |out: &mut String, name: &str, help: &str| {
            let _ = writeln!(out, "# HELP {name} {help}\n# TYPE {name} counter");
        };
        let gauge = |out: &mut String, name: &str, help: &str| {
            let _ = writeln!(out, "# HELP {name} {help}\n# TYPE {name} gauge");
        };

        counter(
            &mut out,
            "miraset_commits_total",
            "Blocks committed since process start.",
        );
        let _ = writeln!(
            out,
            "miraset_commits_total {}",
            self.commits_total.load(Ordering::Relaxed)
        );
        gauge(
            &mut out,
            "miraset_committed_height",
            "Highest committed block height seen by this process.",
        );
        let _ = writeln!(out, "miraset_committed_height {}", self.committed_height());
        // Absent before the first commit rather than a sentinel value.
        if let Some(age) = self.last_commit_age() {
            gauge(
                &mut out,
                "miraset_last_commit_age_seconds",
                "Seconds since this process last observed a commit.",
            );
            let _ = writeln!(
                out,
                "miraset_last_commit_age_seconds {:.3}",
                age.as_secs_f64()
            );
        }

        counter(
            &mut out,
            "miraset_admission_accepted_total",
            "Transactions and receipts admitted to the mempool.",
        );
        for kind in AdmissionKind::ALL {
            let value = self.admission[kind.index()]
                .admitted
                .load(Ordering::Relaxed);
            let _ = writeln!(
                out,
                "miraset_admission_accepted_total{{kind=\"{}\"}} {value}",
                kind.as_str()
            );
        }
        counter(
            &mut out,
            "miraset_admission_rejected_total",
            "Transactions and receipts rejected at admission.",
        );
        for kind in AdmissionKind::ALL {
            for reason in AdmissionReject::ALL {
                let value =
                    self.admission[kind.index()].rejected[reason.index()].load(Ordering::Relaxed);
                let _ = writeln!(
                    out,
                    "miraset_admission_rejected_total{{kind=\"{}\",reason=\"{}\"}} {value}",
                    kind.as_str(),
                    reason.as_str()
                );
            }
        }

        gauge(
            &mut out,
            "miraset_mempool_pending",
            "Items waiting in the mempool (not durable).",
        );
        let _ = writeln!(
            out,
            "miraset_mempool_pending{{kind=\"transaction\"}} {}",
            scrape.mempool_pending_txs
        );
        let _ = writeln!(
            out,
            "miraset_mempool_pending{{kind=\"receipt\"}} {}",
            scrape.mempool_pending_receipts
        );

        if let Some(dropped) = self.p2p_dropped.get() {
            counter(
                &mut out,
                "miraset_p2p_dropped_messages_total",
                "Outbound or inbound consensus messages dropped by the transport.",
            );
            let _ = writeln!(
                out,
                "miraset_p2p_dropped_messages_total {}",
                dropped.load(Ordering::Relaxed)
            );
        }

        gauge(
            &mut out,
            "miraset_safety_halted",
            "1 when consensus is halted for safety.",
        );
        let _ = writeln!(out, "miraset_safety_halted {}", u8::from(scrape.halted));
        gauge(
            &mut out,
            "miraset_draining",
            "1 while the node shuts down and refuses admission.",
        );
        let _ = writeln!(out, "miraset_draining {}", u8::from(scrape.draining));
        gauge(&mut out, "miraset_ready", "1 when /readyz reports ready.");
        let _ = writeln!(out, "miraset_ready {}", u8::from(scrape.ready));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn value(text: &str, series: &str) -> String {
        text.lines()
            .find_map(|line| {
                line.strip_prefix(series)?
                    .strip_prefix(' ')
                    .map(str::to_string)
            })
            .unwrap_or_else(|| panic!("series {series} missing in:\n{text}"))
    }

    #[test]
    fn test_commit_updates_counters_height_and_age() {
        let metrics = NodeMetrics::default();
        assert_eq!(metrics.last_commit_age(), None);
        metrics.record_commit(Some(5));
        metrics.record_commit(Some(3));
        metrics.record_commit(None);
        assert_eq!(metrics.committed_height(), 5, "height never decreases");
        assert!(metrics.last_commit_age().unwrap() < Duration::from_secs(5));
        let text = metrics.render(ScrapeState::default());
        assert_eq!(value(&text, "miraset_commits_total"), "3");
        assert_eq!(value(&text, "miraset_committed_height"), "5");
    }

    #[test]
    fn test_commit_age_is_absent_before_first_commit() {
        let text = NodeMetrics::default().render(ScrapeState::default());
        assert!(!text.contains("miraset_last_commit_age_seconds"));
    }

    #[test]
    fn test_unknown_height_commit_counts_for_age_but_not_height() {
        let metrics = NodeMetrics::default();
        metrics.record_commit(None);
        assert!(metrics.last_commit_age().is_some());
        assert_eq!(metrics.committed_height(), 0);
    }

    #[test]
    fn test_label_enum_indexes_match_all_order() {
        for (i, kind) in AdmissionKind::ALL.iter().enumerate() {
            assert_eq!(kind.index(), i);
        }
        for (i, reason) in AdmissionReject::ALL.iter().enumerate() {
            assert_eq!(reason.index(), i);
        }
    }

    #[test]
    fn test_admission_series_are_fixed_and_counted() {
        let metrics = NodeMetrics::default();
        metrics.record_admitted(AdmissionKind::Transaction);
        metrics.record_rejected(AdmissionKind::Receipt, AdmissionReject::Draining);
        metrics.record_rejected(AdmissionKind::Receipt, AdmissionReject::Draining);
        let text = metrics.render(ScrapeState::default());
        assert_eq!(
            value(
                &text,
                "miraset_admission_accepted_total{kind=\"transaction\"}"
            ),
            "1"
        );
        assert_eq!(
            value(&text, "miraset_admission_accepted_total{kind=\"receipt\"}"),
            "0"
        );
        assert_eq!(
            value(
                &text,
                "miraset_admission_rejected_total{kind=\"receipt\",reason=\"draining\"}"
            ),
            "2"
        );
        let series = text.lines().filter(|l| !l.starts_with('#')).count();
        assert_eq!(
            series, 13,
            "fixed series without a commit or the p2p counter"
        );
    }

    #[test]
    fn test_scrape_state_and_p2p_counter_are_exported() {
        let metrics = NodeMetrics::default();
        let dropped = Arc::new(AtomicU64::new(7));
        metrics.attach_p2p_dropped(Arc::clone(&dropped));
        metrics.attach_p2p_dropped(Arc::new(AtomicU64::new(99)));
        let text = metrics.render(ScrapeState {
            mempool_pending_txs: 4,
            mempool_pending_receipts: 1,
            halted: true,
            draining: false,
            ready: false,
        });
        assert_eq!(value(&text, "miraset_p2p_dropped_messages_total"), "7");
        assert_eq!(
            value(&text, "miraset_mempool_pending{kind=\"transaction\"}"),
            "4"
        );
        assert_eq!(value(&text, "miraset_safety_halted"), "1");
        assert_eq!(value(&text, "miraset_ready"), "0");
    }

    #[test]
    fn test_every_series_has_help_and_type() {
        let metrics = NodeMetrics::default();
        metrics.attach_p2p_dropped(Arc::new(AtomicU64::new(0)));
        let text = metrics.render(ScrapeState::default());
        for line in text.lines().filter(|l| !l.starts_with('#')) {
            let name = line.split(['{', ' ']).next().unwrap();
            assert!(
                text.contains(&format!("# TYPE {name} ")),
                "{name} has no TYPE"
            );
            assert!(
                text.contains(&format!("# HELP {name} ")),
                "{name} has no HELP"
            );
        }
    }
}
