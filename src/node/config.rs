use ed25519_dalek::SigningKey;
use hotstuff_rs::replica::Configuration;
use hotstuff_rs::types::data_types::{BufferSize, ChainID, EpochLength};
use std::time::Duration;

use crate::consensus::SafetyHalt;

/// Lower bound that leaves room for propose/vote round trips even on a local devnet.
const MIN_MAX_VIEW_TIME: Duration = Duration::from_millis(100);

/// Configuration options for a MIRASET L1 node.
///
/// Per protocol ADR-0013 (Accepted) and ADR-0016 (Proposed), HotStuff consensus
/// in `miraset-chain` is isolated to experimental/devnet operation until a
/// superseding settlement decision is explicitly accepted.
#[derive(Clone, Debug)]
pub struct NodeConfig {
    pub signing_key: SigningKey,
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
    pub epoch_length: u32,
    pub max_view_time: Duration,
    pub log_events: bool,
    /// When set, startup fails unless the store's recorded genesis commitment matches.
    pub expected_genesis_hash: Option<[u8; 32]>,
    /// Shared consensus safety halt (TODO 8.3.2). Pass the same handle to the
    /// `GuardedNetwork` and chain store so `/health` reports their faults.
    pub safety_halt: SafetyHalt,
}

impl NodeConfig {
    /// Devnet configuration optimized for local testing and fast block times.
    pub fn devnet(signing_key: SigningKey) -> Self {
        Self {
            signing_key,
            chain_id: 1337,
            experimental_mode: true,
            block_sync_request_limit: 50,
            block_sync_server_advertise_time: Duration::from_millis(500),
            block_sync_response_timeout: Duration::from_millis(1500),
            block_sync_blacklist_expiry_time: Duration::from_secs(10),
            block_sync_trigger_min_view_difference: 5,
            block_sync_trigger_timeout: Duration::from_secs(3),
            progress_msg_buffer_capacity: 1024 * 1024, // 1MB buffer
            epoch_length: 10,
            max_view_time: Duration::from_millis(1000),
            log_events: false,
            expected_genesis_hash: None,
            safety_halt: SafetyHalt::new(),
        }
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
        if self.progress_msg_buffer_capacity == 0 {
            return Err("progress_msg_buffer_capacity must be non-zero");
        }
        // A zero view time makes every view time out immediately, stalling liveness.
        // (Sub-second values are exact since the vendored pacemaker patch; see
        // lib/hotstuff_rs/MIRASET_PATCHES.md.)
        if self.max_view_time < MIN_MAX_VIEW_TIME {
            return Err("max_view_time must be at least 100ms");
        }
        Ok(())
    }

    /// Converts this config into a HotStuff-rs replica `Configuration`.
    pub fn to_hotstuff_config(&self) -> Configuration {
        self.validate()
            .expect("invalid NodeConfig passed to to_hotstuff_config");
        Configuration::builder()
            .me(self.signing_key.clone())
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
}
