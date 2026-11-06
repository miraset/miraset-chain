# MIRASET Chain acceptance and fault-test strategy

Status: Proposed tests; none implemented or executed.
Traceability: [PRD acceptance IDs](../business/miraset-chain-prd.md).
Testing Gate: pending.

## Test layers

| Layer | Required coverage | Evidence |
|---|---|---|
| Unit / golden vectors | Canonical encoding, strict signatures, checked arithmetic, transition predicates | Versioned fixtures; boundary cases; unchanged state on rejection |
| Property / replay | Transaction permutations where permitted, epochs, supply/accounting, deterministic commitments | Seeded generators, shrinking, source-backed invariant assertions |
| Deterministic consensus simulation | Partitions, Byzantine proposals/votes, message duplication/reordering, stale QCs, timeout/view changes | Explicit fault bound, event trace, equal finalized prefix and commitments |
| Storage fault injection | Vote persistence; atomic commit; flush; publication; snapshot and replay | Restart at each boundary, before/after-write and uncertain completion cases |
| Adapter / integration | Authentication, admission, bounded queues, sync and projections | Negative cases; explicit limits; no admission-as-finality responses |
| Real multiprocess | Process death/restart, dropped/delayed links, failed leaders, partitions and healing | Reproducible harness, revision/config, final roots/heights and resource measurements |

A four-validator happy path is smoke coverage only, not fault-bound safety or
production readiness. Simulation must model legal Byzantine actions without
allowing honest nodes to violate the selected algorithm.

## Mandatory assertions

- AT01-02: source and approval records exist; experimental mode cannot implicitly
  activate production settlement.
- AT03-04: identical canonical bytes and state roots across validators/replay;
  unsupported versions, invalid timestamps and overflow rejected atomically.
- AT05: crashes before commit yield no published result; after durable commit
  replay recovers it even if publication never happened; failed/uncertain flush
  never causes premature publication.
- AT06: no two honest validators finalize conflicting blocks at any height
  within the stated fault bound; after stabilization and sufficient honest
  quorum, all healthy validators advance. Define scheduler fairness and the
  exact bounded simulation window per scenario, not a throughput promise.
- AT07: crash before/after safety persistence and before/after vote send never
  permits conflicting same-view votes after restart.
- AT08: unauthorized registry changes rejected; transitions across committees
  preserve the specified safety invariant; peer-list edits cannot activate one.
- AT09: message bytes, queue occupancy, pending work and concurrent verification
  stay within configured limits under invalid traffic. Measure memory/CPU under
  a defined workload, not merely successful rejection counts.
- AT10: corrupt snapshot content, wrong roots/chain/committee, forged proofs,
  missing ancestry, truncation and unsupported schema fail closed.
- AT11: replay cannot double-settle; rejected/admitted/finalized results differ;
  query freshness explicitly reflects finalized and projection heights.
- AT12: startup recovery precedes readiness/voting; shutdown during writes
  preserves recovery guarantees; captured logs contain no secrets/private data.
- AT13: source-approved supply, bonds and accounting identities hold at every
  transition and epoch; prompt fields are absent from durable/signed schemas.

## Evidence record

For each scenario record acceptance ID, implementation revision, command,
toolchain, seed/config, fault schedule, expected invariant, result, state
roots/heights, failures and resource measurements. Failures retain a minimal
reproducer. Never mark tests passed from this design document.

QA owns the test artifact review independently of the implementation author.
Security and reviewer roles inspect assumptions/fault models; the auditor
checks requirements-to-tests coverage and recorded human approvals.
