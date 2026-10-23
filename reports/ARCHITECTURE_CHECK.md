# Architecture Check Report — miraset-chain

**Date:** 2026-07-26
**Scope:** Crate dependency graph, runtime data flow, module boundaries, API contract alignment, concurrency model, persistence, and cross-check of AGENTS.md architecture claims against code.
**Methodology:** `cargo tree`, `cargo metadata`, source review of `crates/miraset-node/src/{lib,state,rpc,storage,epoch,executor,gas,pocc,pocc_manager}.rs`, `crates/miraset-worker/src/*.rs`, `crates/miraset-core/src/*.rs`, and `docs/API_*.md`.

## Executive summary

miraset-chain is a **cleanly layered Rust workspace** with a sensible `core → node → {cli, worker, wallet, tui}` dependency DAG and **no circular dependencies**. The on-chain object model (`miraset-core::types`), the in-memory + Sled state (`miraset-node::state`), the Axum RPC boundary, and the worker inference backend abstraction are well-factored for an MVP.

However, the architecture is currently **a single-node devnet ledger**, not a blockchain:
- **No consensus** is wired in (PoCC scaffolding exists, is clearly documented as inactive, and the producer logs a devnet warning).
- **State commitment is implemented** (`Block.state_root` is a sorted-key blake3 commitment over balances, nonces, objects, and versions), replacing the previous `[0;32]` placeholder.
- **Mixed serialization** (bincode for blocks/hashes, JSON for events) is intentional and now guarded by serialization-stability tests.
- **AGENTS.md has been updated**: the stale `move_vm.rs` reference is clarified and the job-coordinator flow now describes tx-pipeline routing.
- **API docs are in sync**: `docs/API_NODE.md` documents the job coordinator endpoints and the signed `/jobs/submit` request.
- **Module boundaries improved**: `state.rs` is split into `state/{mod,execution,jobs}.rs` and `backend.rs` is split into `backend/{mod,ollama,openai,openllm,llamacpp,tgi,mock}.rs`.

| Area | Rating | Notes |
|------|--------|-------|
| Layering / dependency hygiene | Good | Clean DAG, no cycles |
| Module cohesion | Good | `state.rs` split to `state/{mod,validation,execution,jobs,epoch_hooks}.rs`; `backend.rs` split to `backend/{mod,ollama,openai,openllm,llamacpp,tgi,mock}.rs` |
| Concurrency model | Good | Single `RwLock<StateInner>`; Sled I/O now outside the write lock; lock contract documented on `State` |
| Consensus | Not implemented | PoCC is scaffolding; producer logs devnet warning and PoCC module documents its inactive status |
| State commitment | Implemented | `Block.state_root` is a sorted-key blake3 commitment over balances/nonces/objects/versions |
| Persistence | Good | Sled, mixed serialization, lazy load; storage length validation added; unused `sqlx` workspace dep removed |
| API/doc alignment | Good | Job endpoints documented; `/jobs/submit` now requires signed `Transaction::CreateJob`; AGENTS.md updated |
| Extensibility | Good | Trait-based backends, object model |

---

## 1. Crate dependency graph

```
                    miraset-core (types, crypto, no node deps)
                       ▲      ▲      ▲      ▲      ▲
                       │      │      │      │      │
              miraset-node   │  miraset-wallet  miraset-indexer
                       ▲     │      ▲               (placeholder)
                       │     │      │
                  miraset-cli │  miraset-tui
                  (also wallet)
                       │
                  miraset-worker  ──► miraset-core (no node dep)
                  miraset-launcher (standalone, no miraset-* deps)
```

Verified via `cargo tree --workspace` and per-crate `Cargo.toml`:

