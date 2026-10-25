# Plan: Block Resource Benchmark

**Status:** approved 2026-10-25.

## Goal

Measure and report how many resources block production needs, comparing
0 transactions per block vs the maximum practical transactions-per-block
limit (10,000 = mempool cap). Generate a report after the benchmark
completes.

## Research findings (code-first)

- Block production is in-process: `State::produce_block()` /
  `State::produce_block_with_transactions()`
  (`crates/miraset-node/src/state/mod.rs:292-306`). No RPC/node process
  needed for the benchmark.
- **No max-transactions-per-block limit exists** in block production
  (all pending txs included FIFO, no cap). The only practical cap is the
  mempool capacity of 10,000 (`state/mod.rs:65`, `validation.rs:68`).
- Gas is a flat per-tx fee in `state/execution.rs:25-43`; no block-level
  gas cap.
- No existing bench infrastructure: no `benches/`, no criterion, no
  report writers.
- Test template with cheap gas config:
  `crates/miraset-node/tests/state_tests.rs:8-20`.
- Batch path `produce_block_with_transactions` allows sequential nonces
  from one sender in one batch (`state/mod.rs:322-344`).

## Implementation

### 1. `crates/miraset-node/tests/block_resource_bench.rs` (new)

- `#[ignore]` integration test so `cargo test --workspace` stays fast.
- Module-level `#![allow(clippy::unwrap_used, clippy::expect_used)]`
  per repo convention for tests.
- Cheap gas config helper (all costs = 1), like `state_tests.rs`.
- Scenarios: tx counts N in {0, 1, 100, 1_000, 10_000}; 10k = mempool
  cap used as "max limit".
- Tx batch built with `produce_block_with_transactions` using
  sequential nonces from one funded devnet sender
  (`KeyPair::from_bytes(&[1u8;32])`, funded via `add_balance`).
- Each scenario measured over K = 10 repetitions; metrics:
  - wall-time of `produce_block` (min / median / max, ms)
  - throughput (tx/s)
  - peak process RSS (Linux: `VmHWM` from `/proc/self/status`;
    fallback `VmRSS`; `null` on other OSes)
  - RSS delta before/after scenario
  - serialized block size (bincode)
- After completion writes:
  - `reports/block-resource-bench.json` (machine-readable)
  - `reports/block-resource-bench.md` (summary table + system info:
    OS, CPU core count, timestamp)
- Report path resolved from `CARGO_MANIFEST_DIR/../..` (repo root).

### 2. `package.json`

Add script:

```
"bench:resources": "cargo test -p miraset-node --test block_resource_bench -- --ignored --nocapture"
```

### 3. `README.md`

Short "Resource Benchmark" section with the run command and report
location.

## Verification

1. `cargo test -p miraset-node --test block_resource_bench -- --ignored --nocapture`
2. Check generated `reports/block-resource-bench.{json,md}`.
3. `npm run lint` (clippy unwrap/expect gates).
4. `cargo fmt --all -- --check`.

## Architecture choice

KISS: a single in-process integration test + JSON/Markdown report.
No new dependencies (criterion rejected), no changes to production code.
