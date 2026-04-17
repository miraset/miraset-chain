# Miraset Chain — Rewards & Economics (MVP)

This document normalizes the reward/economic model described in `docs/SOW.md` into a single place: parameters, formulas, accounting rules, and examples.

**MVP defaults (from SOW):**

- Epoch length: **60 minutes**
- Compute unit: **output tokens**
- Fixed price: **`PRICE_PER_TOKEN`** (governance parameter)
- Settlement: **batched per epoch**
- Proof: **mandatory receipt hash** on-chain for every settled job

---

## 1) Two money flows (important)

Miraset has **two** independent incentive flows:

1) **User → Worker payment** (market payment)
- Each job is prepaid into escrow.
- When the job is settled, the worker receives the payment: `tokens * PRICE_PER_TOKEN`.

2) **Protocol rewards** (incentive rewards)
- Per-epoch protocol budget is distributed to:
  - capacity/validator pool
  - compute pool

This separation is intentional:

- Direct payments price the service.
- Protocol rewards bootstrap supply, reliability, and decentralization.

---

## 2) Parameters (governance-controlled)

### 2.1 Epoch & settlement

- `EPOCH_SECONDS = 3600`
- `SUBMIT_WINDOW_SECONDS` (e.g., 40 min)
- `CHALLENGE_WINDOW_SECONDS` (e.g., 10–20 min)

### 2.2 Pricing

- `PRICE_PER_TOKEN` — fixed cost per output token (native token denominated)

### 2.3 Reward budgets

Per epoch `e`:

- `R_total(e)` — total rewards budget for epoch
- `R_val(e)` — validator/capacity reward pool
- `R_comp(e)` — compute reward pool

Constraint:

- `R_total(e) = R_val(e) + R_comp(e)`

Split can be a fixed ratio:

- `R_val(e) = R_total(e) * SPLIT_VAL`
- `R_comp(e) = R_total(e) * (1 - SPLIT_VAL)`

### 2.4 Capacity scoring

- `U_MIN` — minimum uptime to qualify for capacity rewards (suggested: `0.90`)
- `VRAM_CAP_GIB` — saturation cap for VRAM contribution (suggested: `80`)
- `a` — uptime exponent (suggested: `2`)
- `b` — VRAM exponent (suggested: `1`)

### 2.5 Compute scoring

- `Q_i(e)` — optional quality multiplier in `[0,1]` (MVP: `1`)
- `P_i(e)` — penalty multiplier in `[0,1]` (MVP: `1`, fraud → `0`)

---

## 3) Sources of truth (what is measured)

### 3.1 Uptime `U_i(e)`

`U_i(e) ∈ [0,1]` is an epoch-level score derived from signed heartbeats observed by validators/watchers.

MVP recommended mechanics:

- Heartbeat interval: 30–60 seconds.
- For each observation point `k`, observer records `reachability_i(k)` in `{0,1}`.
- Aggregate:

`U_i(e) = (Σ_k reachability_i(k)) / N_samples`

Challenge rule:

- Aggregates are committed in the epoch settlement batch.
- Node can challenge within `CHALLENGE_WINDOW_SECONDS`.

### 3.2 Available VRAM `V_i(e)`

`V_i(e)` is the time-average of `vram_avail_gib` snapshots:

`V_i(e) = avg_over_epoch(vram_avail_gib snapshots)`

VRAM is *saturated* for rewards with `VRAM_CAP_GIB`.

### 3.3 Verified tokens `T_i(e)`

`T_i(e)` is the sum of **verified output tokens** across settled jobs in epoch `e`.

A job is “verified” if:

- It has a valid on-chain `receipt_hash` anchor.
- All required signatures are valid (worker, and optionally coordinator for MVP).
- Reported token counts satisfy job constraints.

---

## 4) Receipt hash (mandatory anchor)

For every job `j`, the chain stores:

- `receipt_hash(j) = H(receipt_payload(j))`

Minimal `receipt_payload` fields (MVP):

- `job_id, epoch_id, worker_pubkey`
- `model_id`
- `request_hash` (prompt+params hash; private inputs remain off-chain)
- `response_stream_hash`
- `output_tokens`
- `price_per_token` (must match epoch parameter)
- timestamps
- signatures

