use ed25519_dalek::SigningKey;
use miraset_chain::core::types::Transaction;
use miraset_chain::mempool::Mempool;
use miraset_chain::network::{ChannelNetwork, ChannelNetworkHub};
use miraset_chain::node::{MirasetNode, NodeConfig, init_genesis_with_params};
use miraset_chain::storage::RedbKVStore;
use std::thread;
use std::time::{Duration, Instant};

mod common;
use common::pocc::Pocc;

fn wait_for(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn test_node_consensus_with_durable_redb() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let db_path = tmp.path().to_path_buf();

    let signing_key = SigningKey::from_bytes(&[10u8; 32]);
    let vk = signing_key.verifying_key();
    let alice_key = SigningKey::from_bytes(&[11u8; 32]);
    let alice_addr = hex::encode(alice_key.verifying_key().to_bytes());
    let hub = ChannelNetworkHub::new();
    let network = ChannelNetwork::new(vk, hub);
    let kv_store = RedbKVStore::open(&db_path).expect("failed to open redb store");

    let pocc = Pocc::new("worker_gpu_redb", 206);
    let validators = vec![(vk, 100)];
    let parties = pocc.genesis_accounts();
    let mut initial_accounts: Vec<(&str, u64)> =
        parties.iter().map(|(a, b)| (a.as_str(), *b)).collect();
    initial_accounts.extend([(alice_addr.as_str(), 20_000), ("bob", 500)]);
    init_genesis_with_params(
        kv_store.clone(),
        1337,
        &validators,
        &initial_accounts,
        &pocc.params(),
    );

    let mempool = Mempool::new();
    let config = NodeConfig::devnet(signing_key.clone());

    // 1. Start node with durable Redb store
    let node = MirasetNode::start(config, kv_store, network, mempool);

    assert!(node.submit_transaction(pocc.register(0)).is_ok());
    wait_for("worker registration", || {
        node.get_worker("worker_gpu_redb").is_some()
    });

    // A signed transfer and a 250-token receipt (cost 500) paid by alice.
    let mut tx = Transaction::new(&alice_addr, 0, "bob", 2_000, 100);
    tx.sign_with(&alice_key);
    assert!(node.submit_transaction(tx).is_ok());
    assert!(
        node.submit_receipt(pocc.receipt("redb_task_01", &alice_addr, 250))
            .is_ok()
    );
    wait_for("transfer and receipt", || {
        node.is_task_settled("redb_task_01")
            && node.get_account("bob").map(|a| a.balance) == Some(2_500)
    });

    // Initial 20_000 - transfer(2_000 + 100) - job cost(500) = 17_400
    assert_eq!(node.get_account(&alice_addr).unwrap().balance, 17_400);
    let worker = node
        .get_worker("worker_gpu_redb")
        .expect("worker not found");
    assert_eq!((worker.completed_tasks, worker.epoch_tokens), (1, 250));

    let highest_qc_block = node.highest_committed_block();
    assert!(highest_qc_block.is_some());

    // Clean drop of active node
    drop(node);

    // 2. Simulate Node restart: Re-open Redb from disk and verify committed state is intact
    let reloaded_kv = RedbKVStore::open(&db_path).expect("failed to reopen redb");
    let net2 = ChannelNetwork::new(vk, ChannelNetworkHub::new());
    let pool2 = Mempool::new();
    let config2 = NodeConfig::devnet(signing_key);

    let node2 = MirasetNode::start(config2, reloaded_kv, net2, pool2);

    // Verify persisted state survived the restart
    assert!(node2.is_task_settled("redb_task_01"));
    let reloaded_alice = node2
        .get_account(&alice_addr)
        .expect("reloaded alice not found");
    assert_eq!(reloaded_alice.balance, 17_400);

    let reloaded_worker = node2
        .get_worker("worker_gpu_redb")
        .expect("reloaded worker not found");
    assert_eq!(reloaded_worker.completed_tasks, 1);
    assert_eq!(reloaded_worker.epoch_tokens, 250);

    drop(node2);

    // 3. Verify fail-closed recovery when chain_id mismatches persisted genesis metadata
    let reloaded_kv_mismatch = RedbKVStore::open(&db_path).expect("failed to reopen redb");
    let net3 = ChannelNetwork::new(vk, ChannelNetworkHub::new());
    let pool3 = Mempool::new();
    let mut bad_config = NodeConfig::devnet(SigningKey::from_bytes(&[10u8; 32]));
    bad_config.chain_id = 9999;
    assert!(MirasetNode::try_start(bad_config, reloaded_kv_mismatch, net3, pool3).is_err());
}
