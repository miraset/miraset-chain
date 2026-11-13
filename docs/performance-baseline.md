# Devnet performance baseline and measurement procedure (TODO 8.5.3)

**Status:** measured devnet baseline. **No throughput, latency or stability target is approved**
(PRD: no target until workload, hardware and procedure are approved through the requirements
gate). The numbers below describe one laptop running five containers; they are not production
evidence, and the drafts in `docs/HotStuff-rs.md` §7 (≥1000 TPS, p50 < 1.5 s WAN) remain drafts.

Decisions (2026-11-10, project owner): measure on the local Docker devnet with `tc netem` WAN
emulation; workload is a transfer + compute-receipt mix; record a baseline only; provide a soak
script and leave the 72-hour run to the owner. The short validation soak was skipped by the
owner (2026-11-10); `soak.sh` has not been run end to end yet.

## Workload

`miraset-chain loadgen` (`src/loadgen/`) offers an **open-loop** load at a fixed rate:

| Property | Value |
|---|---|
| Item mix | transfers and coordinator-signed compute receipts; `--receipt-share` (default 0.15, close to the block caps of 100 transfers and 20 receipts) |
| Senders | `--senders` throwaway Ed25519 accounts (default 32), funded from the faucet before the run; each sends transfers to the next one |
| Receipts | one stream per node; each receipt debits a sender as client (cost 1), credits a per-stream worker (reward 1), 100 tokens |
| Placement | each sender and each receipt stream is pinned to one node. Nodes do not gossip mempools, so an item is proposed only when its node leads a view |
| Rate | `--rate` items/s across all nodes, split evenly over generators; each generator keeps one request in flight |
| Fee | 10 on every transfer (no fee competition) |

**Latency** is measured from the item's **scheduled** send time (so a slow node's queueing is
counted, not hidden) to the arrival of the first `commit_block` event on the observer node
(`/v1/events/blocks` of the first `--rpc`) after which committed state shows the item: the sender's
account nonce has passed the transfer's nonce, or `/v1/tasks/{id}` reports the receipt settled.
Commit events are published after the block's state is written. Stamping error is bounded by the
observer read round (`max_observer_round_ms`, `mean_observer_round_ms`). Items never observed
committed are reported as `uncommitted` and are **excluded from the percentiles**; read the two
together.

**Window:** the first `--warmup-secs` are excluded. `offered`/`admitted`/`rejected`/latency count
items *scheduled* in the window; `committed_per_sec` counts commits *observed* in the window
(throughput, including warm-up items).

Robustness: a dropped commit stream is re-established (`observer_reconnects`); a send that fails
in transport is re-sent only after committed state shows it did not already commit; a receipt
the chain refuses does not hide later settled ones. Memory is bounded for long soaks (counters
plus 4 bytes per in-window latency sample), and progress is logged every 60 s.

## Environment and procedure

```bash
docker compose down -v && docker compose up -d --build   # fresh 5-validator chain
scripts/devnet/bench.sh --profile lan --rates "25 50 100 200 400" --duration 90 --warmup 15
scripts/devnet/bench.sh --profile wan --rates "25 50 100 200 400" --duration 90 --warmup 15
scripts/devnet/soak.sh --duration 72h --rate 20            # not yet run (see below)
```

Each run writes `bench-results/<run>/` (git-ignored): `environment.txt` (git revision and whether
the tree had uncommitted changes, image ID, CPU, memory, kernel, Docker, netem state), one JSON
report per rate, loadgen logs, and `summary.md`. After every rate `check_consensus.py` requires
all five validators to agree at every height (with shaping cleared during the check only: a
per-height check through netem takes hours).

WAN profiles (`scripts/devnet/netem.sh`, applied to each validator's egress through a sidecar
that shares its network namespace): `wan` 25 ms ± 5 ms normal, 0.1% loss per egress (~50 ms
validator RTT); `wan-far` 75 ms ± 15 ms, 0.5% loss (~150 ms RTT). The load generator is not
shaped: its requests arrive undelayed, responses and commit events are delayed once. All five
validators share one host's CPUs, so CPU contention is part of every result.

