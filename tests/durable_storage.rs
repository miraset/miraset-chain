use ed25519_dalek::SigningKey;
use miraset_chain::core::types::{ComputeReceipt, Transaction};
use miraset_chain::mempool::Mempool;
use miraset_chain::network::{ChannelNetwork, ChannelNetworkHub};
use miraset_chain::node::{MirasetNode, NodeConfig, init_genesis};
use miraset_chain::storage::RedbKVStore;
use std::thread;
use std::time::Duration;

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

    let validators = vec![(vk, 100)];
    let initial_accounts = vec![(alice_addr.as_str(), 20_000), ("bob", 500)];
    init_genesis(kv_store.clone(), &validators, &initial_accounts);

    let mempool = Mempool::new();
    let config = NodeConfig::devnet(signing_key.clone());

    // 1. Start node with durable Redb store
    let node = MirasetNode::start(config, kv_store, network, mempool);

    // Submit a signed transaction
    let mut tx = Transaction::new(&alice_addr, 0, "bob", 2_000, 100);
    tx.sign_with(&alice_key);
    assert!(node.submit_transaction(tx).is_ok());

    // Submit signed compute receipt
    let coord_key = SigningKey::from_bytes(&[99u8; 32]);
    let mut receipt = ComputeReceipt::new(
        "redb_task_01",
        &alice_addr,
        "worker_gpu_redb",
        "coord_secure",
        "model_llama3",
        "proof_zk_ok",
        300,
        500,
        450,
    );
    receipt.sign_with(&coord_key);
    assert!(node.submit_receipt(receipt).is_ok());

    // Allow block to be proposed and committed to Redb
    thread::sleep(Duration::from_millis(1500));

    // Verify state committed
    assert!(node.is_task_settled("redb_task_01"));
    let alice = node.get_account(&alice_addr).expect("alice not found");
    // Initial 20_000 - tx(2_000 + 100) - client_cost(500) = 17_400
    assert_eq!(alice.balance, 17_400);

    let bob = node.get_account("bob").expect("bob not found");
    assert_eq!(bob.balance, 2_500);

    let worker = node
        .get_worker("worker_gpu_redb")
        .expect("worker not found");
    assert_eq!(worker.completed_tasks, 1);
    assert_eq!(worker.rewards_accumulated, 450);

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
    assert_eq!(reloaded_worker.rewards_accumulated, 450);

    drop(node2);
}
