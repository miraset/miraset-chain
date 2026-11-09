# MIRASET Chain architecture

Status: Proposed target architecture; not implemented or gate-approved.
Traceability epic: `miraset-chain-hardening`.

## Current architecture

`Cargo.toml` defines the `miraset-chain` library and binary crate (`src/lib.rs` and `src/main.rs`) organized into Hexagonal modules (ADR-0003):
- `src/core/types.rs`: domain types (`Transaction`, `ComputeReceipt`, `WorkerBondAction`, `WorkerInfo`, `EpochInfo`, `EpochRewardConfig`, `BlockPayload`).
- `src/app/chain_app.rs`: `MirasetApp` implementing `hotstuff_rs::app::App<K>` for deterministic block production and validation.
- `src/mempool/pool.rs`: `Mempool` for transaction and receipt admission/deduplication.
- `src/storage/`: `KVStore` adapters (`MemKVStore` and durable `RedbKVStore`).
- `src/network/`: `Network` adapters (`ChannelNetwork` and length-delimited `TcpNetwork`).
- `src/node/`: `NodeConfig` (enforcing `experimental_mode = true`) and `MirasetNode` orchestrator.
- `src/rpc/server.rs`: Axum HTTP/SSE RPC adapter.

See the [baseline](docs/business/miraset-chain-requirements-baseline.md) for source reconciliation.

## R03–R12 implementation contracts (experimental devnet boundary)

The table below specifies the concrete engineering contracts for closing the R03–R12 backlog items while keeping HotStuff isolated as experimental/devnet-only until protocol ADR-0016 is accepted:

| Requirement | Specified contract |
|---|---|
| **R03 Canonical encoding** | Versioned binary envelope (`PROTOCOL_ENCODING_VERSION = 1`) and domain-separated signing/commitment tags (`MIRASET:TX:v1`, `MIRASET:RECEIPT:v1`, `MIRASET:BLOCK:v1`, `MIRASET:STATE:v1`). Reject unknown versions, trailing bytes, or noncanonical encodings. |
| **R04 Deterministic execution** | Replace saturating arithmetic in consensus-visible balance/stake/nonce/reward accounting with checked arithmetic (`checked_add`/`checked_sub`), rejecting invalid or overflowing transitions atomically. Deterministically order transactions and receipts and bind a deterministic state commitment root (`state_root`) into block validation. |
| **R05 Durable commit & recovery** | Persist versioned storage metadata (`schema_version`, `chain_id`, `genesis_hash`) atomically with state updates in `RedbKVStore`, and run fail-closed recovery validation on node startup before participating in consensus. |
| **R06 HotStuff variant** | Vendored `hotstuff_rs` v0.4.0 (`lib/hotstuff_rs`, patches in `lib/hotstuff_rs/MIRASET_PATCHES.md`) Chained HotStuff with 3-chain commit finality (`Prepare -> PreCommit -> Commit`), verified parent/justify QC ancestry, and view synchronization via pacemaker. Upstream v0.4.0 defects are patched in the vendored copy: empty `KVGet::block_data`, whole-second truncation of `max_view_time`, and busy-polling event-bus/block-sync-server threads. Liveness limitation: commits need four consecutive live leaders (strict rotation + consecutive-view 3-chain), so an equal-power 4-validator set halts finality while any one validator is down; use n ≥ 5 for crash-fault testing. |
| **R07 Restart vote/lock safety** | Persist highest voted view/phase and locked QC safety state durably on disk (`SafetyStore`) before emitting validator signatures, preventing equivocation after crash/restart. |
| **R08 Validator authorization** | Keep dynamic validator-set reconfiguration disabled (`validator_set_updates = None`) until an on-chain transition proof and activation boundary are approved; static peer flags cannot mutate consensus committee membership. |
| **R09 Network identity & bounds** | Authenticate validator frames using Ed25519 envelope signatures bound to `chain_id` and sender `VerifyingKey`; enforce frame size (`16 MiB`), connection (`64`), and bounded queue limits (`4096`). |
| **R10 Authenticated sync** | Verify chain ID, block payload hash, state commitment, and parent ancestry during `validate_block_for_sync` and snapshot verification; fail closed on corrupted or mismatched history. |
| **R11 Public node RPC** | Versioned/rate-limited admission and query endpoints that distinguish mempool admission (`accepted`/`admitted`) from 3-chain finality (`committed`) and expose committed block height/hash freshness. |
| **R12 Operational safety & keys** | Enforce `experimental_mode = true` in `NodeConfig::validate()`, require explicit key custody (rejecting implicit fallback keys in non-dev persistent mode with file-permission checks), and provide distinct `/health`, `/livez`, and `/readyz` probes. |

## Proposed Cargo boundaries