## Results (2026-11-10)

Host: Intel Core i7-10870H (8 cores / 16 threads), 30 GiB RAM, Linux 7.0.0-42-generic,
Docker 29.7.2; image `sha256:cd034ec08255…` for both sweeps (code of 0a8f654). Load: 32
senders, receipt share 0.15, 90 s per rate with the first 15 s excluded. Latencies in ms.
Every admitted item committed (`uncommitted` 0, `rejected` 0), and all five validators agreed
at every height after every rate.

**LAN (no shaping)**

| offered/s | committed/s | tx p50 ms | tx p95 ms | tx p99 ms | rcpt p50 ms | rcpt p95 ms | rejected | uncommitted | block interval ms | txs/block | gen lag ms | observer round ms | reconnects |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 25 | 25.06 | 175.2 | 361.8 | 429.2 | 174.4 | 282 | 0 | 0 | 29.8 | 0.6 | 4.4 | 10.6 | 0 |
| 50 | 50.14 | 201.4 | 498.6 | 607.4 | 194.9 | 498.8 | 0 | 0 | 33.8 | 1.4 | 9.3 | 15.2 | 0 |
| 100 | 99.91 | 211.4 | 647.1 | 835.6 | 206.4 | 671.5 | 0 | 0 | 36.9 | 3.1 | 2.7 | 3.5 | 0 |
| 200 | 198.08 | 240.7 | 863.3 | 1080.9 | 241 | 873.3 | 0 | 0 | 43.7 | 7.3 | 3.8 | 10.4 | 0 |
| 400 | 396.79 | 300.2 | 1640.8 | 2304.9 | 300 | 1619.5 | 0 | 0 | 57.4 | 19.4 | 3.3 | 10.2 | 0 |

**WAN (`wan` profile, ~50 ms validator RTT, 0.1% loss)**; the 400/s rate was not measured (the
run was interrupted)

| offered/s | committed/s | tx p50 ms | tx p95 ms | tx p99 ms | rcpt p50 ms | rcpt p95 ms | rejected | uncommitted | block interval ms | txs/block | gen lag ms | observer round ms | reconnects |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 25 | 24.62 | 677.8 | 1330.7 | 5847.6 | 671.8 | 1330 | 0 | 0 | 109 | 2.3 | 8.4 | 276.4 | 0 |
| 50 | 50.18 | 706.9 | 1568.2 | 4273.2 | 704.1 | 1514.6 | 0 | 0 | 118.2 | 5 | 2.7 | 288.5 | 0 |
| 100 | 101.68 | 1381.4 | 14173.8 | 18250.4 | 1427.5 | 13003.5 | 0 | 0 | 193.7 | 18.6 | 2.5 | 615.7 | 0 |
| 200 | 199.27 | 2504.4 | 13833.2 | 18192.3 | 3203.2 | 10833.7 | 0 | 0 | 193.9 | 34.1 | 92.8 | 1165 | 0 |

Reading:

- LAN did not saturate up to 400/s: committed throughput follows the offered rate and blocks
  stay far below the 100-transfer cap (mean 19.4, max 100 at 400/s). Median latency rises
  slowly (175 → 300 ms) but the tail grows with load (p99 0.43 → 2.3 s, max 4.2 s at 400/s),
  consistent with items waiting for their own node's leader turn (finding 1).
- WAN at 25–50/s: median ≈ 0.7 s; p95 ≈ 1.3–1.6 s; p99 4–6 s.
- WAN at 100–200/s: throughput still follows the offered rate, but the **longest gap between
  blocks reached about 10 s** (max block interval 10.0–10.1 s, against 0.36 s at 25/s), and
  p95 rose to 13–14 s (max ≈ 22 s). Not explained yet (finding 10).
- The observer's read round grows to 0.6–1.2 s under WAN at 100–200/s, so commit stamping
  error there is up to about one second; small relative to the tail.

## Findings

Observed while building and running the measurement; none is fixed here.

1. **No mempool gossip.** An item waits until the node that admitted it leads a view, so
   latency depends on leader rotation (with five validators, up to four other leaders before
   it). The LAN tail growth is consistent with this. Candidate for 8.4.3 (submission to
   several nodes was not measured).
