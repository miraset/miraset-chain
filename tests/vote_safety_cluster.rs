//! Vote safety at cluster scale (TODO 8.3.2, PRD R07 / AT07; proposal §5 T3 and T4).
//!
//! Five equal-power validators on the real TCP transport and durable Redb storage, each behind
//! its own `SigningGuard`. A tap *below* the guard logs every phase vote that actually leaves a
//! validator. The check is per signer and per view: never two different `(block, phase)`.
//!
//! - T3: validator 4 is stopped and restarted 20 times after pseudo-random run times (fixed
//!   seed, reproducible); validator 3's chain database is then wiped while its guard is kept,
//!   and it must catch up. A single wiped validator is pulled forward by its peers and never
//!   re-enters old views, so this checks recovery, not guard refusals.
//! - T4: the whole cluster is restored to an earlier copy of its chain databases while every
//!   guard is kept (the snapshot-restore case, TODO 8.2.4). Every validator re-enters views it
//!   already signed in, and new proposals there carry different blocks. The guards must refuse
//!   those votes, and the cluster must resume finalizing once views pass the guard records.
//!
//! In-process restarts drop the replica (threads joined). Killing a real process is covered by
//! the end-to-end tests in `tests/e2e_validator.rs`.

use ed25519_dalek::SigningKey;
use hotstuff_rs::hotstuff::messages::HotStuffMessage;
use hotstuff_rs::hotstuff::types::Phase;
use hotstuff_rs::networking::messages::{Message, ProgressMessage};
use hotstuff_rs::networking::network::Network;
use hotstuff_rs::types::data_types::{BlockHeight, CryptoHash};
use hotstuff_rs::types::update_sets::ValidatorSetUpdates;
use hotstuff_rs::types::validator_set::{ValidatorSet, VerifyingKey};
use miraset_chain::consensus::{
    GuardIdentity, GuardedNetwork, GuardedSigner, OpenMode, SafetyHalt, SigningGuard,
};
use miraset_chain::core::types::Transaction;
use miraset_chain::mempool::Mempool;
use miraset_chain::network::{NetworkRuntime, TcpNetwork};
use miraset_chain::node::{MirasetNode, NodeConfig, genesis_state_commitment, init_genesis};
use miraset_chain::storage::RedbKVStore;
use std::collections::{HashMap, HashSet};
use std::net::{SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const VALIDATORS: usize = 5;
const RESTARTED: usize = 4;
const WIPED: usize = 3;
const RESTARTS: usize = 20;
const VIEW_TIME: Duration = Duration::from_millis(500);
const CONSENSUS_TIMEOUT: Duration = Duration::from_secs(60);
const POLL: Duration = Duration::from_millis(50);
const CHAIN_ID: u64 = 1337;
const INITIAL_BALANCE: u64 = 100_000;
const TRANSFER: u64 = 1_000;
const FEE: u64 = 10;
/// Run time before each restart: 150ms .. 1150ms, from a fixed-seed LCG.
const MIN_RUN_MS: u64 = 150;
const RUN_SPREAD_MS: u64 = 1_000;
const SEED: u64 = 0x5eed_8332;
/// How long the restored cluster is watched for a fork once every validator is held back by
/// its lock floor (20 views).
const RESTORE_STALL_WINDOW: Duration = Duration::from_secs(10);

/// Every phase vote that left any validator: (signer, view) -> distinct (block, phase).
type VoteLog = Arc<Mutex<HashMap<(usize, u64), HashSet<([u8; 32], Phase)>>>>;

#[derive(Clone)]
struct VoteTap {
    inner: TcpNetwork,
    signer: usize,
    log: VoteLog,
}

impl VoteTap {
    fn record(&self, message: &Message) {
        if let Message::ProgressMessage(ProgressMessage::HotStuffMessage(
            HotStuffMessage::PhaseVote(vote),
        )) = message
        {
            self.log
                .lock()
                .expect("vote log")
                .entry((self.signer, vote.view.int()))
                .or_default()
                .insert((vote.block.bytes(), vote.phase));
        }
    }
}

impl Network for VoteTap {
    fn init_validator_set(&mut self, validator_set: ValidatorSet) {
        self.inner.init_validator_set(validator_set);
    }
    fn update_validator_set(&mut self, updates: ValidatorSetUpdates) {
        self.inner.update_validator_set(updates);
    }
    fn broadcast(&mut self, message: Message) {
        self.record(&message);
        self.inner.broadcast(message);
    }
    fn send(&mut self, peer: VerifyingKey, message: Message) {
        self.record(&message);
        self.inner.send(peer, message);
    }
    fn recv(&mut self) -> Option<(VerifyingKey, Message)> {
        self.inner.recv()
    }
}

type GuardedNode = MirasetNode<RedbKVStore, GuardedNetwork<VoteTap>>;

struct RunningNode {
    node: GuardedNode,
    network: GuardedNetwork<VoteTap>,
    signer: Arc<GuardedSigner>,
    // Drop the replica before its transport.
    _runtime: NetworkRuntime,
}

struct Cluster {
    keys: Vec<SigningKey>,
    addrs: Vec<SocketAddr>,
    db_paths: Vec<PathBuf>,
    guard_paths: Vec<PathBuf>,
    validator_set: Vec<(VerifyingKey, u64)>,
    accounts: Vec<(String, u64)>,
    nodes: Vec<Option<RunningNode>>,
    halts: Vec<SafetyHalt>,
    log: VoteLog,
    _dir: tempfile::TempDir,
}

fn free_addr() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    listener.local_addr().expect("local addr")
}