| Crate | Depends on (miraset-*) | Role |
|-------|------------------------|------|
| miraset-core | — | Types (`Transaction`, `ObjectData`, `Event`, `Block`), crypto (`KeyPair`, `Address`, `verify_signature`) |
| miraset-node | core | State (`state/{mod,execution,jobs}.rs`), RPC (Axum), storage (Sled), epoch, gas, executor, PoCC scaffolding |
| miraset-cli | core, node, wallet | `miraset` binary: `node start`, `wallet ...` |
| miraset-worker | core | HTTP worker, inference backends (`backend/*.rs`), receipt signing, node client |
| miraset-wallet | core | Encrypted keystore (Argon2 + AES-GCM) |
| miraset-tui | core, wallet | Terminal UI |
| miraset-launcher | (none) | Standalone desktop launcher |
| miraset-indexer | core | **Placeholder** — empty `Indexer` struct (still unimplemented)

**Findings:**
- **Good:** `miraset-core` has no workspace-internal deps (correct foundation layer). `miraset-worker` depends only on `core`, not `node` — workers communicate with nodes purely over HTTP (`node_client.rs`), which is the right decoupling for a distributed worker fleet.
- **Good:** `miraset-launcher` is independent (no miraset deps) — appropriate for a thin process-spawning wrapper.
- **Note:** `miraset-cli` is the only crate that ties `node` + `wallet` together; this is the composition root and is acceptable.
- **Note:** `miraset-indexer` now provides an in-memory `Indexer` with block/event indexing by type and height (`crates/miraset-indexer/src/lib.rs`). A durable backend remains future work.

---

## 2. Runtime data flow

```
                ┌────────────────────────────────────────────────┐
                │              miraset-cli (miraset)             │
                │  node start → Storage::open → State::new_with_ │
                │  storage → seed genesis → spawn block producer │
                │  → serve_rpc                                  │
                └───────────────────┬────────────────────────────┘
                                    │
        ┌───────────────────────────┼──────────────────────────┐
        ▼                           ▼                          ▼
  run_block_producer         serve_rpc (Axum)            (CLI wallet cmds
  (lib.rs:23, loop)         (rpc.rs:19)                  → reqwest to RPC)
        │                           │
        │  state.update_epoch()     │  GET /balance, /nonce, /block/*,
        │  state.produce_block()    │  /events, /chat/messages, /epoch,
        │                           │  /workers, /jobs, /jobs/{id}
        │                           │  POST /tx/submit, /jobs/submit
        ▼                           ▼
  ┌──────────────────────────────────────────────────────────────┐
  │            State (Arc<RwLock<StateInner>>)                   │
  │  blocks, balances, nonces, objects, owned_objects,          │
  │  pending_txs, current_epoch, chat_messages                  │
  │                                                              │
  │  submit_transaction → verify sig → nonce check → push        │
  │  produce_block      → execute_transaction_inner × N         │
  │                      → build Block → persist to Sled        │
  └───────────────────────────────┬──────────────────────────────┘
                                  │ Sled (lazy load on cache miss)
                                  ▼
                          Storage (blocks=bincode,
                          balances/nonces=u64 LE,
                          events=JSON)

  Worker side:
  miraset-worker (main.rs) → register on-chain (node_client)
                           → heartbeat loop (30s SubmitResourceSnapshot)
                           → serve HTTP (lib.rs router)
  Node → after each produced block, scan JobAssigned events and POST
         {worker}/jobs/accept (fire-and-forget spawn via state/jobs.rs)
  Worker → runs inference (backend trait) → generates receipt
        → POST /tx/submit (SubmitJobResult/AnchorReceipt) to node
```

**Verified facts:**
- Block producer loop (`lib.rs:35-61`) calls `state.update_epoch()` before `state.produce_block()` each tick — matches AGENTS.md. ✓
- Tx path is JSON `Transaction` → `/tx/submit` → `State::submit_transaction` (sig+nonce) → `pending_txs` → `produce_block`. ✓
- Job coordinator: `POST /jobs/submit` now accepts a signed `Transaction::CreateJob`, submits it through `State::submit_transaction`, and block execution auto-assigns the job to a suitable worker. After the block is produced, `State::dispatch_job_assignments` fires HTTP calls to the assigned workers' `/jobs/accept` endpoints. ✓
- Worker registration + heartbeat on startup (`miraset-worker/src/main.rs`). ✓

