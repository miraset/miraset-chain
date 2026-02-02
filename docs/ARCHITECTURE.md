# Miraset Chain — Application Architecture (MVP)

This document describes the target architecture of the **Miraset Chain** application and surrounding services.

It is written to be implementable and consistent with:

- `docs/SOW.md` (scope, PoCC, object model, client requirements)
- `docs/REWARDS.md` (canonical economics and reward formulas)
- `docs/DATA.md` (pattern: off-chain inference + on-chain settlement / billing)

---

## 1) Goals / non-goals

### Goals (MVP)

- **Sui-like** object-centric on-chain state model for jobs and capacity.
- **Epoch accounting**: 60-minute epochs with **batched-per-epoch settlement**.
- **Two incentive flows**:
  - user escrow → worker payment (fixed price per token)
  - protocol rewards (capacity + compute pools)
- **Proof of Compute & Capacity (PoCC)**:
  - capacity: uptime + VRAM availability
  - compute: verified output tokens
- Mandatory **receipt hash anchor** (`receipt_hash`) on-chain for every settled job.
- A worker runtime (client) analogous to **Ollama** for GPU inference.

### Non-goals (MVP)

- Permissionless job marketplace.
- Fully trustless ZK proof of correct model execution.
- Cross-chain interoperability.

---

## 2) Roles & trust boundaries

### Actors

- **Validator**: participates in consensus, finalizes settlement and rewards, may observe heartbeats.
- **Worker**: GPU node executing inference jobs off-chain; submits signed reports and receipt hashes.
- **Coordinator/Scheduler (MVP)**: permissioned job assigner. Signs assignment and optionally co-signs receipts.
- **Watcher/Indexer**: off-chain observer for uptime sampling and event indexing; supports dispute evidence.
- **User/Requester**: funds escrow and requests inference jobs.

### Trust model (MVP)

- Coordinator is **trusted for assignment** (prevents fake jobs) but **not trusted for accounting** alone.
- Token accounting relies on:
  - worker signatures
  - receipt hash commitments
  - deterministic settlement rules
  - disputes + penalties

---

## 3) System decomposition (high-level)

### On-chain components

1. **Consensus / Finality layer** (Modified BFT/PoS)
   - committee / validator set
   - epoch transitions
   - finality for settlement transactions

2. **Runtime / modules** (conceptual)
   - `staking`: validator registration, delegation, unbonding
   - `governance`: parameter updates
   - `pocc_capacity`: worker registrations, VRAM snapshots, uptime commitments
   - `pocc_jobs`: job objects, assignment/auth, escrow
   - `settlement`: epoch batching, disputes, payouts, reward distribution

3. **RPC layer**
   - read access for wallets, explorers, indexers
   - tx submission

### Off-chain components

1. **Worker runtime (Ollama-like)**
   - runs inference engine (Ollama/vLLM/bare-metal)
   - exposes HTTP API for job execution + streaming
   - generates signed usage reports + receipt payload/receipt hash

2. **Coordinator/Scheduler (MVP)**
   - admits users, creates job objects, sets constraints
   - picks workers based on VRAM, model support, policy
   - signs assignment (and optionally co-signs receipts)

3. **Watcher / Indexer**
   - consumes on-chain events and builds queryable DB
   - stores/serves off-chain receipt payloads (by hash)
   - optionally samples worker uptime and produces evidence during disputes

---

## 4) On-chain state model (Sui-like objects)

The chain uses an object-centric approach: job state and settlements are represented as objects that can be mutated by authorized transactions.

### Core objects (MVP)

- `WorkerRegistration`
  - identity: `worker_id`, `owner`, `pubkey`
  - endpoints (where to send jobs)
  - capabilities: `gpu_model`, `vram_total_gib`, supported models
  - optional `stake_bond` (for slashing)
  - status: Active/Jailed

- `ResourceSnapshot`
  - `epoch_id`, `worker_id`
  - `vram_avail_gib`
  - signature

