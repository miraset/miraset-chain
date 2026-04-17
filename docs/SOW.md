# Miraset Chain — Statement of Work (SOW)

> Goal: design and implement a Sui-like blockchain network that rewards nodes for (1) being online and providing GPU capacity (VRAM) and (2) executing *useful* GPU workloads (LLM inference), measured in **tokens** and **verifiable**.
>
> Consensus: **Modified BFT/PoS**
>
> Resource/work model: **Proof of Compute & Capacity (PoCC)** = Capacity availability + Verified compute.
>
> Rewards & economics: see `docs/REWARDS.md` for the canonical formulas, parameters, and examples.

---

## 1) Product overview

Miraset Chain is a compute-first blockchain where participant nodes run a client (analogous to **Ollama**) that:

- Registers hardware capabilities (primarily **VRAM**, optional GPU model, bandwidth, etc.).
- Maintains a verifiable uptime heartbeat.
- Executes assigned LLM inference jobs off-chain.
- Submits signed usage reports and **zk-receipt hashes** on-chain.

The chain performs:

- Validator-set consensus (PoS + BFT finality).
- Job settlement and reward distribution **batched per epoch**.
- Slashing/penalties for misbehavior.

**MVP decisions (fixed for this SOW):**

- L1 model reference: **Sui-like object-centric data model**.
- Epoch duration: **60 minutes**.
- Compute unit: **tokens**.
- Price model: **fixed price per token** (configurable by governance).
- Job source: **assigned coordinator** (permissioned scheduler) for MVP.
- zk receipts: **required** (on-chain anchor as a hash; full ZK statement can evolve).

---

## 2) Scope

### In-scope (MVP)

1. **Standard blockchain functionality** (best-practice baseline)
   - Identity & accounts (ed25519/secp256k1; exact curve TBD by implementation).
   - Transaction format, fee / gas, mempool rules.
   - Blocks, epochs, finality guarantees.
   - Validator set management: staking, delegation, unbonding.
   - Slashing & jailing.
   - Governance (parameter updates at minimum).
   - RPC / node API for wallets, explorers, indexers.
   - Events/logs suitable for indexing.

2. **PoCC subsystem (Proof of Compute & Capacity)**
   - Worker registration (GPU providers) and periodic capacity reporting.
   - Uptime measurement per epoch.
   - Job execution lifecycle (create → assign → execute → report → settle).
   - zk-receipt hash anchoring per job.
   - Batched-per-epoch settlement: payments, rewards, penalties.

3. **Client node (Ollama-like)**
   - Runs the inference engine (can wrap Ollama or be compatible with its API style).
   - Exposes endpoints to accept jobs and stream tokens.
   - Produces a signed usage report and receipt hash.

### Out of scope (for this SOW)

- Permissionless job marketplace (open task submission by anyone) — deferred.
- Fully trustless proof of correct inference (strong ZK for full model execution) — deferred.
- Cross-chain bridges.
- Decentralized storage layer.

---

## 3) Glossary

- **Epoch**: fixed 60-minute accounting window.
- **Validator**: consensus participant producing blocks and verifying settlement.
- **Worker**: GPU provider executing inference jobs (may also be a validator or separate)
- **Coordinator/Scheduler**: assigned service that creates and assigns jobs in MVP.
- **Job**: an inference request that produces streamed tokens.
- **Usage report**: signed record describing measured tokens and metadata.
- **zk-receipt hash**: on-chain commitment to a receipt payload (hash), used for audit/challenge.
- **VRAM_avail**: VRAM made available to the network for the epoch (GiB).
- **Uptime**: fraction of epoch where the node is reachable and behaving.

---

## 4) Architecture (high-level)

### 4.1 On-chain

- **Consensus layer**: Modified BFT/PoS with fast finality.
- **State model**: Sui-like objects.
- **Modules** (conceptual):
  - `staking`
  - `governance`
  - `pocc_capacity`
  - `pocc_jobs`
  - `settlement`

### 4.2 Off-chain

