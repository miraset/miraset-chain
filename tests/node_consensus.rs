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
    let receipt = ComputeReceipt {
        task_id: "compute_task_001".to_string(),
        worker_id: "worker_h100".to_string(),
        model_hash: "hash_llama_70b".to_string(),
        proof_hash: "zkp_proof_verified".to_string(),
        gas_reward: 200,
    };
    assert!(node.submit_receipt(receipt).is_ok());

    // Allow background consensus threads to run for a short duration
    thread::sleep(Duration::from_millis(500));

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
    let receipt = ComputeReceipt {
        task_id: "task_cluster_01".to_string(),
        worker_id: "cluster_worker".to_string(),
        model_hash: "model_deepseek_r1".to_string(),
        proof_hash: "proof_cluster_ok".to_string(),
        gas_reward: 500,
    };
    assert!(nodes[0].submit_receipt(receipt).is_ok());

    // Allow consensus rounds between the 3 validators to progress
    thread::sleep(Duration::from_millis(2500));

    // Clean shutdown of all nodes
    drop(nodes);
}
