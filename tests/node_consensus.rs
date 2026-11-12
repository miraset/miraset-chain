use ed25519_dalek::SigningKey;
use std::thread;
use std::time::{Duration, Instant};

use miraset_chain::core::types::Transaction;
use miraset_chain::mempool::Mempool;
use miraset_chain::network::ChannelNetwork;
use miraset_chain::network::ChannelNetworkHub;
use miraset_chain::node::{
    AdmissionError, MirasetNode, NodeConfig, init_genesis, init_genesis_with_params,
};

mod common;
use common::pocc::{EPOCH_REWARD_BUDGET, Pocc, address, wait_epoch_over};

fn pocc_coordinator(pocc: &Pocc) -> String {
    address(&pocc.coordinator)
}
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

/// Single validator with the PoCC parties of `common::pocc` in genesis.
fn start_single(
    seed: u8,
    pocc: &Pocc,
    extra: &[(&str, u64)],
) -> MirasetNode<MemKVStore, ChannelNetwork> {
    let keypair = SigningKey::from_bytes(&[seed; 32]);
    let vk = keypair.verifying_key();
    let kv_store = MemKVStore::new();
    let parties = pocc.genesis_accounts();
    let mut accounts: Vec<(&str, u64)> = parties.iter().map(|(a, b)| (a.as_str(), *b)).collect();
    accounts.extend_from_slice(extra);
    init_genesis_with_params(
        kv_store.clone(),
        1337,
        &[(vk, 100)],
        &accounts,
        &pocc.params(),
    );
    let network = ChannelNetwork::new(vk, ChannelNetworkHub::new());
    MirasetNode::start(
        NodeConfig::devnet(keypair),
        kv_store,
        network,
        Mempool::new(),
    )
}

fn register_worker(node: &MirasetNode<MemKVStore, ChannelNetwork>, pocc: &Pocc) {
    assert!(node.submit_transaction(pocc.register(0)).is_ok());
    assert!(
        wait_until(CONSENSUS_TIMEOUT, || node
            .get_worker(&pocc.worker_id)
            .is_some()),
        "worker registration not committed"
    );
}

#[test]
fn test_single_node_genesis_and_query() {
    let pocc = Pocc::default();
    let alice_key = SigningKey::from_bytes(&[2u8; 32]);
    let alice_addr = hex::encode(alice_key.verifying_key().to_bytes());
    let node = start_single(1, &pocc, &[(alice_addr.as_str(), 10_000), ("bob", 500)]);

    // Genesis state is queryable.
    let alice = node
        .get_account(&alice_addr)
        .expect("Alice account should exist");
    assert_eq!((alice.balance, alice.nonce), (10_000, 0));
    assert_eq!(node.get_account("bob").unwrap().balance, 500);

    register_worker(&node, &pocc);
    let owner_before = node.get_account(&pocc.owner_address()).unwrap().balance;
    let mut commit_rx = node.subscribe_commits();

    // Alice allows the coordinator to charge her (nonce 0), sends a transfer (nonce 1), and
    // pays a receipt for 150 tokens (cost 150 x 2 = 300).
    assert!(
        node.submit_transaction(pocc.allow(&alice_key, 0, 1_000))
            .is_ok()
    );
    let mut tx = Transaction::new(&alice_addr, 1, "bob", 1000, 50);
    tx.sign_with(&alice_key);
    assert!(node.submit_transaction(tx).is_ok());
    wait_until(CONSENSUS_TIMEOUT, || {
        node.allowance(&alice_addr, &pocc_coordinator(&pocc)) == 1_000
    });
    assert!(
        node.submit_receipt(pocc.receipt("compute_task_001", &alice_addr, 150))
            .is_ok()
    );

    wait_until(CONSENSUS_TIMEOUT, || {
        node.is_task_settled("compute_task_001")
            && node.get_account("bob").map(|a| a.balance) == Some(1_500)
    });

    // Commit events carry the committed payload counts. Commit events read block data back from
    // the KV store, so this also guards the `block_data` compat override.
    let (mut committed_txs, mut committed_receipts) = (0, 0);
    wait_until(CONSENSUS_TIMEOUT, || {
        while let Ok(evt) = commit_rx.try_recv() {
            committed_txs += evt.tx_count;
            committed_receipts += evt.receipt_count;
        }
        committed_txs == 2 && committed_receipts == 1
    });
    assert_eq!((committed_txs, committed_receipts), (2, 1));
    assert_eq!(node.mempool().tx_count(), 0);

    // REWARDS §6: the client pays tokens x price and the worker's owner receives exactly that.
    assert!(node.is_task_settled("compute_task_001"));
    let worker = node.get_worker(&pocc.worker_id).expect("worker");
    assert_eq!((worker.completed_tasks, worker.epoch_tokens), (1, 150));
    // 10_000 - allowance fee(1) - transfer(1000 + 50) - job cost(300) = 8_649; the allowance
    // shrinks by the cost.
    assert_eq!(node.get_account(&alice_addr).unwrap().balance, 8_649);
    assert_eq!(node.allowance(&alice_addr, &pocc_coordinator(&pocc)), 700);
    assert_eq!(
        node.get_account(&pocc.owner_address()).unwrap().balance,
        owner_before + 300
    );
    assert_eq!(node.get_account("bob").unwrap().balance, 1_500);
}

