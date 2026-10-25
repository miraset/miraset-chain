// Block production resource benchmark.
//
// Measures wall-time, throughput, peak process RSS and serialized block size
// for blocks with 0, 1, 100, 1_000 and 10_000 transactions (10_000 = mempool
// capacity, the practical max transactions-per-block limit). Writes a
// JSON + Markdown report to `reports/` when finished.
//
// Ignored by default so `cargo test --workspace` stays fast. Run with:
//   cargo test -p miraset-node --test block_resource_bench -- --ignored --nocapture
// or: npm run bench:resources
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use miraset_core::{KeyPair, Transaction};
use miraset_node::State;
use miraset_node::gas::GasConfig;
use serde::Serialize;

/// Practical max transactions per block: the hard-coded mempool capacity
/// (`state/mod.rs`), since block production itself has no tx cap.
const MAX_TXS_PER_BLOCK: usize = 10_000;
const REPEATS: usize = 10;
const SCENARIOS: &[usize] = &[0, 1, 100, 1_000, MAX_TXS_PER_BLOCK];

fn cheap_gas_config() -> GasConfig {
    GasConfig {
        base_fee: 1,
        per_byte_fee: 1,
        object_read_cost: 1,
        object_write_cost: 1,
        object_create_cost: 1,
        object_delete_cost: 1,
        event_cost: 1,
        storage_price_per_kb: 1,
        storage_rebate_rate: 0.99,
    }
}

/// Current resident set size in KiB (Linux only).
fn current_rss_kb() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    parse_kb_field(&status, "VmRSS:")
}

/// Peak (high-water-mark) resident set size in KiB (Linux only).
fn peak_rss_kb() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    parse_kb_field(&status, "VmHWM:").or_else(|| parse_kb_field(&status, "VmRSS:"))
}

fn parse_kb_field(status: &str, prefix: &str) -> Option<u64> {
    status.lines().find_map(|line| {
        line.strip_prefix(prefix)
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|v| v.parse::<u64>().ok())
    })
}

fn median_ms(samples: &[Duration]) -> f64 {
    let mut sorted: Vec<Duration> = samples.to_vec();
    sorted.sort_unstable();
    let mid = sorted.len() / 2;
    let median = if sorted.len().is_multiple_of(2) {
        (sorted[mid - 1] + sorted[mid]) / 2
    } else {
        sorted[mid]
    };
    median.as_secs_f64() * 1_000.0
}

fn build_transfer_batch(sender: &KeyPair, recipient: &KeyPair, count: usize) -> Vec<Transaction> {
    (0..count as u64)
        .map(|nonce| {
            let mut tx = Transaction::Transfer {
                from: sender.address(),
                to: recipient.address(),
                amount: 1,
                nonce,
                signature: [0; 64],
            };
            miraset_core::sign_transaction(&mut tx, sender).unwrap();
            tx
        })
        .collect()
}

#[derive(Serialize)]
struct ScenarioReport {
    tx_count: usize,
    repeats: usize,
    min_ms: f64,
    median_ms: f64,
    max_ms: f64,
    throughput_tx_per_s: f64,
    peak_rss_kb: Option<u64>,
    rss_before_kb: Option<u64>,
    rss_after_kb: Option<u64>,
    rss_delta_kb: Option<i64>,
    block_bytes: usize,
}

#[derive(Serialize)]
struct BenchReport {
    generated_at: DateTime<Utc>,
    os: String,
    cpu_cores: Option<usize>,
    max_txs_per_block: usize,
    repeats: usize,
    scenarios: Vec<ScenarioReport>,
}

fn run_scenario(tx_count: usize) -> ScenarioReport {
    let recipient = KeyPair::from_bytes(&[2u8; 32]);
    let txs = if tx_count == 0 {
        Vec::new()
    } else {
        let sender = KeyPair::from_bytes(&[1u8; 32]);
        build_transfer_batch(&sender, &recipient, tx_count)
    };

    let rss_before_kb = current_rss_kb();
    let mut samples: Vec<Duration> = Vec::with_capacity(REPEATS);
    let mut block_bytes = 0usize;

    // Fresh state per repetition keeps state-root cost independent of
    // cross-scenario state growth.
    for _ in 0..REPEATS {
        let mut state = State::new();
        state.set_gas_config(cheap_gas_config());
        if let Some(Transaction::Transfer { from, .. }) = txs.first() {
            let from = *from;
            // ~1_300 tokens of gas per transfer under the cheap gas config;
            // fund with a wide margin.
            state.add_balance(&from, (tx_count as i64) * 10_000 + 1_000_000);
        }

        let start = Instant::now();
        let block = if tx_count == 0 {
            state.produce_block().unwrap()
        } else {
            state
                .produce_block_with_transactions(txs.clone(), Utc::now())
                .unwrap()
        };
        samples.push(start.elapsed());
        block_bytes = bincode::serialize(&block).unwrap().len();
    }

    let rss_after_kb = current_rss_kb();
    let peak = peak_rss_kb();
    let median = median_ms(&samples);
    let throughput = if median > 0.0 {
        tx_count as f64 / (median / 1_000.0)
    } else {
        0.0
    };

    ScenarioReport {
        tx_count,
        repeats: REPEATS,
        min_ms: samples
            .iter()
            .map(|d| d.as_secs_f64() * 1_000.0)
            .fold(f64::INFINITY, f64::min),
        median_ms: median,
        max_ms: samples
            .iter()
            .map(|d| d.as_secs_f64() * 1_000.0)
            .fold(f64::NEG_INFINITY, f64::max),
        throughput_tx_per_s: throughput,
        peak_rss_kb: peak,
        rss_before_kb,
        rss_after_kb,
        rss_delta_kb: match (rss_before_kb, rss_after_kb) {
            (Some(b), Some(a)) => Some(a as i64 - b as i64),
            _ => None,
        },
        block_bytes,
    }
}

