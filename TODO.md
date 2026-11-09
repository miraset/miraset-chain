# MIRASET Chain Component Connection Roadmap & TODOs

This document tracks prioritized integration tasks for connecting external and adjacent ecosystem components to `miraset-chain` (HotStuff-rs L1 consensus engine).

---

## 1. Node RPC & API Gateway (Phase 1 — Immediate)
- [x] **1.1. Core RPC Server Infrastructure**
  - Add `tokio` (rt, sync, macros) and `axum` (or lightweight async HTTP router) dependencies to `Cargo.toml`.
  - Create `src/rpc/mod.rs` and `src/rpc/server.rs` defining `RpcServer` and `RpcState`.
  - Support configurable bind address and CORS headers.
- [x] **1.2. Transaction & Ingestion Endpoints**
  - `POST /tx/submit`: Accept serialized `Transaction`, validate syntax/nonce, insert into `Mempool`, return hash/status.
  - `POST /receipt/submit`: Accept `ComputeReceipt`, validate non-empty fields, insert into `Mempool`.
- [x] **1.3. State & Block Tree Query Endpoints**
  - `GET /account/{address}`: Query committed balance and nonce via `BlockTreeCamera`.
  - `GET /worker/{worker_id}`: Query worker info (stake, completed tasks, accumulated rewards) via `BlockTreeCamera`.
  - `GET /block/latest`: Retrieve highest committed block hash and consensus status.
  - `GET /mempool/status`: Query pending transaction and compute receipt counts.
  - `GET /health`: Node liveness and readiness probe.
- [x] **1.4. Client & Integration Testing**
  - Write integration tests verifying HTTP submission of transactions and receipts against a live `MirasetNode`.

---

## 2. Durable Storage Engine Adapter (Phase 2)
- [x] **2.1. Disk-Backed KVStore Implementation**
  - Implement `hotstuff_rs::block_tree::pluggables::KVStore` using persistent embedded database (`redb`).
  - Implement atomic `WriteBatch` ensuring durability before vote emission (ADR-0002).
- [x] **2.2. Crash Recovery & Checkpoint Verification**
  - Reconstruct BlockTree state, highest QC, locked QC, and account states on node restart.
  - Integration tests in `tests/durable_storage.rs` verifying state survives node restart and clean drops.

---

## 3. P2P Transport Adapter (Phase 3)
- [x] **3.1. Framed TCP Network Protocol**
  - Implement `hotstuff_rs::networking::network::Network` over length-delimited framed TCP connections (porting proven design from `demo/crates/miraset-node/src/hotstuff.rs`).
  - Support point-to-point direct validator connections and broadcast queues.
- [x] **3.2. Peer Discovery & DoS Protection**
  - Static validator peer registry for devnet/testnet.
  - Bounded buffers, frame size validation (e.g. max 16 MiB), and disconnect on malformed messages.

---

## 4. GPU Worker, Coordinator & PoCC Verification Pipeline (Phase 4)
*Architecture note: The GPU Worker runs as an independent client-side program/daemon on the provider's machine next to the local inference backend (Ollama, vLLM). It executes tasks and streams outputs directly to the client. The Coordinator verifies client receipt confirmation, co-signs the receipt with client cost debit, and submits it to MIRASET L1 for trustless consensus settlement.*

- [x] **4.1. Compute Receipt & Client Debit Data Structures**
  - Extended `ComputeReceipt` with `client_address`, `coordinator_id`, `tokens_computed`, `client_cost`, `worker_reward`, `client_confirmed`, and coordinator `signature`.
  - Added RPC endpoint `GET /task/{task_id}` returning `TaskSettlementResponse` for Coordinator and Client tracking.
- [x] **4.2. On-Chain Settlement, Client Debit & Anti-Replay**
  - Enforced client delivery confirmation (`client_confirmed == true`) in `MirasetApp::compute_state_updates`.
  - Atomically debited client balance for `client_cost` upon task completion.
  - Implemented on-chain anti-replay using persistent `task:<task_id>` state keys to reject duplicate task submissions across consensus blocks.
  - Credited GPU worker rewards and incremented completed task count in `WorkerInfo`.
