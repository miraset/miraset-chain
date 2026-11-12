//! Block sync server response bounds (TODO 8.5.2 finding F4; vendored patch #12).
//!
//! Any peer that can reach the transport can send a `BlockSyncRequest` (transport identity is
//! still self-declared, TODO 8.4.1). Upstream capped the response with
//! `max(request.limit, config.request_limit)` and pre-allocated `limit` blocks: `limit = 0`
//! returned the whole chain, a large limit returned everything up to the tip, and
//! `limit = u32::MAX` asked the allocator for gigabytes on the `hs-sync-server` thread (an abort,
//! or a panic that halts the validator). The server must answer with at most its own
//! `block_sync_request_limit` blocks, whatever the request says. The in-memory hub does no
//! membership check, so this exercises the server, not the TCP transport's peer filter.

use ed25519_dalek::SigningKey;
use hotstuff_rs::block_sync::messages::{BlockSyncMessage, BlockSyncRequest};
use hotstuff_rs::networking::messages::Message;
use hotstuff_rs::networking::network::Network;
use hotstuff_rs::types::data_types::{BlockHeight, ChainID};
use miraset_chain::mempool::Mempool;
use miraset_chain::network::{ChannelNetwork, ChannelNetworkHub};
use miraset_chain::node::{MirasetNode, NodeConfig, init_genesis};
use miraset_chain::storage::MemKVStore;
use std::thread;
use std::time::{Duration, Instant};

const SERVER_LIMIT: u32 = 5;
const CHAIN_HEIGHT: u64 = 3 * SERVER_LIMIT as u64;
const VIEW_TIME: Duration = Duration::from_millis(300);
const WAIT: Duration = Duration::from_secs(30);

/// A peer outside the validator set, talking to the hub directly.
struct Outsider {
    network: ChannelNetwork,
}

impl Outsider {
    /// Send one request and return the number of blocks in the server's response.
    fn request(&mut self, server: &SigningKey, chain_id: u64, limit: u32) -> usize {
        self.network.send(
            server.verifying_key(),
            Message::from(BlockSyncRequest {
                chain_id: ChainID::new(chain_id),
                start_height: BlockHeight::new(0),
                limit,
            }),
        );
        let deadline = Instant::now() + WAIT;
        while Instant::now() < deadline {
            if let Some((_, Message::BlockSyncMessage(BlockSyncMessage::BlockSyncResponse(r)))) =
                self.network.recv()
            {
                return r.blocks.len();
            }
        }
        panic!("no block sync response for limit {limit}");
    }
}

#[test]
fn test_sync_server_caps_responses_at_its_own_limit() {
    let key = SigningKey::from_bytes(&[61u8; 32]);
    let kv = MemKVStore::new();
    init_genesis(kv.clone(), &[(key.verifying_key(), 1)], &[]);
    let hub = ChannelNetworkHub::new();
    let config = NodeConfig {
        max_view_time: VIEW_TIME,
        block_sync_request_limit: SERVER_LIMIT,
        ..NodeConfig::devnet(key.clone())
    };
    let chain_id = config.chain_id;
    let node = MirasetNode::start(
        config,
        kv,
        ChannelNetwork::new(key.verifying_key(), hub.clone()),
        Mempool::new(),
    );

    let deadline = Instant::now() + WAIT;
    while node.highest_committed_height().unwrap_or(0) < CHAIN_HEIGHT {
        assert!(Instant::now() < deadline, "single validator did not commit");
        thread::sleep(Duration::from_millis(50));
    }

    let outsider_key = SigningKey::from_bytes(&[62u8; 32]);
    let mut outsider = Outsider {
        network: ChannelNetwork::new(outsider_key.verifying_key(), hub),
    };
    assert_eq!(
        outsider.request(&key, chain_id, 1),
        1,
        "smaller limits are honoured"
    );
    assert_eq!(
        outsider.request(&key, chain_id, 0),
        0,
        "limit 0 means no blocks"
    );
    for limit in [SERVER_LIMIT * 10, u32::MAX] {
        assert_eq!(
            outsider.request(&key, chain_id, limit),
            SERVER_LIMIT as usize,
            "request limit {limit} is capped at the server's own limit"
        );
    }

    let height = node.highest_committed_height().unwrap_or(0);
    let deadline = Instant::now() + WAIT;
    while node.highest_committed_height().unwrap_or(0) <= height {
        assert!(
            Instant::now() < deadline,
            "validator stopped after the requests"
        );
        thread::sleep(Duration::from_millis(50));
    }
    assert!(!node.safety_halt().is_halted());
}
