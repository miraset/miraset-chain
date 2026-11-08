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

    let alice_key = SigningKey::from_bytes(&[2u8; 32]);
    let alice_addr = hex::encode(alice_key.verifying_key().to_bytes());
    let coord_key = SigningKey::from_bytes(&[3u8; 32]);

    let hub = ChannelNetworkHub::new();
    let network = ChannelNetwork::new(vk, hub);
    let kv_store = MemKVStore::new();

    let validators = vec![(vk, 100)];
    let initial_accounts = vec![(alice_addr.as_str(), 10_000), ("bob", 500)];

    init_genesis(kv_store.clone(), &validators, &initial_accounts);

    let mempool = Mempool::new();
    let config = NodeConfig::devnet(keypair);

    let node = MirasetNode::start(config, kv_store, network, mempool);
    let mut commit_rx = node.subscribe_commits();

    // Verify genesis state is queryable
    let alice = node
        .get_account(&alice_addr)
        .expect("Alice account should exist");
    assert_eq!(alice.balance, 10_000);
    assert_eq!(alice.nonce, 0);

    let bob = node.get_account("bob").expect("Bob account should exist");
    assert_eq!(bob.balance, 500);

    // Submit a signed transaction to mempool
    let mut tx = Transaction::new(&alice_addr, 0, "bob", 1000, 50);
    tx.sign_with(&alice_key);
    assert!(node.submit_transaction(tx).is_ok());

    // Submit a coordinator-signed compute receipt to mempool
    let mut receipt = ComputeReceipt::new(
        "compute_task_001",
        &alice_addr,
        "worker_h100",
        "coordinator_local",
        "hash_llama_70b",
        "zkp_proof_verified",
        150,
        250,
        200,
    );
    receipt.sign_with(&coord_key);
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

    let alice = node.get_account(&alice_addr).expect("alice not found");
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

    let alice_key = SigningKey::from_bytes(&[4u8; 32]);
    let alice_addr = hex::encode(alice_key.verifying_key().to_bytes());
    let coord_key = SigningKey::from_bytes(&[5u8; 32]);

    let initial_accounts = vec![(alice_addr.as_str(), 50_000), ("bob", 1_000)];

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

    // Submit a signed transaction
    let mut tx = Transaction::new(&alice_addr, 0, "bob", 5_000, 100);
    tx.sign_with(&alice_key);
    assert!(nodes[0].submit_transaction(tx).is_ok());

    // Submit a coordinator-signed compute receipt
    let mut receipt = ComputeReceipt::new(
        "task_cluster_01",
        &alice_addr,
        "cluster_worker",
        "coordinator_cluster",
        "model_deepseek_r1",
        "proof_cluster_ok",
        200,
        50,
        45,
    );
    receipt.sign_with(&coord_key);
    assert!(nodes[0].submit_receipt(receipt).is_ok());

    // Allow consensus rounds between the 3 validators to progress
    thread::sleep(Duration::from_millis(2500));

    // Verify on all 3 nodes that task was settled and alice was debited across consensus
    for node in &nodes {
        assert!(node.is_task_settled("task_cluster_01"));
        let alice = node.get_account(&alice_addr).expect("alice not found");
        // 50_000 - tx(5_000 + 100) - client_cost(50) = 44_850
        assert_eq!(alice.balance, 44_850);
        let worker = node.get_worker("cluster_worker").expect("worker not found");
        assert_eq!(worker.completed_tasks, 1);
        assert_eq!(worker.rewards_accumulated, 45);
    }

    // Clean shutdown of all nodes
    drop(nodes);
}

