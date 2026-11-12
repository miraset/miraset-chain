//! Node metrics in Prometheus text exposition format (TODO 8.4.4, PRD R12).
//!
//! The metric set is fixed and no label takes a value from requests, peers or chain data, so
//! cardinality is bounded by construction. Counters are process-local and reset on restart.

use std::fmt::Write as _;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use super::vote_refusals::{VoteGuardLayer, VoteRefusals};
use crate::node::AdmissionError;

/// Rejection reasons for transaction and receipt admission. Fixed set of label values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionReject {
    Duplicate,
    Draining,
    Invalid,
    Replay,
    MempoolFull,
    SenderLimit,
}

impl AdmissionReject {
    const ALL: [AdmissionReject; 6] = [
        AdmissionReject::Duplicate,
        AdmissionReject::Draining,
        AdmissionReject::Invalid,
        AdmissionReject::Replay,
        AdmissionReject::MempoolFull,
        AdmissionReject::SenderLimit,
    ];

    fn as_str(self) -> &'static str {
        match self {
            AdmissionReject::Duplicate => "duplicate",
            AdmissionReject::Draining => "draining",
            AdmissionReject::Invalid => "invalid",
            AdmissionReject::Replay => "replay",
            AdmissionReject::MempoolFull => "mempool_full",
            AdmissionReject::SenderLimit => "sender_limit",
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

impl From<AdmissionError> for AdmissionReject {
    fn from(err: AdmissionError) -> Self {
        match err {
            AdmissionError::Duplicate => Self::Duplicate,
            AdmissionError::Draining => Self::Draining,
            AdmissionError::Invalid(_) => Self::Invalid,
            AdmissionError::Replay(_) => Self::Replay,
            AdmissionError::MempoolFull => Self::MempoolFull,
            AdmissionError::SenderLimit => Self::SenderLimit,
        }
    }
}

/// Why the RPC refused a submission before admission. Fixed set of label values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RpcReject {
    Unauthorized,
    RateLimited,
}

impl RpcReject {
    const ALL: [RpcReject; 2] = [RpcReject::Unauthorized, RpcReject::RateLimited];