fn cpu_cores() -> Option<usize> {
    std::thread::available_parallelism().ok().map(|n| n.get())
}

fn report_paths() -> (PathBuf, PathBuf) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let dir = root.join("reports");
    std::fs::create_dir_all(&dir).expect("create reports dir");
    (
        dir.join("block-resource-bench.json"),
        dir.join("block-resource-bench.md"),
    )
}

fn write_markdown(report: &BenchReport, path: &std::path::Path) {
    use std::fmt::Write as _;
    let mut md = String::new();
    writeln!(md, "# Block Resource Benchmark").unwrap();
    writeln!(md).unwrap();
    writeln!(md, "- Generated at: {}", report.generated_at.to_rfc3339()).unwrap();
    writeln!(md, "- OS: {}", report.os).unwrap();
    writeln!(
        md,
        "- CPU cores: {}",
        report
            .cpu_cores
            .map_or_else(|| "unknown".into(), |c| c.to_string())
    )
    .unwrap();
    writeln!(md, "- Repeats per scenario: {}", report.repeats).unwrap();
    writeln!(
        md,
        "- Max txs per block (mempool cap): {}",
        report.max_txs_per_block
    )
    .unwrap();
    writeln!(md).unwrap();
    writeln!(md, "| txs/block | median ms | min ms | max ms | tx/s | block bytes | peak RSS KiB | RSS delta KiB |").unwrap();
    writeln!(md, "|---:|---:|---:|---:|---:|---:|---:|---:|").unwrap();
    for s in &report.scenarios {
        writeln!(
            md,
            "| {} | {:.2} | {:.2} | {:.2} | {:.1} | {} | {} | {} |",
            s.tx_count,
            s.median_ms,
            s.min_ms,
            s.max_ms,
            s.throughput_tx_per_s,
            s.block_bytes,
            s.peak_rss_kb.map_or_else(|| "-".into(), |v| v.to_string()),
            s.rss_delta_kb.map_or_else(|| "-".into(), |v| v.to_string()),
        )
        .unwrap();
    }
    writeln!(md).unwrap();
    writeln!(
        md,
        "Notes: `0` = empty-block baseline; `{}` = mempool capacity (practical max txs per block; block production itself has no tx cap). In-memory state, no storage/RPC. Cheap gas config.",
        MAX_TXS_PER_BLOCK
    )
    .unwrap();
    std::fs::write(path, md).expect("write markdown report");
}

#[test]
#[ignore = "resource benchmark; run via `npm run bench:resources`"]
fn block_resource_benchmark() {
    let scenarios: Vec<ScenarioReport> = SCENARIOS
        .iter()
        .map(|&n| {
            eprintln!("[bench] running scenario: {n} txs/block ...");
            run_scenario(n)
        })
        .collect();

    let report = BenchReport {
        generated_at: Utc::now(),
        os: std::env::consts::OS.to_string(),
        cpu_cores: cpu_cores(),
        max_txs_per_block: MAX_TXS_PER_BLOCK,
        repeats: REPEATS,
        scenarios,
    };

    let (json_path, md_path) = report_paths();
    let json = serde_json::to_string_pretty(&report).unwrap();
    std::fs::write(&json_path, json).expect("write json report");
    write_markdown(&report, &md_path);

    eprintln!(
        "[bench] report written:\n  {}\n  {}",
        json_path.display(),
        md_path.display()
    );
    for s in &report.scenarios {
        eprintln!(
            "[bench] txs={:<6} median={:.2}ms throughput={:.1} tx/s block={}B peak_rss={:?}KiB",
            s.tx_count, s.median_ms, s.throughput_tx_per_s, s.block_bytes, s.peak_rss_kb
        );
    }
}