2. **The mempool is unbounded and does linear work per insert** (duplicate scan and full sort by
   fee, sender, nonce). Under sustained overload it grows without limit and admission slows
   down. 8.4.3 admission limits.
   **Fixed after these measurements** (e421ace, TODO 8.4.3): bounded, indexed mempool.
3. **Admission does not verify signatures, nonces or balances**; only duplicates (400) and
   draining (503) are refused. Invalid items occupy the mempool until block production skips
   them. 8.4.3.
   **Fixed after these measurements** (e421ace): admission checks committed state, and items
   that fail execution are evicted. The baseline numbers predate both changes; rerun the sweep
   before comparing.
4. **Blocks are produced continuously, also when empty**: about 30–35 blocks/s on the idle
   five-validator devnet (LAN), about 500/s on a single validator. Every block is stored, so
   chain data grows with wall time, not only with load (the soak will quantify it). Candidate:
   produce empty blocks only at a minimum interval.
5. **Per-block caps** (100 transfers, 20 receipts) bound throughput at one full block per view;
   at the measured rates blocks averaged at most 34 transfers (WAN 200/s) and only individual
   blocks reached the cap (LAN 400/s), so the caps were not the limit in these results.
6. **Ordering at equal fee is by sender, then nonce.** With equal fees a low-sorting sender
   always goes first; under sustained overload higher-sorting senders can starve. 8.4.3.
7. **The mempool is not durable**: items admitted but not yet committed are lost when the node
   restarts (seen in the observer-restart e2e test; clients must resubmit).
8. Commit events (`/v1/events/blocks`) are published after the block's state is written, so a
   client that sees `commit_block` can read the committed state immediately (relied on by the
   latency measurement; verified in `lib/hotstuff_rs`).
9. Finding (b) from the scenario work (quadratic block production) was fixed in 9974e82 before
   these measurements; a full 100-transfer block no longer makes a leader miss its view
   (`tests/block_production.rs`).
10. **Multi-second commit stalls under WAN at ≥100/s.** With the `wan` profile, gaps of about
    10 s without any committed block appeared at 100 and 200 items/s (not at 25–50/s, and never
    on LAN up to 400/s). The cause was not investigated here; candidates are view timeouts with
    backoff after a lost or late proposal, and block sync stalls. Needs a reproduction with
    consensus logs before any WAN finality target is discussed (8.5.2 review, 8.4.1 transport).
    **Mechanism reproduced (2026-11-12, 8.5.2 F2; WAN confirmation pending in 8.5.3):** the pacemaker gives each view in an epoch an
    absolute deadline (epoch start plus k × `max_view_time`), so a view that fails after fast
    views waits out their saved slack, up to `epoch_length × max_view_time` = 10 s with the old
    defaults (10 × 1 s), consistent with the measured 10.0–10.1 s maximum. `tests/view_timing.rs`
    reproduces it in process (one of five validators crashed: longest gap 5.0 s with
    `epoch_length` 10, 2.05 s with 1). Owner decision: `epoch_length` default is now 1, fixed per chain in genesis
    (`protocol.consensus_epoch_length`). The WAN
    rows above were measured with 10 and must be re-measured (8.5.3).

## What remains open for 8.5.3

- Target approval: the owner has to approve (or reject) throughput/finality targets against
  this baseline through the requirements gate.
- The soak: `scripts/devnet/soak.sh` has not been run end to end. Validate it with a short run
  (e.g. `--duration 30m`), then run `--duration 72h` on a machine that can be dedicated for three
  days. Expect chain data to grow with wall time (finding 4).
- Real multi-host WAN measurements (this baseline is emulated on one host).
- Reconfiguration scenarios: blocked while validator-set changes are disabled (8.3.3).

## Idle baseline (2026-11-13, TODO 8.5.3 step 3)

