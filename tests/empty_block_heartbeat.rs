//! Empty-block heartbeat (TODO 8.5.3 finding 11, owner decision 2026-11-13): an idle chain grows
//! by about one empty block per `empty_block_interval`, and a transaction submitted to an idle
//! chain still commits without waiting for the heartbeat.

use ed25519_dalek::SigningKey;
use miraset_chain::core::types::Transaction;
use miraset_chain::mempool::Mempool;
use miraset_chain::network::{ChannelNetwork, ChannelNetworkHub};
use miraset_chain::node::{MirasetNode, NodeConfig, init_genesis_with_params};
use miraset_chain::storage::MemKVStore;
use std::thread;
use std::time::{Duration, Instant};

mod common;
use common::pocc::Pocc;

const VIEW_TIME: Duration = Duration::from_millis(2000);
const INTERVAL: Duration = Duration::from_millis(500);
const IDLE: Duration = Duration::from_secs(4);
const POLL: Duration = Duration::from_millis(20);
/// Commit of a transfer on an idle chain: three quick blocks (≈ 20 ms measured); below the
/// 1 s (clamped) heartbeat a leader would otherwise wait, with headroom for a loaded machine.
const IDLE_TX_DEADLINE: Duration = Duration::from_millis(700);
/// Transfer latency with a crashed validator: a dead leader's view costs about two view times
/// with or without the heartbeat (4.03 s measured 2026-11-13, heartbeat off and on); one more
/// second of margin, below the extra heartbeat waits review H1 would add.
const CRASHED_TX_DEADLINE: Duration = Duration::from_millis(5_000);

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn address(key: &SigningKey) -> String {
    hex::encode(key.verifying_key().to_bytes())
}

fn start(interval: Duration, accounts: &[(&str, u64)]) -> MirasetNode<MemKVStore, ChannelNetwork> {
    let validator = key(60);
    let vk = validator.verifying_key();
    let kv = MemKVStore::new();
    let pocc = Pocc::default();
    let parties = pocc.genesis_accounts();
    let mut all: Vec<(&str, u64)> = parties.iter().map(|(a, b)| (a.as_str(), *b)).collect();
    all.extend_from_slice(accounts);
    init_genesis_with_params(kv.clone(), 1337, &[(vk, 1)], &all, &pocc.params());
    let config = NodeConfig {
        max_view_time: VIEW_TIME,
        empty_block_interval: interval,
        ..NodeConfig::devnet(validator)
    };
    MirasetNode::start(
        config,
        kv,
        ChannelNetwork::new(vk, ChannelNetworkHub::new()),
        Mempool::new(),
    )
}

fn wait(deadline: Duration, what: &str, mut done: impl FnMut() -> bool) -> Duration {
    let start = Instant::now();
    while !done() {
        assert!(start.elapsed() < deadline, "timed out: {what}");
        thread::sleep(POLL);
    }
    start.elapsed()
}

/// Committed blocks over `IDLE` once the chain is running.
fn idle_blocks(node: &MirasetNode<MemKVStore, ChannelNetwork>) -> u64 {
    wait(Duration::from_secs(10), "first commit", || {
        node.highest_committed_height().is_some()
    });
    let before = node.highest_committed_height().unwrap();
    thread::sleep(IDLE);
    node.highest_committed_height().unwrap() - before
}

#[test]
fn test_idle_chain_grows_by_one_empty_block_per_interval() {
    let node = start(INTERVAL, &[]);
    let blocks = idle_blocks(&node);
    let expected = IDLE.as_millis() / INTERVAL.as_millis(); // 8
    assert!(
        (expected as u64 - 2..=expected as u64 + 2).contains(&blocks),
        "{blocks} blocks in {IDLE:?} with a {INTERVAL:?} heartbeat, expected about {expected}"
    );
}

#[test]
fn test_zero_interval_keeps_proposing_every_view() {
    let node = start(Duration::ZERO, &[]);
    let blocks = idle_blocks(&node);
    let heartbeat_blocks = (IDLE.as_millis() / INTERVAL.as_millis()) as u64;
    assert!(
        blocks > heartbeat_blocks * 2,
        "{blocks} blocks in {IDLE:?} without a heartbeat"
    );
}

#[test]
fn test_transfer_on_an_idle_chain_commits_without_waiting_for_the_heartbeat() {
    let alice = key(61);
    let alice_addr = address(&alice);
    // A long interval (clamped to half the view time) makes any heartbeat wait obvious.
    let node = start(Duration::from_secs(60), &[(alice_addr.as_str(), 1_000)]);
    wait(Duration::from_secs(10), "first commit", || {
        node.highest_committed_height().is_some()
    });
    // Let the chain go idle: the last three blocks are empty, so the leader is waiting.
    thread::sleep(VIEW_TIME);

    let mut tx = Transaction::new(&alice_addr, 0, "bob", 7, 1);
    tx.sign_with(&alice);
    node.submit_transaction(tx).expect("admitted");
    let took = wait(IDLE_TX_DEADLINE, "transfer committed", || {
        node.get_account("bob").map(|a| a.balance) == Some(7)
    });
    eprintln!("transfer on an idle chain committed in {took:?}");
}