- [x] **4.3. Cryptographic Signature Validation**
  - Canonical `canonical_sign_bytes` and Ed25519 coordinator signature creation & validation on `ComputeReceipt`.
  - Validate Ed25519 coordinator signatures in `MirasetApp::compute_state_updates` during block production and validation.
- [x] **4.4. Epoch-Level PoCC Accounting**
  - Canonical ADR-0004 & REWARDS.md 60% compute / 40% capacity reward formula implementation (`EpochRewardConfig`).
  - Capacity scoring based on uptime squared, VRAM saturation cap (80 GiB), and verified output tokens.
  - Zero-remainder exact integer allocation accounting with worker metadata tracking.

---

## 5. Commit Event Stream & Indexer Integration (Phase 5)
- [x] **5.1. Block Commitment Event Hook**
  - Expose a tokio broadcast channel on `MirasetNode` that receives `CommitBlockEvent` upon 3-chain finalization (`subscribe_commits()`).
  - Extract block metadata, transaction counts, and receipt counts on commit.
---

## 6. Production CLI & Multi-Validator Node Operations (Phase 6)
- [x] **6.1. CLI Subcommands & Flags**
  - Add CLI arguments for RPC address, P2P address, peer discovery (`--peers <pubkey>@<ip:port>`), data directory, and key file.
  - Implement `keygen` subcommand to generate fresh Ed25519 validator keypairs.
  - Implement `demo` subcommand for in-memory development walkthrough.
  - Wire persistent `RedbKVStore` and framed `TcpNetwork` directly into the node executable.

---

## 7. Worker Stake Bond Registration & Slashing Enforcement (Phase 7 — ADR-0007)
- [x] **7.1. Worker Bond Action & Transaction Encoding**
  - Defined `WorkerBondAction` enum (`Register`, `Unbond`, `Slash`) with `MIN_WORKER_BOND` (100 MIR) and `WORKER_BASE_SLASH_BPS` (10%).
  - Added `Transaction::new_worker_bond(...)` helper routing to `system:worker_registry`.
  - Added worker ownership (`owner_address`) and network discovery `endpoint` to `WorkerInfo`.
- [x] **7.2. State Machine Bond Transition & Slashing**
  - Updated `MirasetApp::compute_state_updates` to validate and execute worker registration, bond additions, unbonding with refund to owner, and slash burning.
  - Required workers to maintain `stake >= MIN_WORKER_BOND` to remain active.
- [x] **7.3. Testing & RPC Validation**
  - Added integration test `test_worker_bond_registration_unbonding_and_slashing` verifying end-to-end consensus transitions.
  - Added RPC test `test_rpc_worker_bond_submission` verifying client/coordinator submission of bond transactions.

---

## 8. HotStuff Production-Gap Closure Backlog

**Traceability:** `miraset-chain-hardening` (PRD R01–R13)
**Status:** Open; this backlog is not a production-readiness declaration or approval to change the settlement architecture. HotStuff remains experimental/devnet-only while protocol ADR-0016 is Proposed and ADR-0013 remains the accepted settlement decision. The checked items above record delivered foundations; they do not mean the production gaps below are closed.

Do not mark an item complete based only on a module existing. Record its acceptance evidence (tests, reviewed source, and required gate) in the linked PRD/requirements artifacts. Do not start high-risk protocol, key-safety, persistence, membership, or settlement changes until their requirements and architecture/security/review approvals are resolved.

### 8.1. Requirements and architecture gates — prerequisite

- [x] **8.1.1. Reconcile this roadmap with the authoritative protocol and engineering sources** (PRD R01, R02, R13).
  - Verify ADR-0013/0016 status and scope; keep unapproved HotStuff isolated from production settlement.
  - Verify original engineering ADRs and relevant SOW, REWARDS, crypto, and public/private-boundary decisions; record exact sources and conflicts in `docs/business/miraset-chain-requirements-baseline.md`.
  - Correct stale or conflicting claims in this file and `docs/HotStuff-rs.md`, including the PoCC terminology and whether a proposal is an approved decision.
  - **Accept:** source matrix and Requirement Gate are reviewed; no unresolved economic or production-scope assumptions are treated as decisions.
  - **Evidence (2026-11-08):** `docs/business/miraset-chain-requirements-baseline.md` adds a source-status/conflict matrix and reviewed (not approved) Requirement Gate snapshot; `docs/HotStuff-rs.md` now states ADR-0013 accepted vs ADR-0016 proposed and corrects PoCC terminology.
