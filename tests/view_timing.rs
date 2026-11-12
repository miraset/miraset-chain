//! Pacemaker view timing with one crashed validator (TODO 8.5.2 lead F2; 8.5.3 finding 10).
//!
//! Five equal-power validators on the in-memory hub; validator 4 never starts, so every fifth
//! view has a dead leader. Inside a pacemaker epoch, views have absolute deadlines (epoch
//! start plus k × `max_view_time`), so the dead leader's view waits out the slack the fast views
//! before it saved. Measured 2026-11-12 with 1 s views: longest commit gap 5.0 s with `epoch_length`
//! 10 (the old default), 2.05 s with 1. Owner decision 2026-11-12: default 1.

use ed25519_dalek::SigningKey;
use miraset_chain::mempool::Mempool;
use miraset_chain::network::{ChannelNetwork, ChannelNetworkHub};
use miraset_chain::node::{DEFAULT_EPOCH_LENGTH, MirasetNode, NodeConfig, init_genesis};
use miraset_chain::storage::MemKVStore;
use std::thread;
use std::time::{Duration, Instant};

const VALIDATORS: usize = 5;
const CRASHED: usize = 4;
const VIEW_TIME: Duration = Duration::from_millis(1_000);
const WARMUP: Duration = Duration::from_secs(3);
const OBSERVE: Duration = Duration::from_secs(25);
const SAMPLE: Duration = Duration::from_millis(20);
const OLD_EPOCH_LENGTH: u32 = 10;
const SEED: u8 = 10;
/// Bound for the default: one dead view, a timeout certificate, and the views the 3-chain
/// commit rule needs to recover.
const MAX_GAP_DEFAULT: Duration = Duration::from_millis(3_500);

/// Longest time validator 0's committed height stood still during the observation window.
fn longest_commit_gap(epoch_length: u32, seed: u8) -> Duration {
    let keys: Vec<SigningKey> = (0..VALIDATORS as u8)
        .map(|i| SigningKey::from_bytes(&[seed + i; 32]))
        .collect();
    let validators: Vec<_> = keys.iter().map(|k| (k.verifying_key(), 1)).collect();
    let hub = ChannelNetworkHub::new();
    let nodes: Vec<_> = keys
        .iter()
        .take(CRASHED)
        .map(|key| {
            let kv = MemKVStore::new();
            init_genesis(kv.clone(), &validators, &[]);
            let config = NodeConfig {
                max_view_time: VIEW_TIME,
                epoch_length,
                ..NodeConfig::devnet(key.clone())
            };
            let network = ChannelNetwork::new(key.verifying_key(), hub.clone());
            MirasetNode::start(config, kv, network, Mempool::new())
        })
        .collect();

    thread::sleep(WARMUP);
    let height = || nodes[0].highest_committed_height().unwrap_or(0);
    let start = Instant::now();
    let mut last_height = height();
    let mut last_change = start;
    let mut longest = Duration::ZERO;
    while start.elapsed() < OBSERVE {
        thread::sleep(SAMPLE);
        let now_height = height();
        let now = Instant::now();
        if now_height != last_height {
            longest = longest.max(now - last_change);
            last_height = now_height;
            last_change = now;
        }
    }
    longest = longest.max(Instant::now() - last_change);
    assert!(last_height > 0, "nothing committed");
    longest
}

#[test]
fn test_default_epoch_length_bounds_commit_gaps_with_a_crashed_leader() {
    // Same keys for both runs, so the dead leader sits at the same view positions.
    let default_gap = longest_commit_gap(DEFAULT_EPOCH_LENGTH, SEED);
    let old_gap = longest_commit_gap(OLD_EPOCH_LENGTH, SEED);
    eprintln!(
        "max_view_time={VIEW_TIME:?}: longest commit gap {default_gap:?} with epoch_length \
         {DEFAULT_EPOCH_LENGTH}, {old_gap:?} with {OLD_EPOCH_LENGTH}"
    );
    assert!(
        default_gap < MAX_GAP_DEFAULT,
        "default epoch length stalled {default_gap:?}"
    );
    assert!(
        old_gap > MAX_GAP_DEFAULT,
        "epoch length {OLD_EPOCH_LENGTH} no longer shows the accumulated-slack stall ({old_gap:?})"
    );
}
