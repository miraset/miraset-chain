use miraset_core::{Address, ObjectId};
use chrono::{DateTime, Utc, Duration as ChronoDuration};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Epoch duration: 60 minutes (as per ARCHITECTURE.md)
pub const EPOCH_DURATION_SECONDS: i64 = 3600;

/// Submit window: 40 minutes
pub const SUBMIT_WINDOW_SECONDS: i64 = 2400;

/// Challenge window: 20 minutes
pub const CHALLENGE_WINDOW_SECONDS: i64 = 1200;

/// Minimum uptime to qualify for capacity rewards (90%)
pub const U_MIN: f64 = 0.90;

/// VRAM saturation cap (GiB)
pub const VRAM_CAP_GIB: f64 = 80.0;

/// Uptime exponent
pub const UPTIME_EXPONENT: f64 = 2.0;

/// VRAM exponent
pub const VRAM_EXPONENT: f64 = 1.0;

/// Fixed price per token (in native tokens, smallest unit)
pub const PRICE_PER_TOKEN: u64 = 10;

/// Epoch rewards split (70% capacity, 30% compute)
pub const CAPACITY_REWARD_SPLIT: f64 = 0.70;
pub const COMPUTE_REWARD_SPLIT: f64 = 0.30;

/// Total epoch reward budget (adjustable by governance)
pub const EPOCH_REWARD_BUDGET: u64 = 1_000_000_000; // 1 billion units per epoch

/// Epoch state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Epoch {
    pub id: u64,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
    pub status: EpochStatus,
    pub worker_stats: HashMap<ObjectId, WorkerEpochStats>,
    pub job_results: HashMap<ObjectId, JobResult>,
    pub total_verified_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum EpochStatus {
    Active,
    SubmitWindow,
    ChallengeWindow,
    Settled,
}

/// Worker statistics for an epoch
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerEpochStats {
    pub worker_id: ObjectId,
    pub owner: Address,
    pub uptime_score: f64,           // U_i(e) ∈ [0,1]
    pub vram_avail_avg_gib: f64,     // V_i(e)
    pub verified_tokens: u64,        // T_i(e)
    pub heartbeat_samples: u64,
    pub heartbeat_success: u64,
}

impl WorkerEpochStats {
    pub fn new(worker_id: ObjectId, owner: Address) -> Self {
        Self {
            worker_id,
            owner,
            uptime_score: 0.0,
            vram_avail_avg_gib: 0.0,
            verified_tokens: 0,
            heartbeat_samples: 0,
            heartbeat_success: 0,
        }
    }

    pub fn record_heartbeat(&mut self, success: bool) {
        self.heartbeat_samples += 1;
        if success {
            self.heartbeat_success += 1;
        }
        self.uptime_score = self.heartbeat_success as f64 / self.heartbeat_samples as f64;
    }

    pub fn add_vram_snapshot(&mut self, vram_gib: f64) {
        // Simple moving average (could be improved with weighted average)
        let n = self.heartbeat_samples as f64;
        self.vram_avail_avg_gib = (self.vram_avail_avg_gib * (n - 1.0) + vram_gib) / n;
    }

    pub fn add_verified_tokens(&mut self, tokens: u64) {
        self.verified_tokens += tokens;
    }

    /// Calculate capacity score C_i(e) as per REWARDS.md
    pub fn capacity_score(&self) -> f64 {
        if self.uptime_score < U_MIN {
            return 0.0;
        }

        let u_component = self.uptime_score.powf(UPTIME_EXPONENT);
        let v_capped = self.vram_avail_avg_gib.min(VRAM_CAP_GIB);
        let v_component = v_capped.powf(VRAM_EXPONENT);

        u_component * v_component
    }
}

/// Job result for settlement
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobResult {
    pub job_id: ObjectId,
    pub worker_id: ObjectId,
    pub requester: Address,
    pub output_tokens: u64,
    pub receipt_hash: [u8; 32],
    pub cost: u64, // tokens * PRICE_PER_TOKEN
}

impl Epoch {
    pub fn new(id: u64, start_time: DateTime<Utc>) -> Self {
        Self {
            id,
            start_time,
            end_time: start_time + ChronoDuration::seconds(EPOCH_DURATION_SECONDS),
            status: EpochStatus::Active,
            worker_stats: HashMap::new(),
            job_results: HashMap::new(),
            total_verified_tokens: 0,
        }
    }

    /// Check if epoch should transition to next phase
    pub fn update_status(&mut self, now: DateTime<Utc>) {
        let elapsed = (now - self.start_time).num_seconds();

        match self.status {
            EpochStatus::Active if elapsed >= EPOCH_DURATION_SECONDS => {
                self.status = EpochStatus::SubmitWindow;
            }
            EpochStatus::SubmitWindow if elapsed >= EPOCH_DURATION_SECONDS + SUBMIT_WINDOW_SECONDS => {
                self.status = EpochStatus::ChallengeWindow;
            }
            EpochStatus::ChallengeWindow if elapsed >= EPOCH_DURATION_SECONDS + SUBMIT_WINDOW_SECONDS + CHALLENGE_WINDOW_SECONDS => {
                self.status = EpochStatus::Settled;
            }
            _ => {}
        }
    }

    /// Record a job result
    pub fn add_job_result(&mut self, result: JobResult) {
        if let Some(stats) = self.worker_stats.get_mut(&result.worker_id) {
            stats.add_verified_tokens(result.output_tokens);
        }
        self.total_verified_tokens += result.output_tokens;
        self.job_results.insert(result.job_id, result);
    }

