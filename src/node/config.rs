use ed25519_dalek::SigningKey;
use hotstuff_rs::replica::Configuration;
use hotstuff_rs::types::data_types::{BufferSize, ChainID, EpochLength};
use hotstuff_rs::types::signer::{LocalKeySigner, ValidatorSigner};
use std::sync::Arc;
use std::time::Duration;

use crate::consensus::SafetyHalt;
use crate::observability::DEFAULT_READY_MAX_COMMIT_AGE;

/// Lower bound that leaves room for propose/vote round trips even on a local devnet.
const MIN_MAX_VIEW_TIME: Duration = Duration::from_millis(100);
/// View timeout; 2 s leaves the empty-block heartbeat (1 s) a full second of margin
/// (owner decision 2026-11-13).
pub const DEFAULT_MAX_VIEW_TIME: Duration = Duration::from_millis(2000);
/// Empty-block heartbeat: at most one empty block per second (owner decision 2026-11-13).
pub const DEFAULT_EMPTY_BLOCK_INTERVAL: Duration = Duration::from_millis(1000);

/// Pacemaker epoch length (views); see [`NodeConfig::epoch_length`].
pub const DEFAULT_EPOCH_LENGTH: u32 = crate::core::types::DEFAULT_CONSENSUS_EPOCH_LENGTH;

/// Configuration options for a MIRASET L1 node.
///
/// Per protocol ADR-0013 (Accepted) and ADR-0016 (Proposed), HotStuff consensus
/// in `miraset-chain` is isolated to experimental/devnet operation until a
/// superseding settlement decision is explicitly accepted.
#[derive(Clone)]
pub struct NodeConfig {
    /// Validator signing authority (key custody D1). The node holds no `SigningKey` itself.
    pub signer: Arc<dyn ValidatorSigner>,
    pub chain_id: u64,
    /// Must be `true` while ADR-0016 remains Proposed; prevents implicit production activation.
    pub experimental_mode: bool,
    pub block_sync_request_limit: u32,
    pub block_sync_server_advertise_time: Duration,
    pub block_sync_response_timeout: Duration,
    pub block_sync_blacklist_expiry_time: Duration,
    pub block_sync_trigger_min_view_difference: u64,
    pub block_sync_trigger_timeout: Duration,
    pub progress_msg_buffer_capacity: u64,
    /// Pacemaker epoch length in views. A consensus parameter: every validator must use the
    /// same value. Views inside an epoch get absolute deadlines (epoch start + k ×
    /// `max_view_time`), so a failed view waits out the slack earlier fast views saved, up to
    /// `epoch_length × max_view_time`. 1 bounds that wait to about one view plus a timeout
    /// certificate (owner decision 2026-11-12, TODO 8.5.2 F2; `tests/view_timing.rs`).
    /// The validator binary takes it from genesis (`protocol.consensus_epoch_length`), so the
    /// genesis commitment covers it and validators of one chain cannot disagree on it.
    pub epoch_length: u32,
    pub max_view_time: Duration,
    /// Without work, a leader proposes an empty block only once this long after its parent
    /// (owner decision 2026-11-13; `src/app/chain_app/heartbeat.rs`). Zero proposes in every
    /// view. The effective value never exceeds half of `max_view_time`, so a waiting leader
    /// always proposes before its replicas time out ([`Self::effective_empty_block_interval`]).
    pub empty_block_interval: Duration,
    /// Strict block-rate limit: a leader proposes any block (empty or not) no earlier than this
    /// after its parent, so the chain makes at most one block per interval. Zero (default)
    /// leaves only the empty-block heartbeat. Capped at half of `max_view_time` like the
    /// heartbeat ([`Self::effective_min_block_interval`]). Throughput is then bounded by
    /// `MAX_BLOCK_TXS` + receipts per interval.
    pub min_block_interval: Duration,
    pub log_events: bool,
    /// When set, startup fails unless the store's recorded genesis commitment matches.
    pub expected_genesis_hash: Option<[u8; 32]>,
    /// Shared consensus safety halt (TODO 8.3.2). Pass the same handle to the
    /// `GuardedNetwork` and chain store so `/v1/health` reports their faults.
    pub safety_halt: SafetyHalt,
    /// `/readyz` reports not ready when no block was committed within this time (TODO 8.4.4).
    pub ready_max_commit_age: Duration,
}