#[test]
fn test_three_validator_cluster_consensus() {
    let pocc = Pocc::new("cluster_worker", 203);
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
    let parties = pocc.genesis_accounts();
    let mut accounts: Vec<(&str, u64)> = parties.iter().map(|(a, b)| (a.as_str(), *b)).collect();
    accounts.extend([(alice_addr.as_str(), 50_000), ("bob", 1_000)]);

    let nodes: Vec<_> = keypairs
        .into_iter()
        .map(|kp| {
            let kv = MemKVStore::new();
            init_genesis_with_params(kv.clone(), 1337, &validators, &accounts, &pocc.params());
            let net = ChannelNetwork::new(kp.verifying_key(), hub.clone());
            MirasetNode::start(NodeConfig::devnet(kp), kv, net, Mempool::new())
        })
        .collect();

    register_worker(&nodes[0], &pocc);
    assert!(
        nodes[0]
            .submit_transaction(pocc.allow(&alice_key, 0, 1_000))
            .is_ok()
    );
    wait_until(CONSENSUS_TIMEOUT, || {
        nodes[0].allowance(&alice_addr, &pocc_coordinator(&pocc)) == 1_000
    });
    let mut tx = Transaction::new(&alice_addr, 1, "bob", 5_000, 100);
    tx.sign_with(&alice_key);
    assert!(nodes[0].submit_transaction(tx).is_ok());
    assert!(
        nodes[0]
            .submit_receipt(pocc.receipt("task_cluster_01", &alice_addr, 25))
            .is_ok()
    );

    // 50_000 - allowance fee(1) - transfer(5_000 + 100) - job cost(25 x 2) = 44_849
    wait_until(CONSENSUS_TIMEOUT, || {
        nodes.iter().all(|n| {
            n.is_task_settled("task_cluster_01")
                && n.get_account(&alice_addr).map(|a| a.balance) == Some(44_849)
        })
    });
    for node in &nodes {
        assert!(node.is_task_settled("task_cluster_01"));
        assert_eq!(node.get_account(&alice_addr).unwrap().balance, 44_849);
        assert_eq!(
            node.get_worker("cluster_worker").unwrap().completed_tasks,
            1
        );
    }
}