Current code after 8.5.2 / 8.4.1 (genesis `consensus_epoch_length` 1, Noise transport, byte
budgets), git `af63ae1`. Isolated Docker devnet, 5 validators, `--profile observe`, no load,
30 min; values from Prometheus over the last 25 min (`docs/measurement-plan.md` §4.5). Same host
as the 2026-11-10 baseline. Per validator; the five agree within the ranges shown.

| Measure | Value |
|---|---|
| Committed blocks | 23.1 /s, all empty (payload 53 B) |
| Commit interval (consensus) | p50 29 ms, p99 0.32 s |
| View timeouts | ≈ 0.0013 /s (one per ~12 min) |
| Chain DB growth | 120–133 MB/h → **2.9–3.2 GB/day, ≈ 1.1 TB/year per validator at zero load** |
| Disk per empty block | ≈ 1.4–1.6 KB (block, certificate with signatures, indexes, redb overhead) |
| Signing guard | 300 KB, constant |
| Resident memory | 68 MB after 30 min, rising ~105 MB/h in step with the database (see finding 12) |
| CPU | 0.11 cores |
| P2P traffic | 52.7 KB/s each way (≈ 4.5 GB/day) |
| Open fds / threads | 22 / 30, flat |
| P2P rejections, unreadable blocks | 0 |
| Disk write bytes (`/proc/self/io`) | not available inside the container |

Findings:

11. **Idle chain growth is dominated by empty blocks.** With epoch length 1 every view that
    commits produces a block whether or not there is work, about 23 per second. That is roughly
    3 GB per validator per day with no transactions at all. Any disk-growth estimate under load
    starts from this floor. Options (empty-block suppression, pruning, longer `max_view_time`
    when idle) are owner decisions (plan §9 decision 4).
    **Decided 2026-11-13:** empty-block heartbeat, at most one empty block per second when idle
    (`src/app/chain_app/heartbeat.rs`, TODO 8.5.3 step 3); this baseline predates it and is
    to be re-measured.
12. **Memory follows the redb page cache, not a leak.** RSS rises smoothly while the file grows
    in allocation steps (node0: 10 MB → 68 MB as the DB went 0.6 MB → 72 MB). `RedbKVStore`
    opens with redb's default 1 GiB cache, so RSS should level off near 1 GiB plus the process
    base. To confirm in the long soak; a configurable cache size would bound it.
13. `miraset_process_write_bytes_total` is missing in the containers (`/proc/self/io` not
    readable there); disk writes must come from `docker stats` or the host until fixed.

## Empty-block heartbeat: idle and 10 000/s (2026-11-13)

Git `0844356` (heartbeat 1 s, `max_view_time` 2 s). Isolated Docker devnet, 5 validators,
`--profile observe`; two 5-minute runs (owner request), metrics over the last 4 minutes, per
validator. For the load run only, the RPC submission limit was raised on the nodes
(`--rpc-submit-rate 100000 --rpc-submit-burst 100000`, scratch compose override) because one
loadgen container is one client and the default 50/s per client would have been measured
instead of the chain. Agreement checked at every recent height after each run (OK).

| Measure | Idle (0 tx) | Offered 10 000/s (256 senders, 15 % receipts) |
|---|---|---|
| Committed | — | **2 216 items/s** (1 847 tx/s + 369 receipts/s); 100 tx + 20 receipts in every block |
| Blocks | 1.0 /s, all empty (was 23/s) | 17 /s, none empty |
| Admission | — | 646 k admitted, 2.25 M refused with 429 (mempool per-sender quota: backpressure) |
| Latency of admitted items | — | tx p50 0.30 s, p90 1.5 s, p95 9.2 s, p99 187 s (queue); receipts p50 2.5 s, p99 9.5 s |
| Commit interval | p50 0.80 s, p99 2.0 s | p50 0.05 s, p99 0.25 s |
| Chain DB growth | ≈ 5 MB/h (1 block/s × ~1.5 KB) → ≈ 130 MB/day, ≈ 47 GB/year | 5.8–6.3 GB/h → **≈ 145 GB/day** (≈ 750 B per committed item) |
| Payload per block | 53 B | 48 KB |
| Memory (RSS) | 11 MB, +5 MB/h | 490–505 MB after 5 min, +4.4 GB/h (redb cache, see finding 12) |
| CPU | 0.026 cores | 0.66–0.96 cores |
| P2P traffic | 5.6 KB/s each way | 680–700 KB/s each way |
| Open fds | 22 | 106–337 (client connections) |
| View timeouts, P2P rejections, unreadable blocks | 0 | 0 |