impl std::fmt::Debug for NodeConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NodeConfig")
            .field(
                "validator",
                &hex::encode(self.signer.public_key().to_bytes()),
            )
            .field("chain_id", &self.chain_id)
            .field("experimental_mode", &self.experimental_mode)
            .field("max_view_time", &self.max_view_time)
            .field("empty_block_interval", &self.empty_block_interval)
            .field("min_block_interval", &self.min_block_interval)
            .finish_non_exhaustive()
    }
}

impl NodeConfig {
    /// Devnet configuration with an unguarded in-process signer (tests and the in-memory demo).
    pub fn devnet(signing_key: SigningKey) -> Self {
        Self::devnet_with_signer(Arc::new(LocalKeySigner::new(signing_key)))
    }

    /// Devnet configuration optimized for local testing and fast block times.
    pub fn devnet_with_signer(signer: Arc<dyn ValidatorSigner>) -> Self {
        Self {
            signer,
            chain_id: 1337,
            experimental_mode: true,
            block_sync_request_limit: 50,
            block_sync_server_advertise_time: Duration::from_millis(500),
            block_sync_response_timeout: Duration::from_millis(1500),
            block_sync_blacklist_expiry_time: Duration::from_secs(10),
            block_sync_trigger_min_view_difference: 5,
            block_sync_trigger_timeout: Duration::from_secs(3),
            progress_msg_buffer_capacity: 1024 * 1024, // 1MB buffer
            epoch_length: DEFAULT_EPOCH_LENGTH,
            max_view_time: DEFAULT_MAX_VIEW_TIME,
            empty_block_interval: DEFAULT_EMPTY_BLOCK_INTERVAL,
            min_block_interval: Duration::ZERO,
            log_events: false,
            expected_genesis_hash: None,
            safety_halt: SafetyHalt::new(),
            ready_max_commit_age: DEFAULT_READY_MAX_COMMIT_AGE,
        }
    }

    /// The heartbeat interval actually used: `empty_block_interval`, at most half the view time.
    pub fn effective_empty_block_interval(&self) -> Duration {
        self.empty_block_interval.min(self.max_view_time / 2)
    }

    /// The strict block-rate limit actually used: at most half the view time.
    pub fn effective_min_block_interval(&self) -> Duration {
        self.min_block_interval.min(self.max_view_time / 2)
    }

