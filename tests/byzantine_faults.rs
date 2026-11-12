//! Equivocation, selective-message and lagging-validator fault scenarios (TODO 8.5.3).
//!
//! Five equal-power validators on the in-memory hub (n = 5, f = 1, quorum 4 votes). Every
//! scenario checks **safety** (all replicas commit the same block at every height they share)
//! and **liveness** (finality keeps advancing within the fault bound).
//!
//! - Equivocation: validator 4's key runs on two replicas (a copied key, custody threat K3),
//!   with no signing guard shared between them. Both clones receive every message from peers,
//!   but each clone's own messages reach only half of the honest validators, as two hosts
//!   reached by different peers would. In validator 4's leader views the clones propose
//!   different blocks, so the honest votes split; each clone also votes for its own block. The
//!   test requires equivocation and honest vote splits to be observed. It is one adversarial
//!   delivery schedule, not an exhaustive search.
//! - Selective messages: validator 0 reaches only some peers, and one peer's messages never
//!   reach validator 0.
//! - Lagging validator: validator 4 is cut off while the others commit several block-sync
//!   batches, then rejoins and must catch up over multiple sync rounds.
//!
//! In-memory and in-process: this is fault-simulation evidence, not an independent safety
//! review (8.5.2) or a measured benchmark. The scenarios run one at a time (`SERIAL`), so their
//! timing does not depend on each other's CPU load.

use ed25519_dalek::SigningKey;
use hotstuff_rs::hotstuff::messages::HotStuffMessage;
use hotstuff_rs::hotstuff::types::Phase;
use hotstuff_rs::networking::messages::{Message, ProgressMessage};
use hotstuff_rs::networking::network::Network;
use hotstuff_rs::types::data_types::{CryptoHash, Power};
use hotstuff_rs::types::update_sets::ValidatorSetUpdates;
use hotstuff_rs::types::validator_set::{ValidatorSet, VerifyingKey};
use miraset_chain::core::types::Transaction;
use miraset_chain::mempool::Mempool;
use miraset_chain::network::{ChannelNetwork, ChannelNetworkHub};
use miraset_chain::node::{MirasetNode, NodeConfig, init_genesis};
use miraset_chain::storage::MemKVStore;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const VALIDATORS: usize = 5;
const VIEW_TIME: Duration = Duration::from_millis(200);
const SCENARIO_TIMEOUT: Duration = Duration::from_secs(90);
const POLL: Duration = Duration::from_millis(50);
const INITIAL_BALANCE: u64 = 10_000_000;
/// Heights that must still be finalized after the fault was observed or applied.
const PROGRESS_AFTER_FAULT: u64 = 5;
/// Pending transfers kept in the clone's mempool only, so its blocks differ from the original's.
/// Kept small: block production re-executes candidates quadratically (slow in debug builds).
const CLONE_ONLY_PENDING: usize = 2;
/// Views of each kind (proposal, vote) in which equivocation must be observed.
const MIN_EQUIVOCATION_VIEWS: usize = 3;
/// Small sync batches make a short lag need several sync rounds.
const SYNC_BATCH: u32 = 5;
const SYNC_ROUNDS: u64 = 4;
/// Time for in-flight messages to drain after a fault is applied.
const SETTLE: Duration = Duration::from_millis(1_000);
/// Readiness window in the lagging scenario: short, so an isolated replica turns stale.
const LAG_READY_WINDOW: Duration = Duration::from_millis(1_000);
/// HotStuff commits a block when a later block's certificate completes its chain, so a replica's
/// committed height trails the newest committed height of its peers by the blocks in flight
/// (pipelining depth 2-3). Readiness may turn on that far below the peers' tip.
const READY_TIP_SLACK: u64 = 3;

/// Serialises the scenarios: each runs 5–6 replicas on short views.
static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// (replica, view) a message left from.
type ReplicaView = (usize, u64);