Findings:

14. **The heartbeat cuts idle cost ~20×:** blocks 23/s → 1/s, disk ≈ 3 GB/day → ≈ 130 MB/day,
    CPU 0.11 → 0.026 cores, P2P 53 → 5.6 KB/s.
15. **Capacity is ≈ 2 200 committed items/s on this host, bounded by the block size cap**
    (`MAX_BLOCK_TXS` 100 + 20 receipts per block, every block full at ≈ 17 blocks/s). 10 000/s
    is not reachable without larger blocks or faster views; the mempool sheds the excess with 429
    and the tail latency of admitted items grows with the queue (p99 ≈ 3 min).
16. **Disk under full load ≈ 145 GB/day per validator** (≈ 750 B per committed item); at a
    hypothetical 10 000 committed items/s that would be ≈ 650 GB/day. Retention/pruning is an
    owner decision before any sustained high-throughput run.

## Strict 1 block/s: idle and 10 000/s, with RAM and network (2026-11-13)

`--min-block-interval-ms 1000` (owner request: at most one block per second, empty or not), on
top of the heartbeat and `max_view_time` 2 s. Same setup as above: 5 validators, two 5-minute
runs, metrics over the last 4 minutes; RPC submission limit raised for the load run.
Network: `eth0` byte counters inside each validator container over 240 s (all traffic: P2P and
client RPC) and `miraset_p2p_bytes_total` (P2P only). RAM: container memory (`docker stats`) and
process RSS. Agreement checked after the runs (OK).

| Measure (per validator) | Idle (0 tx) | Offered 10 000/s |
|---|---|---|
| Blocks | 1.0 /s, all empty | 1.0 /s, all full (100 tx + 20 receipts, 49 KB payload) |
| Committed | — | **120 items/s** (100 tx/s + 20 receipts/s) |
| Admission | — | 35 k admitted, 2.4 M refused with 429 (per-sender quota) |
| Latency of admitted items | — | tx p50 11.9 s, p99 13.6 s; receipts p50 65 s (queue) |
| Disk growth | ≈ 5 MB/h → ≈ 130 MB/day | 0.27–0.44 GB/h by file size (redb steps) ≈ 7–10 GB/day; ≈ 750 B × 120/s ≈ 7.8 GB/day |
| RAM (container) | 8–12 MB | 48–53 MB after 5 min |
| RAM (process RSS) | 10.7 MB, +5 MB/h | 51–56 MB, +0.3 GB/h (redb cache, levels off near 1 GiB) |
| CPU | 0.025 cores | 0.39–0.41 cores |
| Network, all (in / out) | 8.5 / 9.0 KB/s (0.07 Mbit/s each way, ≈ 1.5 GB/day total) | 1.33–1.43 MB/s in / 0.77–0.91 MB/s out (≈ 11 / 7 Mbit/s; mostly client submissions, 99 % refused) |
| Network, P2P only (each way) | 5.6 KB/s | 44.8 KB/s (0.36 Mbit/s) |
| View timeouts, P2P rejections | 0 | 0 |

Cluster of 5: idle RAM ≈ 50 MB, network ≈ 85 KB/s total; under the 10 000/s offer RAM
≈ 245 MB, network ≈ 10.7 MB/s total (≈ 86 Mbit/s), of which P2P ≈ 0.45 MB/s.

> **Superseded block limit (2026-11-13, later the same day):** the runs above used the old cap
> of 100 transactions + 20 receipts per block. Blocks are now bounded by 8 MiB of payload
> instead (`TODO.md` 8.5.3, "Block size 8 MiB"); throughput, disk and network under load must
> be re-measured. A single-validator release run built a 17 491-transfer (5.46 MB) block in
> ≈ 0.6–0.7 s.