fn wait_until(timeout: Duration, mut condition: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    while !condition() {
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(POLL);
    }
    true
}

/// Retry while a just-stopped replica still releases its file lock or listener.
fn retry_open<T, E: std::fmt::Display>(what: &Path, mut open: impl FnMut() -> Result<T, E>) -> T {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match open() {
            Ok(value) => return value,
            Err(_) if Instant::now() < deadline => thread::sleep(POLL),
            Err(e) => panic!("open {}: {e}", what.display()),
        }
    }
}

impl Cluster {
    fn new(alice_addr: &str) -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let keys: Vec<SigningKey> = (0..VALIDATORS)
            .map(|i| SigningKey::from_bytes(&[80 + i as u8; 32]))
            .collect();
        let validator_set = keys.iter().map(|k| (k.verifying_key(), 100)).collect();
        let mut cluster = Self {
            addrs: (0..VALIDATORS).map(|_| free_addr()).collect(),
            db_paths: (0..VALIDATORS)
                .map(|i| dir.path().join(format!("node{i}.redb")))
                .collect(),
            guard_paths: (0..VALIDATORS)
                .map(|i| dir.path().join(format!("node{i}-guard.redb")))
                .collect(),
            keys,
            validator_set,
            accounts: vec![(alice_addr.to_string(), INITIAL_BALANCE), ("bob".into(), 0)],
            nodes: (0..VALIDATORS).map(|_| None).collect(),
            halts: (0..VALIDATORS).map(|_| SafetyHalt::new()).collect(),
            log: VoteLog::default(),
            _dir: dir,
        };
        for i in 0..VALIDATORS {
            cluster.init_chain_db(i);
            cluster.start(i);
        }
        cluster
    }

    fn account_refs(&self) -> Vec<(&str, u64)> {
        self.accounts
            .iter()
            .map(|(a, b)| (a.as_str(), *b))
            .collect()
    }

    fn init_chain_db(&self, i: usize) {
        let store = RedbKVStore::open(&self.db_paths[i]).expect("create redb");
        init_genesis(store, &self.validator_set, &self.account_refs());
    }

    fn start(&mut self, i: usize) {
        let key = self.keys[i].clone();
        let vk = key.verifying_key();
        let identity = GuardIdentity {
            chain_id: CHAIN_ID,
            validator: vk.to_bytes(),
            genesis_hash: genesis_state_commitment(&self.account_refs()),
        };
        let guard_path = &self.guard_paths[i];
        let guard = retry_open(guard_path, || {
            SigningGuard::open(guard_path, identity, OpenMode::CreateIfMissing)
        });
        let db_path = &self.db_paths[i];
        let halt = self.halts[i].clone();
        let store =
            retry_open(db_path, || RedbKVStore::open(db_path)).with_safety_halt(halt.clone());

        let peers: HashMap<_, _> = self
            .keys
            .iter()
            .zip(&self.addrs)
            .map(|(k, a)| (k.verifying_key(), a.to_string()))
            .collect();
        // A just-stopped replica may still be releasing its listener.
        let addr = self.addrs[i];
        let (tcp, runtime) = retry_open(Path::new(&addr.to_string()), || {
            TcpNetwork::bind_with_hosts(vk, addr, peers.clone())
        });
        let tap = VoteTap {
            inner: tcp,
            signer: i,
            log: Arc::clone(&self.log),
        };
        let network = GuardedNetwork::new(tap, guard.clone(), halt.clone());
        // Key custody D2: the signer checks the guard before signing; the network is layer two.
        let signer = GuardedSigner::new(key, guard).expect("guard matches key");

        let mut config = NodeConfig::devnet_with_signer(signer.clone());
        config.max_view_time = VIEW_TIME;
        config.safety_halt = halt;
        let node = MirasetNode::try_start(config, store, network.clone(), Mempool::new())
            .expect("node start");
        self.nodes[i] = Some(RunningNode {
            node,
            network,
            signer,
            _runtime: runtime,
        });
    }

    fn stop(&mut self, i: usize) -> GuardedNetwork<VoteTap> {
        self.nodes[i].take().expect("node running").network
    }

    /// Votes refused by either guard layer (signer first, network second) on validator `i`.
    fn refused_votes(&self, i: usize) -> u64 {
        let running = self.nodes[i].as_ref().expect("node running");
        running.signer.stale_refused()
            + running.signer.conflicting_refused()
            + running.signer.below_lock_refused()
            + running.network.stale_votes_dropped()
            + running.network.conflicting_votes_refused()
    }

    fn lock_floor(&self, i: usize) -> u64 {
        let running = self.nodes[i].as_ref().expect("node running");
        running.signer.lock_floor().expect("guard readable")
    }

    fn node(&self, i: usize) -> &GuardedNode {
        &self.nodes[i].as_ref().expect("node running").node
    }

    fn transfer(&self, via: usize, nonce: u64, signer: &SigningKey) {
        let mut tx = Transaction::new(&self.accounts[0].0, nonce, "bob", TRANSFER, FEE);
        tx.sign_with(signer);
        assert!(self.node(via).submit_transaction(tx).is_ok());
    }

    fn wait_for_bob_everywhere(&self, expected: u64) -> bool {
        wait_until(CONSENSUS_TIMEOUT, || {
            (0..VALIDATORS)
                .all(|i| self.node(i).get_account("bob").map(|a| a.balance) == Some(expected))
        })
    }

    fn assert_no_conflicting_finality(&self) {
        let chains: Vec<Vec<CryptoHash>> = (0..VALIDATORS)
            .map(|i| committed_chain(self.node(i)))
            .collect();
        let shared = chains.iter().map(Vec::len).min().unwrap_or(0);
        assert!(shared > 0, "no committed blocks to compare");
        for height in 0..shared {
            assert!(
                chains.iter().all(|c| c[height] == chains[0][height]),
                "conflicting committed blocks at height {height}"
            );
        }
    }

    /// Every running node's committed chain agrees with `history` wherever both have a block.
    fn assert_extends(&self, history: &[CryptoHash]) {
        for i in 0..VALIDATORS {
            let chain = committed_chain(self.node(i));
            for (height, (ours, finalized)) in chain.iter().zip(history).enumerate() {
                assert_eq!(
                    ours, finalized,
                    "validator {i} finalized a fork at height {height}"
                );
            }
        }
    }

    fn assert_no_equivocation(&self) -> usize {
        let log = self.log.lock().expect("vote log");
        let double: Vec<_> = log
            .iter()
            .filter(|(_, votes)| votes.len() > 1)
            .map(|((signer, view), votes)| (*signer, *view, votes.len()))
            .collect();
        assert!(
            double.is_empty(),
            "equivocation (signer, view, votes): {double:?}"
        );
        log.len()
    }

    fn assert_no_halts(&self) {
        for (i, halt) in self.halts.iter().enumerate() {
            assert!(
                !halt.is_halted(),
                "validator {i} halted: {:?}",
                halt.reason()
            );
        }
    }
}