#[test]
fn test_worker_bond_registration_unbonding_and_slashing() {
    let pocc = Pocc::new("gpu_worker_pro", 204);
    let owner = pocc.owner_address();
    let node = start_single(8, &pocc, &[]);

    // 1. Register with a 500 bond.
    let bond_action = miraset_chain::core::types::WorkerBondAction::Register {
        worker_id: "gpu_worker_pro".to_string(),
        bond_amount: 500,
        vram_gib: 80,
        endpoint: "10.0.0.5:9090".to_string(),
    };
    let mut bond_tx = Transaction::new_worker_bond(&owner, 0, 10, bond_action).unwrap();
    bond_tx.sign_with(&pocc.worker_owner);
    assert!(node.submit_transaction(bond_tx).is_ok());
    wait_until(CONSENSUS_TIMEOUT, || {
        node.get_worker("gpu_worker_pro").is_some()
    });
    let worker = node.get_worker("gpu_worker_pro").expect("registered");
    assert_eq!(
        (worker.stake, worker.active, worker.vram_gib),
        (500, true, 80)
    );
    assert_eq!(worker.owner_address, owner);
    assert_eq!(worker.endpoint, "10.0.0.5:9090");
    // 10_000 - bond(500) - fee(10) = 9_490
    assert_eq!(node.get_account(&owner).unwrap().balance, 9_490);

    // 2. Partial unbond of 100.
    let unbond_action = miraset_chain::core::types::WorkerBondAction::Unbond {
        worker_id: "gpu_worker_pro".to_string(),
        amount: 100,
    };
    let mut unbond_tx = Transaction::new_worker_bond(&owner, 1, 10, unbond_action).unwrap();
    unbond_tx.sign_with(&pocc.worker_owner);
    assert!(node.submit_transaction(unbond_tx).is_ok());
    wait_until(CONSENSUS_TIMEOUT, || {
        node.get_worker("gpu_worker_pro").map(|w| w.stake) == Some(400)
    });
    // Two-step unbond: 100 is unbonding (still slashable) and not yet paid; only the fee left.
    assert_eq!(node.get_worker("gpu_worker_pro").unwrap().unbonding, 100);
    assert_eq!(node.get_account(&owner).unwrap().balance, 9_480);

    let slash = |slash_amount| miraset_chain::core::types::WorkerBondAction::Slash {
        worker_id: "gpu_worker_pro".to_string(),
        slash_amount,
        reason: "task_timeout_delivery_failure".to_string(),
    };

    // 3. A slash by anyone but governance does not apply (TODO 8.5.1: anyone could burn bonds).
    let mut rogue = Transaction::new_worker_bond(&owner, 2, 5, slash(400)).unwrap();
    rogue.sign_with(&pocc.worker_owner);
    assert!(matches!(
        node.submit_transaction(rogue.clone()),
        Err(AdmissionError::Invalid(_))
    ));
    // Execution refuses it too (a proposer need not run admission).
    assert!(node.mempool().insert_tx(rogue).is_ok());
    let height = node.highest_committed_height().unwrap();
    wait_until(CONSENSUS_TIMEOUT, || {
        node.highest_committed_height().unwrap() >= height + 5
    });
    assert_eq!(node.get_worker("gpu_worker_pro").unwrap().stake, 400);

    // 4. Governance slashes 10% (40).
    let mut slash_tx =
        Transaction::new_worker_bond(pocc.governance_address(), 0, 5, slash(40)).unwrap();
    slash_tx.sign_with(&pocc.governance);
    assert!(node.submit_transaction(slash_tx).is_ok());
    wait_until(CONSENSUS_TIMEOUT, || {
        node.get_worker("gpu_worker_pro").map(|w| w.stake) == Some(360)
    });
    let worker = node.get_worker("gpu_worker_pro").unwrap();
    assert_eq!(worker.stake, 360);
    assert!(worker.active);

    // 5. Unbonding does not escape a slash (review H1): a 380 slash takes the 360 stake and 20
    // of the 100 still unbonding; it is clamped instead of failing.
    let mut big =
        Transaction::new_worker_bond(pocc.governance_address(), 1, 5, slash(380)).unwrap();
    big.sign_with(&pocc.governance);
    assert!(node.submit_transaction(big).is_ok());
    wait_until(CONSENSUS_TIMEOUT, || {
        node.get_worker("gpu_worker_pro").map(|w| w.stake) == Some(0)
    });
    let worker = node.get_worker("gpu_worker_pro").unwrap();
    assert_eq!(
        (worker.stake, worker.unbonding, worker.active),
        (0, 80, false)
    );

    // 6. Closing the epoch pays the remaining 80 to the owner.
    wait_epoch_over(&node);
    let before = node.get_account(&owner).unwrap().balance;
    assert!(node.submit_transaction(pocc.close_epoch(2, 0)).is_ok());
    wait_until(CONSENSUS_TIMEOUT, || {
        node.get_current_epoch().map(|e| e.epoch_id) == Some(1)
    });
    assert_eq!(node.get_worker("gpu_worker_pro").unwrap().unbonding, 0);
    assert_eq!(node.get_account(&owner).unwrap().balance, before + 80);
}