**Concerns (resolved):**
- ~~The `/jobs/submit` path bypasses `submit_transaction` (no signature).~~ Now routed through the signed tx pipeline; direct `State::create_job`/`auto_assign_job` RPC methods have been removed.

---

## 3. Concurrency model

**Pattern:** `State` holds `Arc<RwLock<StateInner>>` using `parking_lot::RwLock`. All mutations go through `self.inner.write()`.

**Findings:**
- `submit_transaction` acquires `write()` for the nonce check + `pending_txs.push` (`state/mod.rs:165`). Hold time is short. OK.
- `produce_block` acquires `write()` for transaction execution, balance/object updates, event emission, and state-root computation, then **drops the lock before Sled persistence** (`storage.save_block` + `flush`) (`state/mod.rs:291-318`). This reduces write-lock hold time significantly compared to the earlier implementation.
- Worker dispatch now happens **after** block production via `State::dispatch_job_assignments`, entirely outside the state lock. ✓
- **Lock-order risk:** Only one lock (`StateInner`) plus the per-worker `jobs: Arc<RwLock<HashMap>>` in the worker. No nested acquisition of `StateInner` + worker locks across crates. Low deadlock risk currently.
- **Async + sync lock:** `parking_lot::RwLock` is sync. Axum handlers are async and call `state.*` methods that block on the sync lock. Under contention this blocks the Tokio worker thread. For a single-node devnet this is acceptable; for scale, consider `tokio::sync::RwLock` or spawning blocking tasks for state access.

**Recommendation (partially implemented):**
- ✅ Move Sled `flush()` and `save_block()` **outside** the `StateInner` write lock: done.
- ⏳ Document the lock contract explicitly in `state/mod.rs` doc comments.

---

## 4. Persistence & serialization

**Storage:** Sled (`crates/miraset-node/src/storage.rs`). Trees: `blocks`, `balances`, `nonces`, `events`, `chat`(?).

**Serialization matrix:**

| Data | Format | Location | Rationale |
|------|--------|----------|-----------|
| `Block` | bincode (=1.3.3) | `storage.rs:23/37` | Deterministic, compact |
| `Event` | **JSON** (serde_json) | `storage.rs:90/111` | "tagged enums" — JSON handles `#[serde(tag=...)]` ergonomically |
| Balance / Nonce | 8-byte LE `u64` | `storage.rs:48/66/82/124` | Compact |
| Tx signature message | bincode (zero-sig) | `state.rs:156` | Determinism for sigs |
| Tx hash | bincode + blake3 | `types.rs:367` | Determinism |
| Receipt hash | bincode + blake3 | `worker/receipt.rs` | Determinism |

**Findings:**
- The bincode/JSON split for blocks vs events is **intentional** (commented in `storage.rs:99` "Use JSON for tagged enums") but creates two deserializers for the `Event` type depending on context. If an event is read from Sled it’s JSON; if it’s in-memory it’s whatever the struct holds. This is fine as long as the JSON shape is stable.
- Serialization-stability tests now exist in `crates/miraset-core/src/types.rs` for `Block` bincode and `Event` JSON (`test_block_bincode_stability`, `test_event_json_stability`).
- **Lazy load:** `get_balance`/`get_nonce` fall back to `storage.get_*` on cache miss (`state/mod.rs:105/120`) and populate the in-memory map indirectly. There’s no cache-population on read — each miss hits Sled. Acceptable but worth a benchmark.
- **Durability:** `flush()` is called once per block (`state/mod.rs:307`). Sled is beta; the workspace previously listed `sqlx` + Postgres as a future mainnet path, but the unused `sqlx` dependency has been removed until a concrete migration is implemented.
- Length validation is now implemented in `storage.rs` via `u64_from_le_bytes`, returning `StorageError::CorruptValue` instead of panicking on short/corrupt bytes.