Map these responsibilities onto the authoritative existing implementation if
it is supplied. For the inspected starter, the smallest justified target is:

```text
Cargo.toml                  workspace and node package manifest
src/main.rs                 node composition root
crates/chain-core/          protocol, crypto contracts, execution, consensus
crates/chain-app/           orchestration and I/O ports
crates/chain-infra/         storage, signer, transport and RPC adapters
```

These directories are proposals, not generated crate scaffolds.
Dependency graph: node -> app + infra; infra -> app + core; app -> core.
Core modules must not depend on app, Tokio, databases or RPC frameworks.
Keep consensus and execution as modules initially; split only for enforceable
dependencies, distinct consumers or independent testing needs.

## State ownership and ports

Consensus owns views, proposals, certificates, locks and vote safety state.
Execution owns candidate and finalized application state. Each has one mutable
owner; bounded directed messages connect them. No general event bus, actor
framework or shared `Arc<Mutex<ChainState>>` coordination.

| Port | Responsibility | Required failure behavior |
|---|---|---|
| SafetyStore | Load and durably persist vote/lock state | Halt voting on ambiguous/failed durability |
| ChainStore | Atomic commit bundle, flush, validated history reads | Never publish failed/uncertain commit |
| Signer | Purpose-bound signing with protected key custody | Typed error; no key material in logs |
| PeerTransport | Authenticated bounded sends/receives | Explicit backpressure/rejection/disconnect |
| Clock | Monotonic pacemaker deadlines outside core | Timeout event only; never execution epoch input |
| QueryProjection | Finalized read snapshot and projection height | Expose freshness; cannot authorize transitions |

Port signatures and cancellation contracts must be finalized in the
architecture review before code. Adapters isolate blocking disk work and
bounded CPU verification from async I/O; results return to owning loops.
Rejected queue admission must be visible to callers and metrics.

## Deterministic execution and commit lifecycle

1. Validate canonical transaction, authorization, replay protection and
   consensus-established finalized block ordering.
2. Apply transactions to candidate state using checked arithmetic and
   source-specified rounding and epoch transition order.
3. Derive application records, receipts and state commitment.
4. Atomically persist records plus replay/recovery metadata and any coordinated
   consensus recovery references; flush under the backend durability contract.
5. Only after successful durability publish finalized state, receipts and
   read projections.

If a flush fails or completion is uncertain, stop publication and require
validated recovery; do not retry settlement blindly. Application commit and
consensus restart metadata must describe a coherent recovery point. Exact
transaction boundaries are contingent on original engineering ADR-0002.

Recovery validates schema/version, chain/genesis identity, height continuity,
ancestry, finalized proofs, encoding and deterministic commitments.
Snapshots must be bound to an authenticated finalized checkpoint and state
root, with validated schema and integrity. A hash alone is not proof of
authenticity. Missing/corrupt history fails closed. Migration and rollback
must be explicit, versioned and tested; no automatic destructive repair.

## Consensus specification prerequisite

No existing HotStuff variant was found. Do not claim one is selected here.
Before plan phase P4 (see [PLAN.md](PLAN.md)), a reviewed specification must
define proposal/vote phases,
certificate meaning, safe-vote predicate, lock/unlock predicate, ancestry
requirements, exact commit rule, new-view messages, timeout certificates,
leader scheduling and pacemaker transitions as one coherent algorithm.

Baseline committee candidate: equal weight, n = 3f + 1, quorum = 2f + 1
distinct authorized valid signers. Two quorums intersect in at least f + 1
validators, at least one honest under the stated bound. Intersection alone
does not establish safety: persistent non-equivocation, lock and commit rules
must complete the argument. Reject other committee shapes until explicitly
specified. A QC is not automatically finality.

Signatures bind chain ID, protocol version, purpose, view, committee identity
and canonical payload. Reject duplicate signers and invalid/stale certificates.
Persist algorithm-required safety state before releasing signatures/messages.
Timeouts cannot authorize conflicting votes. Tests must cover restart of both
safety state and available ancestry.

Reconfiguration remains disabled until an on-chain authorization and
safe committee activation specification is approved. Define the old committee's
finalized transition proof, activation boundary, new committee bootstrap,
state transfer and quorum safety across epochs; a transport peer change is not
a validator change.

## Proposed HotStuff v1 contract (for architecture review)

This section defines a concrete proposal to reduce implementation ambiguity.
It is a candidate contract for review and approval, not an accepted protocol
decision.

### Variant

- Chained HotStuff, 3 phases per view: `Prepare -> PreCommit -> Commit`.
- Equal validator weights with static committee in v1:
  - committee size `n = 3f + 1`
  - quorum threshold `2f + 1` distinct valid signatures
- Leader schedule: deterministic round-robin by view number over committee order.

### Block and certificate model