#[test]
fn test_worker_bond_registration_unbonding_and_slashing() {
    let keypair = SigningKey::from_bytes(&[8u8; 32]);
    let vk = keypair.verifying_key();

    let worker_owner_key = SigningKey::from_bytes(&[9u8; 32]);
    let worker_owner_addr = hex::encode(worker_owner_key.verifying_key().to_bytes());

    let hub = ChannelNetworkHub::new();
    let network = ChannelNetwork::new(vk, hub);
    let kv_store = MemKVStore::new();

    let validators = vec![(vk, 100)];
    let initial_accounts = vec![(worker_owner_addr.as_str(), 10_000)];

    init_genesis(kv_store.clone(), &validators, &initial_accounts);

    let mempool = Mempool::new();
    let config = NodeConfig::devnet(keypair);

    let node = MirasetNode::start(config, kv_store, network, mempool);

    // 1. Submit Worker Bond Registration (500 MIR stake)
    let bond_action = miraset_chain::core::types::WorkerBondAction::Register {
        worker_id: "gpu_worker_pro".to_string(),
        bond_amount: 500,
        vram_gib: 80,
        endpoint: "10.0.0.5:9090".to_string(),
    };
    let mut bond_tx = Transaction::new_worker_bond(&worker_owner_addr, 0, 10, bond_action).unwrap();
    bond_tx.sign_with(&worker_owner_key);
    assert!(node.submit_transaction(bond_tx).is_ok());

    thread::sleep(Duration::from_millis(1500));

    // Verify worker registration committed
    let worker = node
        .get_worker("gpu_worker_pro")
        .expect("worker should be registered");
    assert_eq!(worker.stake, 500);
    assert!(worker.active);
    assert_eq!(worker.vram_gib, 80);
    assert_eq!(worker.owner_address, worker_owner_addr);
    assert_eq!(worker.endpoint, "10.0.0.5:9090");

    let owner_acc = node.get_account(&worker_owner_addr).unwrap();
    // 10_000 - bond(500) - fee(10) = 9_490
    assert_eq!(owner_acc.balance, 9_490);
    assert_eq!(owner_acc.nonce, 1);

    // 2. Submit partial Unbond (100 MIR)
    let unbond_action = miraset_chain::core::types::WorkerBondAction::Unbond {
        worker_id: "gpu_worker_pro".to_string(),
        amount: 100,
    };
    let mut unbond_tx =
        Transaction::new_worker_bond(&worker_owner_addr, 1, 10, unbond_action).unwrap();
    unbond_tx.sign_with(&worker_owner_key);
    assert!(node.submit_transaction(unbond_tx).is_ok());

    thread::sleep(Duration::from_millis(1500));

    let worker = node.get_worker("gpu_worker_pro").unwrap();
    assert_eq!(worker.stake, 400); // 500 - 100
    assert!(worker.active);

    let owner_acc = node.get_account(&worker_owner_addr).unwrap();
    // 9_490 + unbond(100) - fee(10) = 9_580
    assert_eq!(owner_acc.balance, 9_580);
    assert_eq!(owner_acc.nonce, 2);

    // 3. Submit Slash (10% base slash = 40 MIR)
    let slash_action = miraset_chain::core::types::WorkerBondAction::Slash {
        worker_id: "gpu_worker_pro".to_string(),
        slash_amount: 40,
        reason: "task_timeout_delivery_failure".to_string(),
    };
    let mut slash_tx =
        Transaction::new_worker_bond(&worker_owner_addr, 2, 5, slash_action).unwrap();
    slash_tx.sign_with(&worker_owner_key);
    assert!(node.submit_transaction(slash_tx).is_ok());

    thread::sleep(Duration::from_millis(1500));

    let worker = node.get_worker("gpu_worker_pro").unwrap();
    assert_eq!(worker.stake, 360); // 400 - 40
    assert!(worker.active); // > 100 MIN_WORKER_BOND

    drop(node);
}

