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
  - **Design (2026-11-09, Proposed, awaiting approval):** `docs/adr/proposal-validator-key-custody.md`. It covers a purpose-bound `ValidatorSigner` seam (vendored patch #9), slashing protection inside the signer, local and remote signers (the remote signer closes cross-host double signing), key-file and process hardening, and per-validator key volumes in the devnet.
  - **D4+D5 delivered (2026-11-09, decision 4 approved):**
    - `src/keys.rs` refuses symlinks (`O_NOFOLLOW`), keys owned by another user and group/other access, and zeroizes secret buffers.
    - `src/process_hardening.rs` disables core dumps and makes processes non-dumpable.
    - The devnet stores one key per volume; nodes mount only their own key read-only, and the faucet key goes only to the `faucet` tool.
    - Evidence: unit, integration and e2e tests, plus the Docker fault suite's key isolation step (§9 of the proposal).
  - **D1+D2 delivered (2026-11-09, decisions 1 and 3 approved):**
    - Vendored patch #9 gives purpose-bound signing (`SignRequest` / `ValidatorSigner`, no raw-bytes signing).
    - `src/consensus/signer.rs` `GuardedSigner` checks the signing guard before a phase-vote signature exists.
    - `NodeConfig` holds a signer, not a key.
    - Evidence: library golden and identical-signature tests; unit tests; integration tests for refusal and failure; the cluster harness on `GuardedSigner`; e2e on the real binary; the Docker fault suite (proposal §10).
  - **D3 delivered (2026-11-09, decision 2 = gRPC):**
    - `miraset-chain signer` holds the key and guard and serves gRPC (`proto/signer.proto`) on a Unix socket: mode 0600, same-uid peers only.
    - Validators started with `--remote-signer` hold no key. They verify every returned signature, check the chain and an optional pinned key, and halt visibly when the signer fails.
    - Evidence: unit, integration (`tests/remote_signer.rs`, including concurrent clients never getting two votes signed per view) and e2e (`tests/e2e_remote_signer.rs`, including SIGKILL of the signer leading to node exit 3). Proposal §11.
    - Open: a cross-host signer (gRPC over mTLS), which depends on 8.4.1. Key rotation (D6) depends on 8.3.3.
  - **Progress (2026-11-09, not complete):** the `[42;32]` fallback key is removed; `--genesis` mode (now the `start` subcommand, 8.4.5) requires `--key-file`, and `src/devnet.rs` creates key files with mode 0600 (`create_new`), refuses group/world-readable or malformed keys, and never prints secrets (`test_key_files_fail_closed`). Signer boundary and devnet key isolation were later delivered by D1–D5 above; KMS integration is not planned.

### 8.4. Networking, synchronization, RPC, and operations

- [ ] **8.4.1. Harden transport identity, authorization, and resource bounds** (PRD R09; HotStuff-rs.md §5.1).
  - Authenticate claimed validator identity against the configured/authorized peer; specify encrypted transport, admission, message/queue/work limits, backpressure, and disconnect behavior.
  - Decide whether static TCP is sufficient for the approved deployment or whether libp2p/QUIC is required; do not add discovery/gossip by assumption.
  - **Accept:** forged identity, unauthorized peers, malformed/oversized frames, replay, overload, and queue saturation are rejected observably and within configured limits.
  - **Design (2026-11-09, Proposed, awaiting approval):** `docs/adr/proposal-transport-identity.md`. Noise `XX` sessions whose per-start static key is attested by the validator key through a purpose-bound `TransportIdentity` signer request; the authenticated session replaces the self-declared sender (which today lets any host that reaches the port spoof the leader's unsigned proposals); configurable pre/post-authentication limits with reason counters; static TCP kept, no libp2p/QUIC.
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
  - **Decisions (2026-11-09, project owner):** readiness = recent commit (no halt, not draining, commit within `--ready-max-commit-age-secs`, default 30); `tracing` logs with a hand-written Prometheus `/metrics` (no metrics dependency).
  - **Delivered (2026-11-09, pending independent review):** `src/observability/` (`readiness.rs` pure readiness rule and one-way drain flag; `metrics.rs` fixed metric set with enum-only labels; `logging.rs` text/JSON `tracing` on stderr with the `hotstuff_rs` `log` bridge). `/livez`, `/readyz`, `/metrics` in `src/rpc/server.rs`; admission returns 503 while draining (`AdmissionError`). `src/main.rs` drains on SIGTERM/SIGINT (`--shutdown-drain-ms`), then stops RPC, consensus and transport and reports discarded mempool items; runtime `println!`/`eprintln!` diagnostics moved to `tracing`.
  - **Evidence:** unit (`src/observability/*` tests, `test_node_config_rejects_readiness_window_shorter_than_a_view`); integration `tests/observability.rs` (non-quorate node reports `no_commit_yet`, losing quorum gives `commit_stale` and healing restores `ready`, drain gives 503 on admission and `draining`, metrics content); E2E `tests/e2e_observability.rs` (real binary: JSON logs only, probes, metrics, SIGTERM drain and exit 0, validator secret absent from logs and metrics, invalid flag refused).
  - **Independent review (2026-11-09, rust-reviewer agent; no CRITICAL/HIGH):** fixed — a safety halt during the drain now still exits 3 and a second signal ends the drain; safety events use the `miraset_safety` target, kept at `warn` whatever `RUST_LOG` says, and an invalid `RUST_LOG` is reported; a commit with an unreadable height still counts for readiness; the `-1` age sentinel was removed; e2e scans for the secret at `RUST_LOG=trace` and in probe bodies; timing-sensitive tests were hardened. Accepted and documented — the admission/drain race is best effort (the mempool is not durable); `/metrics` shares the private RPC port (8.4.3).
  - **Leftovers delivered (2026-11-11, pending independent review):**
    - Catch-up readiness: `/readyz` reports `syncing` while a block-sync session is open, and commits made through block sync no longer count for readiness (`NodeMetrics::last_consensus_commit_age`, handlers for `on_start_sync`/`on_end_sync`/`on_start_view` in `src/node/runner.rs`). A lagging node becomes ready only after it commits through consensus. That commit can finalize an ancestor a few heights below the tip.
    - Metrics: `miraset_syncing`, `miraset_sync_sessions_total`, `miraset_blocks_synced_total`, and `miraset_vote_refused_total{layer=signer|transport, reason=stale|conflicting}` from a shared `VoteRefusals` (`src/observability/vote_refusals.rs`). A validator with `--remote-signer` has no `signer` series.
    - The Docker healthcheck uses `/livez`.
    - Evidence: unit tests (`metrics.rs`, `readiness.rs`, `vote_refusals.rs`). The `tests/byzantine_faults.rs` lagging scenario never reports ready while behind (within 3 heights of slack) and becomes ready after catch-up; it fails on the previous rule ("reported ready at height 18, tip was at least 30"). `tests/e2e_observability.rs` checks the new series on the real binary. `tests/e2e_remote_signer.rs` `test_e2e_safety_halt_during_drain_still_exits_with_halt_code` kills the remote signer during a SIGTERM drain, and the validator exits 3.
  - **Independent review (2026-11-11, rust-reviewer agent; no CRITICAL/HIGH):** it verified the event-ordering and no-StartView-during-sync assumptions against the vendored library. Fixed: a sync session now clears the earlier consensus-commit stamp, because a commit from before a lag shorter than the readiness window could report a lagging node ready. The catch-up test compares against the peers' live tip instead of a pre-heal snapshot. Documented: `/readyz` can alternate between `ready` and `syncing` on a slow chain that triggers timeout syncs (WAN/netem). The healthcheck change is safe because no compose service waits on `service_healthy`.
  - **Remaining for closure:** owner sign-off. The admission/drain race stays best effort and was accepted earlier.
- [ ] **8.4.5. Complete CLI/genesis operations only within the approved public-node scope** (HotStuff-rs.md §5.11).
  - Specify and implement genesis/config validation, explicit node start modes, and safe database maintenance commands; remove ambiguous implicit production startup behavior.
  - **Accept:** invalid/mismatched genesis and configuration fail before voting; maintenance commands cannot silently discard state.
  - **Progress (2026-11-09, not complete):** `src/node/genesis.rs` adds a validated JSON genesis (chain ID, validators with P2P addresses, accounts); startup fails on a non-validator key, a stored validator set or genesis state commitment that differs from the file, and the node handles SIGTERM. `devnet-init`, `pubkey`, and `sign-transfer` subcommands support the Docker devnet (`docs/devnet.md`). Database maintenance commands and explicit start modes were delivered on 2026-11-10 (below).
  - **Independent review (2026-11-10, rust-reviewer agent; no CRITICAL):** fixed — the first version opened the database writable for `inspect`/`verify` (redb's writable open writes a recovery flag to the header, so "writes nothing" was false), made raw-file backups that needed repair on first open, and could panic on a damaged file; restore guidance overstated safety. **Finding recorded for 8.5.2:** the signing guard records votes, not HotStuff's lock; restoring older chain data or wiping it (the 8.3.2 resync procedure) rolls back the lock and highest views, so until the node passes its old views it may vote without its old lock. Docs now say to treat such a validator as one of the f faulty validators and to restore or resync one at a time; persisting the lock in the guard would close it.
  - **Decisions (2026-11-10, project owner):** maintenance commands `db inspect`, `db verify`, `db compact`, `db backup` (no prune or reset); validators start only with an explicit `start` subcommand, the implicit `--genesis` form is removed at once, and running with no arguments prints help. Deviation, for safety: `db backup` copies chain data **without** the signing guard, because restoring an older guard would roll back slashing protection.
  - **Delivered (2026-11-10):** `src/storage/maintenance.rs` and `src/storage/readonly.rs`. `inspect` and `verify` open the database read-only (they never write the file and refuse one that needs crash recovery); `verify` checks recovery metadata (optionally against the genesis file) and, for every committed height, recomputes the block hash and checks height, parent link, justify chain ID and increasing views, and the canonical payload against the data hash, plus nothing indexed above the tip; a damaged file fails cleanly instead of panicking. It does not re-verify QC signatures (checked on insert). `compact` (writable, exclusive lock) runs redb's page-checksum check and repair first and changes no records. `backup` makes a logical copy of one read-only snapshot into a new 0600 file, closes it cleanly, verifies it fully against the source, and removes it on failure. All refuse a database a running validator holds (shared or exclusive `flock`). Typed errors with exit codes 4 (in use), 5 (needs recovery / verification failed), 1 (other). `src/main.rs`: `start`, `db …`; `--dev` and the implicit modes removed. Docker compose, the e2e harness, `README.md` (stale "Hello, world" text replaced) and `docs/devnet.md` updated.
  - **Evidence:** unit (`src/storage/maintenance.rs`: block checks accept a consistent chain and reject wrong index, height, parent, genesis link, foreign chain ID, non-increasing view, data hash, datum count and garbage payloads; missing database refused and not created); integration `tests/db_maintenance.rs` on a chain a real node committed (inspect/verify; **source file byte-identical after inspect and verify**; another chain or genesis rejected; a tampered height index and a replaced block payload are detected at the right height; an unclean-shutdown file is refused read-only without being modified and repaired by compact; truncated and garbage files fail without panicking; every command refuses a held database with exit code 4 and writes nothing; backup is 0600, verified over all heights, never overwrites; compact keeps the committed tip and reports no repair on a clean file); E2E `tests/e2e_cli.rs` (no arguments prints help, the old form is rejected; `db inspect` refuses a running validator; after SIGTERM, verify against the genesis file, backup, refused second backup and compact succeed, the backup is restored in place of the database, and the validator restarts on it and keeps finalizing). Docker devnet (2026-11-10, isolated compose project, before the review fixes): five validators on `start` pass the full fault suite (all nodes agree to height 2222), and `db verify --genesis` on a stopped node's volume checks 2465 committed heights. Re-checked after the review fixes: read-only `db inspect` and `db verify --genesis` on a stopped node's volume pass (2810 heights), and `db inspect` on a running validator's volume from another container is refused with exit 4.

### 8.5. PoCC boundary and production evidence

- [ ] **8.5.1. Align receipt validation and economics with authoritative PoCC sources** (PRD R13; HotStuff-rs.md §5.6).
  - Treat PoCC as Proof of Compute & Capacity, not proof of cryptographic inference correctness. Verify receipt authorization, worker/client/coordinator roles, replay/challenge rules, and accounting against SOW, REWARDS, and accepted decisions.
  - Do not claim a `proof_hash` proves correct inference or payment; keep expensive verification out of synchronous consensus execution unless a reviewed bounded verification design explicitly requires otherwise.
  - **Accept:** source-backed accounting properties and adversarial receipt tests pass; unsupported proof claims are removed from docs.
- [ ] **8.5.2. Close consensus, storage, and network audit checklist items** (HotStuff-rs.md §7.3).
  - Independently review HotStuff safety/locking/QCs, pacemaker/view sync, block sync, block-tree persistence, and application validator-set updates.
  - **Accept:** findings and dispositions are recorded; no unresolved critical/high issue is treated as production-approved.
  - **Open finding (2026-11-10, from the 8.4.5 review):** the signing guard (8.3.2) records phase votes, not HotStuff's lock or highest views, which live in the chain database. Wiping chain data (the documented resync) or restoring an older database rolls them back: same-view double votes are still refused, but until the validator passes its old views it may vote without its old lock. Current mitigation is procedural (`docs/devnet.md`: treat it as one of the f faulty validators, one at a time). Candidate fix: persist the locked certificate and highest view in the guard and refuse to vote below them.
- [ ] **8.5.3. Establish measured performance and fault-test evidence** (HotStuff-rs.md §§7.1–7.2).
  - Define representative workload, hardware, WAN conditions, and repeatable measurement procedure before accepting throughput/finality targets.
  - Add leader-failure, selective-message, equivocation, partition, restart, and lagging-validator/reconfiguration scenarios. Run 72-hour stability testing only after the workload and environment are defined.
  - **Accept:** reproducible results meet targets approved through the requirements gate; no unmeasured target or happy-path test is presented as production evidence.
  - **Progress (2026-11-09, not complete):** `docker-compose.yml` runs a private 5-validator devnet; `scripts/devnet/fault_suite.sh` covers crash/restart, pause, and network partition with `scripts/devnet/check_consensus.py` asserting per-height agreement, quorum progress, and catch-up (passing locally). Open: defined workload/hardware, measurements, 72-hour run.
  - **Equivocation, selective-message and lagging-validator scenarios (2026-11-09):** `tests/byzantine_faults.rs`, n = 5 equal power on the in-memory hub, run one at a time. (1) Validator 4's key runs on two replicas with no shared guard (copied key, custody K3); each clone reaches a different half of the honest validators. The test requires ≥3 views with conflicting proposals, ≥3 with conflicting votes and ≥1 where honest votes split, then checks that honest replicas and both clones never commit different blocks at any height and finality keeps advancing. One adversarial delivery schedule, not an exhaustive search. (2) Validator 0 reaches only two peers and one peer cannot reach it; the hub's drop counter proves the faults took effect; no conflicting commits, progress during and after. (3) Validator 4 is isolated for 4 block-sync batches (`block_sync_request_limit = 5`), then must catch up and rejoin. `ChannelNetworkHub` gained explicit clones (`register_clone`, `block_clone_link`) and a drop counter; `register` stays idempotent for restarts.
  - **Found by these scenarios:** (a) **block sync never caught up** when one response could not commit anything (upstream restarted each request at committed + 1, and its server ignored the start height in the speculative range); fixed as vendored patch #10 (`lib/hotstuff_rs/MIRASET_PATCHES.md`); scenario 3 fails on upstream and passes patched. (b) **Block production is quadratic in mempool size:** `MirasetApp::produce_block` re-executes the growing candidate list per transaction (about n²/2 applications for up to 100 txs). Measured on one validator: about 0.18s per full 100-tx block in a release build; in a debug build 25 txs took about 25s and 50 txs about 89s, so a debug-build leader with a full mempool misses its views. **Fixed (2026-11-10, 9974e82):** greedy linear selection (each candidate kept if it applies on top of those kept before it, `src/app/chain_app/execution.rs`); `tests/block_production.rs` finalizes a full 100-transfer block within the deadline in a debug build and checks that invalid candidates are skipped while valid ones are kept.
  - **Measurement tooling and devnet baseline (2026-11-10, not complete):** decisions by the owner — local Docker devnet with `tc netem` WAN emulation, transfer + compute-receipt mix, baseline only (no target approved), soak script provided with the 72-hour run left to the owner. `miraset-chain loadgen` (`src/loadgen/`, 0a8f654) offers open-loop load pinned per node and measures latency from the scheduled send to the commit event after which committed state shows the item; `scripts/devnet/bench.sh` (rate sweep, environment record, per-height agreement after every rate), `netem.sh` (`lan`/`wan`/`wan-far`), `soak.sh` (RSS, fds, data size, height, restarts). Tests: 19 unit tests, `tests/e2e_loadgen.rs` (all admitted items commit; invalid/unfundable configs refused; observer node restart with reconnect). rust-reviewer findings fixed before measuring (SSE reconnect, receipt confirmation stall, re-send of committed items, coordinated omission, bounded memory). Results in `docs/performance-baseline.md`: LAN 25–400/s all committed, no saturation, p50 175–300 ms, p99 0.43–2.3 s; WAN (~50 ms RTT, 0.1% loss) 25–200/s all committed, p50 0.7–2.5 s, with ~10 s commit stalls at ≥100/s (unexplained, finding 10); WAN 400/s not measured. Open: target approval; `soak.sh` not yet run end to end (short validation skipped by the owner) and the 72-hour run; WAN stall investigation; multi-host WAN; reconfiguration (8.3.3). Findings for 8.4.3: no mempool gossip, unbounded mempool with per-insert sort, admission without signature/nonce/balance checks, equal-fee ordering by sender, mempool not durable; continuous empty-block production grows storage with wall time.
  - **Independent review (2026-11-09, rust-reviewer agent; no CRITICAL/HIGH):** fixed — patch #10 now continues ahead only as a no-progress retry and never blacklists on a lost fork, and the server honours the start height; hub `register` idempotent again with explicit clones; the selective test proves drops; the equivocation test proves honest vote splits; lag baseline read after the mailbox drains; scenarios serialised. These are in-process simulations, not measured production evidence.
- [ ] **8.5.4. Gate testnet and release on independent verification** (HotStuff-rs.md §8).
  - Run a private multi-validator devnet with the approved fault suite, then obtain explicit authorization before any public testnet or production settlement use.
  - Complete independent security and code reviews and the DoD audit; hand off to CI/CD only after all required gates pass. Do not deploy from this checklist.
  - **Accept:** gate evidence, human approvals where required, test results, and remaining limitations are recorded; deployment remains a separate authorized decision.