- Each proposed block includes:
  - `height`, `view`, `parent_id`, `payload_root`, `state_root_candidate`,
    `timestamp`, `proposer_id`, `chain_id`, `protocol_version`.
- A quorum certificate (QC) binds:
  - `phase`, `view`, `block_id`, `chain_id`, `protocol_version`,
    `committee_id`, signer bitmap/list, aggregated signature bytes or signer set.
- Genesis carries a fixed genesis QC and fixed committee identity.

### Safety rules

- **Vote eligibility:** a validator votes only if:
  1. proposal extends known ancestry
  2. proposal carries valid parent/justify QC
  3. proposal view is strictly greater than last voted view
  4. proposal does not violate current lock rule.
- **Lock rule:** lock on highest valid `PreCommit` QC seen.
- **Safe vote rule:** vote only for proposals extending the locked block, unless
  a higher-view QC justifies lock advancement.
- **Persistence rule:** persist vote/lock safety state durably before emitting a vote.
- **No equivocation:** at most one vote per `(view, phase)` per validator key.

### Commit and finality rule

- A block `B` is finalized when there is a 3-chain of valid QCs:
  - `QC_prepare(B)`
  - `QC_precommit(child_of_B)`
  - `QC_commit(grandchild_of_B)`.
- Finalization is strictly monotonic by height.
- Finalized blocks must be contiguous with no gaps.

### View change and pacemaker

- Each validator has a local timeout schedule:
  - `timeout(view) = base_timeout * 2^k`, capped at `max_timeout`,
    where `k` is consecutive timeout count.
- On timeout, validator emits signed timeout message for `(view, high_qc)`.
- A timeout certificate (TC) forms at quorum and moves the protocol to next view.
- New-view message includes validator high QC and optional TC.
- Next leader proposes using highest justified QC from gathered new-view data.

### Replay and restart invariants

- On restart, node reconstructs:
  - highest QC
  - lock QC
  - last voted `(view, phase)`
  - finalized tip and parent continuity.
- Node refuses to resume voting if persisted safety state is missing/ambiguous.

### Out-of-scope in v1

- Dynamic validator reconfiguration
- Weighted voting
- Slashing semantics
- BLS threshold scheme migration details

## Library evaluation shortlist

These are candidates, not verified selections or new dependencies.

| Concern | Candidate | Selection evidence required |
|---|---|---|
| Async I/O / channels | Tokio | Bounded queues, cancellation and blocking isolation |
| Ed25519 / hashing | ed25519-dalek, blake3 | Approved crypto scope, strict verification and key validation |
| Typed/contextual errors | thiserror, anyhow | Library/application separation and explicit failure propagation |
| Atomic persistence | redb; RocksDB if requirements justify it | Commit/flush crash semantics, deployment portability, existing backend compatibility |
| RPC / transport | axum, rustls; libp2p if justified | Identity model, size limits, TLS/auth, complexity and license review |
| Observability | tracing; metrics ecosystem | Secret-safe instrumentation and bounded label cardinality |
| Property testing | proptest | Reproducible seeds, shrinking and invariant coverage |
| Encoding | Explicit versioned envelope plus reviewed codec | Canonical bytes, bounded decoding, schema evolution and golden vectors |

Investigate maintained Rust HotStuff libraries before writing consensus; reuse
only when the exact variant, safety persistence, committee rules and licensing
fit the reviewed specification. Do not substitute Raft for HotStuff.
Do not introduce direct dependencies on unmaintained HotStuff repositories.
If no maintained library matches the required contract, implement the consensus
core in-repo with explicit tests and versioned compatibility boundaries.
Do not introduce BLS until its approved scope is verified. Aggregation is not
threshold signing; signer accounting, key validation and rogue-key protection
require an explicit reviewed scheme.

## RPC, synchronization and lifecycle

RPC admission reports rejection or admission, never unearned finality.
Queries expose finalized height/root, projection height and freshness.
Recovering or non-quorate nodes must not claim readiness for finalizing work;
liveness and process health are separate signals.

Peer sync validates chain/committee identity and finalized proofs before
applying history. Bound concurrent peers, requests, byte budgets, ancestry
fetches and verification work. Validate configurable limits before startup.
TLS alone does not authorize a validator or settlement signer.

Shutdown stops new admission, drains bounded accepted work according to
documented deadlines, completes required durable writes and exits with explicit
failure on uncertain storage. Restart runs recovery before voting/readiness.
Private worker streams and prompt/dispatch persistence are out of scope.

## Approval and ADR handling

The user requested Hexagonal Architecture; algorithm, storage, encoding,
economics and reconfiguration choices remain unresolved.
[ADR policy](docs/adr/README.md) reserves engineering numbering until original
ADRs are recovered. This proposal accepts/supersedes no ADR.
Architecture Gate and human approval: **pending**.
