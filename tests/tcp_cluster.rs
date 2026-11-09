//! Multi-replica consensus over the real TCP transport and durable Redb storage (PRD R06).
//!
//! Equal-power validators (f = 1) on localhost: commit, crash one validator, restart it from its
//! own database, and verify catch-up. Safety is checked by comparing the committed block hash at
//! every height on every running node.
//!
//! Liveness note: `hotstuff_rs` v0.4.0 rotates leaders strictly by view, sends votes to the leader
//! of `view + 1`, and commits only on a 3-chain of *consecutive-view* QCs. A commit therefore needs
//! four consecutive live leaders. With n = 4 and one validator down that never happens, so the
//! 4-validator scenario only expects progress to resume after restart; crash-tolerant liveness is
//! exercised with n = 5 (four consecutive live leaders every five views).

use ed25519_dalek::SigningKey;
use hotstuff_rs::types::data_types::{BlockHeight, CryptoHash};
use miraset_chain::core::types::Transaction;
use miraset_chain::mempool::Mempool;
use miraset_chain::network::{NetworkRuntime, TcpNetwork};
use miraset_chain::node::{MirasetNode, NodeConfig, init_genesis};
use miraset_chain::storage::RedbKVStore;
use std::collections::HashMap;
use std::net::{SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

const CONSENSUS_TIMEOUT: Duration = Duration::from_secs(30);
const POLL_INTERVAL: Duration = Duration::from_millis(50);
const INITIAL_BALANCE: u64 = 100_000;
const TRANSFER: u64 = 1_000;
const FEE: u64 = 10;

type TcpNode = MirasetNode<RedbKVStore, TcpNetwork>;

struct RunningNode {
    node: TcpNode,
    // Field order matters: drop the replica before tearing down its transport.
    _runtime: NetworkRuntime,
}

struct Cluster {
    keys: Vec<SigningKey>,
    addrs: Vec<SocketAddr>,
    db_paths: Vec<PathBuf>,
    nodes: Vec<Option<RunningNode>>,
    alice_addr: String,
    _dir: tempfile::TempDir,
}

fn free_addr() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    listener.local_addr().expect("local addr")
}

fn wait_until(timeout: Duration, mut condition: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if condition() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(POLL_INTERVAL);
    }
}

impl Cluster {
    fn new(validators: usize, alice_addr: &str) -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let keys: Vec<SigningKey> = (0..validators)
            .map(|i| SigningKey::from_bytes(&[60 + i as u8; 32]))
            .collect();
        let addrs: Vec<SocketAddr> = (0..validators).map(|_| free_addr()).collect();
        let validator_set: Vec<_> = keys.iter().map(|k| (k.verifying_key(), 100)).collect();
        let db_paths: Vec<PathBuf> = (0..validators)
            .map(|i| dir.path().join(format!("node{i}.redb")))
            .collect();

        for path in &db_paths {
            let store = RedbKVStore::open(path).expect("open redb");
            init_genesis(
                store,
                &validator_set,
                &[(alice_addr, INITIAL_BALANCE), ("bob", 0)],
            );
        }

        let mut cluster = Self {
            keys,
            addrs,
            db_paths,
            nodes: (0..validators).map(|_| None).collect(),
            alice_addr: alice_addr.to_string(),
            _dir: dir,
        };
        for i in 0..validators {
            cluster.start(i);
        }
        cluster
    }

    fn start(&mut self, i: usize) {
        let key = self.keys[i].clone();
        let peers: HashMap<_, _> = self
            .keys
            .iter()
            .zip(&self.addrs)
            .map(|(k, a)| (k.verifying_key(), a.to_string()))
            .collect();
        let (network, runtime) =
            TcpNetwork::bind_with_hosts(key.verifying_key(), self.addrs[i], peers)
                .expect("bind validator transport");
        let store = open_existing(&self.db_paths[i]);
        let node = MirasetNode::try_start(NodeConfig::devnet(key), store, network, Mempool::new())
            .expect("node recovery/start");
        self.nodes[i] = Some(RunningNode {
            node,
            _runtime: runtime,
        });
    }

    fn crash(&mut self, i: usize) {
        self.nodes[i] = None;
    }

    fn node(&self, i: usize) -> &TcpNode {
        &self.nodes[i].as_ref().expect("node running").node
    }

    fn running(&self) -> impl Iterator<Item = &TcpNode> {
        self.nodes.iter().flatten().map(|n| &n.node)
    }

    fn transfer(&self, via: usize, nonce: u64, signer: &SigningKey) {
        let mut tx = Transaction::new(&self.alice_addr, nonce, "bob", TRANSFER, FEE);
        tx.sign_with(signer);
        assert!(self.node(via).submit_transaction(tx).is_ok());
    }

    fn wait_for_bob(&self, nodes: &[usize], expected: u64) -> bool {
        wait_until(CONSENSUS_TIMEOUT, || {
            nodes
                .iter()
                .all(|&i| self.node(i).get_account("bob").map(|a| a.balance) == Some(expected))
        })
    }

    /// Assert that all running nodes agree on the committed block at every shared height.
    fn assert_no_conflicting_finality(&self) -> u64 {
        let chains: Vec<Vec<CryptoHash>> = self.running().map(committed_chain).collect();
        let shared = chains.iter().map(Vec::len).min().unwrap_or(0);
        assert!(shared > 0, "no committed blocks to compare");
        for height in 0..shared {
            let first = chains[0][height];
            assert!(
                chains.iter().all(|c| c[height] == first),
                "conflicting committed blocks at height {height}"
            );
        }
        shared as u64
    }
}