**Recommendation:**
- ✅ Add a serialization-stability test suite — done.
- ✅ Add length validation in `storage.rs` — done.
- ✅ Remove unused `sqlx` workspace dependency — done.
- ⏳ Document the mainnet storage migration path once Postgres (or another durable backend) is adopted.

---

## 5. Consensus & state commitment

**Consensus:** Not implemented in the runtime path. `PoccConsensus`, `Validator`, `ValidatorSet`, `ComputeProof` are defined in `pocc.rs` (~600 LOC) and re-exported (`lib.rs:18`), and `PoccManager` exists (`pocc_manager.rs`, 32 LOC), but:
- `run_block_producer` (`lib.rs:35`) does **not** call into `PoccConsensus` for proposer selection or block signing.
- `produce_block` (`state/mod.rs:252`) does **not** verify validator signatures or compute a validator-signed block header.
- `pocc.rs` now documents at module level that it is **scaffolding only / not wired into the block producer** and the network runs in single-author devnet mode.

**State commitment:** `State::produce_block` now computes a deterministic `state_root` via `compute_state_root` (`state/mod.rs:214`) — a sorted-key blake3 hash over balances, nonces, object hashes, and object versions. Genesis still uses `[0;32]` (`types.rs:392`), which is acceptable for height 0.

**Epoch/reward path:** `epoch.rs` (~350 LOC) implements 60-min epochs with submit/challenge windows, worker stats, and a 70/30 capacity/compute reward split. `State::update_epoch` is called every producer tick. This is the most mature non-consensus subsystem.

**Move VM:** `crates/miraset-node/src/move_vm.rs` does not exist. `executor.rs` defines a parallel `ExecutionContext` that is **not called** from `produce_block`; all transaction execution happens in `state/execution.rs`. AGENTS.md has been updated to reflect this.

**Recommendation:**
- ✅ Mark the producer as "devnet single-author" with a runtime warning — done (`lib.rs:36-38`).
- ✅ Compute a real `state_root` and include it in blocks — done.
- ✅ Update AGENTS.md regarding `move_vm.rs` — done.
- ⏳ Decide whether to wire PoCC or remove/fence the dead consensus code more aggressively than module-level comments.

---

## 6. API contract alignment (docs vs code)

### Node (`docs/API_NODE.md` vs `crates/miraset-node/src/rpc.rs`)

| Endpoint | In docs? | In code? | Notes |
|----------|----------|----------|-------|
| `GET /health`, `/status`, `/ping` | ✓ | ✓ | `/status` maps to `get_health` (alias) — doc describes both |
| `GET /block/latest`, `/block/{height}` | ✓ | ✓ | |
| `GET /balance/{address}`, `/nonce/{address}` | ✓ | ✓ | |
| `GET /events`, `/chat/messages` | ✓ | ✓ | |
| `POST /tx/submit` | ✓ | ✓ | |
| `GET /jobs`, `POST /jobs/submit`, `GET /jobs/{id}` | ✓ | ✓ | Now documented with signed `Transaction::CreateJob` request |
| `GET /workers` | ✓ | ✓ | |
| `GET /epoch` | ✓ | ✓ | |

**Finding:** Job-coordinator endpoints are now documented in `docs/API_NODE.md` with request/response schemas and the auto-assign behavior. The `/jobs/submit` endpoint now requires a signed `Transaction::CreateJob` and routes through the tx pipeline, which is also reflected in the Limitations section.

### Worker (`docs/API_WORKER.md` vs `crates/miraset-worker/src/lib.rs`)

| Endpoint | In docs? | In code? |
|----------|----------|----------|
| `GET /health`, `/status`, `/ping` | ✓ | ✓ |
| `POST /jobs/accept` | ✓ | ✓ |
| `POST /jobs/run` | ✓ | ✓ |
| `GET /jobs/{id}/status` | ✓ | ✓ |
| `POST /jobs/{id}/report` | ✓ | ✓ |
| `GET /jobs/{id}/stream` | ✓ | ✓ |