#[test]
fn test_epoch_token_accumulation_and_system_rollover() {
    let keypair = SigningKey::from_bytes(&[10u8; 32]);
    let vk = keypair.verifying_key();

    let owner_key = SigningKey::from_bytes(&[11u8; 32]);
    let owner_addr = hex::encode(owner_key.verifying_key().to_bytes());

    let client_key = SigningKey::from_bytes(&[12u8; 32]);
    let client_addr = hex::encode(client_key.verifying_key().to_bytes());

    let coord_key = SigningKey::from_bytes(&[13u8; 32]);

    let hub = ChannelNetworkHub::new();
    let network = ChannelNetwork::new(vk, hub);
    let kv_store = MemKVStore::new();

    let validators = vec![(vk, 100)];
    let initial_accounts = vec![(owner_addr.as_str(), 10_000), (client_addr.as_str(), 5_000)];

    init_genesis(kv_store.clone(), &validators, &initial_accounts);

    let mempool = Mempool::new();
    let config = NodeConfig::devnet(keypair);

    let node = MirasetNode::start(config, kv_store, network, mempool);

    // 1. Register worker with bond
    let bond_action = miraset_chain::core::types::WorkerBondAction::Register {
        worker_id: "epoch_worker_1".to_string(),
        bond_amount: 500,
        vram_gib: 80,
        endpoint: "127.0.0.1:9090".to_string(),
    };
    let mut reg_tx = Transaction::new_worker_bond(&owner_addr, 0, 10, bond_action).unwrap();
    reg_tx.sign_with(&owner_key);
    assert!(node.submit_transaction(reg_tx).is_ok());

    thread::sleep(Duration::from_millis(1500));

    // Verify worker registered
    let worker = node
        .get_worker("epoch_worker_1")
        .expect("worker should exist");
    assert_eq!(worker.stake, 500);
    assert_eq!(worker.epoch_tokens, 0);

    // 2. Submit compute receipt (1000 tokens computed)
    let mut receipt = ComputeReceipt::new(
        "task_epoch_001",
        &client_addr,
        "epoch_worker_1",
        "coord_1",
        "model_llama_3",
        "zkp_proof",
        1000, // tokens_computed
        50,   // client_cost
        40,   // worker_reward
    );
    receipt.sign_with(&coord_key);
    assert!(node.submit_receipt(receipt).is_ok());

    thread::sleep(Duration::from_millis(1500));

    // Verify worker epoch_tokens and epoch accumulation
    let worker = node.get_worker("epoch_worker_1").unwrap();
    assert_eq!(worker.epoch_tokens, 1000);

    let epoch = node
        .get_current_epoch()
        .expect("current epoch should exist");
    assert_eq!(epoch.epoch_id, 0);
    assert_eq!(epoch.total_tokens, 1000);
    assert_eq!(epoch.total_receipts, 1);

    // 3. Submit epoch rollover transaction to system:epoch_manager with target epoch 0
    let mut rollover_tx = Transaction::new(
        &owner_addr,
        1,
        "system:epoch_manager",
        0,  // amount
        10, // fee
    );
    rollover_tx.payload = 0u64.to_be_bytes().to_vec();
    rollover_tx.sign_with(&owner_key);
    assert!(node.submit_transaction(rollover_tx).is_ok());

    thread::sleep(Duration::from_millis(1500));

    // Verify epoch has advanced to 1
    let next_epoch = node.get_current_epoch().expect("next epoch should exist");
    assert_eq!(next_epoch.epoch_id, 1);
    assert_eq!(next_epoch.total_tokens, 0);
    assert_eq!(
        next_epoch.status,
        miraset_chain::core::types::EpochStatus::Active
    );

    // Verify historical epoch 0 is stored and finalized
    let finalized_epoch = node.get_epoch(0).expect("finalized epoch 0 should exist");
    assert_eq!(finalized_epoch.epoch_id, 0);
    assert_eq!(
        finalized_epoch.status,
        miraset_chain::core::types::EpochStatus::Finalized
    );
    assert!(finalized_epoch.distribution.is_some());

    // Verify worker's epoch_tokens reset to 0 for epoch 1
    let worker_after = node.get_worker("epoch_worker_1").unwrap();
    assert_eq!(worker_after.epoch_tokens, 0);
    assert!(worker_after.rewards_accumulated > 0);

    drop(node);
}