    /// Validates that the node configuration respects the current protocol gate boundaries.
    pub fn validate(&self) -> Result<(), &'static str> {
        if !self.experimental_mode {
            return Err(
                "production settlement mode is disabled: ADR-0016 is Proposed and ADR-0013 remains Accepted",
            );
        }
        if self.chain_id == 0 {
            return Err("chain_id must be non-zero");
        }
        if self.epoch_length == 0 {
            return Err("epoch_length must be non-zero");
        }
        // A zero limit makes block sync serve and fetch nothing (vendored patch #12).
        if self.block_sync_request_limit == 0 {
            return Err("block_sync_request_limit must be non-zero");
        }
        if self.progress_msg_buffer_capacity == 0 {
            return Err("progress_msg_buffer_capacity must be non-zero");
        }
        // A zero view time makes every view time out immediately, stalling liveness.
        // (Sub-second values are exact since the vendored pacemaker patch; see
        // lib/hotstuff_rs/MIRASET_PATCHES.md.)
        if self.max_view_time < MIN_MAX_VIEW_TIME {
            return Err("max_view_time must be at least 100ms");
        }
        // Finality needs several views, so a shorter window would flap between ready and not.
        if self.ready_max_commit_age < self.max_view_time {
            return Err("ready_max_commit_age must be at least max_view_time");
        }
        Ok(())
    }

    /// Converts this config into a HotStuff-rs replica `Configuration`.
    pub fn to_hotstuff_config(&self) -> Configuration {
        self.validate()
            .expect("invalid NodeConfig passed to to_hotstuff_config");
        Configuration::builder()
            .signer(Arc::clone(&self.signer))
            .chain_id(ChainID::new(self.chain_id))
            .block_sync_request_limit(self.block_sync_request_limit)
            .block_sync_server_advertise_time(self.block_sync_server_advertise_time)
            .block_sync_response_timeout(self.block_sync_response_timeout)
            .block_sync_blacklist_expiry_time(self.block_sync_blacklist_expiry_time)
            .block_sync_trigger_min_view_difference(self.block_sync_trigger_min_view_difference)
            .block_sync_trigger_timeout(self.block_sync_trigger_timeout)
            .progress_msg_buffer_capacity(BufferSize::new(self.progress_msg_buffer_capacity))
            .epoch_length(EpochLength::new(self.epoch_length))
            .max_view_time(self.max_view_time)
            .log_events(self.log_events)
            .build()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_config_enforces_experimental_boundary() {
        let key = SigningKey::from_bytes(&[1u8; 32]);
        let mut cfg = NodeConfig::devnet(key);
        assert!(cfg.validate().is_ok());

        cfg.experimental_mode = false;
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn test_node_config_rejects_zero_block_sync_limit() {
        let key = SigningKey::from_bytes(&[1u8; 32]);
        let base = NodeConfig::devnet(key);
        let zero = NodeConfig {
            block_sync_request_limit: 0,
            ..base.clone()
        };
        assert!(zero.validate().is_err());
        assert!(base.validate().is_ok());
    }

    #[test]
    fn test_empty_block_interval_never_exceeds_half_the_view_time() {
        let base = NodeConfig::devnet(SigningKey::from_bytes(&[1u8; 32]));
        assert_eq!(
            base.effective_empty_block_interval(),
            DEFAULT_EMPTY_BLOCK_INTERVAL
        );
        assert!(base.effective_empty_block_interval() * 2 <= base.max_view_time);
        let short_views = NodeConfig {
            max_view_time: Duration::from_millis(500),
            ..base.clone()
        };
        assert_eq!(
            short_views.effective_empty_block_interval(),
            Duration::from_millis(250)
        );
        let off = NodeConfig {
            empty_block_interval: Duration::ZERO,
            ..base
        };
        assert_eq!(off.effective_empty_block_interval(), Duration::ZERO);
    }

    #[test]
    fn test_node_config_bounds_max_view_time() {
        let key = SigningKey::from_bytes(&[1u8; 32]);
        let base = NodeConfig::devnet(key);

        for invalid in [Duration::ZERO, Duration::from_millis(99)] {
            let cfg = NodeConfig {
                max_view_time: invalid,
                ..base.clone()
            };
            assert!(cfg.validate().is_err(), "{invalid:?} should be rejected");
        }

        for valid in [Duration::from_millis(350), Duration::from_millis(1500)] {
            let cfg = NodeConfig {
                max_view_time: valid,
                ..base.clone()
            };
            assert!(cfg.validate().is_ok(), "{valid:?} should be accepted");
        }
    }

    #[test]
    fn test_node_config_rejects_readiness_window_shorter_than_a_view() {
        let base = NodeConfig::devnet(SigningKey::from_bytes(&[1u8; 32]));
        let too_short = NodeConfig {
            ready_max_commit_age: base.max_view_time - Duration::from_millis(1),
            ..base.clone()
        };
        assert_eq!(
            too_short.validate(),
            Err("ready_max_commit_age must be at least max_view_time")
        );
        let equal = NodeConfig {
            ready_max_commit_age: base.max_view_time,
            ..base
        };
        assert!(equal.validate().is_ok());
    }
}