- **Worker runtime**: runs inference engine (Ollama/vLLM/bare-metal). Streams tokens to requester via coordinator.
- **Coordinator**: assigns jobs based on VRAM/capability and policy.
- **Auditors/Watchers** (optional in MVP): sample jobs or verify receipts.

### 4.3 Data flow (MVP)

1. Worker registers and advertises capacity.
2. Worker sends heartbeats to validators / watchers.
3. Coordinator creates job and assigns to worker.
4. Worker runs inference and streams output.
5. Worker submits signed usage report + zk-receipt hash.
6. At epoch end, validators finalize settlement batch and distribute rewards.

---

## 5) Standard blockchain requirements (baseline)

### 5.1 Accounts, transactions, fees

- All state transitions are driven by transactions.
- Fees (gas) are paid by sender. Gas schedule is governance-controlled.
- Replay protection via nonce / sequence.

### 5.2 Validator set, staking, slashing

- Validators must stake the native token.
- Delegation supported.
- Unbonding period (parameter) to prevent instant exit.
- Slashing conditions (MVP):
  - Double-sign / equivocation.
  - Persistent downtime.
  - Fraud in settlement attestations (if proven).

### 5.3 Governance

MVP governance should allow updating:

- Epoch parameters.
- Reward budgets and weights.
- Fixed token pricing for compute.
- Slashing thresholds and caps.

---

## 6) PoCC: Proof of Compute & Capacity

PoCC is split into two measurable dimensions:

1. **Capacity** (VRAM availability + uptime)
2. **Compute** (useful jobs measured in tokens + verifiability)

### 6.1 Capacity measurement

**Node heartbeat**

- Worker/validator emits periodic heartbeats (e.g., every 30s–60s) to a set of observers.
- Observers are primarily validators; may include dedicated watchers.

Recommended MVP approach (“best” for simplicity + robustness):

- Heartbeat signed by the node key.
- Observers aggregate observations per epoch (off-chain), commit aggregate on-chain in settlement.
- Node can challenge incorrect aggregates during a challenge window.

**VRAM availability**

- Worker periodically publishes a signed `ResourceSnapshot` with:
  - `vram_total_gib`
  - `vram_avail_gib` (promised to network)
  - optional: `gpu_model`, `pci_id`, bandwidth
- For MVP, we assume *soft attestation*: punish only for egregious mismatch discovered during audits.

### 6.2 Compute measurement (tokens)

- Compute work is measured as **verified output tokens** (and optionally input tokens) for each job.
- A job produces:
  - a signed usage report
  - a **zk-receipt hash** anchored on-chain

**Fixed pricing**

- `PRICE_PER_TOKEN` is a governance parameter.
- In MVP, the scheduler uses this price to quote jobs.

### 6.3 zk-receipt hash (required)

MVP requires anchoring a receipt commitment on-chain:

- `receipt_payload` includes at minimum:
  - `job_id`
  - `epoch_id`
  - `worker_pubkey`
  - `model_id`
  - `request_hash` (e.g., hash of prompt + parameters; inputs can remain private)
  - `response_stream_hash` (hash over streamed output chunks or final output)
  - `output_tokens` (integer)
  - `price_per_token` (must equal the epoch effective parameter)
  - `timestamp_start`, `timestamp_end`
  - `worker_signature`
  - optional: `coordinator_signature`

- On-chain stores only `receipt_hash = H(receipt_payload)` plus minimal indexes.

Notes:

- “zk” here means the interface is ZK-ready: later, the receipt can include a proof or proof reference.
- MVP may start with hash commitment + signatures, but the **anchor is mandatory**.

---

## 7) Sui-like object model (proposed)

This is a contract-level data model for the chain runtime.

### 7.1 Objects

1. `WorkerRegistration`
   - `worker_id` (object id)
   - `owner` (address)
   - `pubkey`
   - `endpoints` (RPC/GRPC/HTTP)
   - `capabilities` (gpu_model, vram_total_gib, supported_models)
   - `stake_bond` (optional for workers; required if we want slashing)
   - `status` (Active/Jailed)

