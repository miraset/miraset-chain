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


