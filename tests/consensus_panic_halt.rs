//! A panic on a `hotstuff_rs` thread halts the node visibly (TODO 8.3.2). Own test binary:
//! the panic hook is process-wide.
//!
//! The chain store here has *no* halt handle, so only the panic hook can report the fault.

use ed25519_dalek::SigningKey;
use miraset_chain::consensus::SafetyHalt;
use miraset_chain::mempool::Mempool;
use miraset_chain::network::{ChannelNetwork, ChannelNetworkHub};
use miraset_chain::node::{MirasetNode, NodeConfig, init_genesis};
use miraset_chain::storage::RedbKVStore;
use std::thread;
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(20);

#[test]
fn test_consensus_thread_panic_triggers_safety_halt() {
    let keypair = SigningKey::from_bytes(&[61u8; 32]);
    let vk = keypair.verifying_key();
    let dir = tempfile::TempDir::new().unwrap();
    let store = RedbKVStore::open(dir.path().join("chain.redb")).unwrap();
    init_genesis(store.clone(), &[(vk, 100)], &[("alice", 1_000)]);

    let halt = SafetyHalt::new();
    halt.install_consensus_panic_hook();
    let mut config = NodeConfig::devnet(keypair);
    config.safety_halt = halt.clone();
    let network = ChannelNetwork::new(vk, ChannelNetworkHub::new());
    let node = MirasetNode::start(config, store.clone(), network, Mempool::new());

    let deadline = Instant::now() + TIMEOUT;
    while node.highest_committed_height().is_none() {
        assert!(Instant::now() < deadline, "no commit before fault");
        thread::sleep(Duration::from_millis(25));
    }
    assert!(!halt.is_halted());

    store.set_fail_next_write(true);
    while !halt.is_halted() {
        assert!(Instant::now() < deadline, "panic did not halt the node");
        thread::sleep(Duration::from_millis(25));
    }
    let reason = halt.reason().unwrap();
    assert!(
        reason.starts_with("consensus thread hs-algorithm panicked"),
        "{reason}"
    );

    // The algorithm thread is dead; `Replica::drop` would re-panic joining it.
    std::mem::forget(node);
}