2. `ResourceSnapshot`
   - `epoch_id`
   - `worker_id`
   - `vram_avail_gib`
   - `signature`

3. `InferenceJob`
   - `job_id`
   - `epoch_id`
   - `requester` (address)
   - `model_id`
   - `max_tokens`
   - `assigned_worker_id`
   - `fixed_price_per_token` (copied from parameter)
   - `escrow_amount` (prepaid)
   - `status` (Created/Assigned/Running/Completed/Challenged/Finalized)

4. `JobResult`
   - `job_id`
   - `worker_id`
   - `output_tokens`
   - `receipt_hash`
   - `worker_signature`
   - optional `coordinator_signature`

5. `EpochBatch`
   - `epoch_id`
   - `batch_root` (Merkle root of claimed results)
   - `total_verified_tokens`
   - `settled` (bool)
   - `validator_committee_signature` (threshold signature or aggregated attestation)

6. `ReceiptAnchor`
   - `job_id`
   - `epoch_id`
   - `receipt_hash`

### 7.2 Events (indexer-friendly)

- `WorkerRegistered`
- `ResourceSnapshotSubmitted`
- `JobCreated`
- `JobAssigned`
- `JobCompleted`
- `ReceiptAnchored`
- `EpochSettled`
- `RewardsDistributed`
- `Slashed`

---

## 8) Batched-per-epoch settlement (60 minutes)

### 8.1 Timing

Per epoch `e`:

- `submit_window`: workers submit `JobResult` and anchor receipt hashes.
- `challenge_window`: anyone can challenge incorrect reports (governance parameter; e.g., 10–20 min).
- `finalize`: validators finalize `EpochBatch(e)`.

### 8.2 Settlement invariants

- A job can be settled only once.
- `price_per_token` must equal the effective epoch parameter.
- `output_tokens` must be within job constraints (`<= max_tokens`) and consistent with receipt hash.
- Escrow and fees must balance:
  - user escrow covers job cost
  - worker receives payment (minus protocol fee if any)

### 8.3 Challenge / dispute (MVP)

MVP dispute is based on:

- mismatched receipt hash vs claimed fields
- invalid signatures
- token count inconsistencies
- coordinator-signed assignment mismatch

If a worker is proven fraudulent:

- slash worker bond (if required)
- remove/penalize rewards for epoch
- possibly jail

---

## 9) Rewards (summary)

Rewards are accounted **per epoch** and split into two pools:

- **Capacity/Validator rewards** — based on uptime + available VRAM.
- **Compute rewards** — based on verified jobs (measured in output tokens) and penalties.

The canonical reward parameters, formulas, accounting rules, and examples live in:

- `docs/REWARDS.md`

---

## 10) Client (Ollama-like) requirements

The worker client SHOULD provide:

- `POST /jobs/accept` (job assignment)
- `POST /jobs/{id}/run` (start)
- `GET /jobs/{id}/stream` (stream tokens)
- `POST /jobs/{id}/report` (signed usage report + receipt hash)
- `GET /health` (heartbeat)

Minimum telemetry:

- current VRAM available
- model inventory
- job queue depth
- last successful heartbeat

Security:

- All reports signed by worker private key.
- Coordinator assignment signed (MVP) to prevent spoofed jobs.

---

## 11) Acceptance criteria (MVP)

1. Epoch accounting at 60 minutes works end-to-end.
2. Nodes can register capacity and produce uptime heartbeats.
3. Jobs can be assigned, executed off-chain, and results settled on-chain batched-per-epoch.
4. Every settled job has an on-chain `receipt_hash` anchor.
5. Rewards are computed with the specified formulas and are reproducible from indexed data.
6. Fraudulent reports can be challenged and penalized.

---

## 12) Open questions (tracked)

- Exact cryptographic suite and signature aggregation method.
- Whether workers must post a bond (recommended for slashing).
- Whether validators and workers are the same entities or separate roles.
- Optional: input token accounting and/or latency-SLA multipliers.