Worker docs are in sync. ✓ The worker doc also documents the backend selection matrix (Ollama/OpenAI-compat/OpenLLM/llama.cpp/TGI) which matches `backend.rs`.

**Recommendation (implemented):** Job coordinator endpoints are now in `docs/API_NODE.md` with request/response schemas, auto-assign behavior, and an updated Limitations section noting the signed-tx requirement.

---

## 7. Module boundaries & cohesion (refactor candidates)

### `crates/miraset-node/src/state.rs` — **split implemented**
The original monolithic `state.rs` has been refactored into:
- `state/mod.rs` — `State`, `StateInner`, getters, `produce_block`, object helpers, lock contract
- `state/validation.rs` — `submit_transaction` (signature + nonce + per-type validation)
- `state/execution.rs` — `execute_transaction_inner` for all tx types
- `state/jobs.rs` — worker-model matching, job assignment helpers, worker endpoint lookup, and post-block dispatch
- `state/epoch_hooks.rs` — `update_epoch`, heartbeat, VRAM snapshot, job completion recording

Tests were moved to `crates/miraset-node/tests/state_tests.rs` in a prior change.

### `crates/miraset-worker/src/backend.rs` — **split implemented**
The backend implementations are now in `backend/{mod,ollama,openai,openllm,llamacpp,tgi,mock}.rs`.

### `crates/miraset-core/src/types.rs` (665 LOC) — acceptable, could split `types/{tx,object,event,block}.rs`.

---

## 8. AGENTS.md accuracy cross-check

| AGENTS.md claim | Verified? |
|-----------------|-----------|
| Workspace members & roles | ✓ accurate |
| `bincode = "=1.3.3"` pin | ✓ accurate (`Cargo.toml:28`) |
| `edition = "2024"` | ✓ accurate |
| Block producer calls `update_epoch` then `produce_block` | ✓ accurate (`lib.rs:45/47`) |
| RPC endpoints list | ✓ accurate |
| Job coordinator auto-assigns + dispatches HTTP | ✓ accurate (now via tx pipeline + post-block dispatch) |
| Worker endpoints list | ✓ accurate |
| Epoch: 60-min, 70/30 split | ✓ accurate (`epoch.rs`) |
| Gas enforced in `ExecutionContext` **not** `produce_block` | ✓ accurate (confirmed — see Security H1) |
| PoCC not wired into block production | ✓ accurate |
| Move integration is scaffold/placeholder; `move_vm.rs` does not exist; `executor.rs` is unused parallel path | ✓ accurate after update |
| `miraset-indexer` is now an in-memory indexer (not placeholder) | ✓ accurate |
| Storage: JSON for events, bincode for blocks | ✓ accurate (`storage.rs:29/97`) |
| `Arc<RwLock<StateInner>>` with `parking_lot` | ✓ accurate |
| Genesis `[1u8;32]`, 1 trillion tokens | ✓ accurate (`main.rs:230-232`) |
| Wallet keystore `~/.miraset/wallet.json` | ✓ accurate |

**Action (completed):** AGENTS.md was updated to clarify that `move_vm.rs` does not exist and that `executor.rs` is a parallel, currently-unused execution path. The job-coordinator description was also updated to reflect tx-pipeline routing.

---

## 9. Architectural recommendations (prioritized)

