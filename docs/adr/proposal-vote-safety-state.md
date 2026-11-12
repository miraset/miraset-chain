# Proposal: Vote safety state persistence (TODO 8.3.2)

**Status**: Design approved by the project owner 2026-11-09 with the refinements in §10;
implementation in progress (not a numbered ADR; see `docs/adr/README.md`). §§1–9 are kept as
proposed; where they differ, §10 governs.
**Date**: 2026-11-09
**Requirement**: PRD R07 / AT07 ("crash/restart cannot produce conflicting votes for a view");
`docs/HotStuff-rs.md` §5.8 (record each signature before it leaves the node; refuse to sign
for a view at or below the last signed view).
**Scope**: experimental devnet consensus (ADR-0016 Proposed). Key custody / external signer is
TODO 8.3.4 and is only addressed as a seam here.

## 1. What the node signs today

Evidence from the vendored `lib/hotstuff_rs` (0.4.0 + MIRASET patches):

| Message | Signed content | Equivocation risk |
|---|---|---|
| `PhaseVote` | `(chain_id, view, block, phase)` | **Yes**: two votes in one view for different blocks/phases are conflicting. This is the safety-critical signature. |
| `TimeoutVote` | `(chain_id, view)` | No: Ed25519 is deterministic, so a re-sign is byte-identical. |
| `Proposal`, `Nudge`, `NewView` | **not signed** | Not a vote-safety issue; their authenticity rests on transport identity, which is self-declared today (TODO 8.4.1). |

## 2. How phase votes are protected today (implicit)

Order of operations for a vote (`hotstuff/implementation.rs`):

1. Entering view *v* persists `highest_view_entered = v` (`:174`) **before** any vote in *v*.
2. A proposal is validated and inserted; `block_tree.update` persists the new lock (`locked_pc`).
3. The guard `highest_view_voted < v` (`:419`, `:527`) allows at most one phase vote per view.
4. The vote is signed and **sent** (`:437`, `:546`), and only then is `highest_view_phase_voted = v`
   persisted (`:439`, `:548`).

On restart the replica starts at `max(highest_view_entered, highest_pc.view, highest_tc.view) + 1`
(`algorithm.rs:70`). Every view it may have voted in is at or below `highest_view_entered`, so it
never re-enters one. Our `RedbKVStore` commits each write batch with redb's default immediate
(fsync) durability.

**Conclusion:** a simple crash/restart cannot produce conflicting phase votes **if** these hold:

- **A1.** Each block-tree write is durable before the next step. This holds for Redb but not for
  `MemKVStore`, which is test-only.
- **A2.** The chain database is never rolled back, restored, wiped or re-initialized.
- **A3.** A failed write stops the replica before it votes.
- **A4.** Only one process at a time ever signs with a given validator key.

## 3. Gaps

| # | Gap | Consequence |
|---|---|---|
| G1 | Safety depends on an undocumented invariant (view-entry persistence), and the vote record itself is persisted *after* sending. | A future library or upstream change could silently break restart safety. |
| G2 | `KVStore::write` returns nothing, so our Redb adapter `expect`s and panics inside `hs-algorithm`. | Safe (no further votes), but the node halts **silently**: the process stays up and `/health` still says `ok`. |
| G3 | Vote safety state lives in the same database as chain data. | A resync, snapshot restore (8.2.4), `docker compose down -v` or re-genesis resets the view to 0. After catching up, the validator can sign a different `PhaseVote` in a view it already voted in. **Nothing prevents this today.** |
| G4 | No guard against two processes using the same key (failover, copied key file). | Two nodes vote independently, which is equivocation by construction. |
| G5 | No crash/restart vote-safety tests. | AT07 is unproven. |

## 4. Proposed design

