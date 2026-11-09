# Proposal: Vote safety state persistence (TODO 8.3.2)

**Status**: Proposed — awaiting approval (not a numbered ADR; see `docs/adr/README.md`)
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