| Priority | Recommendation | Status | Notes |
|----------|----------------|--------|-------|
| P0 | Route job creation through the tx pipeline (`/tx/submit` with signed `CreateJob`) instead of direct `State::create_job` from RPC. | ✅ Done | `/jobs/submit` now accepts `Transaction::CreateJob` and calls `State::submit_transaction`; auto-assignment happens in `state/execution.rs`; dispatch happens post-block in `run_block_producer`. |
| P0 | Compute a real `state_root` and include in `Block`. | ✅ Done | `State::compute_state_root` hashes sorted balances/nonces/objects/versions; included in every produced block. |
| P0 | Decide consensus status: wire PoCC or mark producer as devnet-only with a runtime warning. | ✅ Done | Producer logs a devnet warning; `pocc.rs` module header documents scaffolding-only status. |
| P1 | Split `state.rs` into `state/{validation,execution,jobs,epoch_hooks}.rs`. | ✅ Done | `state/{mod,validation,execution,jobs,epoch_hooks}.rs` now split by concern. |
| P1 | Split `backend.rs` into `backend/{ollama,openai_compat,...}.rs`. | ✅ Done | `crates/miraset-worker/src/backend/{mod,ollama,openai,openllm,llamacpp,tgi,mock}.rs`. |
| P1 | Move Sled `flush`/`save_block` outside the `StateInner` write lock. | ✅ Done | Block built under lock; Sled I/O performed after `drop(w)`. |
| P1 | Update `docs/API_NODE.md` with job coordinator endpoints. | ✅ Done | Endpoints documented; `/jobs/submit` schema updated for signed `CreateJob`. |
| P1 | Update AGENTS.md: remove `move_vm.rs` reference. | ✅ Done | Reference clarified; job-coordinator flow updated. |
| P2 | Implement `miraset-indexer` or remove from workspace. | ✅ Done | `miraset-indexer` now provides an in-memory `Indexer` with type/height event indexing. |
| P2 | Add serialization-stability tests for bincode + event JSON. | ✅ Done | `test_block_bincode_stability` and `test_event_json_stability` in `crates/miraset-core/src/types.rs`. |
| P2 | Document the lock contract and async/sync lock interaction. | ✅ Done | Top-level `State` doc comment explains lock acquisition, Sled I/O outside the lock, and async/sync caveats. |
| P3 | Choose mainnet storage (Sled vs Postgres) and remove the unused `sqlx` dep if not pursuing Postgres. | ✅ Done | Removed unused `sqlx` workspace dependency; Sled remains the active devnet storage. |

---

## 10. Architecture diagram (runtime)

```
┌─────────────────────────────────────────────────────────────────────┐
│                         miraset CLI binary                          │
│  ┌───────────────┐   ┌───────────────────┐   ┌──────────────────┐   │
│  │  node start   │   │  wallet commands  │   │  (TUI separate)  │   │
│  └──────┬────────┘   └─────────┬─────────┘   └──────────────────┘   │
└─────────┼──────────────────────┼───────────────────────────────────┘
          │                      │ reqwest (JSON)
          ▼                      ▼
┌─────────────────────────────────────────────────────────────────────┐
│                       Node (Axum RPC + State)                       │
│  RPC: /tx/submit /jobs/submit /balance /nonce /block/* /events ...  │
│  State: Arc<RwLock<StateInner>>                                     │
│    pending_txs ─► produce_block ─► execute_transaction_inner ─►     │
│    Block{state_root:blake3} ─► Sled{blocks:bincode, events:json}    │
│  Epoch: update_epoch() per tick; rewards 70/30                      │
│  Consensus: NONE (PoCC scaffolding, devnet-only warning)            │
└──────────────────┬──────────────────────────────────────────────────┘
                    │ after block: scan JobAssigned events → POST
                    │ {worker}/jobs/accept (spawn, fire-and-forget)
                    ▼
┌─────────────────────────────────────────────────────────────────────┐
│                         Worker (Axum HTTP)                          │
│  /jobs/accept /jobs/run /jobs/{id}/{status,stream,report}           │
│  Backend trait: Ollama | OpenAI-compat | OpenLLM | llama.cpp | TGI  │
│  Receipt: bincode + blake3, signed by worker key                    │
│  NodeClient: POST /tx/submit (SubmitJobResult, AnchorReceipt,       │
│              SubmitResourceSnapshot heartbeat @30s)                 │
└─────────────────────────────────────────────────────────────────────┘
```
