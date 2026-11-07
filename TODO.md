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
- [ ] **2.1. Disk-Backed KVStore Implementation**
  - Implement `hotstuff_rs::block_tree::pluggables::KVStore` using a persistent embedded database (`redb` or `rocksdb` / `mdbx-sys`).
  - Implement atomic `WriteBatch` ensuring fsync/WAL durability before vote emission (ADR-0002).
- [ ] **2.2. Crash Recovery & Checkpoint Verification**
  - Reconstruct BlockTree state, highest QC, locked QC, and account states on node restart.
  - Fail closed if persisted consensus state or BlockTree history is corrupted.

---

## 3. P2P Transport Adapter (Phase 3)
- [ ] **3.1. Framed TCP Network Protocol**
  - Implement `hotstuff_rs::networking::network::Network` over length-delimited framed TCP connections (porting proven design from `demo/crates/miraset-node/src/hotstuff.rs`).
  - Support point-to-point direct validator connections and broadcast queues.
- [ ] **3.2. Peer Discovery & DoS Protection**
  - Static validator peer registry for devnet/testnet.
  - Bounded buffers, frame size validation (e.g. max 16 MiB), and disconnect on malformed messages.

---

## 4. GPU Worker & PoCC Verification Pipeline (Phase 4)
- [ ] **4.1. Cryptographic Signature Validation**
  - Validate Ed25519 signatures on compute receipts in `MirasetApp::validate_block`.
  - Ensure worker is registered and has sufficient bond/stake.
- [ ] **4.2. Receipt Anti-Replay & Token Accounting**
  - Prevent duplicate task submission across blocks using challenge nonces and task registries.
  - Apply canonical PoCC reward formulas (60% compute / 40% capacity) on epoch boundary.

---

## 5. Commit Event Stream & Indexer Integration (Phase 5)
- [ ] **5.1. Block Commitment Event Hook**
  - Expose a tokio broadcast channel on `MirasetNode` that receives `CommitBlockEvent` upon 3-chain finalization.
- [ ] **5.2. External Indexer Connector**
  - Provide a stream or WebSocket/SSE endpoint so `miraset-indexer` can project finalized blocks into queryable storage.