- [x] **8.1.2. Complete architecture and implementation gates before production hardening** (PRD R03–R12).
  - Specify the canonical encoding, deterministic execution, durable commit/recovery, exact consensus variant, validator authorization, network identity, sync trust model, and RPC security contracts.
  - Resolve architecture proposals and required human approvals before implementation; keep the experimental boundary explicit in startup/configuration and docs.
  - **Accept:** approved decisions are recorded in the existing architecture/ADR workflow; no proposed design is represented as implemented or accepted.
  - **Evidence (2026-11-08):** `ARCHITECTURE.md` and `docs/business/miraset-chain-prd.md` specify the R03–R12 implementation contracts; `src/node/config.rs` (`NodeConfig::validate()`, `test_node_config_enforces_experimental_boundary`) and `src/main.rs` enforce `experimental_mode = true` and prevent implicit production activation while ADR-0016 remains Proposed.

### 8.2. Deterministic execution and durable state

- [x] **8.2.1. Canonicalize protocol payloads and commitments** (PRD R03).
  - Define versioned canonical encodings and domain-separated transaction, block, receipt, and state commitments; add golden vectors and reject noncanonical or unsupported encodings.
  - **Accept:** golden-byte, round-trip, malformed-input, and cross-version tests pass.
  - **Evidence (2026-11-08):** `src/core/types.rs` defines `PROTOCOL_ENCODING_VERSION = 1`, domain tags (`DOMAIN_TX_SIGN_V1`, `DOMAIN_TX_COMMIT_V1`, `DOMAIN_RECEIPT_SIGN_V1`, `DOMAIN_RECEIPT_COMMIT_V1`, `DOMAIN_BLOCK_COMMIT_V1`, `DOMAIN_STATE_COMMIT_V1`), canonical binary envelopes (`to_canonical_bytes`/`from_canonical_bytes`), and tests (`test_canonical_encoding_golden_vectors_and_commitments`, `test_canonical_encoding_rejects_malformed_and_trailing_bytes`, `test_canonical_encoding_rejects_unsupported_version`); `src/app/chain_app.rs` enforces canonical round-trip and domain-separated block commitment hashes.
- [x] **8.2.2. Harden deterministic state transitions** (PRD R04).
  - Replace saturating arithmetic in consensus-visible accounting with the source-approved checked/error behavior; specify transaction ordering, timestamps, epoch transitions, and reward rounding from authoritative sources.
  - Add the approved state commitment to block validation; do not add a gas model until its limits and accounting are specified.
  - **Accept:** replay from the same finalized inputs produces identical commitments; invalid input leaves candidate state unchanged; overflow and ordering edge cases are tested.
  - **Evidence (2026-11-08):** `src/app/chain_app.rs` (`compute_state_updates_with_commitment`, `apply_transaction_to_sender`), `src/core/types.rs` (`BlockPayload::state_root`, `EpochRewardConfig::try_distribute`), and `src/mempool/pool.rs` replace saturating arithmetic with checked arithmetic, enforce deterministic `BTreeMap`/`BTreeSet` ordering and per-sender ascending nonce ordering, and bind `state_root` into block production and validation (`test_apply_transaction_rejects_arithmetic_overflow`, `test_reward_distribution_is_deterministic_across_worker_order`).
- [x] **8.2.3. Implement atomic durable application commit and fail-closed recovery** (PRD R05).
  - Persist application records and versioned recovery metadata as one approved commit bundle; flush successfully before publishing finalized state or events.
  - Validate schema/version, chain/genesis identity, height continuity, ancestry, and commitments on restart; stop on uncertain writes or inconsistent history.
  - **Accept:** crash/fault injection at write and flush boundaries preserves acknowledged commits; corrupt or incomplete history prevents voting/readiness.
  - **Evidence (2026-11-08):** `src/storage/redb.rs` (`RecoveryMetadata`, `try_write`, `validate_recovery_state`, `test_redb_fault_injection_preserves_acknowledged_commits`, `test_recovery_validation_fails_closed_on_mismatch_or_corruption`), `src/node/runner.rs` (`init_genesis_with_chain_id`, `MirasetNode::try_start`), and `tests/durable_storage.rs` verify atomic commit sequencing, fault-injection rollback, and fail-closed startup recovery.