17. **Strict 1 block/s caps throughput at 120 items/s** (block size cap per second), about 18×
    below the heartbeat-only capacity (≈ 2 200/s); disk under full load drops from ≈ 145 GB/day
    to ≈ 8 GB/day and P2P from ≈ 690 KB/s to 45 KB/s per validator. Client-facing bandwidth at
    an over-capacity offer is dominated by refused submissions, not by consensus.

## 8 MiB blocks at 1 block/s: idle and 10 000/s (2026-11-13)

Same setup as the strict 1 block/s runs (5 validators, `--min-block-interval-ms 1000`, two
5-minute runs, metrics over the last 4 minutes, RPC submission limit raised for the load run),
after blocks became bounded by 8 MiB of payload instead of 100 transactions + 20 receipts
(`d61f7e9`). Agreement OK up to height 329 after the runs.

| Measure (per validator) | Idle (0 tx) | Offered 10 000/s (+ 15 % receipts) |
|---|---|---|
| Blocks | 1.0 /s, all empty (53 B) | 0.92 /s, mean interval 1.08 s, max 2.57 s; mean 7 507 tx + 256 receipts, max 9 968 tx; mean payload 2.95 MB |
| Committed | — | **7 174 items/s** (6 938 tx/s + 237 receipts/s) |
| Admission | — | 2.01 M tx admitted, 0.45 M refused 429 (per-sender quota); receipts 68 k admitted, 0.37 M refused 429 |
| Latency of admitted items | — | tx p50 7.2 s, p99 9.9 s; receipts p50 4.8 s, p99 6.1 s |
| Commit interval | p50 0.80 s, p99 1.97 s | p50 1.18–1.25 s, p99 4.7–4.8 s |
| Disk growth | ≈ 5.5 MB/h → ≈ 130 MB/day | 2.03 GB in ≈ 5.5 min (≈ 6.5 MB/s) → **≈ 0.5 TB/day**; payload alone 2.7 MB/s ≈ 234 GB/day (storage ≈ 2.3× payload) |
| RAM (container) | 8–14 MB | 1.14–1.16 GiB after 5 min |
| RAM (process RSS) | 10.7 MB | 1.2–1.24 GB, still growing at the end (redb cache plus ≈ 50 k pending items); not shown to level off in 5 min |
| CPU | 0.03 cores | 0.99–1.03 cores |
| Network, all (in / out) | 8.7 / 9.2 KB/s | 3.5–3.6 MB/s in / 3.0–3.2 MB/s out (≈ 29 / 25 Mbit/s) |
| Network, P2P only (each way) | 5.6 KB/s | 2.15–2.22 MB/s (≈ 17.5 Mbit/s) |
| View timeouts, timeout certificates, P2P rejections | 0 | 0 |

Cluster of 5: idle RAM ≈ 55 MB, network ≈ 90 KB/s; under load RAM ≈ 5.7 GiB, network
≈ 33 MB/s (≈ 260 Mbit/s), of which P2P ≈ 11 MB/s.

18. **8 MiB blocks raise throughput at 1 block/s from 120 to ≈ 7 200 items/s (60×)**; blocks
    averaged 2.95 MB, about a third of the limit. The limit now is admission, not block size:
    without mempool gossip a block carries only its leader's pool, and the per-sender quota
    refused 18 % of offered transactions and 84 % of receipts. Costs scale with it: ≈ 1 CPU
    core, ≈ 1.2 GB RSS and ≈ 0.5 TB/day of disk per validator, ≈ 17.5 Mbit/s P2P each way.
    Commit intervals stretch (p99 ≈ 4.8 s, 0.92 blocks/s) without view timeouts; the
    signature-cost risk from the review stays open at full 8 MiB blocks, which this load did
    not reach. RSS growth must be checked in a longer run.

## Stress test: stepped load to saturation (2026-11-13)

`scripts/devnet/stress.sh` (raw outputs kept outside the repository). 5 validators, 8 MiB blocks,
`--min-block-interval-ms 1000`, `max_view_time` 2 s, RPC submission limit raised, each node
limited to 2.5 GiB of memory. Transfers only (receipts are capped per coordinator and would hide
the transaction ceiling). Four loadgens × 256 senders × 7 500 tx/s were to start 150 s apart
(7.5k → 15k → 22.5k → 30k tx/s); metrics over the last 90 s of each step. Host: 16 cores,
30 GB RAM, shared by the 5 validators, Prometheus and the loadgens.

