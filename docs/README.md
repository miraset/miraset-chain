# Documentation index and governance

Status: Draft engineering package. Documentation authoring is complete only
as a draft; source verification and SDLC implementation gates remain pending.

## Structure and canonical locations

```text
README.md                   current executable status and development entry
RDP.md                      requirements/development overview
PLAN.md                     vertical slices, prerequisites and gate sequence
ARCHITECTURE.md             current and proposed architecture
CONTRIBUTING.md             contributor workflow and Rust checks
SECURITY.md                 reporting and proposed threat controls
docs/
  business/                 source matrix and PRD
  adr/                      engineering decision register and lifecycle
  stories/                  acceptance/fault-test strategy
```

| Document | Purpose |
|---|---|
| [RDP](../RDP.md) | Scope and deliverable status |
| [PRD](business/miraset-chain-prd.md) | Requested behavior, acceptance IDs and unresolved decisions |
| [Baseline matrix](business/miraset-chain-requirements-baseline.md) | Verified checkout evidence and missing source inventory |
| [Knowledge sources](business/knowledge-sources.md) | Authoritative vault locator, PoCC terminology and source precedence |
| [Plan](../PLAN.md) | Work ordering, dependency/approval gates |
| [Architecture](../ARCHITECTURE.md) | Proposed boundaries and safety/persistence contracts |
| [ADR register](adr/README.md) | Separate numbering and immutable decision history |
| [Test strategy](stories/miraset-chain-tests.md) | Required fault evidence, not test results |
| [Contributing](../CONTRIBUTING.md) | Workflow and validation commands |
| [Security](../SECURITY.md) | Trust boundaries, reporting and controls |

## Ownership and maintenance

Repository maintainers own approval and lifecycle of this package; no named
team or approval is inferred. Analyst/PM own source-backed requirements;
architect owns technical decisions; SM owns story risk/caps; QA owns test
evidence; security/reviewer/auditor own independent reviews. These roles are
required review responsibilities, not claims that agents already performed them.

Use descriptive kebab-case document names within existing directories.
Keep one canonical document per subject and link instead of duplicating it.
Every new artifact states status, traceability slug, source/evidence gaps and
gate outcome. Update artifacts when implementation or decisions change.
Keep accepted decisions and release evidence; archive obsolete proposals with
explicit successor links. Do not delete acceptance history.

Create API, database, runbook, security/review and deployment artifacts when
their slice defines an actual reviewed surface. Empty skeletons must not be
misrepresented as completed deliverables. Verify relative links and
documentation claims before handoff.