- [x] **8.2.4. Separate storage responsibilities and define retention/snapshot behavior** (HotStuff-rs.md §5.2; PRD R05/R10).
  - Decide the backend based on the approved durability/compatibility review; separate consensus, canonical history, application state, and receipt/index data as required.
  - Specify pruning and snapshot format, integrity, migration, and rollback behavior before implementing them.
  - **Accept:** restart, migration, pruning, and snapshot tests preserve verifiable finalized state and fail closed on corruption.
  - **Evidence (2026-11-08):** `src/storage/redb.rs` separates `blocktree`, `cf_state`, `cf_receipts`, and `cf_snapshots` tables, implements `StateSnapshot` with `state_root` and `integrity_hash` verification, protected-prefix pruning (`prune_keys_with_prefix`), and fail-closed schema migration (`migrate_schema`), verified by `test_redb_separated_tables_snapshot_restore_and_corruption_rejection` and `test_redb_pruning_and_schema_migration_fail_closed`.

### 8.3. Consensus safety, validator membership, and key custody

- [x] **8.3.1. Verify the exact HotStuff variant and its safety/liveness contract** (PRD R06).
  - Compare the pinned `hotstuff_rs` behavior with a reviewed specification for proposal ancestry, vote eligibility, QC/TC meaning, lock/unlock rules, commit rule, view changes, and quorum assumptions.
  - Keep HotStuff non-production if the library cannot be shown to satisfy the approved contract; do not infer safety from the crate name or passing happy-path tests.
  - **Accept:** independent review and fault simulations establish no conflicting finality within the stated fault bound and progress after network stabilization.
  - **Evidence (2026-11-09):** `ARCHITECTURE.md` specifies the pinned `hotstuff_rs` v0.4.0 3-chain Chained HotStuff contract and keeps it isolated in experimental mode. `src/network/channel.rs` (`isolate_peer`, `heal_peer`, `block_link`, `clear_faults`) and `tests/node_consensus.rs` (`test_four_validator_fault_bound_and_partition_healing`) verify $n = 4, f = 1$ (power 3/3/3/1) progress with an isolated validator, identical finalized state on the honest quorum, and catch-up via block sync after healing.
  - **Library defects found and contained:** (1) `KVGet::block_data` returns empty block data (consumed iterator), which broke block sync and commit-time payload reads — fixed in the vendored copy `lib/hotstuff_rs` (`src/storage/block_data_tests.rs` fails on upstream, passes patched); (2) the pacemaker truncated `max_view_time` to whole seconds — fixed with exact `Duration` arithmetic (`test_four_validator_fault_bound_and_partition_healing` runs with 350ms views); busy-polling library threads were also fixed. All patches are listed in `lib/hotstuff_rs/MIRASET_PATCHES.md` and must be re-applied on any upgrade and offered upstream. This is fault-simulation evidence only; independent safety review remains open under 8.5.2.
  - **Multi-process evidence and liveness limitation (2026-11-09):** `tests/tcp_cluster.rs` runs validators over the real TCP transport (persistent per-peer connections, `src/network/tcp.rs`) with durable Redb storage, checking identical committed hashes at every height. `test_five_validator_tcp_redb_progress_with_one_crashed` shows $n = 5$ keeps finalizing with one crashed validator; `test_four_validator_tcp_redb_crash_restart_and_catch_up` shows $n = 4$ stays consistent and resumes after restart. **Equal-power $n = 4$ cannot finalize while any one validator is down:** strict leader rotation, votes to the leader of `view + 1`, and the consecutive-view 3-chain commit rule require four consecutive live leaders. Any devnet/testnet fault suite must use $n \ge 5$ (or account for this) until a leader-selection/commit-rule decision is made.