Why it matters:

- Makes token accounting auditable.
- Enables disputes without putting full prompts/outputs on-chain.

---

## 5) Reward formulas (explicit)

### 5.1 Normalization

Saturating VRAM normalization:

- `cap_vram(V) = min(V, VRAM_CAP_GIB)`
- `n_vram_i(e) = cap_vram(V_i(e)) / VRAM_CAP_GIB`  (range `[0,1]`)

Eligibility:

- if `U_i(e) < U_MIN` then `ScoreVal_i(e) = 0`

### 5.2 Capacity / ValidatorReward

Capacity score per node `i`:

- `ScoreVal_i(e) = (U_i(e)^a) * (n_vram_i(e)^b)`

Capacity reward:

- `ValidatorReward_i(e) = R_val(e) * ScoreVal_i(e) / Σ_j ScoreVal_j(e)`

If denominator is zero:

- `R_val(e)` is burned or rolled over (governance choice; MVP: roll over).

### 5.3 ComputeReward

Adjusted token score:

- `AdjTokens_i(e) = T_i(e) * Q_i(e) * P_i(e)`

Compute reward:

- `ComputeReward_i(e) = R_comp(e) * AdjTokens_i(e) / Σ_j AdjTokens_j(e)`

If denominator is zero:

- `R_comp(e)` is burned or rolled over (governance choice; MVP: roll over).

---

## 6) Job payment formula (fixed price)

For job `j` settled in epoch `e`:

- `JobCost(j) = output_tokens(j) * PRICE_PER_TOKEN`

Accounting rules:

- User prepays `escrow_amount >= JobCost(j)`.
- On settlement:
  - transfer `JobCost(j)` to worker
  - refund leftover escrow to user (optional; or keep as prepaid balance)

---

## 7) Worked example

Assume epoch `e`:

- `R_val(e) = 1,000` tokens
- `R_comp(e) = 2,000` tokens
- `VRAM_CAP_GIB = 80`, `U_MIN = 0.90`, `a=2`, `b=1`

### 7.1 Capacity

Node A:
- `U_A = 0.99`
- `V_A = 40 GiB` → `n_vram_A = 0.5`
- `ScoreVal_A = 0.99^2 * 0.5 ≈ 0.49005`

Node B:
- `U_B = 0.95`
- `V_B = 80 GiB` → `n_vram_B = 1`
- `ScoreVal_B = 0.95^2 * 1 ≈ 0.9025`

Sum: `1.39255`

- `ValidatorReward_A ≈ 1000 * 0.49005 / 1.39255 ≈ 351.9`
- `ValidatorReward_B ≈ 648.1`

### 7.2 Compute

Worker A:
- `T_A = 120,000` verified output tokens, `Q=1`, `P=1`

Worker B:
- `T_B = 80,000` verified output tokens, `Q=1`, `P=1`

Sum: `200,000`

- `ComputeReward_A = 2000 * 120k / 200k = 1200`
- `ComputeReward_B = 800`

---

## 8) Edge cases & rules

1. **Sybil by splitting VRAM**
   - Saturation (`VRAM_CAP_GIB`) reduces benefit of “monster nodes”.
   - Consider minimum stake/bond per worker to increase Sybil cost.

2. **Token report fraud**
   - Requires valid receipt hash and signatures.
   - Disputes set `P_i(e)` to `0` for that epoch and trigger slashing if bonded.

3. **Uptime spoofing**
   - Signed heartbeats prevent spoofed identity.
   - Multiple observers reduce single point failure.

4. **No jobs in an epoch**
   - Compute pool rolls over (MVP) or can be burned.

5. **Coordinator trust (MVP)**
   - Coordinator assignment signature prevents fake jobs.
   - Later: permissionless job marketplace + cryptographic auditing.

---

## 9) Implementation notes (non-normative)

To keep chain costs low:

- Store only `receipt_hash` and minimal indexing on-chain.
- Batch per-epoch roots (`EpochBatch`) anchor many job results.
- Keep detailed receipt payloads off-chain but retrievable by indexers.
