use ed25519_dalek::SigningKey;
use std::thread;
use std::time::Duration;

use miraset_chain::core::types::{ComputeReceipt, Transaction};
use miraset_chain::mempool::Mempool;
use miraset_chain::network::ChannelNetwork;
use miraset_chain::network::ChannelNetworkHub;
use miraset_chain::node::{MirasetNode, NodeConfig, init_genesis};
use miraset_chain::storage::MemKVStore;

#[test]
fn test_single_node_genesis_and_query() {
    let keypair = SigningKey::from_bytes(&[1u8; 32]);
    let vk = keypair.verifying_key();

    let hub = ChannelNetworkHub::new();
    let network = ChannelNetwork::new(vk, hub);
    let kv_store = MemKVStore::new();

    let validators = vec![(vk, 100)];
    let initial_accounts = vec![("alice", 10_000), ("bob", 500)];

    init_genesis(kv_store.clone(), &validators, &initial_accounts);

    let mempool = Mempool::new();
    let config = NodeConfig::devnet(keypair);

    let node = MirasetNode::start(config, kv_store, network, mempool);
    let mut commit_rx = node.subscribe_commits();

    // Verify genesis state is queryable
    let alice = node
        .get_account("alice")
        .expect("Alice account should exist");
    assert_eq!(alice.balance, 10_000);
    assert_eq!(alice.nonce, 0);

    let bob = node.get_account("bob").expect("Bob account should exist");
    assert_eq!(bob.balance, 500);

    // Submit transaction to mempool
    let tx = Transaction::new("alice", 0, "bob", 1000, 50);
    assert!(node.submit_transaction(tx).is_ok());

    // Submit compute receipt to mempool
    let receipt = ComputeReceipt::new(
        "compute_task_001",
        "alice",
        "worker_h100",
        "coordinator_local",
        "hash_llama_70b",
        "zkp_proof_verified",
        150,
        250,
        200,
    );
    assert!(node.submit_receipt(receipt).is_ok());

    // Allow background consensus threads to run and commit blocks
    thread::sleep(Duration::from_millis(1500));

    // Verify commit event was broadcast and received
    let mut received_event = false;
    while let Ok(evt) = commit_rx.try_recv() {
        if !evt.block_hash.is_empty() {
            received_event = true;
            break;
        }
    }
    assert!(
        received_event,
        "Expected to receive at least one commit event"
    );

    // Verify task settlement and client debit / worker reward
    assert!(node.is_task_settled("compute_task_001"));

    let worker = node.get_worker("worker_h100").expect("worker not recorded");
    assert_eq!(worker.completed_tasks, 1);
    assert_eq!(worker.rewards_accumulated, 200);

    let alice = node.get_account("alice").expect("alice not found");
    // Initial 10_000 - tx(1000 + 50) - client_cost(250) = 8_700
    assert_eq!(alice.balance, 8_700);

    let bob = node.get_account("bob").expect("bob not found");
    assert_eq!(bob.balance, 1_500);

    // Clean drop of the node
    drop(node);
}

#[test]
fn test_three_validator_cluster_consensus() {
    let hub = ChannelNetworkHub::new();

    let keypairs: Vec<SigningKey> = (1..=3)
        .map(|i| SigningKey::from_bytes(&[i as u8; 32]))
        .collect();

    let validators: Vec<_> = keypairs
        .iter()
        .map(|kp| (kp.verifying_key(), 100))
        .collect();

    let initial_accounts = vec![("alice", 50_000), ("bob", 1_000)];

    let mut nodes = Vec::new();

    for kp in keypairs {
        let vk = kp.verifying_key();
        let kv = MemKVStore::new();
        init_genesis(kv.clone(), &validators, &initial_accounts);

        let net = ChannelNetwork::new(vk, hub.clone());
        let pool = Mempool::new();
        let config = NodeConfig::devnet(kp);

        let node = MirasetNode::start(config, kv, net, pool);
        nodes.push(node);
    }

    // Submit transaction
    let tx = Transaction::new("alice", 0, "bob", 5_000, 100);
    assert!(nodes[0].submit_transaction(tx).is_ok());

    // Submit compute receipt
    let receipt = ComputeReceipt::new(
        "task_cluster_01",
        "alice",
        "cluster_worker",
        "coordinator_cluster",
        "model_deepseek_r1",
        "proof_cluster_ok",
        200,
        50,
        45,
    );
    assert!(nodes[0].submit_receipt(receipt).is_ok());

    // Allow consensus rounds between the 3 validators to progress
    thread::sleep(Duration::from_millis(2500));

    // Verify on all 3 nodes that task was settled and alice was debited across consensus
    for node in &nodes {
        assert!(node.is_task_settled("task_cluster_01"));
        let alice = node.get_account("alice").expect("alice not found");
        // 50_000 - tx(5_000 + 100) - client_cost(50) = 44_850
        assert_eq!(alice.balance, 44_850);
        let worker = node.get_worker("cluster_worker").expect("worker not found");
        assert_eq!(worker.completed_tasks, 1);
        assert_eq!(worker.rewards_accumulated, 45);
    }

    // Clean shutdown of all nodes
    drop(nodes);
}