**What actually ran:** loadgens 3 and 4 never sent load. Their faucet funding transfers were
refused with 503 (mempools full), so the offered load was 7.5k tx/s, then 15k tx/s for the rest
of the run. The ceiling is already visible at 15k.

| Measure (per validator) | 7.5k tx/s offered | 15k tx/s offered (3 windows, same picture) |
|---|---|---|
| Committed | **7 405 tx/s** (all admitted) | **5 900–7 130 tx/s** (saturated) |
| Blocks | 1.0 /s, 2.76 MB | **0.51–0.57 /s**, 4.3–4.8 MB (up to 19 678 tx, the mempool cap) |
| View timeouts | 0 | **0.24–0.29 /s** (one view in four), timeout certificates 0.28–0.37 /s |
| Commit interval | p50 1.0 s, p99 3.9–4.7 s | p50 1.6–2.1 s, p99 4.9–9.5 s |
| Admission | — | 503 "mempool full" for 55 % of offered transactions (3.6 M), 429 per-sender quota 0.03 M |
| Latency of committed tx (whole run) | — | p50 8.4–12.6 s, p99 18–19 s, max 35 s |
| CPU (process) | 0.86–0.88 cores | 1.15–1.30 cores (peaks ≈ 2 cores) |
| RAM (process RSS) | 0.61–0.66 GB | 1.3–1.7 GB (peak container 1.6 GiB of the 2.5 GiB limit) |
| Disk (`chain.redb`) | ≈ 0.95 GB after 2.5 min | 3.8 GB after ≈ 10 min (≈ 6.3 MB/s, ≈ 0.55 TB/day) |
| Network, all (in / out) | 2.9–3.0 / 2.6–2.8 MB/s | 3.9–4.2 / 3.2–3.7 MB/s (≈ 33 / 30 Mbit/s) |
| Network, P2P (each way) | ≈ 2.2 MB/s | 1.9–2.7 MB/s |
| Faults | none | no OOM kill, no restart, no ERROR log line, no P2P rejection; agreement OK up to height 432 |

Host: used memory peaked at 22 GB (9.4 GB still available); the cluster moved ≈ 37 MB/s
(≈ 300 Mbit/s) at saturation.

19. **Measured ceiling on this host: ≈ 7 000 tx/s** (5 validators, 1 block/s, 8 MiB blocks),
    below the earlier 10–12k estimate. Past it, offering more does not commit more: blocks grow
    toward the 20 000-transaction mempool cap (4–5 MB), a full block no longer fits a 2 s view
    (the leader waits up to 1 s for the block interval, then builds the block; replicas then
    re-execute it; signature checks are serial on both), so one view in four times out and
    the block rate halves. Committed throughput stays near 7 000 tx/s, but latency and its
    tail double.
20. **Overload is shed safely:** mempools fill and refuse with 503, nothing crashes or forks,
    memory stays bounded (≤ 1.7 GB RSS), and the chain keeps committing through the view
    timeouts. The cost is latency (p99 ≈ 19 s) and refused submissions.
21. **Where the ceiling comes from, by measured effect:**
    - per-transaction CPU on one thread: signature check plus execution, once by the leader and
      once by every replica;
    - the strict 1 s block interval spends part of the 2 s view before production starts.

    Candidate fixes, none measured yet:
    - verify signatures in parallel (or once, at admission, with a cache);
    - start the strict interval when the parent is received, not from its timestamp;
    - cap a block's transactions so a block always fits the view;
    - raise `max_view_time`;
    - drop the strict block interval: blocks then come as fast as the CPU allows (see the
      heartbeat-only run).
22. **Test-harness gap:** the stress script funds each loadgen's senders when it starts, which
    fails once mempools are full; offered load above 15k tx/s was not reached. Pre-funding
    (a loadgen "fund only" mode) is needed to go further.

