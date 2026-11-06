# Requirements and Development Plan (RDP)

Status: Draft. Traceability epic: `miraset-chain-hardening`.

"RDP" is interpreted here as Requirements and Development Plan. Detailed
requirements live in the [PRD](docs/business/miraset-chain-prd.md); executable
work sequencing lives in [PLAN.md](PLAN.md). These are the canonical documents
for their respective subjects, rather than duplicate specifications.

## Objective

Develop a Rust-only, self-hosted permissioned HotStuff experimental chain
with deterministic execution, restart-safe consensus, durable recovery, and
bounded authenticated interfaces. Prefer mature compatible libraries, while
keeping protocol rules and authoritative state independent of adapters.

## Evidence and authority

The [baseline matrix](docs/business/miraset-chain-requirements-baseline.md)
records the inspected starter checkout and missing source inputs.
Requirements in this draft derive from the user's request; unavailable vault
documents are not treated as verified facts.

Protocol ADR-0016 is Proposed according to the request. Until acceptance is
verified, work must remain isolated experimental/devnet work and must not
supersede protocol ADR-0013. Original engineering ADR-0001 and ADR-0002 must
be obtained and reviewed before implementation. Their numbering is separate
from protocol ADR numbering.

## Deliverables

| Deliverable | Current state | Completion condition |
|---|---|---|
| Source-backed requirements | Draft, source gaps recorded | Exact source lines, statuses, conflicts resolved |
| Architecture and dependency boundaries | Proposed | Original ADRs reviewed; human Architecture approval |
| Execution and crash recovery | Not implemented | Determinism, accounting and crash tests pass |
| Consensus and membership | Not implemented | Specified safety/liveness and activation tests pass |
| Network, synchronization and RPC | Not implemented | Authenticated bounded admission and recovery demonstrated |
| Operational evidence | Not implemented | Fault simulation, multiprocess tests, independent review |

## Decision boundaries

No default is invented for supply, reward rates, bonds, credits-to-MIR mapping,
validator rewards, production settlement, committee changes, or BLS usage.
The architecture proposal narrows engineering options without accepting them.
Documentation and test-design work can continue independently; code requiring
these unresolved contracts cannot be presented as conforming implementation.

## Release constraint

An experimental milestone is not production acceptance. All criteria in the
[test strategy](docs/stories/miraset-chain-tests.md) require actual evidence,
and deployment authorization remains separate from implementation review.