- [x] **8.3.2. Persist vote and lock safety state before emitting signatures** (PRD R07; HotStuff-rs.md §5.8).
  - Add crash-safe safety storage for the required view/phase/block and lock state; fail closed if persistence fails or recovery is ambiguous.
  - **Accept:** crash/restart tests cannot produce conflicting votes for the same view/phase; key material is never logged.
  - **Design (2026-11-09, approved by project owner):** `docs/adr/proposal-vote-safety-state.md` §10 records the decisions and refinements.
  - **Delivered (2026-11-09):** vendored patch #7 records the vote before sending; `src/consensus/` adds `SigningGuard` (own Redb file bound to chain, validator and genesis; fsync before release; same-host file lock), `GuardedNetwork` (outermost transport wrapper; stale and conflicting votes are dropped, guard faults halt) and `SafetyHalt` (`/health` 503, exit code 3, consensus-thread panic hook). `RedbKVStore::with_safety_halt` makes chain write failures visible. The validator binary opens the guard with `--signing-guard` / `--init-signing-guard` and refuses a missing guard when chain data exists. The devnet gives each guard its own volume and `restart: on-failure`. Vendored patch #8 fixes a pacemaker panic found by the cluster test.
  - **Evidence:** unit tests (`src/consensus/*`, `src/storage/redb.rs`); integration tests (`tests/vote_safety.rs` T2/T4/T5, `tests/consensus_panic_halt.rs`, `tests/rpc_api.rs`, `tests/vote_safety_cluster.rs` T3 with 20 restarts and T4 whole-cluster restore with guards kept); E2E (`tests/e2e_validator.rs` real binary with SIGKILL, refusal and wipe; `scripts/devnet/fault_suite.sh` with SIGKILL and resync scenarios, passed). Test map: `docs/adr/proposal-vote-safety-state.md` §10.
  - **Patch #8 regression:** `lib/hotstuff_rs/src/pacemaker/view_regression_tests.rs` (deterministic; fails on upstream with `NonIncreasingViewError`).
  - **Not closed by this item:** independent review of patches #7/#8 (8.5.2); cross-host double signing (8.3.4).
- [ ] **8.3.3. Keep validator reconfiguration disabled until a safe activation protocol is approved** (PRD R08).
  - Specify on-chain authorization, old/new committee transition, quorum safety, bootstrap, and state catch-up. Do not treat CLI peer changes as validator-set changes.
  - **Accept:** unauthorized updates are rejected and transition tests cover old/new committee overlap, boundary cases, and lagging nodes.
- [ ] **8.3.4. Define protected validator key custody** (PRD R12; HotStuff-rs.md §5.8).
  - Remove deterministic production fallback keys; specify file permissions and a reviewed signer boundary before adding an external signer/KMS integration.
  - **Accept:** missing/invalid key configuration fails explicitly; key generation and signing tests do not expose secrets in output or logs.
  - **Progress (2026-11-09, not complete):** the `[42;32]` fallback key is removed; `--genesis` mode requires `--key-file`, and `src/devnet.rs` creates key files with mode 0600 (`create_new`), refuses group/world-readable or malformed keys, and never prints secrets (`test_key_files_fail_closed`). Open: signer boundary/KMS design; the devnet compose mounts all keys in every container.

### 8.4. Networking, synchronization, RPC, and operations

- [ ] **8.4.1. Harden transport identity, authorization, and resource bounds** (PRD R09; HotStuff-rs.md §5.1).
  - Authenticate claimed validator identity against the configured/authorized peer; specify encrypted transport, admission, message/queue/work limits, backpressure, and disconnect behavior.
  - Decide whether static TCP is sufficient for the approved deployment or whether libp2p/QUIC is required; do not add discovery/gossip by assumption.
  - **Accept:** forged identity, unauthorized peers, malformed/oversized frames, replay, overload, and queue saturation are rejected observably and within configured limits.
  - **Progress (2026-11-09, not complete):** `src/network/tcp.rs` now uses persistent per-peer connections with bounded per-peer queues (a dead peer only drops its own messages), `host:port` peers resolved on reconnect, `TCP_USER_TIMEOUT`/keepalive and an inbound idle timeout to clear half-open connections, and a dropped-message counter. Open: authenticated/encrypted identity (sender key is still self-declared), replay protection, configurable limits.
- [ ] **8.4.2. Add authenticated history and state synchronization** (PRD R10; HotStuff-rs.md §5.7).
  - Specify checkpoint trust, finalized proof/ancestry validation, state-root verification, byte/request/concurrency budgets, and recovery before voting.
  - **Accept:** forged finality, incomplete ancestry, corrupt snapshots, and wrong chain/committee identity are rejected; valid lagging-node recovery is tested.