#[test]
fn test_epoch_token_accumulation_and_system_rollover() {
    let pocc = Pocc::new("epoch_worker_1", 205);
    let client_key = SigningKey::from_bytes(&[12u8; 32]);
    let client = hex::encode(client_key.verifying_key().to_bytes());
    let node = start_single(10, &pocc, &[(client.as_str(), 5_000)]);
    register_worker(&node, &pocc);
    assert!(
        node.submit_transaction(pocc.allow(&client_key, 0, 5_000))
            .is_ok()
    );
    wait_until(CONSENSUS_TIMEOUT, || {
        node.allowance(&client, &pocc_coordinator(&pocc)) == 5_000
    });

    // 1. A receipt for 1000 tokens.
    assert!(
        node.submit_receipt(pocc.receipt("task_epoch_001", &client, 1000))
            .is_ok()
    );
    wait_until(CONSENSUS_TIMEOUT, || {
        node.get_current_epoch().map(|e| e.total_receipts) == Some(1)
    });
    assert_eq!(
        node.get_worker("epoch_worker_1").unwrap().epoch_tokens,
        1000
    );
    let epoch = node.get_current_epoch().unwrap();
    assert_eq!(
        (epoch.epoch_id, epoch.total_tokens, epoch.reward_budget),
        (0, 1000, EPOCH_REWARD_BUDGET)
    );

    // 2. Anyone but governance cannot close the epoch (closing mints the budget).
    let mut rogue = Transaction::new(pocc.owner_address(), 1, "system:epoch_manager", 0, 10);
    rogue.payload = 0u64.to_be_bytes().to_vec();
    rogue.sign_with(&pocc.worker_owner);
    assert!(matches!(
        node.submit_transaction(rogue.clone()),
        Err(AdmissionError::Invalid(_))
    ));
    assert!(node.mempool().insert_tx(rogue).is_ok());
    let height = node.highest_committed_height().unwrap();
    wait_until(CONSENSUS_TIMEOUT, || {
        node.highest_committed_height().unwrap() >= height + 5
    });
    assert_eq!(node.get_current_epoch().unwrap().epoch_id, 0);

    // 3. Governance closes epoch 0 once it has lasted EPOCH_SECONDS of block time.
    wait_epoch_over(&node);
    let owner_before = node.get_account(&pocc.owner_address()).unwrap().balance;
    assert!(node.submit_transaction(pocc.close_epoch(0, 0)).is_ok());
    wait_until(CONSENSUS_TIMEOUT, || {
        node.get_current_epoch().map(|e| e.epoch_id) == Some(1)
    });

    // 60% of the budget is the compute pool; the only worker earned all of it. Capacity is not
    // measured, so the 40% capacity pool rolls over into epoch 1.
    let compute_pool = EPOCH_REWARD_BUDGET * 6_000 / 10_000;
    let finalized = node.get_epoch(0).expect("finalized epoch 0");
    assert_eq!(
        finalized.status,
        miraset_chain::core::types::EpochStatus::Finalized
    );
    let distribution = finalized.distribution.expect("distribution");
    assert_eq!(
        distribution.worker_rewards.get("epoch_worker_1"),
        Some(&compute_pool)
    );
    assert_eq!(
        distribution.undistributed,
        EPOCH_REWARD_BUDGET - compute_pool
    );
    assert_eq!(
        node.get_account(&pocc.owner_address()).unwrap().balance,
        owner_before + compute_pool
    );

    let next = node.get_current_epoch().unwrap();
    assert_eq!((next.epoch_id, next.total_tokens), (1, 0));
    assert_eq!(next.status, miraset_chain::core::types::EpochStatus::Active);
    assert_eq!(
        next.reward_budget,
        EPOCH_REWARD_BUDGET + (EPOCH_REWARD_BUDGET - compute_pool)
    );
    let worker = node.get_worker("epoch_worker_1").unwrap();
    assert_eq!(
        (worker.epoch_tokens, worker.rewards_accumulated),
        (0, compute_pool)
    );
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
