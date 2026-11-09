# Vendored hotstuff_rs: MIRASET modifications

This directory is a vendored copy of `hotstuff_rs` **0.4.0** (crates.io, upstream commit
`f1ce8ffea72a0d3c93c5f2627ecf404021d1a9e8`, <https://github.com/parallelchain-io/hotstuff_rs>),
Copyright © 2023 ParallelChain Lab, licensed under the Apache License 2.0 (see `LICENSE`).

`miraset-chain` depends on it through `hotstuff_rs = { path = "lib/hotstuff_rs" }`. The packaged
`Cargo.toml` was replaced by the upstream `Cargo.toml.orig`; `.github/` and `readme_assets/` were
dropped. Every source change is marked with a `MIRASET PATCH` comment. To rebase onto a newer
upstream, re-apply each patch below and re-run both the upstream tests and the miraset tests.

| # | File | Change | Why | Regression evidence |
|---|---|---|---|---|
| 1 | `src/block_tree/pluggables.rs` (`KVGet::block_data`) | Collect datums once instead of `find`-then-`collect` on the same iterator. | Upstream returned empty `Data` for every stored block: block sync served empty blocks (lagging replicas could never catch up) and commit-time payload reads saw no transactions. | `src/storage/block_data_tests.rs` (fails on upstream, passes patched); `tests/node_consensus.rs` partition healing; `tests/tcp_cluster.rs` restart catch-up. |
| 2 | `src/pacemaker/implementation.rs` | View deadlines use exact `Duration` arithmetic (`view_deadline_offset`) instead of `Duration::from_secs(max_view_time.as_secs() * n)`. | Sub-second `max_view_time` truncated to 0, so every view timed out immediately and nothing committed. | `test_four_validator_fault_bound_and_partition_healing` runs with 350ms views. |
| 3 | `src/event_bus.rs` | `recv_timeout(10ms)` instead of a `try_recv` loop with no wait. | Spun a full CPU core per replica. | Docker devnet CPU profile (`docs/devnet.md`). |
| 4 | `src/networking/receiving.rs` (`BlockSyncServerStub::recv_request`), `src/block_sync/server.rs` | Request receive waits until the next advertisement is due (max 10ms) instead of `try_recv` + `yield_now`. | Spun a full CPU core per replica. | Docker devnet CPU profile. |
| 5 | `src/algorithm.rs`, `src/event_bus.rs`, `src/networking/receiving.rs`, `src/block_sync/server.rs` | Named threads (`hs-algorithm`, `hs-event-bus`, `hs-poller`, `hs-sync-server`); spawn failures panic with a message. | Per-thread CPU/debug attribution. | — |
| 6 | `Cargo.toml` | `[lints.rust]` allows `dead_code` and `mismatched_lifetime_syntaxes`. | Keep upstream warnings out of miraset-chain's clippy gate; no code change. | — |

## Upstream test suite (2026-11-09)

Run with `CARGO_TARGET_DIR=target/hotstuff_rs cargo test --manifest-path lib/hotstuff_rs/Cargo.toml`,
compared against a pristine 0.4.0 copy:

| Test | Upstream 0.4.0 | Patched |
|---|---|---|
| lib unit tests | pass | pass |
| `block_sync_test` | pass | pass |
| `multiple_validator_set_updates_test` | pass (30s) | pass (20s) |
| `pacemaker_initial_view_sync_test` | pass | pass |
| `progress_and_validator_set_update_test` | **hangs** (no progress after growing 1 -> 3 validators; the test polls without a timeout) | **hangs** at the same step |

The `progress_and_validator_set_update_test` hang is pre-existing upstream behaviour around
validator-set expansion and is relevant to TODO 8.3.3 (reconfiguration stays disabled).

## Known upstream behaviour intentionally left unchanged

- **Liveness needs four consecutive live leaders.** Strict leader rotation, votes sent to the
  leader of `view + 1`, and the consecutive-view 3-chain commit rule mean an equal-power
  4-validator set cannot finalize while any validator is down. Changing leader selection or the
  commit rule is a consensus-protocol decision (TODO 8.3.1 / 8.5.2), not a bug fix.
- **The network poller yields when `Network::recv` returns `None`.** MIRASET's `Network`
  implementations wait briefly inside `recv`, so the poller does not spin.

These patches have not been reviewed upstream; they are part of the independent consensus review
scope (TODO 8.5.2) and should be offered upstream.
