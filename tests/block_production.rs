//! Block production selects mempool candidates greedily and in linear work (TODO 8.5.3 finding
//! b, PRD R04): each candidate is kept if it applies on top of those kept before it, and a full
//! mempool no longer makes a leader miss its views.

use ed25519_dalek::SigningKey;
use miraset_chain::core::types::Transaction;
use miraset_chain::mempool::{InsertError, Mempool};
use miraset_chain::network::{ChannelNetwork, ChannelNetworkHub};
use miraset_chain::node::{MirasetNode, NodeConfig, init_genesis_with_params};

mod common;
use common::pocc::Pocc;
use miraset_chain::storage::MemKVStore;
use sha2::{Digest, Sha256};
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
    let pocc = Pocc::default();
    let parties = pocc.genesis_accounts();
    let mut all: Vec<(&str, u64)> = parties.iter().map(|(a, b)| (a.as_str(), *b)).collect();
    all.extend_from_slice(accounts);
    init_genesis_with_params(kv.clone(), 1337, &[(vk, 1)], &all, &pocc.params());
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
        assert!(mempool.insert_tx(transfer(&alice, nonce, "bob", 1)).is_ok());
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
    let (alice_addr, carol_addr, dave_addr) = (address(&alice), address(&carol), address(&dave));

    let mempool = Mempool::new();
    // Alice: nonces 0..5 apply in order; the pool refuses nonce 7 (it would leave a gap).
    for nonce in 0..5 {
        assert!(
            mempool
                .insert_tx(transfer(&alice, nonce, "bob", 10))
                .is_ok()
        );
    }
    assert_eq!(
        mempool.insert_tx(transfer(&alice, 7, "bob", 10)),
        Err(InsertError::NonceGap)
    );
    // Carol: signed by someone else (forged sender).
    let mut forged = Transaction::new(&carol_addr, 0, "bob", 10, 1);
    forged.sign_with(&forger);
    assert!(mempool.insert_tx(forged).is_ok());
    // Dave: first transfer spends almost everything, the second cannot be funded after it.
    assert!(mempool.insert_tx(transfer(&dave, 0, "bob", 90)).is_ok());
    assert!(mempool.insert_tx(transfer(&dave, 1, "bob", 90)).is_ok());
    // Receipts: one valid; one whose client did not confirm delivery.
    // Receipts (fixture coordinator and worker, registered in the same block): one valid; one
    // whose client did not confirm delivery.
    let pocc = Pocc::default();
    assert!(mempool.insert_tx(pocc.register(0)).is_ok());
    // Alice's nonce 5 allows the coordinator to charge her (applied before receipts).
    assert!(mempool.insert_tx(pocc.allow(&alice, 5, 1_000)).is_ok());
    let settled = pocc.receipt("task-ok", &alice_addr, 5);
    let mut unconfirmed = pocc.receipt("task-unconfirmed", &alice_addr, 5);
    unconfirmed.client_confirmed = false;
    pocc.sign(&mut unconfirmed);
    assert!(mempool.insert_receipt(settled).is_ok());
    assert!(mempool.insert_receipt(unconfirmed).is_ok());

    let accounts = [
        (alice_addr.as_str(), 1_000),
        (carol_addr.as_str(), 1_000),
        (dave_addr.as_str(), 100),
    ];
    let node = start(&accounts, mempool);
    wait(FULL_MEMPOOL_DEADLINE, "valid candidates committed", || {
        node.is_task_settled("task-ok")
            && node.get_account(&alice_addr).map(|a| a.nonce) == Some(6)
            && node.get_account(&dave_addr).map(|a| a.nonce) == Some(1)
    });
    // A few more blocks: the invalid candidates must still not apply.
    let height = node.highest_committed_height().unwrap();
    wait(FULL_MEMPOOL_DEADLINE, "more blocks", || {
        node.highest_committed_height().unwrap() >= height + 3
    });

    assert_eq!(
        node.get_account(&alice_addr).unwrap().nonce,
        6,
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

/// TODO 8.4.3: chains that cannot apply must not keep a valid transaction out of every block,
/// whatever they pay, and however many there are. 500 senders offer a high fee at a nonce their account has not reached
/// (as a chain left over from another node's commits would); an honest low-fee transfer must
/// still commit.
#[test]
fn test_unappliable_high_fee_chains_do_not_crowd_out_valid_transactions() {
    let alice = key(47);
    let alice_addr = address(&alice);
    let mempool = Mempool::new();
    for seed in 0..500u32 {
        let bytes = Sha256::digest(seed.to_be_bytes());
        let squatter = SigningKey::from_bytes(&bytes.into());
        let mut tx = Transaction::new(address(&squatter), 1, "bob", 1, 1_000);
        tx.sign_with(&squatter);
        assert!(mempool.insert_tx(tx).is_ok());
    }
    assert!(mempool.insert_tx(transfer(&alice, 0, "bob", 7)).is_ok());

    let node = start(&[(alice_addr.as_str(), BALANCE)], mempool);
    wait(FULL_MEMPOOL_DEADLINE, "honest transfer committed", || {
        node.get_account("bob").map(|a| a.balance) == Some(7)
    });
}

/// Review finding N1 (TODO 8.4.3): transactions that pass admission but fail execution (here a
/// malformed system payload) must leave the pool instead of failing in every block, so many
/// high-fee ones cannot exhaust the producer's failure budget and keep a valid transaction out.
#[test]
fn test_candidates_that_fail_execution_are_evicted_and_do_not_block_progress() {
    let alice = key(48);
    let alice_addr = address(&alice);
    let squatters: Vec<SigningKey> = (0..300u32)
        .map(|seed| SigningKey::from_bytes(&Sha256::digest((seed + 1_000).to_be_bytes()).into()))
        .collect();
    let addresses: Vec<String> = squatters.iter().map(address).collect();
    let mut accounts: Vec<(&str, u64)> = addresses.iter().map(|a| (a.as_str(), 10_000)).collect();
    accounts.push((alice_addr.as_str(), BALANCE));

    let mempool = Mempool::new();
    for squatter in &squatters {
        let mut tx = Transaction::new(address(squatter), 0, "system:worker_registry", 1, 1_000);
        tx.payload = b"not a worker action".to_vec();
        tx.sign_with(squatter);
        assert!(mempool.insert_tx(tx).is_ok());
    }
    assert!(mempool.insert_tx(transfer(&alice, 0, "bob", 7)).is_ok());

    let node = start(&accounts, mempool.clone());
    wait(FULL_MEMPOOL_DEADLINE, "honest transfer committed", || {
        node.get_account("bob").map(|a| a.balance) == Some(7)
    });
    wait(FULL_MEMPOOL_DEADLINE, "failed candidates evicted", || {
        mempool.tx_count() == 0
    });
    assert!(
        addresses
            .iter()
            .all(|a| node.get_account(a).map(|acc| acc.nonce) == Some(0))
    );
}