fn open_existing(path: &Path) -> RedbKVStore {
    // A crashed replica's threads may still hold the database for a moment after drop.
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match RedbKVStore::open(path) {
            Ok(store) => return store,
            Err(_) if Instant::now() < deadline => thread::sleep(POLL_INTERVAL),
            Err(e) => panic!("reopen {}: {e}", path.display()),
        }
    }
}

/// Committed block hashes from height 0 up to the highest committed block.
fn committed_chain(node: &TcpNode) -> Vec<CryptoHash> {
    let snapshot = node.block_tree_camera().snapshot();
    let Some(highest) = snapshot.highest_committed_block().expect("read highest") else {
        return Vec::new();
    };
    let top = snapshot
        .block_height(&highest)
        .expect("read height")
        .expect("committed block has height")
        .int();
    (0..=top)
        .map(|h| {
            snapshot
                .block_at_height(BlockHeight::new(h))
                .expect("read block at height")
                .expect("committed height has block")
        })
        .collect()
}

#[test]
fn test_four_validator_tcp_redb_crash_restart_and_catch_up() {
    let alice_key = SigningKey::from_bytes(&[70u8; 32]);
    let alice_addr = hex::encode(alice_key.verifying_key().to_bytes());
    let mut cluster = Cluster::new(4, &alice_addr);
    let all: Vec<usize> = (0..4).collect();

    // 1. Healthy cluster commits a transfer everywhere.
    cluster.transfer(0, 0, &alice_key);
    assert!(cluster.wait_for_bob(&all, TRANSFER), "healthy commit");
    let healthy_height = cluster.assert_no_conflicting_finality();

    // 2. Crash validator 3 while a transfer is pending; survivors must stay consistent.
    cluster.crash(3);
    for i in 0..3 {
        cluster.transfer(i, 1, &alice_key);
    }
    thread::sleep(Duration::from_secs(3));
    cluster.assert_no_conflicting_finality();

    // 3. Restart validator 3 from its own durable database; the pending transfer commits and the
    //    restarted node catches up.
    cluster.start(3);
    assert!(
        cluster.wait_for_bob(&all, 2 * TRANSFER),
        "progress after restart and catch-up"
    );
    let alice = cluster.node(3).get_account(&alice_addr).expect("alice");
    assert_eq!(alice.balance, INITIAL_BALANCE - 2 * (TRANSFER + FEE));
    assert_eq!(alice.nonce, 2);
    assert!(cluster.assert_no_conflicting_finality() > healthy_height);
}

#[test]
fn test_five_validator_tcp_redb_progress_with_one_crashed() {
    let alice_key = SigningKey::from_bytes(&[71u8; 32]);
    let alice_addr = hex::encode(alice_key.verifying_key().to_bytes());
    let mut cluster = Cluster::new(5, &alice_addr);
    let all: Vec<usize> = (0..5).collect();

    cluster.transfer(0, 0, &alice_key);
    assert!(cluster.wait_for_bob(&all, TRANSFER), "healthy commit");
    let healthy_height = cluster.assert_no_conflicting_finality();

    // Crash one validator (f = 1); the remaining 4-of-5 quorum keeps finalizing.
    cluster.crash(4);
    let survivors: Vec<usize> = (0..4).collect();
    for &i in &survivors {
        cluster.transfer(i, 1, &alice_key);
    }
    assert!(
        cluster.wait_for_bob(&survivors, 2 * TRANSFER),
        "quorum progress with one crashed validator"
    );
    let degraded_height = cluster.assert_no_conflicting_finality();
    assert!(degraded_height > healthy_height);

    // Restart it from its own database; it catches up and all nodes agree at every height.
    cluster.start(4);
    assert!(
        cluster.wait_for_bob(&all, 2 * TRANSFER),
        "restarted validator catches up"
    );
    assert!(cluster.assert_no_conflicting_finality() >= degraded_height);
}
