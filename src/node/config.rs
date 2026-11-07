use ed25519_dalek::SigningKey;
use hotstuff_rs::replica::Configuration;
use hotstuff_rs::types::data_types::{BufferSize, ChainID, EpochLength};
use std::time::Duration;

/// Configuration options for a MIRASET L1 node.
#[derive(Clone)]
pub struct NodeConfig {
    pub signing_key: SigningKey,
    pub chain_id: u64,
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
}

impl NodeConfig {
    /// Devnet configuration optimized for local testing and fast block times.
    pub fn devnet(signing_key: SigningKey) -> Self {
        Self {
            signing_key,
            chain_id: 1337,
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
        }
    }

    /// Converts this config into a HotStuff-rs replica `Configuration`.
    pub fn to_hotstuff_config(&self) -> Configuration {
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