/// Outgoing proposals and phase votes per replica and view: distinct blocks / (block, phase).
#[derive(Default)]
struct Outbox {
    proposals: HashMap<ReplicaView, HashSet<[u8; 32]>>,
    votes: HashMap<ReplicaView, HashSet<([u8; 32], Phase)>>,
}

type OutboxLog = Arc<Mutex<Outbox>>;

/// Records what leaves a replica, below consensus and above the hub.
#[derive(Clone)]
struct Tap {
    inner: ChannelNetwork,
    replica: usize,
    log: OutboxLog,
}

impl Tap {
    fn record(&self, message: &Message) {
        let Message::ProgressMessage(ProgressMessage::HotStuffMessage(message)) = message else {
            return;
        };
        let mut log = self.log.lock().expect("outbox log");
        match message {
            HotStuffMessage::Proposal(proposal) => {
                log.proposals
                    .entry((self.replica, proposal.view.int()))
                    .or_default()
                    .insert(proposal.block.hash.bytes());
            }
            HotStuffMessage::PhaseVote(vote) => {
                log.votes
                    .entry((self.replica, vote.view.int()))
                    .or_default()
                    .insert((vote.block.bytes(), vote.phase));
            }
            _ => {}
        }
    }
}

impl Network for Tap {
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

type Node = MirasetNode<MemKVStore, Tap>;

struct Cluster {
    hub: ChannelNetworkHub,
    keys: Vec<SigningKey>,
    /// Replica `i < VALIDATORS` runs validator `i`; any further replica is a clone.
    nodes: Vec<Node>,
    log: OutboxLog,
    alice: SigningKey,
}

impl Cluster {
    /// Start one replica per validator plus one clone for each entry of `cloned`.
    fn start(seed: u8, cloned: &[usize], tune: impl Fn(&mut NodeConfig)) -> Self {
        let keys: Vec<SigningKey> = (0..VALIDATORS as u8)
            .map(|i| SigningKey::from_bytes(&[seed + i; 32]))
            .collect();
        let validators: Vec<_> = keys.iter().map(|k| (k.verifying_key(), 1)).collect();
        let alice = SigningKey::from_bytes(&[seed + VALIDATORS as u8; 32]);
        let alice_addr = address(&alice);
        let accounts = [(alice_addr.as_str(), INITIAL_BALANCE)];

        let hub = ChannelNetworkHub::new();
        let log = OutboxLog::default();
        let runs: Vec<usize> = (0..VALIDATORS).chain(cloned.iter().copied()).collect();
        let nodes = runs
            .iter()
            .enumerate()
            .map(|(replica, &validator)| {
                let key = keys[validator].clone();
                let kv = MemKVStore::new();
                init_genesis(kv.clone(), &validators, &accounts);
                let network = Tap {
                    inner: if replica < VALIDATORS {
                        ChannelNetwork::new(key.verifying_key(), hub.clone())
                    } else {
                        ChannelNetwork::new_clone(key.verifying_key(), hub.clone())
                    },
                    replica,
                    log: Arc::clone(&log),
                };
                let mut config = NodeConfig {
                    max_view_time: VIEW_TIME,
                    ..NodeConfig::devnet(key)
                };
                tune(&mut config);
                MirasetNode::start(config, kv, network, Mempool::new())
            })
            .collect();
        Self {
            hub,
            keys,
            nodes,
            log,
            alice,
        }
    }

    fn vk(&self, validator: usize) -> VerifyingKey {
        self.keys[validator].verifying_key()
    }

    fn height(&self, replica: usize) -> u64 {
        self.nodes[replica].highest_committed_height().unwrap_or(0)
    }

    fn min_height(&self, replicas: &[usize]) -> u64 {
        replicas.iter().map(|&r| self.height(r)).min().unwrap_or(0)
    }

    fn wait_all_reach(&self, replicas: &[usize], height: u64, what: &str) {
        let deadline = Instant::now() + SCENARIO_TIMEOUT;
        while self.min_height(replicas) < height {
            assert!(
                Instant::now() < deadline,
                "{what}: heights {:?}, wanted {height}",
                replicas.iter().map(|&r| self.height(r)).collect::<Vec<_>>()
            );
            thread::sleep(POLL);
        }
    }

