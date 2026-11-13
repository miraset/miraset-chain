//! Block size limit (owner decision 2026-11-13): blocks are bounded by `MAX_BLOCK_BYTES` (8 MiB)
//! of payload instead of 100 transactions and 20 receipts. A backlog larger than one block
//! fills the first block close to the limit, never over it, and the rest follows.

use ed25519_dalek::SigningKey;
use miraset_chain::app::chain_app::{MAX_BLOCK_BYTES, RECEIPT_RESERVED_BYTES};
use miraset_chain::core::types::{BlockPayload, Transaction};
use miraset_chain::mempool::Mempool;
use miraset_chain::network::{ChannelNetwork, ChannelNetworkHub};
use miraset_chain::node::{MirasetNode, NodeConfig, init_genesis_with_params};
use miraset_chain::storage::MemKVStore;
use std::thread;
use std::time::{Duration, Instant};

mod common;
use common::pocc::Pocc;

const VIEW_TIME: Duration = Duration::from_millis(4_000);
const SENDERS: u8 = 20;
const PER_SENDER: u64 = 100;
/// Transaction payload; 2 000 such transfers (≈ 8.2 MB) are more than one block holds.
const TX_PAYLOAD: usize = 4 * 1024;

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn address(key: &SigningKey) -> String {
    hex::encode(key.verifying_key().to_bytes())
}

#[test]
fn test_a_large_backlog_fills_blocks_up_to_max_block_bytes() {
    let senders: Vec<SigningKey> = (0..SENDERS).map(|i| key(120 + i)).collect();
    let addrs: Vec<String> = senders.iter().map(address).collect();
    let validator = key(119);
    let vk = validator.verifying_key();
    let kv = MemKVStore::new();
    let pocc = Pocc::default();
    let parties = pocc.genesis_accounts();
    let mut accounts: Vec<(&str, u64)> = parties.iter().map(|(a, b)| (a.as_str(), *b)).collect();
    accounts.extend(addrs.iter().map(|a| (a.as_str(), 1_000_000)));
    init_genesis_with_params(kv.clone(), 1337, &[(vk, 1)], &accounts, &pocc.params());
    // One block per 2 s, so the whole backlog is pooled before the next block is cut.
    let config = NodeConfig {
        max_view_time: VIEW_TIME,
        min_block_interval: VIEW_TIME / 2,
        ..NodeConfig::devnet(validator)
    };
    let node = MirasetNode::start(
        config,
        kv,
        ChannelNetwork::new(vk, ChannelNetworkHub::new()),
        Mempool::new(),
    );
    let wait_until = |deadline: Duration, what: &str, done: &dyn Fn() -> bool| {
        let start = Instant::now();
        while !done() {
            assert!(start.elapsed() < deadline, "timed out: {what}");
            thread::sleep(Duration::from_millis(20));
        }
    };
    wait_until(Duration::from_secs(20), "first commit", &|| {
        node.highest_committed_height().is_some()
    });
    let start_height = node.highest_committed_height().unwrap();

    for nonce in 0..PER_SENDER {
        for (sender, addr) in senders.iter().zip(&addrs) {
            let mut tx = Transaction::new(addr, nonce, "bob", 1, 1);
            tx.payload = vec![7; TX_PAYLOAD];
            tx.sign_with(sender);
            node.submit_transaction(tx).expect("admitted");
        }
    }
    let total = u64::from(SENDERS) * PER_SENDER;
    wait_until(Duration::from_secs(60), "backlog committed", &|| {
        node.get_account("bob").map(|a| a.balance) == Some(total)
    });

    let snapshot = node.block_tree_camera().snapshot();
    let mut blocks = Vec::new(); // (transactions, payload bytes) of each block with work
    for height in start_height + 1..=node.highest_committed_height().unwrap() {
        let hash = node
            .committed_block_at_height(height)
            .expect("committed block");
        let data = snapshot.block_data(&hash).unwrap().expect("block data");
        let bytes = data.vec()[0].bytes();
        let payload = BlockPayload::from_bytes(bytes).expect("canonical payload");
        if !payload.transactions.is_empty() {
            blocks.push((payload.transactions.len(), bytes.len()));
        }
    }
    eprintln!("blocks with work (transactions, bytes): {blocks:?}");
    assert!(
        blocks.iter().all(|&(_, bytes)| bytes <= MAX_BLOCK_BYTES),
        "a block exceeds {MAX_BLOCK_BYTES} bytes: {blocks:?}"
    );
    let (largest_txs, largest_bytes) = blocks.iter().copied().max_by_key(|b| b.1).unwrap();
    assert!(largest_txs > 120, "still the old count limit: {blocks:?}");
    // Full to within one transaction of the transaction budget.
    let tx_budget = MAX_BLOCK_BYTES - RECEIPT_RESERVED_BYTES;
    assert!(
        largest_bytes > tx_budget - 2 * (TX_PAYLOAD + 1024),
        "largest block {largest_bytes} bytes, budget {tx_budget}"
    );
    assert!(blocks.len() >= 2, "the backlog cannot fit one block");
}
