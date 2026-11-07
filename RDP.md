# Requirements and Development Plan (RDP)

Status: Draft  
Traceability epic: `miraset-chain-hardening`

This RDP is rewritten as a standard step-by-step workflow.  
Authoritative requirement details remain in the [PRD](docs/business/miraset-chain-prd.md), and detailed implementation sequencing remains in [PLAN.md](PLAN.md).

## Goal

Build a Rust-based, permissioned HotStuff blockchain node in experimental/devnet mode with:

- deterministic execution
- restart-safe consensus behavior
- durable crash-safe recovery
- authenticated and bounded network/RPC surfaces

## Normal implementation steps

### Step 1 — Confirm authoritative sources

1. Read and verify the source map in [docs/business/knowledge-sources.md](docs/business/knowledge-sources.md).
2. Confirm protocol decision status from vault ADR index (`protocol/decisions/README.md`).
3. Confirm implementation baseline and engineering ADR sources.

**Output:** verified source matrix in [docs/business/miraset-chain-requirements-baseline.md](docs/business/miraset-chain-requirements-baseline.md).

---

### Step 2 — Lock scope and boundaries

1. Keep implementation scope to public Rust blockchain code only.
2. Keep product/private components out of this repository.
3. Treat HotStuff as experimental/devnet-only until protocol acceptance changes.

**Output:** aligned scope with no private-product spillover.

---

### Step 3 — Approve architecture before coding

1. Finalize architecture choices from [ARCHITECTURE.md](ARCHITECTURE.md).
2. Define consensus variant, safety rules, storage boundaries, and API boundaries.
3. Record required approvals and unresolved decisions.

**Output:** approved architecture contracts and decision log.

---

### Step 4 — Implement core protocol and execution

1. Implement canonical protocol data structures and versioned encoding.
2. Implement deterministic state transition logic and checked arithmetic.
3. Add tests for canonical bytes, deterministic replay, and invalid input rejection.

**Output:** satisfies PRD R03/R04 (+ R13 invariants where applicable).

---

### Step 5 — Implement durability and restart safety

1. Implement candidate-state execution and atomic commit.
2. Flush durable state before publication.
3. Implement strict recovery validation and deterministic replay.

**Output:** satisfies PRD R05/R07 with crash/restart evidence.

---

### Step 6 — Implement HotStuff, membership, and network surfaces

1. Implement the approved HotStuff variant (proposal/vote/QC/lock/commit/view-change).
2. Implement membership controls and safe activation policy.
3. Implement authenticated, bounded peer sync and public RPC semantics.

**Output:** satisfies PRD R06/R08/R09/R10/R11 in experimental mode.

---

### Step 7 — Operational hardening and evidence

1. Add observability, readiness/health separation, and safe shutdown.
2. Execute fault scenarios and multiprocess failure tests from [docs/stories/miraset-chain-tests.md](docs/stories/miraset-chain-tests.md).
3. Run independent review gates (requirement, architecture, security, code review, audit).

**Output:** satisfies PRD R12 and complete acceptance evidence (AT01–AT13).

## Deliverables

| Deliverable | Definition of done |
|---|---|
| Source-backed requirements | All requirement sources cited and verified |
| Approved architecture package | Contracts + approvals + unresolved items tracked |
| Rust implementation slices | PRD coverage with tests per slice |
| Fault and recovery evidence | Reproducible logs/results for failure scenarios |
| Gate artifacts | Recorded outcomes for required SDLC gates |

## Non-negotiable constraints

- Do not invent missing protocol/economic rules.
- Do not claim production settlement while ADR-0016 is Proposed.
- Do not treat compile/smoke success as acceptance evidence.
- Do not bypass required human approvals for high-risk gates.

## Open gaps to resolve

Current project state highlights the following gaps that still need decisions:

- confirm or restore original engineering ADR sources in this checkout (upstream direct `docs/adr/0001` and `0002` paths currently not found)
- validate the pinned baseline (`b14588025d9641ab26dcab67573bfdeb51227e6d`, branch `dev`) against the intended historical implementation revision
- review and ratify the proposed HotStuff v1 contract in `ARCHITECTURE.md` (or replace it with an approved alternative)
- lock dependency policy: no unmaintained HotStuff repository dependencies
- complete required architecture/security/review gate outcomes

## References

- [PRD](docs/business/miraset-chain-prd.md)
- [PLAN](PLAN.md)
- [Architecture](ARCHITECTURE.md)
- [Requirements baseline](docs/business/miraset-chain-requirements-baseline.md)
- [Knowledge sources](docs/business/knowledge-sources.md)
- [Test strategy](docs/stories/miraset-chain-tests.md)
