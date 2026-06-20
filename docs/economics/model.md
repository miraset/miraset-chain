# Proof of Compute & Capacity (PoCC)

## Capacity Rewards
**Purpose:** Compensate workers for maintaining available GPU resources.
**Measured by:** Uptime (heartbeats) and VRAM snapshots.
**Formula:** `R_capacity(worker) ∝ (uptime_score × available_VRAM)`

## Compute Rewards
**Purpose:** Pay workers for actual inference work performed.
**Measured by:** Verified output tokens and model difficulty.
**Formula:** `R_compute(worker) ∝ (total_verified_tokens × model_difficulty_multiplier)`

## Token Flows
- 85% → Worker Direct Payment
- 10% → Protocol Treasury
- 5%  → Validator Rewards

## Reward Distribution (Per Epoch)
- 60% → Compute Reward Pool
- 40% → Capacity Reward Pool

## Network Parameters
- `epoch_duration`: 60 minutes
- `min_worker_stake`: 100 tokens
- `slash_percentage`: 10%
