//! Measurement-campaign metrics (TODO 8.5.3, `docs/measurement-plan.md` I1–I4, I7).
//!
//! - **I1 block shape:** transactions, receipts and payload bytes per committed block, and how
//!   many committed blocks were empty (the main driver of idle disk growth).
//! - **I2 view health:** current view, view timeouts, timeout certificates collected, and the
//!   interval between consecutive commits.
//! - **I3 storage:** chain database and signing-guard file sizes, read at scrape time.
//! - **I4 process:** resident memory, CPU seconds, open fds, threads and bytes written, from
//!   `/proc/self` at scrape time (Linux; absent elsewhere).
//! - **I7 transport traffic:** P2P bytes sent and received (counters owned by the transport).
//! - **Block timing (stress test, 2026-11-13):** how long a leader waited before building a
//!   block (block-rate limit and heartbeat), how long it took to build it, and how long
//!   validating a proposal took, to locate where a view's time goes.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// Fixed-bucket histogram over non-negative integer observations. The last bucket counts
/// values above every bound, so `+Inf` is always the cumulative total of the buckets.
pub(crate) struct Histogram {
    bounds: &'static [u64],
    buckets: Vec<AtomicU64>,
    count: AtomicU64,
    sum: AtomicU64,
}

impl Histogram {
    pub(crate) fn new(bounds: &'static [u64]) -> Self {
        Self {
            bounds,
            buckets: (0..=bounds.len()).map(|_| AtomicU64::new(0)).collect(),
            count: AtomicU64::new(0),
            sum: AtomicU64::new(0),
        }
    }

    pub(crate) fn observe(&self, value: u64) {
        let index = self
            .bounds
            .iter()
            .position(|bound| value <= *bound)
            .unwrap_or(self.bounds.len());
        self.buckets[index].fetch_add(1, Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Relaxed);
        self.sum.fetch_add(value, Ordering::Relaxed);
    }

    /// Render as a Prometheus histogram; `scale` divides bounds and sum (e.g. 1000 for ms → s).
    fn render(&self, out: &mut String, name: &str, help: &str, scale: f64) {
        let _ = writeln!(out, "# HELP {name} {help}\n# TYPE {name} histogram");
        let mut cumulative = 0;
        for (bound, bucket) in self.bounds.iter().zip(&self.buckets) {
            cumulative += bucket.load(Ordering::Relaxed);
            let le = *bound as f64 / scale;
            let _ = writeln!(out, "{name}_bucket{{le=\"{le}\"}} {cumulative}");
        }
        cumulative += self.buckets[self.bounds.len()].load(Ordering::Relaxed);
        let _ = writeln!(out, "{name}_bucket{{le=\"+Inf\"}} {cumulative}");
        let sum = self.sum.load(Ordering::Relaxed) as f64 / scale;
        let _ = writeln!(out, "{name}_sum {sum}");
        let _ = writeln!(out, "{name}_count {cumulative}");
    }
}

// Blocks hold up to 8 MiB (≈ 23 000 transfers) since 2026-11-13.
const TXS_BOUNDS: &[u64] = &[
    0, 1, 5, 10, 25, 50, 100, 250, 500, 1_000, 2_500, 5_000, 10_000, 15_000, 20_000, 25_000,
];
const BYTES_BOUNDS: &[u64] = &[
    256, 1_024, 4_096, 16_384, 65_536, 262_144, 1_048_576, 2_097_152, 4_194_304, 6_291_456,
    8_388_608,
];
/// Validation phase names, in [`ValidationPhases`] field order.
const VALIDATE_PHASES: [&str; 4] = ["decode", "signatures", "execute", "finish"];
/// Block production wait/build and validation times (ms).
const STEP_MS_BOUNDS: &[u64] = &[
    1, 5, 10, 25, 50, 100, 250, 500, 750, 1_000, 1_500, 2_000, 3_000, 5_000,
];
const INTERVAL_MS_BOUNDS: &[u64] = &[50, 100, 250, 500, 1_000, 2_000, 5_000, 10_000, 30_000];

/// Transport byte counters (I7), owned by the P2P transport and shared with metrics.
#[derive(Debug, Default)]
pub struct TransportTraffic {
    sent: AtomicU64,
    received: AtomicU64,
}

impl TransportTraffic {
    /// Bytes of an established-session frame written to a peer.
    pub fn record_sent(&self, bytes: usize) {
        self.sent.fetch_add(bytes as u64, Ordering::Relaxed);
    }

