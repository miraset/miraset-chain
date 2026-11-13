//! Block sync with blocks near `MAX_BLOCK_BYTES` over the real TCP transport (owner decision
//! 2026-11-13: 8 MiB blocks; vendored hotstuff_rs patch #15).
//!
//! A sync response carries up to `block_sync_request_limit` (50) blocks. Without a byte cap, a
//! few 7 MiB blocks make a response larger than a transport frame (16 MiB): the server's send is
//! dropped and the lagging validator never catches up. Five validators, one starts only after
//! the others committed more than a frame of block data, and must catch up.

use ed25519_dalek::SigningKey;
use miraset_chain::core::types::Transaction;
use miraset_chain::mempool::Mempool;
use miraset_chain::network::{NetworkRuntime, TcpNetwork, TransportIdentity};
use miraset_chain::node::{MirasetNode, NodeConfig, init_genesis};
use miraset_chain::storage::RedbKVStore;
use std::collections::HashMap;
use std::net::{SocketAddr, TcpListener};
use std::thread;
use std::time::{Duration, Instant};

const VALIDATORS: usize = 5;
const LATE: usize = 4;
const SENDERS: u8 = 25;
const PER_SENDER: u64 = 200;
/// 5 000 transfers × ≈ 4.3 KB ≈ 21 MB of blocks: three blocks, more than one frame.
const TX_PAYLOAD: usize = 4 * 1024;
const BALANCE: u64 = 1_000_000;
const TRANSPORT_GENESIS: [u8; 32] = [0u8; 32];

type TcpNode = MirasetNode<RedbKVStore, TcpNetwork>;

fn free_addr() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    listener.local_addr().expect("addr")
}

fn wait_until(timeout: Duration, what: &str, mut done: impl FnMut() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(start.elapsed() < timeout, "timed out: {what}");
        thread::sleep(Duration::from_millis(100));
    }
    eprintln!("{what}: {:?}", start.elapsed());
}

fn start(
    key: &SigningKey,
    addr: SocketAddr,
    peers: &HashMap<ed25519_dalek::VerifyingKey, String>,
    store: RedbKVStore,
) -> (TcpNode, NetworkRuntime) {
    let config = NodeConfig::devnet(key.clone());
    let identity = TransportIdentity::for_key(key.clone(), config.chain_id, TRANSPORT_GENESIS)
        .expect("transport identity");
    let (network, runtime) =
        TcpNetwork::bind_with_hosts(identity, addr, peers.clone()).expect("bind transport");
    let node = MirasetNode::try_start(config, store, network, Mempool::new()).expect("start");
    (node, runtime)
}

#[test]
fn test_late_validator_syncs_blocks_larger_than_a_frame() {
    let dir = tempfile::tempdir().expect("tempdir");
    let keys: Vec<SigningKey> = (0..VALIDATORS)
        .map(|i| SigningKey::from_bytes(&[150 + i as u8; 32]))
        .collect();
    let addrs: Vec<SocketAddr> = (0..VALIDATORS).map(|_| free_addr()).collect();
    let peers: HashMap<_, _> = keys
        .iter()
        .zip(&addrs)
        .map(|(k, a)| (k.verifying_key(), a.to_string()))
        .collect();
    let validator_set: Vec<_> = keys.iter().map(|k| (k.verifying_key(), 100)).collect();
    let senders: Vec<SigningKey> = (0..SENDERS)
        .map(|i| SigningKey::from_bytes(&[180 + i; 32]))
        .collect();
    let sender_addrs: Vec<String> = senders
        .iter()
        .map(|k| hex::encode(k.verifying_key().to_bytes()))
        .collect();
    let mut accounts: Vec<(&str, u64)> =
        sender_addrs.iter().map(|a| (a.as_str(), BALANCE)).collect();
    accounts.push(("bob", 0));
    let stores: Vec<RedbKVStore> = (0..VALIDATORS)
        .map(|i| {
            let store = RedbKVStore::open(dir.path().join(format!("node{i}.redb"))).expect("redb");
            init_genesis(store.clone(), &validator_set, &accounts);
            store
        })
        .collect();

    let mut running: Vec<(TcpNode, NetworkRuntime)> = (0..LATE)
        .map(|i| start(&keys[i], addrs[i], &peers, stores[i].clone()))
        .collect();

    // No mempool gossip: every early validator gets every transfer, so any leader can include it.
    for nonce in 0..PER_SENDER {
        for (sender, addr) in senders.iter().zip(&sender_addrs) {
            let mut tx = Transaction::new(addr, nonce, "bob", 1, 1);
            tx.payload = vec![7; TX_PAYLOAD];
            tx.sign_with(sender);
            for (node, _) in &running {
                node.submit_transaction(tx.clone()).expect("admitted");
            }
        }
    }
    let total = u64::from(SENDERS) * PER_SENDER;
    let bob = |node: &TcpNode| node.get_account("bob").map_or(0, |a| a.balance);
    wait_until(Duration::from_secs(120), "backlog committed", || {
        bob(&running[0].0) == total
    });
    let target = running[0].0.highest_committed_height().expect("height");

    let late = start(&keys[LATE], addrs[LATE], &peers, stores[LATE].clone());
    running.push(late);
    wait_until(Duration::from_secs(120), "late validator caught up", || {
        running[LATE].0.highest_committed_height().unwrap_or(0) >= target
    });
    assert_eq!(bob(&running[LATE].0), total);
    assert_eq!(
        running[LATE].0.committed_block_at_height(target),
        running[0].0.committed_block_at_height(target),
        "same block at height {target}"
    );
}
