# Measurement plan: stability, reliability, resources, TPS, disk growth

**Status:** approved 2026-11-13 with the owner decisions in §9. Extends TODO 8.5.3 and
`docs/performance-baseline.md` (measured 2026-11-10, before the consensus changes of 8.5.2/8.4.1).
**Scope:** the current code (HEAD after 8.4.1) on the Docker devnet, then on dedicated hosts.
No number here is a target; targets need owner approval through the requirements gate.

## 1. Why re-measure now

The 2026-11-10 baseline predates changes that move every number:

| Change | Expected effect |
|---|---|
| Pacemaker `consensus_epoch_length` 10 → 1 (8.5.2 F2) | shorter stalls after a failed view; more timeout certificates under loss; **far more empty blocks** (an idle devnet reached height 1032 in ~40 s) |
| Noise transport, encrypted chunks, attestations (8.4.1) | CPU per message, handshake cost on reconnect |
| Byte budgets and backpressure (F6), sync limits (F4/F5) | memory ceiling, catch-up speed |
| Lock floor in the signing guard (F1) | one extra durable write per vote (already 1 fsync) |

Empty-block production rate is the first thing to pin down: at tens of blocks per second, an idle
chain's disk growth could dominate every estimate below.

## 2. What we already have

| Need | Existing tool |
|---|---|
| Offered load, latency from send to commit | `miraset-chain loadgen` (`src/loadgen/`), open-loop, transfers + receipts |
| Rate sweep with environment record | `scripts/devnet/bench.sh` + `bench_summary.py` |
| Long run sampling RSS, fds, data size, height, restarts | `scripts/devnet/soak.sh` + `soak_summary.py` (never run end to end) |
| WAN emulation | `scripts/devnet/netem.sh` (`lan`, `wan`, `wan-far`) |
| Fault injection + per-height agreement | `scripts/devnet/fault_suite.sh`, `check_consensus.py` |
| Node metrics | `/metrics`: commits, height, last-commit age, admission/RPC rejections, mempool, sync, vote refusals, P2P drops and rejections, halt/ready |
| Logs | `--log-format json`, `RUST_LOG`, safety target always on at `warn` |
| Storage | `db inspect / verify / compact / backup` |

## 3. Gaps to close before measuring (instrumentation)

| # | Gap | Proposed addition | Size |
|---|---|---|---|
| I1 | No per-block shape | `miraset_block_txs` / `miraset_block_receipts` / `miraset_block_bytes` histograms; `miraset_empty_blocks_total` | S |
| I2 | No commit latency or view health on `/metrics` | `miraset_view` gauge, `miraset_view_timeouts_total`, `miraset_timeout_certificates_total`, commit-interval histogram (block-to-block) | S |
| I3 | Disk size only from outside (`du`) | `miraset_chain_db_bytes`, `miraset_guard_db_bytes` gauges (file size at scrape) | S |
| I4 | Process resources only via Docker | `miraset_process_resident_bytes`, `_cpu_seconds_total`, `_open_fds`, thread count (procfs on Linux) | S |
| I5 | No time series kept | Prometheus + Grafana profile in compose (`--profile observe`), 15 s scrape, kept per run | M |
| I6 | Logs are not aggregated per run | soak/bench scripts save `docker compose logs --timestamps` per node, JSON format; summary counts warn/error by target | S |
| I7 | Transport load not visible | per-peer bytes sent/received counters (`miraset_p2p_bytes_total{direction}`) | S |

## 4. Measurements

### 4.1 Throughput (TPS) and latency
- **Sweep** `bench.sh` at offered rates 25 → 50 → 100 → 200 → 400 → 800 → 1600 items/s until
  saturation (committed rate < 95 % of offered, or p99 latency keeps rising), for `lan`, `wan`,
  `wan-far`; 120 s per rate after 20 s warm-up; 3 repetitions; workload mix as the baseline
  (transfers + 15 % receipts).
- **Report:** committed TPS (sustained, not peak), p50/p95/p99/max latency send → commit, longest
  block gap, mean/max txs per block, rejected/dropped items, agreement at every height.
- **Ceiling analysis:** where the bottleneck is (leader block production, 100-tx block cap
  `MAX_BLOCK_TXS`, mempool admission, signature checks, fsync) from CPU profiles
  (`perf record` / `cargo flamegraph` on one validator at the saturation rate).

