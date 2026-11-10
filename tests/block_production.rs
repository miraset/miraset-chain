//! Block production selects mempool candidates greedily and in linear work (TODO 8.5.3 finding
//! b, PRD R04): each candidate is kept if it applies on top of those kept before it, and a full
//! mempool no longer makes a leader miss its views.

use ed25519_dalek::SigningKey;
use miraset_chain::core::types::{ComputeReceipt, Transaction};
use miraset_chain::mempool::Mempool;
use miraset_chain::network::{ChannelNetwork, ChannelNetworkHub};
use miraset_chain::node::{MirasetNode, NodeConfig, init_genesis};
use miraset_chain::storage::MemKVStore;
use std::thread;
use std::time::{Duration, Instant};

const VIEW_TIME: Duration = Duration::from_millis(500);
const POLL: Duration = Duration::from_millis(20);
const BALANCE: u64 = 1_000_000;
/// One full block's worth of transfers from one sender (the producer takes up to 100).
const FULL_BLOCK_TXS: u64 = 100;
/// Debug build, one validator: the quadratic producer needed minutes for this; linear needs
/// well under a second of work per block.
const FULL_MEMPOOL_DEADLINE: Duration = Duration::from_secs(20);

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn address(key: &SigningKey) -> String {
    hex::encode(key.verifying_key().to_bytes())
}

fn transfer(from: &SigningKey, nonce: u64, to: &str, amount: u64) -> Transaction {
    let mut tx = Transaction::new(address(from), nonce, to, amount, 1);
    tx.sign_with(from);
    tx
}

fn start(accounts: &[(&str, u64)], mempool: Mempool) -> MirasetNode<MemKVStore, ChannelNetwork> {
    let validator = key(40);
    let vk = validator.verifying_key();
    let kv = MemKVStore::new();
    init_genesis(kv.clone(), &[(vk, 1)], accounts);
    let config = NodeConfig {
        max_view_time: VIEW_TIME,
        ..NodeConfig::devnet(validator)
    };
    MirasetNode::start(
        config,
        kv,
        ChannelNetwork::new(vk, ChannelNetworkHub::new()),
        mempool,
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

#[test]
fn test_full_mempool_is_finalized_quickly() {
    let alice = key(41);
    let alice_addr = address(&alice);
    let mempool = Mempool::new();
    for nonce in 0..FULL_BLOCK_TXS * 2 {
        assert!(mempool.insert_tx(transfer(&alice, nonce, "bob", 1)));
    }
    let node = start(&[(alice_addr.as_str(), BALANCE)], mempool);

    let took = wait(FULL_MEMPOOL_DEADLINE, "two full blocks committed", || {
        node.get_account(&alice_addr).map(|a| a.nonce) == Some(FULL_BLOCK_TXS * 2)
    });
    assert_eq!(
        node.get_account("bob").map(|a| a.balance),
        Some(FULL_BLOCK_TXS * 2)
    );
    eprintln!("{} transfers finalized in {took:?}", FULL_BLOCK_TXS * 2);
}

#[test]
fn test_greedy_selection_keeps_valid_candidates_and_skips_invalid_ones() {
    let alice = key(42);
    let carol = key(43);
    let dave = key(44);
    let forger = key(45);
    let coordinator = key(46);
    let (alice_addr, carol_addr, dave_addr) = (address(&alice), address(&carol), address(&dave));

    let mempool = Mempool::new();
    // Alice: nonces 0..5 apply in order; nonce 7 leaves a gap and must wait.
    for nonce in 0..5 {
        assert!(mempool.insert_tx(transfer(&alice, nonce, "bob", 10)));
    }
    assert!(mempool.insert_tx(transfer(&alice, 7, "bob", 10)));
    // Carol: signed by someone else (forged sender).
    let mut forged = Transaction::new(&carol_addr, 0, "bob", 10, 1);
    forged.sign_with(&forger);
    assert!(mempool.insert_tx(forged));
    // Dave: first transfer spends almost everything, the second cannot be funded after it.
    assert!(mempool.insert_tx(transfer(&dave, 0, "bob", 90)));
    assert!(mempool.insert_tx(transfer(&dave, 1, "bob", 90)));
    // Receipts: one valid; one whose client did not confirm delivery.
    let mut settled = ComputeReceipt::new(
        "task-ok",
        &alice_addr,
        "worker-1",
        "coord",
        "model",
        "proof",
        10,
        5,
        5,
    );
    settled.sign_with(&coordinator);
    let mut unconfirmed = ComputeReceipt::new(
        "task-unconfirmed",
        &alice_addr,
        "worker-1",
        "coord",
        "model",
        "proof",
        10,
        5,
        5,
    );
    unconfirmed.client_confirmed = false;
    unconfirmed.sign_with(&coordinator);
    assert!(mempool.insert_receipt(settled));
    assert!(mempool.insert_receipt(unconfirmed));

    let accounts = [
        (alice_addr.as_str(), 1_000),
        (carol_addr.as_str(), 1_000),
        (dave_addr.as_str(), 100),
    ];
    let node = start(&accounts, mempool);
    wait(FULL_MEMPOOL_DEADLINE, "valid candidates committed", || {
        node.is_task_settled("task-ok")
            && node.get_account(&alice_addr).map(|a| a.nonce) == Some(5)
            && node.get_account(&dave_addr).map(|a| a.nonce) == Some(1)
    });
    // A few more blocks: the invalid candidates must still not apply.
    let height = node.highest_committed_height().unwrap();
    wait(FULL_MEMPOOL_DEADLINE, "more blocks", || {
        node.highest_committed_height().unwrap() >= height + 3
    });

    assert_eq!(
        node.get_account(&alice_addr).unwrap().nonce,
        5,
        "gap kept out"
    );
    assert_eq!(
        node.get_account(&carol_addr).unwrap().nonce,
        0,
        "forged kept out"
    );
    let dave_acc = node.get_account(&dave_addr).unwrap();
    assert_eq!(
        (dave_acc.nonce, dave_acc.balance),
        (1, 9),
        "unfunded kept out"
    );
    assert!(!node.is_task_settled("task-unconfirmed"));
    // bob: 5 × 10 from alice + 90 from dave.
    assert_eq!(node.get_account("bob").unwrap().balance, 140);
}