    /// Calculate epoch rewards distribution
    pub fn calculate_rewards(&self) -> EpochRewards {
        let capacity_pool = (EPOCH_REWARD_BUDGET as f64 * CAPACITY_REWARD_SPLIT) as u64;
        let compute_pool = (EPOCH_REWARD_BUDGET as f64 * COMPUTE_REWARD_SPLIT) as u64;

        let mut rewards = EpochRewards {
            epoch_id: self.id,
            capacity_pool,
            compute_pool,
            worker_rewards: HashMap::new(),
        };

        // Calculate total capacity score
        let total_capacity_score: f64 = self.worker_stats.values()
            .map(|w| w.capacity_score())
            .sum();

        // Calculate total verified tokens
        let total_tokens = self.total_verified_tokens;

        // Distribute rewards to each worker
        for (worker_id, stats) in &self.worker_stats {
            let capacity_score = stats.capacity_score();
            
            // Capacity reward: proportional to capacity score
            let capacity_reward = if total_capacity_score > 0.0 {
                ((capacity_pool as f64) * (capacity_score / total_capacity_score)) as u64
            } else {
                0
            };

            // Compute reward: proportional to verified tokens
            let compute_reward = if total_tokens > 0 {
                ((compute_pool as f64) * (stats.verified_tokens as f64 / total_tokens as f64)) as u64
            } else {
                0
            };

            rewards.worker_rewards.insert(*worker_id, WorkerReward {
                worker_id: *worker_id,
                owner: stats.owner,
                capacity_reward,
                compute_reward,
                total_reward: capacity_reward + compute_reward,
                uptime_score: stats.uptime_score,
                vram_avg_gib: stats.vram_avail_avg_gib,
                verified_tokens: stats.verified_tokens,
            });
        }

        rewards
    }
}

/// Epoch rewards distribution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EpochRewards {
    pub epoch_id: u64,
    pub capacity_pool: u64,
    pub compute_pool: u64,
    pub worker_rewards: HashMap<ObjectId, WorkerReward>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerReward {
    pub worker_id: ObjectId,
    pub owner: Address,
    pub capacity_reward: u64,
    pub compute_reward: u64,
    pub total_reward: u64,
    pub uptime_score: f64,
    pub vram_avg_gib: f64,
    pub verified_tokens: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use miraset_core::crypto::KeyPair;

    #[test]
    fn test_epoch_creation() {
        let start = Utc::now();
        let epoch = Epoch::new(0, start);
        
        assert_eq!(epoch.id, 0);
        assert_eq!(epoch.status, EpochStatus::Active);
        assert_eq!((epoch.end_time - epoch.start_time).num_seconds(), EPOCH_DURATION_SECONDS);
    }

    #[test]
    fn test_worker_stats_uptime() {
        let kp = KeyPair::generate();
        let worker_id = [1u8; 32];
        let mut stats = WorkerEpochStats::new(worker_id, kp.address());

        stats.record_heartbeat(true);
        stats.record_heartbeat(true);
        stats.record_heartbeat(false);

        assert_eq!(stats.uptime_score, 2.0 / 3.0);
    }

    #[test]
    fn test_capacity_score_below_min_uptime() {
        let kp = KeyPair::generate();
        let worker_id = [1u8; 32];
        let mut stats = WorkerEpochStats::new(worker_id, kp.address());

        // Only 80% uptime (below U_MIN of 90%)
        for _ in 0..8 {
            stats.record_heartbeat(true);
        }
        for _ in 0..2 {
            stats.record_heartbeat(false);
        }

        stats.vram_avail_avg_gib = 24.0;

        assert_eq!(stats.capacity_score(), 0.0);
    }

    #[test]
    fn test_capacity_score_above_min_uptime() {
        let kp = KeyPair::generate();
        let worker_id = [1u8; 32];
        let mut stats = WorkerEpochStats::new(worker_id, kp.address());

        // 100% uptime
        for _ in 0..10 {
            stats.record_heartbeat(true);
        }

        stats.vram_avail_avg_gib = 24.0;

        let score = stats.capacity_score();
        assert!(score > 0.0);
        // U=1.0, V=24 => score = 1.0^2 * 24^1 = 24
        assert!((score - 24.0).abs() < 0.001);
    }

    #[test]
    fn test_epoch_rewards_distribution() {
        let start = Utc::now();
        let mut epoch = Epoch::new(0, start);

        let kp1 = KeyPair::generate();
        let kp2 = KeyPair::generate();
        let worker1 = [1u8; 32];
        let worker2 = [2u8; 32];

        let mut stats1 = WorkerEpochStats::new(worker1, kp1.address());
        let mut stats2 = WorkerEpochStats::new(worker2, kp2.address());

        // Worker 1: 100% uptime, 24 GiB VRAM, 1000 tokens
        for _ in 0..10 {
            stats1.record_heartbeat(true);
        }
        stats1.vram_avail_avg_gib = 24.0;
        stats1.verified_tokens = 1000;

        // Worker 2: 95% uptime, 16 GiB VRAM, 500 tokens
        for _ in 0..19 {
            stats2.record_heartbeat(true);
        }
        stats2.record_heartbeat(false);
        stats2.vram_avail_avg_gib = 16.0;
        stats2.verified_tokens = 500;

        epoch.worker_stats.insert(worker1, stats1);
        epoch.worker_stats.insert(worker2, stats2);
        epoch.total_verified_tokens = 1500;

        let rewards = epoch.calculate_rewards();

        assert_eq!(rewards.epoch_id, 0);
        assert!(rewards.worker_rewards.contains_key(&worker1));
        assert!(rewards.worker_rewards.contains_key(&worker2));

        let reward1 = &rewards.worker_rewards[&worker1];
        let reward2 = &rewards.worker_rewards[&worker2];

        // Worker 1 should get more rewards (better uptime, more VRAM, more tokens)
        assert!(reward1.total_reward > reward2.total_reward);
    }
}