    /// Bytes of an established-session frame read and decrypted from a peer.
    pub fn record_received(&self, bytes: usize) {
        self.received.fetch_add(bytes as u64, Ordering::Relaxed);
    }

    /// Total bytes sent.
    pub fn sent(&self) -> u64 {
        self.sent.load(Ordering::Relaxed)
    }

    /// Total bytes received.
    pub fn received(&self) -> u64 {
        self.received.load(Ordering::Relaxed)
    }
}

/// Files whose size is exported (I3).
#[derive(Clone, Debug)]
pub struct StorageFiles {
    /// The chain database (redb; its file length grows in allocation steps).
    pub chain_db: PathBuf,
    /// The signing guard, when this node has one.
    pub signing_guard: Option<PathBuf>,
}

/// Where a live proposal's validation spent its time, by phase.
#[derive(Debug, Default, Clone, Copy)]
pub struct ValidationPhases {
    /// Size check, decoding, canonical re-encoding check and payload hash.
    pub decode: Duration,
    /// Parallel signature pass.
    pub signatures: Duration,
    /// Executing transactions and receipts against state.
    pub execute: Duration,
    /// Writing changed state and computing the state commitment.
    pub finish: Duration,
}

/// Measurement-campaign metrics owned by [`super::NodeMetrics`].
pub struct RuntimeMetrics {
    block_txs: Histogram,
    block_receipts: Histogram,
    block_bytes: Histogram,
    empty_blocks_total: AtomicU64,
    unreadable_blocks_total: AtomicU64,
    commit_interval_ms: Histogram,
    produce_wait_ms: Histogram,
    produce_build_ms: Histogram,
    validate_ms: Histogram,
    validate_phase_ms: [Histogram; 4],
    last_commit: Mutex<Option<Instant>>,
    view: AtomicU64,
    view_timeouts_total: AtomicU64,
    timeout_certificates_total: AtomicU64,
    storage: OnceLock<StorageFiles>,
    traffic: OnceLock<Arc<TransportTraffic>>,
}

impl std::fmt::Debug for RuntimeMetrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeMetrics")
            .field("view", &self.view.load(Ordering::Relaxed))
            .field(
                "empty_blocks_total",
                &self.empty_blocks_total.load(Ordering::Relaxed),
            )
            .finish_non_exhaustive()
    }
}

impl Default for RuntimeMetrics {
    fn default() -> Self {
        Self {
            block_txs: Histogram::new(TXS_BOUNDS),
            block_receipts: Histogram::new(TXS_BOUNDS),
            block_bytes: Histogram::new(BYTES_BOUNDS),
            empty_blocks_total: AtomicU64::new(0),
            unreadable_blocks_total: AtomicU64::new(0),
            commit_interval_ms: Histogram::new(INTERVAL_MS_BOUNDS),
            produce_wait_ms: Histogram::new(STEP_MS_BOUNDS),
            produce_build_ms: Histogram::new(STEP_MS_BOUNDS),
            validate_ms: Histogram::new(STEP_MS_BOUNDS),
            validate_phase_ms: std::array::from_fn(|_| Histogram::new(STEP_MS_BOUNDS)),
            last_commit: Mutex::new(None),
            view: AtomicU64::new(0),
            view_timeouts_total: AtomicU64::new(0),
            timeout_certificates_total: AtomicU64::new(0),
            storage: OnceLock::new(),
            traffic: OnceLock::new(),
        }
    }
}