- [ ] **8.4.3. Complete the public node RPC contract** (PRD R11; HotStuff-rs.md §5.9).
  - Define versioned, authenticated/rate-limited transaction and receipt admission and finalized-state queries; expose projection height/freshness and distinguish admission from finality.
  - Decide and document the supported transport and endpoint names. Add missing query/event operations only where required by the approved scope.
  - **Accept:** unauthorized/over-limit requests are rejected; replayed settlement is rejected; stale projections are labeled; API tests cover success and failure behavior.
- [ ] **8.4.4. Add safe operational observability and lifecycle behavior** (PRD R12; HotStuff-rs.md §5.10).
  - Add secret-safe structured logs, bounded-cardinality metrics, distinct liveness/readiness checks, and shutdown that stops admission and resolves or reports pending durable work.
  - **Accept:** startup, error, readiness, shutdown, and secret-redaction tests pass; recovering/non-quorate nodes do not report ready to finalize.
- [ ] **8.4.5. Complete CLI/genesis operations only within the approved public-node scope** (HotStuff-rs.md §5.11).
  - Specify and implement genesis/config validation, explicit node start modes, and safe database maintenance commands; remove ambiguous implicit production startup behavior.
  - **Accept:** invalid/mismatched genesis and configuration fail before voting; maintenance commands cannot silently discard state.
  - **Progress (2026-11-09, not complete):** `src/node/genesis.rs` adds a validated JSON genesis (chain ID, validators with P2P addresses, accounts); startup fails on a non-validator key, a stored validator set or genesis state commitment that differs from the file, and the node handles SIGTERM. `devnet-init`, `pubkey`, and `sign-transfer` subcommands support the Docker devnet (`docs/devnet.md`). Open: database maintenance commands, production start modes.

### 8.5. PoCC boundary and production evidence

- [ ] **8.5.1. Align receipt validation and economics with authoritative PoCC sources** (PRD R13; HotStuff-rs.md §5.6).
  - Treat PoCC as Proof of Compute & Capacity, not proof of cryptographic inference correctness. Verify receipt authorization, worker/client/coordinator roles, replay/challenge rules, and accounting against SOW, REWARDS, and accepted decisions.
  - Do not claim a `proof_hash` proves correct inference or payment; keep expensive verification out of synchronous consensus execution unless a reviewed bounded verification design explicitly requires otherwise.
  - **Accept:** source-backed accounting properties and adversarial receipt tests pass; unsupported proof claims are removed from docs.
- [ ] **8.5.2. Close consensus, storage, and network audit checklist items** (HotStuff-rs.md §7.3).
  - Independently review HotStuff safety/locking/QCs, pacemaker/view sync, block sync, block-tree persistence, and application validator-set updates.
  - **Accept:** findings and dispositions are recorded; no unresolved critical/high issue is treated as production-approved.
- [ ] **8.5.3. Establish measured performance and fault-test evidence** (HotStuff-rs.md §§7.1–7.2).
  - Define representative workload, hardware, WAN conditions, and repeatable measurement procedure before accepting throughput/finality targets.
  - Add leader-failure, selective-message, equivocation, partition, restart, and lagging-validator/reconfiguration scenarios. Run 72-hour stability testing only after the workload and environment are defined.
  - **Accept:** reproducible results meet targets approved through the requirements gate; no unmeasured target or happy-path test is presented as production evidence.
  - **Progress (2026-11-09, not complete):** `docker-compose.yml` runs a private 5-validator devnet; `scripts/devnet/fault_suite.sh` covers crash/restart, pause, and network partition with `scripts/devnet/check_consensus.py` asserting per-height agreement, quorum progress, and catch-up (passing locally). Open: equivocation, selective-message and lagging-validator scenarios, defined workload/hardware, measurements, 72-hour run.
- [ ] **8.5.4. Gate testnet and release on independent verification** (HotStuff-rs.md §8).
  - Run a private multi-validator devnet with the approved fault suite, then obtain explicit authorization before any public testnet or production settlement use.
  - Complete independent security and code reviews and the DoD audit; hand off to CI/CD only after all required gates pass. Do not deploy from this checklist.
  - **Accept:** gate evidence, human approvals where required, test results, and remaining limitations are recorded; deployment remains a separate authorized decision.
