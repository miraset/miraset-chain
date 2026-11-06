# MIRASET Chain implementation plan

Status: Draft; implementation blocked on authoritative inputs and gated review.
Traceability epic: `miraset-chain-hardening`.

## Reviewable vertical slices

| Slice / task slug | Prerequisites | Deliverable | Exit evidence |
|---|---|---|---|
| A / authoritative-sources | Intended checkout and vault access | Verified source matrix, PRD, approved architecture and stories | AT01; original ADRs read; Requirement and Architecture gates |
| B / deterministic-recovery | A; approved economics, encoding and durability contract | Canonical protocol, candidate execution, atomic persistence and replay | AT03-05, AT13; crash injection, epoch replay, corruption rejection |
| C / restart-safe-hotstuff | B; coherent algorithm and quorum spec | Votes, certificates, ancestry, locking, pacemaker and safety persistence | AT06-07; deterministic adversarial simulation |
| D / safe-membership | C; authorized activation protocol | Fixed authorized genesis committee; separately reviewed on-chain reconfiguration | AT08; transition/partition/restart proofs and tests |
| E / bounded-node-interfaces | B-C; D for reconfiguration exposure | Authenticated transport, finalized-history sync and versioned RPC | AT09-11; malformed/overload and recovery tests |
| F / operational-fault-evidence | B-E | Protected keys, health/readiness, metrics, shutdown, multiprocess fault harness | AT02, AT12; real faults, independent security/review and DoD audit |

Bounds, errors and observability belong in each slice; F consolidates them
rather than retrofitting safety after unbounded code is shipped. D may first
deliver a fixed genesis committee with reconfiguration rejected. That is not
completion of the requested safe reconfiguration capability.

## Per-slice execution

1. Route analyst -> PM -> architect -> SM with shared slug and prior artifact.
2. Record exact scope, dependencies, assumptions, risk and autonomy cap.
3. Obtain human Architecture approval for high-risk code.
4. Route dev -> QA -> security -> reviewer -> auditor; do not self-validate the
   next gate and do not advance with unresolved checklist items.
5. Record human Security and Code Review approvals. Hand off passed artifacts
   to CI/CD, without deploying or changing repository visibility.

Required role availability is recorded in the
[baseline](docs/business/miraset-chain-requirements-baseline.md).
This documentation pass does not impersonate these roles or pass their gates.
High-risk consensus, recovery, authentication and settlement require the full
flow. Approval absence is a blocker, not inferred consent.

## Validation commands

Run existing tests before changes. After each Rust slice, run the smallest
relevant Cargo test selectors plus:

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Use `cargo test module::tests::test_name -- --exact` before a workspace exists,
or the appropriate `-p` package selector after it exists. Record exact commands,
toolchain, seeds, exit codes and tested revision. Formatting/clippy success
does not replace algorithm/economic acceptance tests.

## Risk and estimation

Documentation-only work can proceed inline. Proposed implementation slices
are high-risk because consensus, durable history, authorization and accounting
can violate safety or financial invariants. Per-story autonomy caps must be
assigned by SM under regulatory guidance; no cap is relaxed by this plan.

Engineering estimates in hours/weeks are **not yet defensible**: baseline,
economics, algorithm, storage and reusable library suitability are unknown.
After A, estimate each reviewed story with assumptions and uncertainty rather
than promising a full blockchain deadline.

## Current checkpoint

Draft RDP/PRD, architecture, plan and acceptance strategy exist.
Source-backed requirement approval, specialist reviews, implementation,
security scanning, code review, human approvals and deployment are pending.
Do not describe this documentation checkpoint as slice A acceptance.
