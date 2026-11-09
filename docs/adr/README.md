# Engineering ADR register

Protocol ADRs in the external vault use a separate numbering namespace.

## Registered Engineering ADRs

| ADR | Title | Status | Date |
|-----|-------|--------|------|
| [0003](0003-hotstuff-rs-consensus-engine.md) | HotStuff-rs as MIRASET L1 Consensus Engine and Hexagonal Architecture | accepted | 2026-11-07 |

*Note: ADR-0001 (Prompts outside signed/durable chain state) and ADR-0002 (Candidate execution, atomic durable commit, validated replay) were established in foundational baseline requirements.*

## Pending engineering proposals

The [architecture proposal](../../ARCHITECTURE.md) records:

- Hexagonal workspace and single-owner bounded orchestration.
- Durable commit/recovery adapter contract and canonical encoding evaluation.
- Exact experimental HotStuff variant and safety-state persistence.
- Authorized committee activation and authenticated bounded node interfaces.

Design proposals:

- [Vote safety state persistence](proposal-vote-safety-state.md) (TODO 8.3.2, PRD R07) —
  design approved by the project owner 2026-11-09 (§10); implementation in progress. Not a
  numbered ADR until the register is recovered.

Do not assign an engineering number until the original register is recovered
and the next unused number is verified. Do not write synthetic Accepted records
or overwrite ADR-0001/0002.

## Lifecycle

Use `NNNN-short-slug.md` once numbering is verified. Include status, context,
verified sources, decision, alternatives, consequences and approval record.
Proposed records become Accepted only through the required approval gate.
Successors explicitly link to the superseded decision; preserve original
accepted history rather than silently rewriting it.