    fn as_str(self) -> &'static str {
        match self {
            RpcReject::Unauthorized => "unauthorized",
            RpcReject::RateLimited => "rate_limited",
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
    /// Same encoding, for the last commit observed outside a block-sync session.
    last_consensus_commit_offset_ns: AtomicU64,
    syncing: AtomicBool,
    sync_sessions_total: AtomicU64,
    blocks_synced_total: AtomicU64,
    admission: [AdmissionCounters; AdmissionKind::ALL.len()],
    rpc_rejected: [AtomicU64; RpcReject::ALL.len()],
    p2p_dropped: OnceLock<Arc<AtomicU64>>,
    vote_refusals: [OnceLock<Arc<VoteRefusals>>; VoteGuardLayer::ALL.len()],
}

impl Default for NodeMetrics {
    fn default() -> Self {
        Self {
            started: Instant::now(),
            commits_total: AtomicU64::new(0),
            committed_height: AtomicU64::new(0),
            last_commit_offset_ns: AtomicU64::new(0),
            last_consensus_commit_offset_ns: AtomicU64::new(0),
            syncing: AtomicBool::new(false),
            sync_sessions_total: AtomicU64::new(0),
            blocks_synced_total: AtomicU64::new(0),
            admission: Default::default(),
            rpc_rejected: Default::default(),
            p2p_dropped: OnceLock::new(),
            vote_refusals: Default::default(),
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
    /// counts for readiness, because the block is final either way. A commit made while a
    /// block-sync session is open does not count for readiness: the node may still be behind.
    pub fn record_commit(&self, height: Option<u64>) {
        self.commits_total.fetch_add(1, Ordering::Relaxed);
        if let Some(height) = height {
            self.committed_height.fetch_max(height, Ordering::Relaxed);
        }
        let stamp = self.now_stamp();
        self.last_commit_offset_ns.store(stamp, Ordering::Release);
        if !self.is_syncing() {
            self.last_consensus_commit_offset_ns
                .store(stamp, Ordering::Release);
        }
    }

    /// Time since the last commit this process observed; `None` before the first.
    pub fn last_commit_age(&self) -> Option<Duration> {
        self.age_of(self.last_commit_offset_ns.load(Ordering::Acquire))
    }

    /// Time since the last commit observed outside block sync; `None` before the first.
    pub fn last_consensus_commit_age(&self) -> Option<Duration> {
        self.age_of(self.last_consensus_commit_offset_ns.load(Ordering::Acquire))
    }

    /// The replica entered block sync: it is behind a peer and not voting. The consensus-commit
    /// stamp is cleared, so readiness needs a fresh consensus commit after every session (a
    /// commit from before the lag would otherwise report a lagging node ready).
    pub fn record_sync_start(&self) {
        self.sync_sessions_total.fetch_add(1, Ordering::Relaxed);
        self.syncing.store(true, Ordering::Release);
        self.last_consensus_commit_offset_ns
            .store(0, Ordering::Release);
    }

    /// The replica left block sync after inserting `blocks` blocks.
    pub fn record_sync_end(&self, blocks: u64) {
        self.blocks_synced_total
            .fetch_add(blocks, Ordering::Relaxed);
        self.syncing.store(false, Ordering::Release);
    }

    /// The replica started a consensus view, so no sync session is open. Defensive only: every
    /// non-error session end publishes an end event, and an error panics the algorithm thread.
    pub fn record_view_started(&self) {
        self.syncing.store(false, Ordering::Release);
    }

    pub fn is_syncing(&self) -> bool {
        self.syncing.load(Ordering::Acquire)
    }

    /// Nanoseconds since start plus one, so zero can mean "never".
    fn now_stamp(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_nanos())
            .unwrap_or(u64::MAX - 1)
            .saturating_add(1)
    }

    fn age_of(&self, stamp: u64) -> Option<Duration> {
        match stamp {
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

    pub fn record_rpc_rejected(&self, reason: RpcReject) {
        self.rpc_rejected[reason.index()].fetch_add(1, Ordering::Relaxed);
    }

    /// Export the transport's dropped-message counter. Only the first call takes effect.
    pub fn attach_p2p_dropped(&self, counter: Arc<AtomicU64>) {
        let _ = self.p2p_dropped.set(counter);
    }

    /// Export one guard layer's phase-vote refusals. Only the first call per layer takes effect.
    pub fn attach_vote_refusals(&self, layer: VoteGuardLayer, refusals: Arc<VoteRefusals>) {
        let _ = self.vote_refusals[layer.index()].set(refusals);
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

        counter(
            &mut out,
            "miraset_rpc_rejected_total",
            "Submissions refused by the RPC before admission (API key, rate limit).",
        );
        for reason in RpcReject::ALL {
            let value = self.rpc_rejected[reason.index()].load(Ordering::Relaxed);
            let _ = writeln!(
                out,
                "miraset_rpc_rejected_total{{reason=\"{}\"}} {value}",
                reason.as_str()
            );
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

        self.render_sync(&mut out);
        self.render_vote_refusals(&mut out);

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

    fn render_sync(&self, out: &mut String) {
        let _ = writeln!(
            out,
            "# HELP miraset_syncing 1 while a block-sync session is open.\n# TYPE miraset_syncing gauge\n\
             miraset_syncing {}",
            u8::from(self.is_syncing())
        );
        let _ = writeln!(
            out,
            "# HELP miraset_sync_sessions_total Block-sync sessions started since process start.\n\
             # TYPE miraset_sync_sessions_total counter\nmiraset_sync_sessions_total {}",
            self.sync_sessions_total.load(Ordering::Relaxed)
        );
        let _ = writeln!(
            out,
            "# HELP miraset_blocks_synced_total Blocks inserted through block sync since process start.\n\
             # TYPE miraset_blocks_synced_total counter\nmiraset_blocks_synced_total {}",
            self.blocks_synced_total.load(Ordering::Relaxed)
        );
    }

    /// Only attached layers are exported; reasons are fixed.
    fn render_vote_refusals(&self, out: &mut String) {
        let attached: Vec<_> = VoteGuardLayer::ALL
            .iter()
            .filter_map(|layer| Some((*layer, self.vote_refusals[layer.index()].get()?)))
            .collect();
        if attached.is_empty() {
            return;
        }
        let _ = writeln!(
            out,
            "# HELP miraset_vote_refused_total Phase votes refused by the signing guard.\n\
             # TYPE miraset_vote_refused_total counter"
        );
        for (layer, refusals) in attached {
            for (reason, value) in [
                ("stale", refusals.stale()),
                ("conflicting", refusals.conflicting()),
                ("below_lock", refusals.below_lock()),
            ] {
                let _ = writeln!(
                    out,
                    "miraset_vote_refused_total{{layer=\"{}\",reason=\"{reason}\"}} {value}",
                    layer.as_str()
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            series, 26,
            "fixed series without a commit, the p2p counter or vote refusals"
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
        metrics.attach_vote_refusals(VoteGuardLayer::Signer, Arc::default());
        metrics.record_commit(Some(1));
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

    #[test]
    fn test_commits_during_sync_do_not_count_for_consensus_age() {
        let metrics = NodeMetrics::default();
        metrics.record_sync_start();
        metrics.record_commit(Some(10));
        assert!(metrics.is_syncing());
        assert!(metrics.last_commit_age().is_some());
        assert_eq!(metrics.last_consensus_commit_age(), None);
        metrics.record_sync_end(10);
        metrics.record_commit(Some(11));
        assert!(metrics.last_consensus_commit_age().is_some());
        let text = metrics.render(ScrapeState::default());
        assert_eq!(value(&text, "miraset_syncing"), "0");
        assert_eq!(value(&text, "miraset_sync_sessions_total"), "1");
        assert_eq!(value(&text, "miraset_blocks_synced_total"), "10");
    }

    #[test]
    fn test_sync_start_discards_an_earlier_consensus_commit() {
        let metrics = NodeMetrics::default();
        metrics.record_commit(Some(5));
        assert!(metrics.last_consensus_commit_age().is_some());
        metrics.record_sync_start();
        metrics.record_sync_end(0);
        assert_eq!(metrics.last_consensus_commit_age(), None);
        assert!(
            metrics.last_commit_age().is_some(),
            "the any-commit age is kept"
        );
    }

    #[test]
    fn test_view_start_clears_a_sync_left_open() {
        let metrics = NodeMetrics::default();
        metrics.record_sync_start();
        metrics.record_view_started();
        assert!(!metrics.is_syncing());
    }

    #[test]
    fn test_vote_refusals_exported_per_attached_layer() {
        let metrics = NodeMetrics::default();
        let transport = Arc::new(VoteRefusals::default());
        transport.record_stale();
        transport.record_conflicting();
        transport.record_conflicting();
        metrics.attach_vote_refusals(VoteGuardLayer::Transport, Arc::clone(&transport));
        let text = metrics.render(ScrapeState::default());
        assert_eq!(
            value(
                &text,
                "miraset_vote_refused_total{layer=\"transport\",reason=\"stale\"}"
            ),
            "1"
        );
        assert_eq!(
            value(
                &text,
                "miraset_vote_refused_total{layer=\"transport\",reason=\"conflicting\"}"
            ),
            "2"
        );
        assert!(
            !text.contains("layer=\"signer\""),
            "unattached layer (remote signer) is absent"
        );
    }
}