- `InferenceJob`
  - `job_id`, `epoch_id`, requester
  - model, max_tokens
  - assigned worker
  - `fixed_price_per_token`
  - escrow amount
  - status: Created/Assigned/Running/Completed/Challenged/Finalized

- `JobResult`
  - `job_id`, `worker_id`
  - `output_tokens`
  - `receipt_hash`
  - worker signature (+ optional coordinator signature)

- `EpochBatch`
  - `epoch_id`
  - batch root (optional Merkle root for scaling)
  - `total_verified_tokens`
  - committee signature / attestation

- `ReceiptAnchor`
  - `job_id`, `epoch_id`, `receipt_hash`

### Indexing/events

Every critical transition emits an event (explorer/indexer friendly):

- `WorkerRegistered`, `ResourceSnapshotSubmitted`
- `JobCreated`, `JobAssigned`, `JobCompleted`
- `ReceiptAnchored`
- `EpochSettled`, `RewardsDistributed`, `Slashed`

---

## 5) Off-chain interfaces

### 5.1 Worker HTTP API (MVP)

The worker runtime SHOULD implement endpoints (names can change; behaviors are the contract):

- `POST /jobs/accept` — accept an assignment
- `POST /jobs/{id}/run` — start processing
- `GET /jobs/{id}/stream` — stream output tokens
- `POST /jobs/{id}/report` — submit signed usage report + receipt hash
- `GET /health` — heartbeat/health

> The worker must be able to compute `response_stream_hash` deterministically from the streamed response.

### 5.2 Coordinator API (MVP)

- Create job and escrow (on-chain)
- Assign job to worker (on-chain + worker call)
- Optionally store receipt payloads off-chain for retrieval by hash

### 5.3 Indexer API (MVP)

- Query workers, jobs, epochs, rewards
- Fetch receipt payload by `receipt_hash`

---

## 6) Data contracts (receipt payload & hashing)

### 6.1 Receipt payload

A receipt payload is a structured blob that is **hashed** and anchored on-chain.

Minimal fields (MVP):

- `job_id`, `epoch_id`
- `worker_pubkey`
- `model_id`
- `request_hash` (hash of prompt + parameters)
- `response_stream_hash`
- `output_tokens`
- `price_per_token` (must match epoch effective parameter)
- `timestamp_start`, `timestamp_end`
- `worker_signature`
- optional `coordinator_signature`

### 6.2 Hashing and serialization

- Use a canonical serialization format (e.g., BCS-like) so hashes are deterministic.
- `receipt_hash = H( receipt_payload )`
- On-chain stores only `receipt_hash` and minimal indexes.

### 6.3 Why store payload off-chain

- Receipts can be large (stream hashes, metadata).
- Keeping payload off-chain reduces chain state bloat.
- The on-chain hash still allows verifying integrity.

---

## 7) Core flows (sequence-style)

### 7.1 Worker registration & uptime

1. Worker submits `WorkerRegistration` on-chain.
2. Worker periodically submits `ResourceSnapshot` (VRAM availability) + keeps `GET /health` responding.
3. Validators/watchers sample reachability and aggregate `U_i(e)`.
4. Aggregates are committed during epoch settlement.

### 7.2 Job lifecycle (assigned coordinator)

1. User funds a prepaid balance or escrow.
2. Coordinator creates `InferenceJob` with constraints and escrow.
3. Coordinator assigns Worker:
   - on-chain: update job assignment
   - off-chain: calls `POST /jobs/accept` on worker
4. Worker executes and streams output via `GET /jobs/{id}/stream`.
5. Worker produces usage report + receipt payload, computes `receipt_hash`.
6. Worker anchors `ReceiptAnchor(job_id, epoch_id, receipt_hash)` on-chain and submits `JobResult`.

### 7.3 Epoch settlement (batched-per-epoch)