### 4.2 Stability (soak)
- **Validate** `soak.sh --duration 30m` first, then **24 h** at 50 % of the measured LAN saturation
  rate, then **72 h** at the same rate (owner's machine).
- **Pass/flag criteria (evidence, not targets):** no restarts, no safety halts, agreement at every
  height, RSS slope ≈ 0 after warm-up (linear fit over the last 80 %), fd count flat, p99 latency
  per hour does not trend upward, block gaps stay within the short-run distribution.

### 4.3 Reliability (fault behaviour under load)
- Run `fault_suite.sh` **while** `loadgen` offers 50 % of saturation, on `lan` and `wan`.
- Add scenarios: leader killed in its own view, two sequential crashes (still ≤ f at a time),
  disk-full on one validator (expect safety halt, exit 3, recovery after space is freed),
  clock skew of 30 s and 90 s on one validator (attestation tolerance 60 s; expect 90 s to be
  refused and visible as `bad_attestation`), remote-signer restart under load.
- **Report per fault:** time to detect (readiness/metrics), commit gap during the fault, time to
  recover full agreement, items lost or duplicated (loadgen accounting), rejection counters.

### 4.4 System resources
- Per validator, sampled every 15 s through I4/I5 (and `docker stats` as cross-check): CPU %,
  RSS, open fds, threads, disk write bytes/s and fsyncs/s (`/proc/<pid>/io`), network bytes/s (I7).
- Measured at idle, at each sweep rate, and across the soak; reported as resource per committed
  item (CPU-ms/tx, bytes written/tx, network bytes/tx) so it extrapolates.

### 4.5 Disk growth and its estimate
Model (fit from measurements, then checked against the soak):

```
bytes/day = B_empty × empty_blocks/day
          + (B_block + b_tx × txs/block + b_rc × receipts/block) × loaded_blocks/day
          + state growth (new accounts, workers, epochs)  + guard (constant)
```

- **Measure** `B_empty` (chain DB bytes per empty block incl. certificate and indexes) on an idle
  devnet for 30 min; `b_tx`, `b_rc` from runs at fixed rates; read growth from I3 every 15 s and
  fit by linear regression (report slope ± error).
- **Check** that `db compact` reclaims space (redb free pages) and record the before/after ratio.
- **Produce** a table: GB/day and GB/year per validator at idle, 10, 100 and saturation TPS,
  plus the projection for the 72 h soak, compared against what the soak actually wrote.
- Expected first finding: idle growth is driven by empty blocks at the current block rate; if
  that is large, the options in §8 (empty-block suppression, pruning) become owner decisions.

## 5. Logging plan
- All validators run `--log-format json`, `RUST_LOG=info` (the safety target is always on).
- Each run stores per-node logs with timestamps next to its reports; the summary counts lines by
  level and target, and lists every `warn`/`error` message kind with first/last time.
- Must-be-zero in normal runs: `SAFETY HALT`, guard refusals other than after deliberate wipes,
  `p2p_rejected_total` other than injected faults; anything else is a finding.
- Check that no log line contains key material (grep for hex of the test keys) — regression test
  for custody K-rules.

## 6. Analysis and reporting
- One results directory per run: environment (hardware, kernel, Docker, git revision, profile),
  loadgen JSON, Prometheus snapshot (or CSV export), logs, `db inspect` before/after, summary.
- `docs/performance-baseline.md` gets a new dated section per campaign; findings are numbered and
  routed to TODO items; nothing is presented as a target until approved.
- Regression guard: a short CI-sized run (5 min, LAN, fixed rate) whose numbers are compared with
  the last baseline; >20 % worse TPS or p99, or any agreement failure, fails the check.

## 7. Order of work and effort (estimates)

| Step | Content | Estimate |
|---|---|---|
| 1 | Instrumentation I1–I4, I7 (metrics + tests) | 1 day |
| 2 | Observe profile (Prometheus/Grafana), log capture in scripts (I5, I6) | 0.5 day |
| 3 | Idle block-rate and disk baseline (30 min) → first disk model | 0.5 day |
| 4 | TPS/latency sweep LAN/WAN/WAN-far ×3 + CPU profile at saturation | 1 day (mostly machine time) |
| 5 | Fault suite under load + new fault scenarios | 1 day |
| 6 | Soak 30 min → 24 h (machine time) | 1 day elapsed |
| 7 | Report: disk table, resource-per-item, findings to TODO | 0.5 day |
| 8 | 72 h soak on a dedicated machine (owner) | 3 days elapsed |

## 8. Decisions needed from the owner
1. **Where to measure:** this Docker host only, or also dedicated hosts / real multi-host WAN.
2. **Soak length on this machine:** 24 h here and 72 h on a dedicated machine, or 72 h here.
3. **Targets:** propose TPS/finality/disk targets after step 4, or keep "baseline only".
4. **Empty blocks:** if idle disk growth is large, whether to change block production (propose
   only when there is work, keeping liveness via timeouts) — a consensus-behaviour change.
5. **Observe stack:** add Prometheus + Grafana to `docker-compose.yml` as an opt-in profile.

## 9. Owner decisions (2026-11-13)

| # | Decision |
|---|---|
| 1 | Measure on this Docker host first; dedicated or multi-host runs later with the same scripts. |
| 2 | Soak on this machine: 30 min only (tooling validation). The owner runs the long soaks (24 h / 72 h). |
| 3 | Add Prometheus + Grafana to `docker-compose.yml` as an opt-in `observe` profile. |
| 4 | Empty blocks: measure first and report numbers and options; no consensus change without a decision. |