impl RuntimeMetrics {
    /// A committed block's payload shape (I1). Counts every committed block on this node,
    /// including blocks committed through block sync.
    pub fn record_block(&self, txs: usize, receipts: usize, payload_bytes: usize) {
        self.block_txs.observe(txs as u64);
        self.block_receipts.observe(receipts as u64);
        self.block_bytes.observe(payload_bytes as u64);
        if txs == 0 && receipts == 0 {
            self.empty_blocks_total.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// A committed block whose payload could not be read (it is committed but not shaped).
    pub fn record_unreadable_block(&self) {
        self.unreadable_blocks_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Time of a commit (I2). Only consensus commits form intervals: a block-sync burst would
    /// fill the lowest buckets with historical blocks, so it resets the interval instead.
    pub fn record_commit_time(&self, at: Instant, via_consensus: bool) {
        let Ok(mut last) = self.last_commit.lock() else {
            return;
        };
        if !via_consensus {
            *last = None;
            return;
        }
        if let Some(previous) = last.replace(at) {
            let interval = at.saturating_duration_since(previous).as_millis();
            self.commit_interval_ms
                .observe(u64::try_from(interval).unwrap_or(u64::MAX));
        }
    }

    /// A block this node produced as leader: time spent waiting before building it (block-rate
    /// limit, heartbeat) and building it.
    pub fn record_block_production(&self, waited: Duration, built: Duration) {
        self.produce_wait_ms.observe(millis(waited));
        self.produce_build_ms.observe(millis(built));
    }

    /// A live proposal this node validated (block sync excluded), and its phases when it was
    /// valid (an invalid one stops early, so its phases are not recorded).
    pub fn record_block_validation(&self, took: Duration, phases: Option<&ValidationPhases>) {
        self.validate_ms.observe(millis(took));
        let Some(phases) = phases else {
            return;
        };
        let [decode, signatures, execute, finish] = &self.validate_phase_ms;
        decode.observe(millis(phases.decode));
        signatures.observe(millis(phases.signatures));
        execute.observe(millis(phases.execute));
        finish.observe(millis(phases.finish));
    }

    /// The view this node entered.
    pub fn record_view(&self, view: u64) {
        self.view.fetch_max(view, Ordering::Relaxed);
    }

    /// A view of this node timed out.
    pub fn record_view_timeout(&self) {
        self.view_timeouts_total.fetch_add(1, Ordering::Relaxed);
    }

    /// This node collected a timeout certificate.
    pub fn record_timeout_certificate(&self) {
        self.timeout_certificates_total
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Export file sizes for these paths (first call wins).
    pub fn attach_storage(&self, files: StorageFiles) {
        let _ = self.storage.set(files);
    }

    /// Export transport byte counters (first call wins).
    pub fn attach_traffic(&self, traffic: Arc<TransportTraffic>) {
        let _ = self.traffic.set(traffic);
    }

    pub(crate) fn render(&self, out: &mut String) {
        self.block_txs.render(
            out,
            "miraset_block_txs",
            "Transactions per committed block.",
            1.0,
        );
        self.block_receipts.render(
            out,
            "miraset_block_receipts",
            "Compute receipts per committed block.",
            1.0,
        );
        self.block_bytes.render(
            out,
            "miraset_block_payload_bytes",
            "Payload bytes per committed block.",
            1.0,
        );
        self.commit_interval_ms.render(
            out,
            "miraset_commit_interval_seconds",
            "Time between consecutive consensus commits on this node (block sync excluded).",
            1000.0,
        );
        self.produce_wait_ms.render(
            out,
            "miraset_block_produce_wait_seconds",
            "Time a leader waited before building a block (block-rate limit, heartbeat).",
            1000.0,
        );
        self.produce_build_ms.render(
            out,
            "miraset_block_produce_build_seconds",
            "Time a leader took to build a block (selection, execution, encoding).",
            1000.0,
        );
        self.validate_ms.render(
            out,
            "miraset_block_validate_seconds",
            "Time this node took to validate a live proposal.",
            1000.0,
        );
        for (histogram, phase) in self.validate_phase_ms.iter().zip(VALIDATE_PHASES) {
            histogram.render(
                out,
                &format!("miraset_block_validate_{phase}_seconds"),
                &format!("Time a live proposal's validation spent in its {phase} phase."),
                1000.0,
            );
        }
        counter(
            out,
            "miraset_empty_blocks_total",
            "Committed blocks with no transaction and no receipt.",
            self.empty_blocks_total.load(Ordering::Relaxed),
        );
        counter(
            out,
            "miraset_unreadable_blocks_total",
            "Committed blocks whose payload could not be read for metrics.",
            self.unreadable_blocks_total.load(Ordering::Relaxed),
        );
        gauge(
            out,
            "miraset_view",
            "Highest consensus view this node has entered.",
            self.view.load(Ordering::Relaxed),
        );
        counter(
            out,
            "miraset_view_timeouts_total",
            "Consensus views that timed out on this node.",
            self.view_timeouts_total.load(Ordering::Relaxed),
        );
        counter(
            out,
            "miraset_timeout_certificates_total",
            "Timeout certificates this node collected.",
            self.timeout_certificates_total.load(Ordering::Relaxed),
        );
        if let Some(files) = self.storage.get() {
            render_storage(out, files);
        }
        if let Some(traffic) = self.traffic.get() {
            let _ = writeln!(
                out,
                "# HELP miraset_p2p_bytes_total P2P bytes on the wire, by direction.\n\
                 # TYPE miraset_p2p_bytes_total counter\n\
                 miraset_p2p_bytes_total{{direction=\"sent\"}} {}\n\
                 miraset_p2p_bytes_total{{direction=\"received\"}} {}",
                traffic.sent(),
                traffic.received()
            );
        }
        render_process(out);
    }
}

fn counter(out: &mut String, name: &str, help: &str, value: u64) {
    let _ = writeln!(
        out,
        "# HELP {name} {help}\n# TYPE {name} counter\n{name} {value}"
    );
}

fn gauge(out: &mut String, name: &str, help: &str, value: u64) {
    let _ = writeln!(
        out,
        "# HELP {name} {help}\n# TYPE {name} gauge\n{name} {value}"
    );
}

fn render_storage(out: &mut String, files: &StorageFiles) {
    let _ = writeln!(
        out,
        "# HELP miraset_storage_bytes Size of the node's database files.\n\
         # TYPE miraset_storage_bytes gauge"
    );
    let mut line = |file: &str, path: &PathBuf| {
        if let Ok(meta) = std::fs::metadata(path) {
            let _ = writeln!(
                out,
                "miraset_storage_bytes{{file=\"{file}\"}} {}",
                meta.len()
            );
        }
    };
    line("chain_db", &files.chain_db);
    if let Some(guard) = &files.signing_guard {
        line("signing_guard", guard);
    }
}

/// Process resources from `/proc/self` (I4). Nothing is emitted where procfs is unavailable.
fn render_process(out: &mut String) {
    let Some(stats) = ProcessStats::read() else {
        return;
    };
    gauge(
        out,
        "miraset_process_resident_bytes",
        "Resident memory of the node process.",
        stats.resident_bytes,
    );
    let _ = writeln!(
        out,
        "# HELP miraset_process_cpu_seconds_total User and system CPU time of the node process.\n\
         # TYPE miraset_process_cpu_seconds_total counter\n\
         miraset_process_cpu_seconds_total {}",
        stats.cpu_seconds
    );
    gauge(
        out,
        "miraset_process_open_fds",
        "Open file descriptors of the node process.",
        stats.open_fds,
    );
    gauge(
        out,
        "miraset_process_threads",
        "Threads of the node process.",
        stats.threads,
    );
    if let Some(written) = stats.write_bytes {
        counter(
            out,
            "miraset_process_write_bytes_total",
            "Bytes the node process caused to be written to storage.",
            written,
        );
    }
}

#[derive(Debug, PartialEq)]
pub(crate) struct ProcessStats {
    resident_bytes: u64,
    cpu_seconds: f64,
    open_fds: u64,
    threads: u64,
    write_bytes: Option<u64>,
}

impl ProcessStats {
    fn read() -> Option<Self> {
        let status = std::fs::read_to_string("/proc/self/status").ok()?;
        let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
        // The directory listing itself holds one descriptor while it is read.
        let open_fds = (std::fs::read_dir("/proc/self/fd").ok()?.count() as u64).saturating_sub(1);
        let io = std::fs::read_to_string("/proc/self/io").ok();
        // SAFETY: sysconf has no preconditions; it only reads a system constant.
        let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
        Self::parse(&status, &stat, io.as_deref(), open_fds, ticks.max(1) as u64)
    }

    pub(crate) fn parse(
        status: &str,
        stat: &str,
        io: Option<&str>,
        open_fds: u64,
        ticks_per_second: u64,
    ) -> Option<Self> {
        let field = |text: &str, key: &str| {
            text.lines()
                .find_map(|line| line.strip_prefix(key))
                .and_then(|rest| rest.split_whitespace().next())
                .and_then(|value| value.parse::<u64>().ok())
        };
        let resident_kib = field(status, "VmRSS:")?;
        let threads = field(status, "Threads:")?;
        // Fields after the parenthesised command name; utime and stime are fields 14 and 15.
        let after_comm = stat.rsplit_once(')')?.1;
        let fields: Vec<&str> = after_comm.split_whitespace().collect();
        let utime: u64 = fields.get(11)?.parse().ok()?;
        let stime: u64 = fields.get(12)?.parse().ok()?;
        Some(Self {
            resident_bytes: resident_kib * 1024,
            cpu_seconds: (utime + stime) as f64 / ticks_per_second as f64,
            open_fds,
            threads,
            write_bytes: io.and_then(|io| field(io, "write_bytes:")),
        })
    }
}

#[cfg(test)]
#[path = "runtime_metrics_tests.rs"]
mod tests;
