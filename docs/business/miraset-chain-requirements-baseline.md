# MIRASET Chain requirements baseline

Traceability slug: `miraset-chain-hardening`

Status: **BLOCKED before implementation**. This is a source-gap inventory,
not an approved specification, architecture decision, or readiness assessment.

Update 2026-11-06: the maintainer supplied an accessible vault at
`/home/daochild/.obsidian/miraset-knowledge-vault`. The initial source-gap
inventory below is historical; a full requirements cross-check is still
pending. See [knowledge sources](knowledge-sources.md) for verified access,
source precedence and PoCC terminology. Original engineering ADRs remain
unavailable in this checkout.

## Verified checkout

Baseline commit: `b9b3886` (`add opencode config`).

- `Cargo.toml:1-7` declares a Rust 2024 binary package with no dependencies
  and no workspace declaration.
- `src/main.rs:1-3` prints `Hello, world!`; it contains no chain implementation.
- `AGENTS.md:1-3` delegates workflow instructions to the installed
  `@daochild/agents-config` package.
- The tracked-file inventory has no tests, `docs/adr`, storage implementation,
  consensus implementation, or authoritative protocol documents.
- At baseline inspection, `ARCHITECTURE.md`, `CONTRIBUTING.md`, and
  `SECURITY.md` were absent. They now exist as draft documentation, not as
  evidence of implemented behavior.

Command: `cargo test`

Result: exit 0; **zero tests**. This verifies only that the starter binary
compiles under the test profile, not any blockchain behavior.

## Requirements matrix

The requested vault and engineering sources are unavailable in this checkout.
Their contents and line numbers have not been verified. "User-stated" below
means a constraint supplied in the implementation request, not a verified
decision status. No protocol ADR number is used as engineering ADR evidence.

| Requirement | Source file:line | Decision status | Existing implementation | Missing work | Acceptance test |
|---|---|---|---|---|---|
| Establish authoritative requirements | `AGENTS.md:1-3`; vault sources unavailable | Blocked on sources | Starter package only | Obtain source checkout and vault; complete exact citations | Every requirement traced to verified source and status |
| Preserve approved settlement architecture; isolate HotStuff | Protocol decisions 0013 and 0016: lines unavailable | 0016 Proposed, user-stated; acceptance unverified | None | Verify decisions; retain experimental/devnet boundary | Experimental path cannot be mistaken for approved production settlement |
| Hexagonal modular Rust workspace and bounded single-owner orchestration | `Cargo.toml:1-7`; architecture requested by user | Requested; implementation architecture gate pending | Single binary | Analyze approved baseline; specify ports and crate boundaries | Inward dependencies; independently testable synchronous transitions |
| Approved Ed25519, Blake3, and restricted BLS scope | Protocol decision 0008: lines unavailable | Unverified | None | Verify approved uses; select compatible mature libraries | Invalid keys/signatures and cross-domain replay rejected |
| Durable candidate execution and validated replay | Engineering ADR-0002: absent | Accepted, user-stated; source unverified | None | Obtain ADR; specify atomic write, flush, publication, recovery contract | Crash injection; corrupt/incomplete history fails closed |
| Deterministic epochs and economic invariants | Protocol SOW, REWARDS, simulator: lines unavailable | Unverified | None | Verify supply, bonds, rewards, rounding, ordering, settlement mapping | Equal commitments; checked arithmetic; epoch-boundary deterministic replay |
| Precisely specified HotStuff safety and liveness | Protocol decision 0016: lines unavailable | Experimental only pending verified acceptance | None | Select coherent variant; specify quorum, ancestry, lock and commit rules, pacemaker | Partition/equivocation/reordering simulations; no conflicting finality within fault bound; eventual progress |
| Persist safety state before voting | Algorithm specification and engineering sources unavailable | Required by user; algorithm unspecified | None | Define restart-safe vote and lock persistence | Crash/restart never permits conflicting votes |
| Authorized safe committee reconfiguration | Protocol decision 0016: lines unavailable | Unverified | None | Specify on-chain authorization and safe activation, not peer-list hot swapping | Unauthorized changes rejected; quorum intersection across activation |
| Authenticated bounded transport, history sync, and node RPC | Protocol decision 0016: lines unavailable | Unverified | None | Specify admission, authentication, limits, finality proofs and projections | Replay/malformed overload rejection; authenticated recovery; freshness exposed |
| Public chain/private product separation; no durable prompts | Engineering ADR-0001 absent; product decision 0001 and chain decisions: lines unavailable | Accepted engineering constraint, user-stated; sources unverified | No product or chain code | Verify boundary; exclude private services and prompt persistence | Signed and persisted chain records contain no prompts or private inference output |
| Operational hardening and independent verification | User request; source specification pending | Requested; gates pending | None | Protected keys, health/readiness, tracing, metrics, shutdown, real fault tests | Bounded resources; multiprocess failure recovery; repeatable commands and measured results |

## Required source inputs

Provide an accessible vault root containing:

- `protocol/decisions/README.md`
- `protocol/decisions/0013-hybrid-sui-coordinator.md`
- `protocol/decisions/0016-permissioned-hotstuff-network.md`
- `protocol/decisions/0008-crypto-suite.md`
- `protocol/SOW.md`
- `protocol/REWARDS.md`
- `finance/simulations/financial_simulator.py`
- `product/development/MIRASET_CHAIN_DECISIONS.md`
- `product/development/decisions/0001-public-chain-private-product-code.md`

Also provide the intended implementation checkout/ref and original engineering
ADRs, especially ADR-0001 and ADR-0002. Do not reconstruct accepted engineering
ADRs from a vault summary or the request.

## Workflow prerequisites

The installed SDLC regulatory guidance was read from
`node_modules/@daochild/agents-config/instructions/skills/sdlc-regulatory/SKILL.md`.
Its nine required specialist roles are not exposed as callable agent types in
this session: `sdlc-analyst`, `sdlc-pm`, `sdlc-architect`, `sdlc-sm`, `sdlc-dev`,
`sdlc-qa`, `sdlc-security`, `sdlc-reviewer`, and `sdlc-auditor`.
The orchestrator cannot claim their gates passed or substitute its own work
for their independent validation.

Consensus, key handling, durable recovery, membership, and settlement are
high-risk changes. Architecture, Security, and Code Review require explicit
human approvals under the configured workflow. No such gate has passed.
The requested Hexagonal direction is recorded, but no algorithm, storage,
economic, or reconfiguration decision is approved by this document.

## Implemented scope and limitations

This baseline inventory and the linked [draft documentation package](../README.md)
were added. No Rust behavior, dependency,
repository visibility, accepted decision, or production architecture was
changed. No blockchain acceptance evidence exists in this checkout.

Continuation requires the authoritative sources and available specialist
roles. Independent non-conflicting baseline inspection is complete; inventing
a settlement model or replacing the missing implementation with a scaffold
would not satisfy the request.
