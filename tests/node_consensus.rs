use ed25519_dalek::SigningKey;
use std::thread;
use std::time::{Duration, Instant};

use miraset_chain::core::types::{ComputeReceipt, Transaction};
use miraset_chain::mempool::Mempool;
use miraset_chain::network::ChannelNetwork;
use miraset_chain::network::ChannelNetworkHub;
use miraset_chain::node::{MirasetNode, NodeConfig, init_genesis};
use miraset_chain::storage::MemKVStore;

/// Upper bound for a consensus-dependent condition; tests return as soon as it holds.
const CONSENSUS_TIMEOUT: Duration = Duration::from_secs(20);
const POLL_INTERVAL: Duration = Duration::from_millis(25);

/// Poll `condition` until it holds or `timeout` elapses. Returns whether it held.
fn wait_until(timeout: Duration, mut condition: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if condition() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(POLL_INTERVAL);
    }
}

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

    // Wait for background consensus to commit the transaction and receipt
    wait_until(CONSENSUS_TIMEOUT, || {
        node.is_task_settled("compute_task_001")
            && node.get_account("bob").map(|a| a.balance) == Some(1_500)
    });

    // Verify commit events were broadcast with the committed payload counts. Commit events read
    // block data back from the KV store, so this also guards the `block_data` compat override.
    let mut received_event = false;
    let (mut committed_txs, mut committed_receipts) = (0, 0);
    wait_until(CONSENSUS_TIMEOUT, || {
        while let Ok(evt) = commit_rx.try_recv() {
            if !evt.block_hash.is_empty() {
                received_event = true;
                committed_txs += evt.tx_count;
                committed_receipts += evt.receipt_count;
            }
        }
        committed_txs == 1 && committed_receipts == 1
    });
    assert_eq!((committed_txs, committed_receipts), (1, 1));
    // Committed entries are pruned from the mempool by the commit handler.
    assert_eq!(node.mempool().tx_count(), 0);
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

    // Wait for consensus rounds between the 3 validators to settle the task everywhere
    wait_until(CONSENSUS_TIMEOUT, || {
        nodes.iter().all(|n| {
            n.is_task_settled("task_cluster_01")
                && n.get_account(&alice_addr).map(|a| a.balance) == Some(44_850)
        })
    });

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

    wait_until(CONSENSUS_TIMEOUT, || {
        node.get_worker("gpu_worker_pro").is_some()
    });

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

    wait_until(CONSENSUS_TIMEOUT, || {
        node.get_worker("gpu_worker_pro").map(|w| w.stake) == Some(400)
    });

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

    wait_until(CONSENSUS_TIMEOUT, || {
        node.get_worker("gpu_worker_pro").map(|w| w.stake) == Some(360)
    });

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

    wait_until(CONSENSUS_TIMEOUT, || {
        node.get_worker("epoch_worker_1").is_some()
    });

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

    wait_until(CONSENSUS_TIMEOUT, || {
        node.get_current_epoch().map(|e| e.total_receipts) == Some(1)
    });

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

    wait_until(CONSENSUS_TIMEOUT, || {
        node.get_current_epoch().map(|e| e.epoch_id) == Some(1)
    });

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

#[test]
fn test_four_validator_fault_bound_and_partition_healing() {
    // n = 4 validators: 3 honest validators (power 3 each = 9) and 1 faulty/isolated
    // validator (power 1, < 1/3 of total 10 stake; quorum >= 7).
    // With InterleavedWrr, threshold=2 (views 4..7) and threshold=3 (views 7..10)
    // guarantee 6 consecutive honest leader slots regardless of VerifyingKey sort order.
    let hub = ChannelNetworkHub::new();

    let keypairs: Vec<SigningKey> = (21..=24)
        .map(|i| SigningKey::from_bytes(&[i as u8; 32]))
        .collect();
    let vks: Vec<_> = keypairs.iter().map(|kp| kp.verifying_key()).collect();
    let validators = vec![(vks[0], 3), (vks[1], 3), (vks[2], 3), (vks[3], 1)];

    let alice_key = SigningKey::from_bytes(&[25u8; 32]);
    let alice_addr = hex::encode(alice_key.verifying_key().to_bytes());
    let initial_accounts = vec![(alice_addr.as_str(), 100_000), ("bob", 1_000)];

    // Isolate validator 3 (faulty/unreachable, within < 1/3 stake bound)
    hub.isolate_peer(vks[3]);

    let mut nodes = Vec::new();
    for kp in keypairs {
        let vk = kp.verifying_key();
        let kv = MemKVStore::new();
        init_genesis(kv.clone(), &validators, &initial_accounts);
        let net = ChannelNetwork::new(vk, hub.clone());
        let pool = Mempool::new();
        // Sub-second view time: regression for the vendored pacemaker patch (upstream truncated
        // 350ms to 0s, so no view ever completed and nothing committed).
        let config = NodeConfig {
            max_view_time: Duration::from_millis(350),
            ..NodeConfig::devnet(kp)
        };
        nodes.push(MirasetNode::start(config, kv, net, pool));
    }

    // Gossip transaction to the honest validators' mempools while validator 3 is isolated
    let mut tx = Transaction::new(&alice_addr, 0, "bob", 2_000, 50);
    tx.sign_with(&alice_key);
    for honest_node in &nodes[..3] {
        assert!(honest_node.submit_transaction(tx.clone()).is_ok());
    }

    // Wait for honest 3-of-4 quorum to advance views around the isolated validator and commit
    wait_until(CONSENSUS_TIMEOUT, || {
        nodes[..3]
            .iter()
            .all(|n| n.get_account("bob").map(|a| a.balance) == Some(3_000))
    });

    // Verify all 3 honest validators finalized the transaction identically with no conflicting finality
    for honest_node in &nodes[..3] {
        let bob = honest_node.get_account("bob").expect("bob should exist");
        assert_eq!(bob.balance, 3_000);
        let alice = honest_node
            .get_account(&alice_addr)
            .expect("alice should exist");
        assert_eq!(alice.balance, 97_950);
        assert!(honest_node.highest_committed_block().is_some());
    }

    // Heal the partition so validator 3 rejoins and catches up via BlockSync/consensus
    hub.clear_faults();
    let mut tx2 = Transaction::new(&alice_addr, 1, "bob", 1_000, 50);
    tx2.sign_with(&alice_key);
    for honest_node in &nodes[..3] {
        assert!(honest_node.submit_transaction(tx2.clone()).is_ok());
    }

    wait_until(CONSENSUS_TIMEOUT, || {
        nodes
            .iter()
            .all(|n| n.get_account("bob").map(|a| a.balance) == Some(4_000))
    });

    // Verify all 4 validators (including the healed lagging validator) converge on the finalized state
    for node in &nodes {
        let bob = node
            .get_account("bob")
            .expect("bob should exist on all nodes");
        assert_eq!(bob.balance, 4_000);
        let alice = node
            .get_account(&alice_addr)
            .expect("alice should exist on all nodes");
        assert_eq!(alice.balance, 96_900);
        assert_eq!(alice.nonce, 2);
    }

    drop(nodes);
}