1. At epoch end, validators collect:
   - settled `JobResult`s
   - VRAM snapshots
   - uptime aggregates
2. Validators build `EpochBatch(e)` and finalize settlement tx:
   - pays job costs from escrow to workers
   - computes rewards using canonical formulas (see `docs/REWARDS.md`)
   - applies penalties and slashing when proven
3. Disputes:
   - during `CHALLENGE_WINDOW`, anyone can challenge invalid receipts/signatures.

---

## 8) Storage & indexing strategy

### On-chain (minimal)

- Objects needed for correctness, settlement, and auditing.
- Compact anchors: `receipt_hash` rather than full receipt payload.

### Off-chain (recommended)

- Indexer DB for explorer and accounting reproducibility
  - Postgres recommended for relational queries
- Receipt payload store keyed by `receipt_hash`
  - can be Postgres, S3-compatible blob store, or content-addressed storage

### Deterministic accounting

An external auditor should be able to recompute:

- `T_i(e)` from settled jobs
- rewards distribution from event streams

---

## 9) Security considerations

### Threats

- **Fraudulent token reporting** (inflate `output_tokens`).
- **Uptime spoofing** (fake health without availability).
- **Coordinator abuse** (assigning unfairly, censorship).
- **Replay** (resubmitting old receipts).
- **Consensus faults** (double-signing).

### Controls (MVP)

- Signed heartbeats and signed usage reports.
- Mandatory receipt hash anchors.
- Challenge window + penalties/slashing with proofs (invalid signature, inconsistent fields).
- Replay protection via job id uniqueness and `job settled only once` invariant.

### Roadmap hardening

- Add audit sampling (replication / spot-checking).
- Introduce worker bonds mandatory for compute participation.
- Move from coordinator trust to permissionless job creation.

---

## 10) Operations

- Config management: genesis parameters, price per token, reward budgets.
- Metrics:
  - chain: block/epoch timing, mempool
  - PoCC: number of active workers, VRAM capacity, tokens per epoch
  - settlement: disputes, slashes
- Logging and tracing across components (worker/coordinator/indexer).
- Key management: separate keys for node identity vs payout addresses (recommended).
- Upgrades: governance-triggered parameter upgrades; protocol upgrades via standard on-chain signaling.

---

## 11) Implementation guidance (Rust-first)

> The repository currently states “Rust backed app”. Below is a concrete, buildable direction for a Rust workspace.

### Suggested workspace layout

- `crates/`
  - `chain-node/` — node binary, networking, RPC
  - `runtime/` — on-chain logic (modules) and state types
  - `types/` — shared types (receipt payload, job ids, hashing)
  - `crypto/` — key handling, signatures, hashing
  - `worker/` — worker runtime (Ollama-like HTTP server + adapters)
  - `coordinator/` — scheduler service (MVP)
  - `indexer/` — chain event consumer + DB

### Rust stack recommendations

- HTTP server (worker/coordinator): `axum` + `tokio`
- Serialization for signed payloads: `serde` (+ a canonical binary format; choose early)
- Hashing: `blake3` or `sha2` (pick one chain-wide)
- DB/indexer: `sqlx` + Postgres
- Observability: `tracing`, `tracing-subscriber`, Prometheus exporter

### Strongly recommended “contracts” to implement early

- Canonical receipt payload encoding + hashing (cross-language reproducibility).
- Deterministic epoch settlement calculations matching `docs/REWARDS.md`.

---

## 12) MVP → Next steps

### MVP deliverables

- Worker registration, VRAM snapshots, uptime scoring.
- Job object lifecycle + escrow + receipt anchoring.
- Epoch batching + settlement and reward distribution.
- Indexer for auditability.

### Next (post-MVP)

- Permissionless job marketplace.
- Stronger proof-of-compute (replication/spot-check committees; later ZK proofs).
- QoS/latency SLAs and quality multipliers.
- Decentralized coordinator.