fn committed_chain(node: &GuardedNode) -> Vec<CryptoHash> {
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
                .expect("read block")
                .expect("committed height has block")
        })
        .collect()
}

/// Deterministic run times so a failure is reproducible.
fn run_times() -> impl Iterator<Item = Duration> {
    let mut state = SEED;
    std::iter::repeat_with(move || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        Duration::from_millis(MIN_RUN_MS + (state >> 33) % RUN_SPREAD_MS)
    })
}

fn new_cluster(alice_seed: u8) -> (Cluster, SigningKey) {
    let alice_key = SigningKey::from_bytes(&[alice_seed; 32]);
    let alice_addr = hex::encode(alice_key.verifying_key().to_bytes());
    let cluster = Cluster::new(&alice_addr);
    cluster.transfer(0, 0, &alice_key);
    assert!(cluster.wait_for_bob_everywhere(TRANSFER), "healthy commit");
    (cluster, alice_key)
}

/// T3: repeated restarts and a single-validator chain wipe never cause equivocation.
#[test]
fn test_restarts_and_single_wipe_never_equivocate() {
    let (mut cluster, alice_key) = new_cluster(90);

    for run_time in run_times().take(RESTARTS) {
        thread::sleep(run_time);
        drop(cluster.stop(RESTARTED));
        cluster.start(RESTARTED);
    }

    // TODO 8.5.2: the replica passes its HotStuff lock to the signer, so the guard holds a
    // lock floor that the chain wipe below cannot roll back.
    let floor_before_wipe = cluster.lock_floor(WIPED);
    assert!(floor_before_wipe > 0, "guard never recorded a lock");
    drop(cluster.stop(WIPED));
    std::fs::remove_file(&cluster.db_paths[WIPED]).expect("wipe chain db");
    cluster.init_chain_db(WIPED);
    cluster.start(WIPED);
    assert!(
        cluster.lock_floor(WIPED) >= floor_before_wipe,
        "lock floor survived the chain wipe"
    );

    cluster.transfer(0, 1, &alice_key);
    assert!(
        cluster.wait_for_bob_everywhere(2 * TRANSFER),
        "progress and catch-up after restarts and wipe"
    );
    cluster.assert_no_conflicting_finality();
    cluster.assert_no_halts();
    assert!(
        cluster.assert_no_equivocation() > 0,
        "vote tap recorded nothing"
    );
    let restarted_voted = cluster
        .log
        .lock()
        .expect("vote log")
        .keys()
        .any(|(signer, _)| *signer == RESTARTED);
    assert!(restarted_voted, "restarted validator never voted");
    // Liveness of the lock floor (TODO 8.5.2): honest replicas that keep their chain data, and
    // one wiped replica pulled forward by its peers, never build on a certificate below it.
    for i in 0..VALIDATORS {
        let running = cluster.nodes[i].as_ref().expect("node running");
        assert_eq!(
            running.signer.below_lock_refused(),
            0,
            "validator {i} refused an honest vote below its lock floor"
        );
    }
}

