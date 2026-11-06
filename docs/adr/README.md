# Engineering ADR register

Status: Original engineering decisions unavailable; no numbered ADR created.

The user identifies engineering ADR-0001 (prompts outside signed/durable chain
state) and ADR-0002 (candidate execution, atomic durable commit, validated
replay) as Accepted. Their original files were not found in this checkout.
Those descriptions are constraints from the request, not reconstructed ADRs.

Protocol ADRs in the external vault use a separate numbering namespace.
Protocol ADR-0016 must not be represented as engineering ADR-0016.

## Pending engineering proposals

The [architecture proposal](../../ARCHITECTURE.md) records:

- Hexagonal workspace and single-owner bounded orchestration.
- Durable commit/recovery adapter contract and canonical encoding evaluation.
- Exact experimental HotStuff variant and safety-state persistence.
- Authorized committee activation and authenticated bounded node interfaces.

Do not assign an engineering number until the original register is recovered
and the next unused number is verified. Do not write synthetic Accepted records
or overwrite ADR-0001/0002.

## Lifecycle

Use `NNNN-short-slug.md` once numbering is verified. Include status, context,
verified sources, decision, alternatives, consequences and approval record.
Proposed records become Accepted only through the required approval gate.
Successors explicitly link to the superseded decision; preserve original
accepted history rather than silently rewriting it.