    /// Validator reconfiguration is disabled (TODO 8.3.3; reviewed in 8.5.2): no replica,
    /// honest or Byzantine-proposed, ever records a validator-set update, and the committed set
    /// is still the genesis set.
    fn assert_validator_set_unchanged(&self, replicas: &[usize]) {
        let mut genesis = ValidatorSet::new();
        for key in &self.keys {
            genesis.put(&key.verifying_key(), Power::new(1));
        }
        for &replica in replicas {
            let snapshot = self.nodes[replica].block_tree_camera().snapshot();
            assert_eq!(
                snapshot.validator_set_update_block_height().expect("read"),
                None,
                "replica {replica} recorded a validator-set update"
            );
            let committed = snapshot.committed_validator_set().expect("read");
            assert!(
                committed.validators_and_powers() == genesis.validators_and_powers(),
                "replica {replica} committed validator set changed"
            );
        }
    }

    /// Safety: at every height two replicas both committed, they committed the same block.
    fn assert_no_conflicting_commits(&self, replicas: &[usize]) {
        let chains: Vec<Vec<CryptoHash>> = replicas
            .iter()
            .map(|&r| committed_chain(&self.nodes[r]))
            .collect();
        let reference = chains.iter().max_by_key(|c| c.len()).expect("replicas");
        for (chain, replica) in chains.iter().zip(replicas) {
            for (height, hash) in chain.iter().enumerate() {
                assert_eq!(
                    hash, &reference[height],
                    "replica {replica} committed a conflicting block at height {height}"
                );
            }
        }
    }
}

fn address(key: &SigningKey) -> String {
    hex::encode(key.verifying_key().to_bytes())
}

/// Committed block hashes from height 0 to the highest committed height, read in one pass.
fn committed_chain(node: &Node) -> Vec<CryptoHash> {
    let top = node.highest_committed_height().unwrap_or(0);
    (0..=top)
        .map_while(|height| node.committed_block_at_height(height))
        .collect()
}

#[test]
fn test_cloned_validator_key_equivocates_without_breaking_safety() {
    const BYZANTINE: usize = 4;
    const CLONE: usize = VALIDATORS; // replica 5 runs validator 4's key as well
    let _serial = serial();
    let cluster = Cluster::start(60, &[BYZANTINE], |_| {});
    let honest: Vec<usize> = (0..BYZANTINE).collect();
    let byzantine_replicas = [BYZANTINE, CLONE];
    // Mailbox 0 is the original replica and mailbox 1 the clone: the original reaches
    // validators 0 and 1, the clone reaches validators 2 and 3.
    let key = cluster.vk(BYZANTINE);
    for (mailbox, unreachable) in [(0, [2, 3]), (1, [0, 1])] {
        for validator in unreachable {
            cluster
                .hub
                .block_clone_link(key, mailbox, cluster.vk(validator));
        }
    }

    // Only the clone knows these transfers, so in validator 4's leader views the clone proposes
    // a block with them and the original proposes a different one. Refilled as they commit.
    let alice_addr = address(&cluster.alice);
    let mut next_nonce = 0;
    let deadline = Instant::now() + SCENARIO_TIMEOUT;
    let (proposal_views, vote_views, split_views) = loop {
        while cluster.nodes[CLONE].mempool().tx_count() < CLONE_ONLY_PENDING {
            let mut tx = Transaction::new(&alice_addr, next_nonce, "bob", 1, 1);
            tx.sign_with(&cluster.alice);
            assert!(cluster.nodes[CLONE].submit_transaction(tx).is_ok());
            next_nonce += 1;
        }
        let observed = equivocations(&cluster.log, &byzantine_replicas, &honest);
        if observed.0 >= MIN_EQUIVOCATION_VIEWS
            && observed.1 >= MIN_EQUIVOCATION_VIEWS
            && observed.2 >= 1
        {
            break observed;
        }
        assert!(
            Instant::now() < deadline,
            "too little equivocation observed (proposal views {}, vote views {}, honest \
             vote splits {})",
            observed.0,
            observed.1,
            observed.2
        );
        thread::sleep(POLL);
    };

    let after = cluster.min_height(&honest) + PROGRESS_AFTER_FAULT;
    cluster.wait_all_reach(&honest, after, "honest progress after equivocation");
    cluster.assert_no_conflicting_commits(&honest);
    // The clones run correct code: they also only commit quorum-certified blocks.
    cluster.assert_no_conflicting_commits(&(0..=CLONE).collect::<Vec<_>>());
    cluster.assert_validator_set_unchanged(&(0..=CLONE).collect::<Vec<_>>());
    eprintln!(
        "equivocation: {proposal_views} views with conflicting proposals, {vote_views} with \
         conflicting votes, {split_views} where honest votes split; honest height {}",
        cluster.min_height(&honest)
    );
}

/// For `replicas` (all running one key): views with two different proposals, (view, phase)
/// pairs with two different votes, and conflicting-proposal views in which the `honest`
/// replicas voted for different blocks (the equivocation reached them).
fn equivocations(log: &OutboxLog, replicas: &[usize], honest: &[usize]) -> (usize, usize, usize) {
    let log = log.lock().expect("outbox log");
    let mut proposals: HashMap<u64, HashSet<[u8; 32]>> = HashMap::new();
    let mut votes: HashMap<(u64, Phase), HashSet<[u8; 32]>> = HashMap::new();
    for (&(replica, view), blocks) in &log.proposals {
        if replicas.contains(&replica) {
            proposals.entry(view).or_default().extend(blocks);
        }
    }
    for (&(replica, view), items) in &log.votes {
        if replicas.contains(&replica) {
            for &(block, phase) in items {
                votes.entry((view, phase)).or_default().insert(block);
            }
        }
    }
    let conflicting: Vec<u64> = proposals
        .iter()
        .filter(|(_, blocks)| blocks.len() > 1)
        .map(|(view, _)| *view)
        .collect();
    let honest_splits = conflicting
        .iter()
        .filter(|&&view| {
            let blocks: HashSet<[u8; 32]> = honest
                .iter()
                .filter_map(|&r| log.votes.get(&(r, view)))
                .flatten()
                .filter(|(_, phase)| phase.is_generic() || phase.is_prepare())
                .map(|(block, _)| *block)
                .collect();
            blocks.len() > 1
        })
        .count();
    (
        conflicting.len(),
        votes.values().filter(|b| b.len() > 1).count(),
        honest_splits,
    )
}

#[test]
fn test_selective_message_loss_keeps_safety_and_progress() {
    let _serial = serial();
    let cluster = Cluster::start(70, &[], |_| {});
    let all: Vec<usize> = (0..VALIDATORS).collect();
    cluster.wait_all_reach(&all, 2, "startup");

    // Validator 0 reaches only validators 1 and 4; validator 3 never reaches validator 0.
    // Votes go to the next leader, so some of 0's leader views lose votes as well.
    cluster.hub.block_link(cluster.vk(0), cluster.vk(2));
    cluster.hub.block_link(cluster.vk(0), cluster.vk(3));
    cluster.hub.block_link(cluster.vk(3), cluster.vk(0));
    let faulted_at = cluster.min_height(&all);
    let dropped_before = cluster.hub.dropped_deliveries();

    cluster.wait_all_reach(
        &all,
        faulted_at + PROGRESS_AFTER_FAULT,
        "progress under selective message loss",
    );
    assert!(
        cluster.hub.dropped_deliveries() > dropped_before,
        "the blocked links never dropped a message"
    );
    cluster.assert_no_conflicting_commits(&all);

    cluster.hub.clear_faults();
    let healed_at = cluster.min_height(&all);
    cluster.wait_all_reach(
        &all,
        healed_at + PROGRESS_AFTER_FAULT,
        "progress after heal",
    );
    cluster.assert_no_conflicting_commits(&all);
}

#[test]
fn test_lagging_validator_catches_up_over_several_sync_rounds() {
    const LAGGING: usize = 4;
    let _serial = serial();
    let cluster = Cluster::start(80, &[], |config| {
        config.block_sync_request_limit = SYNC_BATCH;
        config.ready_max_commit_age = LAG_READY_WINDOW;
    });
    let all: Vec<usize> = (0..VALIDATORS).collect();
    let others: Vec<usize> = (0..LAGGING).collect();
    cluster.wait_all_reach(&all, 2, "startup");

    cluster.hub.isolate_peer(cluster.vk(LAGGING));
    // Messages already in its mailbox are still processed; read the baseline after they drain.
    thread::sleep(SETTLE);
    let lag_start = cluster.height(LAGGING);
    let lag = u64::from(SYNC_BATCH) * SYNC_ROUNDS;
    cluster.wait_all_reach(
        &others,
        lag_start + lag,
        "progress without the lagging validator",
    );
    // Isolated, it cannot have advanced past what it already knew.
    assert!(cluster.height(LAGGING) <= lag_start + 1);

    // Measure the gap before healing, so sync cannot start in between.
    let target = cluster.min_height(&others);
    let behind = target.saturating_sub(cluster.height(LAGGING));
    assert!(
        behind > u64::from(SYNC_BATCH),
        "lag of {behind} must need more than one sync batch"
    );
    let lagging = &cluster.nodes[LAGGING];
    assert!(
        lagging.readiness().is_err(),
        "isolated replica reports not ready"
    );
    cluster.hub.heal_peer(&cluster.vk(LAGGING));
    assert_not_ready_until_caught_up(&cluster, LAGGING, target);
    cluster.assert_no_conflicting_commits(&all);
    let metrics = lagging.render_metrics();
    let blocks_synced = metrics
        .lines()
        .find_map(|line| line.strip_prefix("miraset_blocks_synced_total "))
        .and_then(|value| value.parse::<u64>().ok());
    assert!(
        blocks_synced.is_some_and(|blocks| blocks > 0),
        "catch-up went through block sync:\n{metrics}"
    );

    // Back in the set, it contributes to further finality.
    let rejoined_at = cluster.min_height(&all);
    cluster.wait_all_reach(
        &all,
        rejoined_at + PROGRESS_AFTER_FAULT,
        "progress after rejoin",
    );
    cluster.assert_no_conflicting_commits(&all);
}

/// Catch-up readiness (TODO 8.4.4): while the replica is behind it must never report ready;
/// it must become ready after it catches up. Block-sync commits do not count for readiness.
fn assert_not_ready_until_caught_up(cluster: &Cluster, replica: usize, target: u64) {
    let node = &cluster.nodes[replica];
    let others: Vec<usize> = (0..VALIDATORS).filter(|&r| r != replica).collect();
    let deadline = Instant::now() + SCENARIO_TIMEOUT;
    let mut saw_syncing = false;
    loop {
        // Tip before readiness, own height after: both only grow, so a ready replica is
        // compared with a tip no newer than the moment it reported ready.
        let tip = cluster.min_height(&others);
        let readiness = node.readiness();
        let height = cluster.height(replica);
        match readiness {
            Ok(()) => {
                assert!(
                    height + READY_TIP_SLACK >= tip,
                    "reported ready at height {height}, peers' tip was at least {tip}"
                );
                if height >= target {
                    break;
                }
            }
            Err(reason) => saw_syncing |= reason.as_str() == "syncing",
        }
        assert!(
            Instant::now() < deadline,
            "lagging replica at {height} (wanted {target}) not ready: {readiness:?}"
        );
        thread::sleep(Duration::from_millis(10));
    }
    // Sessions can be short; the metric is the robust check, this one is informative.
    eprintln!("lagging replica observed syncing in a probe: {saw_syncing}");
}
