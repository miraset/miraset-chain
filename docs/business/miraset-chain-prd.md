# MIRASET Chain product requirements document

Status: Draft, source verification pending.
Traceability epic: `miraset-chain-hardening`.
Author: AI documentation pass; specialist Requirement Gate review pending.

The authoritative vault is now located in the
[source map](knowledge-sources.md). PoCC means Proof of Compute & Capacity
and belongs to R13's economics requirements, separate from R06 HotStuff.
Its accepted pool split and full formulas must be reviewed before coding;
this draft does not define cryptographic inference-correctness proofs.

## Purpose and scope

Provide public Rust protocol, cryptography, consensus, deterministic execution,
chain storage, networking, node RPC, and related documentation/tests.
The experimental chain must preserve the approved production settlement
architecture until a separately recorded decision authorizes a change.

Permissioned validators order and finalize state transitions. Workers are
separate private product identities. The master account may authorize registry
operations but cannot replace quorum or rewrite finalized history.

Excluded: inference runtimes, worker implementations, payment integrations,
customer credit ledgers, desktop wallets, product orchestration, CLI/TUI,
indexers and launchers. No publication or repository visibility change is
authorized. Receipt commitments attest only to authorized commitments, not
correct inference or independently verified payment.

## Functional requirements and acceptance IDs

| ID / slug | Requirement | Acceptance |
|---|---|---|
| R01 / authoritative-sources | Verify repository implementation, original engineering ADRs and all requested vault sources | AT01: exact source matrix; conflicts and acceptance records reviewed |
| R02 / experimental-boundary | Keep unaccepted HotStuff isolated from production settlement | AT02: build/config/docs clearly identify experimental mode; no implicit production activation |
| R03 / canonical-protocol | Versioned canonical transactions, blocks, receipts, IDs and commitments | AT03: stable golden bytes; round trips; noncanonical/unsupported encodings rejected |
| R04 / deterministic-execution | Synchronous candidate-state transitions, committed validated timestamps, checked integer/fixed-point economics | AT04: identical replay commitments and epoch ordering; invalid input leaves state unchanged |
| R05 / durable-recovery | Atomic application records and recovery metadata; flush before publication; validated replay | AT05: injected crashes preserve acknowledged commits; corruption fails closed |
| R06 / hotstuff-correctness | One precisely specified variant with authenticated ancestry, proposals, votes, certificates, locks and commit rule | AT06: no conflicting finality with at most f Byzantine validators; eventual progress after stabilization |
| R07 / restart-safety | Persist vote/lock safety state before externally emitting required votes | AT07: crash/restart cannot produce conflicting votes for a view |
| R08 / safe-membership | Authorized on-chain changes with specified safe activation; no peer-list hot swap | AT08: unauthorized change rejection; old/new committee safety and catch-up tests |
| R09 / bounded-network | Authenticate peers and messages; bound bytes, queues, pending work and verification | AT09: malformed, unauthorized, replayed/stale traffic rejected within configured limits |
| R10 / authenticated-sync | Recover finalized history through validated proofs and deterministic replay | AT10: forged finality, missing ancestry, corrupt snapshots/history rejected |
| R11 / public-node-rpc | Versioned authenticated/rate-limited admission and queries; expose projection freshness | AT11: admission is not finality; stale reads labeled; replay settlement rejected |
| R12 / operational-hardening | Protected keys, contextual errors, safe tracing, health/readiness, metrics and shutdown | AT12: startup/shutdown/error tests; no secrets or private output in logs |
| R13 / economic-boundary | Preserve verified supply, rewards, bonds and accounting; exclude prompts from signed/durable state | AT13: source-backed accounting properties and persistence schema checks |

The [source matrix](miraset-chain-requirements-baseline.md) is authoritative
for evidence availability; this table is a requested behavior contract, not
an assertion that referenced decisions were read.

## Nonfunctional constraints

- All node/protocol implementations are Rust; Tokio is an I/O runtime, not
  a dependency of deterministic state transitions.
- Core dependency direction points inward. Storage, RPC and transports are
  replaceable adapters behind explicit ports.
- Mutable consensus and execution are single-owner loops with bounded channels.
  Finalized state is authoritative; read projections are not a second ledger.
- No production `unwrap`/`expect`, unchecked arithmetic, silent recovery
  defaults, stub success, or unbounded externally triggered work.
- No wall-clock time, external calls, filesystem access, randomness or
  floating-point economics inside replicated transitions.
- No fixed throughput/latency target until workload, hardware and repeatable
  measurements are defined.

## Edge cases

Required cases include duplicate/reordered transactions and consensus messages,
wrong chain/version/purpose signatures, duplicate signers, unknown validators,
stale certificates, equivocation, partitions, failed leaders, malformed keys,
arithmetic overflow, timestamp violations, partial writes, failed flush,
corrupt snapshots, disk exhaustion, incomplete ancestry, overload and shutdown
during pending persistence. Each must have an explicit reject/retry/recovery
contract rather than success-shaped fallback behavior.

## Open decisions and approval record

| Decision | Required evidence / decision owner | Status |
|---|---|---|
| Intended implementation baseline | Repository/ref and original engineering ADRs; maintainer | Missing |
| Settlement architecture | Protocol ADR-0013/0016 and explicit acceptance; protocol owner | Unverified; experimental only |
| Economics and timestamp ordering | SOW, REWARDS, simulator and source decisions; protocol owner | Missing |
| Consensus variant and committee model | Algorithm spec and quorum-intersection reasoning; architect + human reviewer | Proposed scope only |
| Storage and encoding | Durability/compatibility evidence; architect + reviewer | Not selected |
| Reconfiguration and BLS scope | Safe activation spec and crypto decision; protocol/security owners | Not selected |

Requirement Gate: **pending**, not passed.