**D1. Persist before send (vendored patch #7).** In both vote paths, move
`set_highest_view_phase_voted(view)` before `send`. This costs nothing and makes the library's own
record match §5.8 instead of relying only on A1 plus view-entry ordering.

**D2. Independent signing guard at the transport boundary (no further library change).**
`GuardedNetwork<N: Network>` wraps any `Network`. For each outgoing
`Message::ProgressMessage(HotStuffMessage::PhaseVote(v))` it calls
`SigningGuard::authorize(&v)` and forwards the message only on `Ok`. A signature that never leaves
the process cannot equivocate.

`SigningGuard`, backed by its own Redb file, separate from the chain database:

- It records `(chain_id, validator_pubkey, genesis_hash)` on first use and refuses to open on a
  mismatch.
- It keeps `last = (view, block, phase)`. It allows a vote iff `v.view > last.view`, or iff the vote
  is identical to `last` (an idempotent re-send). Otherwise it refuses.
- It durably commits (fsync) the new `last` **before** returning `Ok`.
- Redb's exclusive file lock rejects a second process using the same guard file on the same host
  (partial G4; cross-host is TODO 8.3.4).
- **Location:** `--signing-guard <path>`, defaulting to `<data_dir>/signing-guard.redb`.
  Operators are told to keep it when wiping chain data. In the Docker devnet it gets its own volume.

**D3. Fail closed and visibly.** If the guard refuses a vote, or any guard or chain-store write
fails:

- The vote is dropped.
- A shared `SafetyHalt` flag is set with a reason.
- `/health` returns **503** `{"status":"halted","reason":…}`. A separate `/ready` endpoint is
  deferred to 8.4.4.
- The process exits non-zero after a bounded grace period, so the orchestrator restarts it.

On restart, recovery validation and the guard reload. A guard refusal is logged without key
material.

**D4. Seam for 8.3.4.** `SigningGuard` is the point where an external signer or KMS slots in later,
when signing itself moves out of process. Until then the library keeps signing in memory, and the
guard only controls what leaves the node.

## 5. Tests (AT07 acceptance)

| ID | Level | Scenario | Pass criterion |
|---|---|---|---|
| T1 | unit | Guard rules: monotonic views, idempotent identical re-send, lower or conflicting same view refused, survives reopen, chain/key/genesis mismatch refused, corrupted file refused | all pass; no key bytes in errors |
| T2 | lib (vendored) | Vote record persisted before send (D1), via a `Network` stub that checks `highest_view_phase_voted` on each outgoing vote | record ≥ vote view at send time |
| T3 | integration | 5 validators on Redb; one validator crashed and restarted ≥ 20 times at random points while a vote-tap records every `PhaseVote` per signer | no signer has two different `(block, phase)` for one view; chain keeps finalizing |
| T4 | integration | Chain DB wiped but guard kept, then resync | the validator emits no vote for any view ≤ its guard view |
| T5 | integration | Injected guard/chain write failure | no further votes; `/health` 503; process exits non-zero |

## 6. Alternatives considered

- **Status quo plus tests (D1 + T2/T3 only).** Smallest change, but leaves G2–G4 open. A routine
  resync could cause equivocation. Rejected.
- **Guard hook inside the library, before `PhaseVote::new`.** Closest to "record before signing",
  but adds a second library seam and patch to maintain, for no safety gain while signing stays in
  process. Revisit with the external signer (8.3.4).
- **External signer/KMS now.** This is the eventual target in §5.8, but it depends on key-custody
  decisions (8.3.4) that aren't made yet.

## 7. Cost

- One extra fsync per phase vote (about one per view). On the Docker devnet that is tens of fsyncs
  per second per node, which is acceptable. Production numbers are gated by 8.5.3.
- One extra file per validator, plus an operator rule never to delete it with chain data.

## 8. Decisions needed

1. Approve **D2** (transport-boundary guard) over a library-internal hook.
2. **Fatal behavior:** halt and exit non-zero (proposed), or halt and stay up, reporting 503.
3. **Guard location default:** inside `data_dir` (proposed; simplest), or a mandatory separate path
   such as its own Docker volume.
4. Whether the guard must already exist for a validator joining with a non-empty chain DB, so that
   a lost guard file is refused rather than silently recreated. Proposed: **refuse**, with an
   explicit `--init-signing-guard` flag for first start.

## 9. Out of scope / follow-ups

- Unsigned proposals and nudges, and authenticated transport identity: 8.4.1.
- Cross-host double-signing (failover) and the external signer: 8.3.4.
- Readiness endpoint and structured logs: 8.4.4.

## 10. Approval record and refinements (2026-11-09)

**Approved by:** project owner, 2026-11-09, in the engineering session that implemented it.

### Decisions (§8)

| # | Decision |
|---|---|
| 1 | **D2 approved:** the guard sits at the transport boundary, not inside the library. |
| 2 | **Halt and exit non-zero**, but only for guard faults and chain-store write failures (see R1). Exit code **3** after a 5s grace period; the devnet uses `restart: on-failure`. |
| 3 | Default `<data_dir>/signing-guard.redb`; the Docker devnet gives each validator its own `nodeN-guard` volume, so `docker compose` volume operations on chain data leave it alone. |
| 4 | **Refuse** to start with a non-empty chain database and no guard; `--init-signing-guard` overrides it. |

### Refinements to §4

- **R1. Refusals vs faults (supersedes D3's "any refusal halts").** A vote the guard will not
  sign is dropped and counted, and the node keeps running. This covers both a **stale** vote
  (view below the record) and a **conflicting** vote (same view, different block or phase).
  The node halts only when the guard cannot decide safely: storage failure, vote for another
  chain, poisoned lock, or a chain-store write failure (G2).
  - *Why conflicts do not halt:* in review we first proposed halting on a same-view conflict,
    on the theory that it only happens through a bug. T4 disproved that. After a chain-data
    wipe, a replica legitimately reaches its last signed view again on a different block, and
    halting there would turn the documented resync procedure into a restart loop. The guard
    already prevents the equivocation, so the halt adds no safety.
- **R2. Both `send` and `broadcast` are checked.** The library sends phase votes only with
  `send` today. Checking `broadcast` as well protects against upstream changes.
- **R3. The guard is the outermost `Network` layer.** Verified: the library sends every message
  through `Network::send`/`broadcast` (`networking/sending.rs`), and `TcpNetwork` and
  `ChannelNetwork` deliver self-addressed messages inside `send`. A leader's vote to itself
  therefore passes the guard before it can end up in a phase certificate.
- **R4. Phase-vote events stay local.** The library publishes `Event::PhaseVote` with the signed
  vote. MIRASET does not forward these events. They must never reach RPC or logs as sendable
  messages, because that would bypass the guard.
- **R5. After a halt, nothing goes out.** `GuardedNetwork` forwards no further outgoing message
  of any kind. The validator binary exits without joining replica threads, since joining a
  panicked thread would panic again. Redb commits are atomic, so nothing durable is lost.

### Implementation map

| Item | Where | Test |
|---|---|---|
| D1 (patch #7) | `lib/hotstuff_rs/src/hotstuff/implementation.rs` | T2 `tests/vote_safety.rs::test_phase_vote_is_recorded_before_it_is_sent` (RED on upstream) |
| Guard rules, identity, corruption, file lock | `src/consensus/signing_guard.rs` | T1 `src/consensus/signing_guard_tests.rs` |
| Transport wrapper, R1/R2/R5 | `src/consensus/guarded_network.rs` | unit tests in the same file |
| Halt flag, `/health` 503 | `src/consensus/safety_halt.rs`, `src/rpc/server.rs` | T5 `tests/vote_safety.rs::test_chain_store_write_failure_halts_visibly` |
| Chain write failure → halt (G2) | `src/storage/redb.rs` (`with_safety_halt`) | T5 |
| CLI wiring, decision 4, exit code 3 | `src/main.rs` | manual smoke run (first start, restart, missing guard refused, `--init-signing-guard`) |
| Chain wipe with guard kept | — | T4 `tests/vote_safety.rs::test_wiped_chain_data_cannot_resign_old_views` (single validator) |
| Devnet volumes and restart policy | `docker-compose.yml`, `Dockerfile`, `docs/devnet.md` | — |

- **R6. Any consensus-thread panic halts visibly.** Found while running T3. An `hs-*` thread
  can die for reasons other than a store write: a library error that hits `expect`, such as the
  pacemaker defect fixed as vendored patch #8. The node then stopped participating while
  `/health` still said `ok`. The validator binary now installs a panic hook that triggers
  `SafetyHalt` for any panic on an `hs-*` thread (`SafetyHalt::install_consensus_panic_hook`).
- **R7. T4 at cluster scale means restoring the whole cluster.** A single wiped validator in a
  live cluster is pulled forward by its peers and never re-enters old views. The real risk is
  that every validator rolls back at once, for example a snapshot restore (8.2.4) with the
  guards kept.

### Test levels

| Level | What | Where |
|---|---|---|
| Unit | Guard rules, identity, corruption, file lock, startup rule (decision 4), identical re-send after reopen; `GuardedNetwork` refusals vs faults; `SafetyHalt` and the panic-reason filter; Redb write failure → halt | `src/consensus/*`, `src/storage/redb.rs` |
| Integration (in-process replicas) | T2 vote recorded before send; T4 single-validator wipe; T5 store write failure → halt and `/health` 503; consensus-thread panic → halt; `/health` 503 and no `reason` when healthy | `tests/vote_safety.rs`, `tests/consensus_panic_halt.rs`, `tests/rpc_api.rs` |
| Integration (5 validators, TCP + Redb) | T3: 20 restarts at seeded pseudo-random points plus a single-validator wipe; T4: whole-cluster restore to older chain data with guards kept. Guards refused 65–88 stale and 5 conflicting votes per run; no equivocation; finality agrees on all nodes | `tests/vote_safety_cluster.rs` |
| E2E (real binary) | SIGKILL and restart keep the guard record; missing guard with chain data → exit 1 until `--init-signing-guard`; wiped chain data with guard kept recovers, with stale votes dropped and no halt | `tests/e2e_validator.rs` |
| E2E (Docker devnet) | Fault suite with new SIGKILL and resync (chain-data volume deleted, guard volume kept) scenarios, plus a no-`SAFETY HALT` log check; passed 2026-11-09 in an isolated compose project | `scripts/devnet/fault_suite.sh` |

Exit code 3 is not covered end to end: triggering it needs an injected fault, which the
production binary does not expose. In-process tests cover the halt itself (T5 and the panic
test).

Vendored patch #8 has a deterministic regression test,
`lib/hotstuff_rs/src/pacemaker/view_regression_tests.rs`, which fails on upstream with the
production error.

**Remaining outside 8.3.2:** independent review of patches #7 and #8 (TODO 8.5.2) and
cross-host double signing (TODO 8.3.4).

Cross-host double signing remains with TODO 8.3.4.


## 11. Lock floor (TODO 8.5.2, 2026-11-12)

The 8.4.5 review found that the guard kept phase votes but not HotStuff's lock (`locked_pc`),
which lives in the chain database: a resync or restore let a validator vote without its old
lock. Fixed as follows.

- **Vendored patch #11.** `SignRequest::PhaseVote` carries `justify_view` (the certificate the
  vote builds on: the proposed block's `justify`, or the nudge's `justify`) and `locked_view`
  (the replica's lock after processing that certificate). They are context, not signed content;
  signatures and the wire format of `PhaseVote` are unchanged. The remote signer protocol
  (`proto/signer.proto`) carries them as fields 5 and 6; a sender that omits them sends 0,
  which a guard holding any lock refuses.
- **Guard.** A fresh vote must have `justify_view >= lock_floor`; the floor is then raised to
  `locked_view` in the same fsynced transaction as the vote record. Identical re-sends and the
  transport layer (`GuardedNetwork`, no context) skip the check. A refusal is
  `Authorization::BelowLock`, counted as `miraset_vote_refused_total{reason="below_lock"}`.
- **Why views suffice.** `safe_pc` accepts a certificate whose view is above the lock's view or
  whose block extends the locked block. A certificate on a block that extends the locked block
  has a view at or above the lock's view, so an honest replica that still has its lock never
  trips the floor; a replica that lost it is held to the same rule.
- **Consequence (owner decision 2026-11-12: fail closed).** If more than f validators are
  restored below their locks, they refuse every vote on the rolled-back fork and finality stops
  until newer chain data is back. No override exists. T4 now asserts exactly that: no fork of
  the finalized history during the stall, and resumed progress after the newer chain databases
  are restored. With the floor check disabled, T4 fails with "validator 0 finalized a fork at
  height 14", which is what the previous T4 (which expected the cluster to resume) allowed.
- **Independent review (2026-11-12, rust-reviewer; no CRITICAL).** Fixed: (HIGH) the unsigned
  context is bounded: `justify_view` or `locked_view` above the vote's view is refused as
  `InvalidLockContext` before any write, so a bad caller cannot raise the floor permanently;
  (HIGH) the remote-signer fields are proto3 `optional` and required, so an older node without
  them gets an invalid-argument error instead of silently skipping the check; (MEDIUM) the
  guard's doc now says it enforces a necessary condition of `safe_pc`, not the full rule;
  T3 now asserts zero below-lock refusals for the four validators that keep their chain data,
  including the one restarted 20 times (liveness evidence on real vote sequences); the wiped
  validator may refuse votes on proposals built on its stale highest certificate before it
  catches up, which is the intended refusal. Accepted: one `error!` log per refused view during a fail-closed stall
  (about one per second with 1s views); no reset path by design (owner decision).
- **Single validator (n = 1).** A lone validator whose chain data is wiped is the same case with
  f = 0: it refuses to rebuild from genesis and stays without commits until the chain data is
  back (`tests/e2e_validator.rs::test_e2e_wiped_chain_data_with_guard_kept_refuses_fork_until_data_is_back`).
