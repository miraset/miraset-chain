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
| 7 | `src/hotstuff/implementation.rs` (`on_receive_proposal`, `on_receive_nudge`) | `set_highest_view_phase_voted(view)` runs **before** the phase vote is sent instead of after. | Vote safety (TODO 8.3.2, `docs/adr/proposal-vote-safety-state.md` D1): the replica's own vote record must be durable before a signature can leave the process. | `tests/vote_safety.rs` `test_phase_vote_is_recorded_before_it_is_sent` (fails on upstream with "vote for view 1 sent with no vote record", passes patched). |
| 8 | `src/pacemaker/implementation.rs` (`on_receive_timeout_vote`, fallback-TC branch) | Enter `tc.view + 1` only if it is above the current view; `highest_tc` is still updated. | Upstream called `update_view` unconditionally, so a replica that advanced through phase certificates without storing that TC (e.g. after a chain-data wipe, `highest_tc = None`) got `NonIncreasingViewError { cur_view: 6, next_view: 1 }` and the `hs-algorithm` thread panicked ("Pacemaker failure!"). | `src/pacemaker/view_regression_tests.rs` (deterministic: replica at view 10 with no stored TC receives a fallback TC for view 0; fails on upstream with `NonIncreasingViewError { cur_view: 10, next_view: 1 }`, passes patched; test-only `extern crate self as hotstuff_rs` in `src/lib.rs` lets it reuse `tests/common/mem_db.rs`). Also `tests/vote_safety_cluster.rs` (2 failures in 3 runs before, 5 of 5 passed after). |
| 9 | `src/types/signer.rs` (new), `src/types/crypto_primitives.rs` (`Keypair`), `src/replica.rs` (`Configuration`), the three signing sites (`hotstuff/messages.rs` `PhaseVote::new`, `pacemaker/messages.rs` `timeout_vote`, `block_sync/messages.rs` `advertise_block`) and their callers | Purpose-bound signing. Every signature goes through a `ValidatorSigner` as a typed `SignRequest`, and `Keypair` has no raw-bytes `sign`. `SignRequest::signing_bytes()` reproduces upstream's borsh tuples, so signatures and the wire format are unchanged. `Configuration` takes exactly one of `.me(key)` (upstream behaviour, via `LocalKeySigner`) or `.signer(..)`. `SignError::Refused` skips the message; `SignError::Failed` becomes `HotStuffError::SignerFailed` / `PacemakerError::SignerFailed`, or a panic on the block-sync server thread. | Key custody D1 (TODO 8.3.4, `docs/adr/proposal-validator-key-custody.md`): a signer must see what it signs, so slashing protection can run before a signature exists, and the node can never obtain a signature over arbitrary bytes. | `src/types/signer_tests.rs`: golden signing bytes, signatures byte-identical to upstream raw signing, constructors propagate refusals. Upstream suite passes unchanged with `.me(..)`. MIRASET tests: `tests/vote_safety.rs::test_signer_refusal_sends_no_phase_vote_and_does_not_halt`, `tests/signer_fault_halt.rs`. |
| 6 | `Cargo.toml` | `[lints.rust]` allows `dead_code` and `mismatched_lifetime_syntaxes`. | Keep upstream warnings out of miraset-chain's clippy gate; no code change. | — |
| 10 | `src/block_sync/client.rs` (`sync_with`), `src/block_tree/accessors/public.rs` (`blocks_from_height_to_newest`) | Client: requests still start at `highest committed + 1`; if such a response brings nothing new although this session inserted blocks above that height, retry once from after the last inserted block (`ahead`), and reset after any response that made progress. A block that fails `is_correct`/`safe_block` in an `ahead` response ends the session **without** blacklisting the server (our speculative tip lost a fork; the server is honest). Server: speculative blocks are returned from `start_height` on, like committed blocks. | Upstream ended the session when a whole response committed nothing (fewer blocks than the consecutive-view 3-chain rule needs, e.g. across view gaps from timed-out leaders), and every later session repeated the same request: the lagging replica never caught up. The server ignored `start_height` in its speculative range, so the same loop could happen there with a speculative run longer than the limit. Every block is still checked (`is_correct`, `safe_block`, app validation) before insertion; nothing can be committed that upstream could not commit. | `tests/byzantine_faults.rs::test_lagging_validator_catches_up_over_several_sync_rounds` (`block_sync_request_limit = 5`, lag of 20+ heights): upstream stays at the lag height for the full 90s (one 5-block sync, then `EndSync … 0` every 3s); patched catches up in about 10s. Upstream suite below still passes. Independent review (2026-11-09) found the first version (always continue ahead) could blacklist an honest server after a fork and missed the server side; both fixed. |
| 11 | `src/types/signer.rs` (`SignRequest::PhaseVote`), `src/hotstuff/messages.rs` (`PhaseVote::new`), `src/hotstuff/implementation.rs` (`sign_phase_vote` and both vote paths) | Phase-vote sign requests carry `justify_view` (view of the proposed block's or nudge's `justify`) and `locked_view` (`locked_pc.view` after `block_tree.update(justify)`). Context only: `signing_bytes()` ignores them, so signatures are unchanged. | Lock safety across chain-data loss (TODO 8.5.2, `docs/adr/proposal-vote-safety-state.md` §11): the miraset signing guard keeps a durable lock floor outside the chain database and refuses votes building on a certificate below it. | `src/types/signer_tests.rs::test_phase_vote_lock_context_is_not_signed`; miraset `tests/vote_safety_cluster.rs` T4 (a whole-cluster restore below the locks finalizes a fork at height 14 with the floor check disabled; with it, no fork and progress after newer data returns) and T3 (floor survives a chain wipe). |

## Upstream test suite

Run with `CARGO_TARGET_DIR=target/hotstuff_rs cargo test --manifest-path lib/hotstuff_rs/Cargo.toml`.
The first column is a pristine 0.4.0 copy. The later columns add patches in order.

| Test | Upstream 0.4.0 | Patches #1–#6 | Patches #1–#7 | Patches #1–#8 | Patches #1–#9 (2026-11-09) | Patches #1–#10 (2026-11-09) | Patches #1–#11 (2026-11-12) |
|---|---|---|---|---|---|---|---|
| lib unit tests | pass | pass | pass | pass | pass (6) | pass (6) | pass (7) |
| `block_sync_test` | pass | pass | pass (7s) | pass (10s) | pass (9s) | pass (10s) | pass (10s) |
| `multiple_validator_set_updates_test` | pass (30s) | pass (20s) | pass (38s) | pass (10s) | pass (30s) | pass (38s) | pass (30s) |
| `pacemaker_initial_view_sync_test` | pass | pass | pass (10s) | pass (10s) | pass (20s) | pass (20s) | pass (10s) |
| `progress_and_validator_set_update_test` | **hangs** | **flaky**: 1 pass (10s), 1 hang (>90s) in 2 runs | **flaky**: 1 pass (10s), 2 hangs (>90s) in 3 runs | not re-run (known flaky) | not re-run (known flaky) | not re-run (known flaky) | not re-run (known flaky) |

The test stops making progress after the validator set grows from 1 to 3, and it polls with no
timeout. It sometimes passes, sometimes hangs, with or without patch #7, so the patch does not
cause it.

The `progress_and_validator_set_update_test` hang is pre-existing upstream behaviour (intermittent) around
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