/// T4: restoring every validator to older chain data, with guards kept. Same-view votes are
/// refused as stale or conflicting, and (TODO 8.5.2) votes building on certificates below the
/// lock each validator held are refused too: the restored cluster must not finalize a fork of
/// the history it already finalized. It stays stopped until newer chain data is back
/// (owner decision 2026-11-12: fail closed, no automatic override), then resumes on that history.
#[test]
fn test_cluster_restore_to_older_chain_data_is_refused_by_guards() {
    let (mut cluster, alice_key) = new_cluster(91);
    let snapshot_dir = tempfile::tempdir().expect("snapshot dir");
    let copy_all = |cluster: &Cluster, tag: &str| -> Vec<PathBuf> {
        (0..VALIDATORS)
            .map(|i| {
                let copy = snapshot_dir.path().join(format!("node{i}-{tag}.redb"));
                std::fs::copy(&cluster.db_paths[i], &copy).expect("copy chain db");
                copy
            })
            .collect()
    };
    let stop_all = |cluster: &mut Cluster| {
        for i in 0..VALIDATORS {
            drop(cluster.stop(i));
        }
    };
    let restore_all = |cluster: &mut Cluster, copies: &[PathBuf]| {
        for (i, copy) in copies.iter().enumerate() {
            std::fs::copy(copy, &cluster.db_paths[i]).expect("restore chain db");
        }
        for i in 0..VALIDATORS {
            cluster.start(i);
        }
    };

    // Snapshot every chain database (validators stopped), then finalize past it.
    stop_all(&mut cluster);
    let older = copy_all(&cluster, "older");
    for i in 0..VALIDATORS {
        cluster.start(i);
    }
    cluster.transfer(0, 1, &alice_key);
    assert!(
        cluster.wait_for_bob_everywhere(2 * TRANSFER),
        "progress past snapshot"
    );
    // A prefix of the history finalized before the restore.
    let finalized_history = committed_chain(cluster.node(0));
    stop_all(&mut cluster);
    let newer = copy_all(&cluster, "newer");

    // Restore all chain databases to the older snapshot; guards keep their newer records.
    restore_all(&mut cluster, &older);
    assert!(
        finalized_history.len() > committed_chain(cluster.node(0)).len(),
        "the restore rolled back finalized blocks"
    );

    // Resubmitting transfer 1 must not finalize it on a fork of the finalized history. First
    // wait until every validator has passed its stale views and is held back only by its lock
    // floor (where the cluster used to resume), then keep watching for a while.
    cluster.transfer(0, 1, &alice_key);
    let all_below_lock = |cluster: &Cluster| {
        (0..VALIDATORS).all(|i| {
            let running = cluster.nodes[i].as_ref().expect("node running");
            running.signer.below_lock_refused() > 0
        })
    };
    let reached = wait_until(CONSENSUS_TIMEOUT, || {
        cluster.assert_extends(&finalized_history);
        all_below_lock(&cluster)
    });
    assert!(reached, "not every validator reached its lock floor");
    let stall_deadline = Instant::now() + RESTORE_STALL_WINDOW;
    while Instant::now() < stall_deadline {
        cluster.assert_extends(&finalized_history);
        thread::sleep(POLL);
    }
    let refused: u64 = (0..VALIDATORS).map(|i| cluster.refused_votes(i)).sum();
    assert!(refused > 0, "restored validators never tried to re-sign");
    cluster.assert_no_equivocation();

    // Recovery: put the newer chain data back. The cluster resumes on the finalized history.
    stop_all(&mut cluster);
    restore_all(&mut cluster, &newer);
    cluster.transfer(0, 2, &alice_key);
    assert!(
        cluster.wait_for_bob_everywhere(3 * TRANSFER),
        "progress resumes once newer chain data is back"
    );
    cluster.assert_extends(&finalized_history);
    cluster.assert_no_conflicting_finality();
    cluster.assert_no_halts();
    cluster.assert_no_equivocation();
}