/// Transfers on an idle chain with a crashed validator commit within a dead leader's view cost.
/// Five validators, one never starts; each transfer goes to every live validator (no mempool
/// gossip) after the chain has gone idle. Review H1 (a block with work kept waiting heartbeats
/// after a view timeout) needs the dead view within the three blocks after the transfer; this
/// timing is random here, so the test guards the bound but does not reliably reproduce H1.
#[test]
fn test_transfers_commit_promptly_with_a_crashed_validator() {
    const VALIDATORS: u8 = 5;
    const TRANSFERS: u64 = 5;
    let senders: Vec<SigningKey> = (0..TRANSFERS as u8).map(|i| key(80 + i)).collect();
    let addrs: Vec<String> = senders.iter().map(address).collect();
    let keys: Vec<SigningKey> = (0..VALIDATORS).map(|i| key(70 + i)).collect();
    let validators: Vec<_> = keys.iter().map(|k| (k.verifying_key(), 1)).collect();
    let hub = ChannelNetworkHub::new();
    let pocc = Pocc::default();
    let parties = pocc.genesis_accounts();
    let mut accounts: Vec<(&str, u64)> = parties.iter().map(|(a, b)| (a.as_str(), *b)).collect();
    accounts.extend(addrs.iter().map(|a| (a.as_str(), 1_000)));
    let nodes: Vec<_> = keys
        .iter()
        .take(usize::from(VALIDATORS) - 1)
        .map(|k| {
            let kv = MemKVStore::new();
            init_genesis_with_params(kv.clone(), 1337, &validators, &accounts, &pocc.params());
            let config = NodeConfig {
                max_view_time: VIEW_TIME,
                ..NodeConfig::devnet(k.clone())
            };
            let network = ChannelNetwork::new(k.verifying_key(), hub.clone());
            MirasetNode::start(config, kv, network, Mempool::new())
        })
        .collect();
    wait(Duration::from_secs(20), "first commit", || {
        nodes[0].highest_committed_height().is_some()
    });

    let mut slowest = Duration::ZERO;
    for (sender, to) in senders.iter().zip(1..) {
        thread::sleep(VIEW_TIME);
        let mut tx = Transaction::new(address(sender), 0, format!("to{to}"), 7, 1);
        tx.sign_with(sender);
        for node in &nodes {
            node.submit_transaction(tx.clone()).expect("admitted");
        }
        let took = wait(Duration::from_secs(30), "transfer committed", || {
            nodes[0].get_account(&format!("to{to}")).map(|a| a.balance) == Some(7)
        });
        eprintln!("transfer {to}: {took:?}");
        slowest = slowest.max(took);
    }
    eprintln!("slowest transfer with a crashed validator: {slowest:?}");
    assert!(
        slowest < CRASHED_TX_DEADLINE,
        "slowest transfer {slowest:?} (view time {VIEW_TIME:?})"
    );
}

/// Strict block-rate limit (owner request 2026-11-13): with work waiting in every view, the
/// chain still makes at most one block per `min_block_interval`.
#[test]
fn test_min_block_interval_limits_blocks_even_with_work() {
    const LIMIT: Duration = Duration::from_millis(500);
    const SENDERS: u8 = 40;
    let senders: Vec<SigningKey> = (0..SENDERS).map(|i| key(100 + i)).collect();
    let addrs: Vec<String> = senders.iter().map(address).collect();
    let validator = key(60);
    let vk = validator.verifying_key();
    let kv = MemKVStore::new();
    let pocc = Pocc::default();
    let parties = pocc.genesis_accounts();
    let mut accounts: Vec<(&str, u64)> = parties.iter().map(|(a, b)| (a.as_str(), *b)).collect();
    accounts.extend(addrs.iter().map(|a| (a.as_str(), 1_000)));
    init_genesis_with_params(kv.clone(), 1337, &[(vk, 1)], &accounts, &pocc.params());
    let config = NodeConfig {
        max_view_time: VIEW_TIME,
        min_block_interval: LIMIT,
        ..NodeConfig::devnet(validator)
    };
    let node = MirasetNode::start(
        config,
        kv,
        ChannelNetwork::new(vk, ChannelNetworkHub::new()),
        Mempool::new(),
    );
    wait(Duration::from_secs(10), "first commit", || {
        node.highest_committed_height().is_some()
    });
    // Keep the mempool non-empty: one transfer per sender per nonce, well ahead of the chain.
    for nonce in 0..10 {
        for (sender, addr) in senders.iter().zip(&addrs) {
            let mut tx = Transaction::new(addr, nonce, "bob", 1, 1);
            tx.sign_with(sender);
            let _ = node.submit_transaction(tx);
        }
    }
    let before = node.highest_committed_height().unwrap();
    thread::sleep(IDLE);
    let blocks = node.highest_committed_height().unwrap() - before;
    let most = (IDLE.as_millis() / LIMIT.as_millis()) as u64 + 1;
    assert!(
        blocks <= most,
        "{blocks} blocks in {IDLE:?} with a {LIMIT:?} limit (at most {most})"
    );
    assert!(
        blocks >= most / 2,
        "chain stalled: {blocks} blocks in {IDLE:?}"
    );
    assert!(
        node.get_account("bob").is_some_and(|a| a.balance > 0),
        "work was committed under the limit"
    );
}
