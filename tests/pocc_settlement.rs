//! TODO 8.5.1: receipt settlement rules enforced by block execution, not only by RPC admission.
//! Adversarial receipts go straight into the mempool (bypassing admission, as a malicious or
//! buggy proposer would), next to one valid receipt; only the valid one may settle, and the
//! payment must move exactly `tokens × price` from the client to the worker's owner.

use ed25519_dalek::SigningKey;
use miraset_chain::mempool::Mempool;
use miraset_chain::network::{ChannelNetwork, ChannelNetworkHub};
use miraset_chain::node::{MirasetNode, NodeConfig, init_genesis_with_params};
use miraset_chain::storage::MemKVStore;
use std::thread;
use std::time::{Duration, Instant};

mod common;
use common::pocc::{PRICE_PER_TOKEN, Pocc, address, key, wait_epoch_over};

const TIMEOUT: Duration = Duration::from_secs(20);
const CLIENT_BALANCE: u64 = 10_000;
const TOKENS: u64 = 40;

fn wait(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + TIMEOUT;
    while !done() {
        assert!(Instant::now() < deadline, "timed out: {what}");
        thread::sleep(Duration::from_millis(20));
    }
}

fn start(pocc: &Pocc, client: &str) -> MirasetNode<MemKVStore, ChannelNetwork> {
    start_with(pocc, client, &[])
}

fn start_with(
    pocc: &Pocc,
    client: &str,
    extra: &[(&str, u64)],
) -> MirasetNode<MemKVStore, ChannelNetwork> {
    start_params(pocc, client, extra, &pocc.params())
}

fn start_params(
    pocc: &Pocc,
    client: &str,
    extra: &[(&str, u64)],
    params: &miraset_chain::core::types::ProtocolParams,
) -> MirasetNode<MemKVStore, ChannelNetwork> {
    let validator = key(150);
    let vk = validator.verifying_key();
    let kv = MemKVStore::new();
    let parties = pocc.genesis_accounts();
    let mut accounts: Vec<(&str, u64)> = parties.iter().map(|(a, b)| (a.as_str(), *b)).collect();
    accounts.push((client, CLIENT_BALANCE));
    accounts.extend_from_slice(extra);
    init_genesis_with_params(kv.clone(), 1337, &[(vk, 1)], &accounts, params);
    let network = ChannelNetwork::new(vk, ChannelNetworkHub::new());
    MirasetNode::start(NodeConfig::devnet(validator), kv, network, Mempool::new())
}

/// The client allows the fixture coordinator to charge it `amount` (its nonce 0).
fn allow(
    node: &MirasetNode<MemKVStore, ChannelNetwork>,
    pocc: &Pocc,
    client: &SigningKey,
    amount: u64,
) {
    assert!(
        node.submit_transaction(pocc.allow(client, 0, amount))
            .is_ok()
    );
    let coordinator = address(&pocc.coordinator);
    wait("allowance", || {
        node.allowance(&address(client), &coordinator) == amount
    });
}

#[test]
fn test_only_authorized_paid_worker_signed_receipts_settle() {
    let pocc = Pocc::default();
    let client = address(&key(151));
    let node = start(&pocc, &client);
    assert!(node.submit_transaction(pocc.register(0)).is_ok());
    wait("worker registered", || {
        node.get_worker(&pocc.worker_id).is_some()
    });
    allow(&node, &pocc, &key(151), 2 * TOKENS * PRICE_PER_TOKEN);
    let owner_before = node.get_account(&pocc.owner_address()).unwrap().balance;

    let mempool = node.mempool();
    let insert = |receipt| assert!(mempool.insert_receipt(receipt).is_ok());

    // Co-signed by a key that is not a genesis coordinator.
    let mut forged = pocc.receipt("forged-coordinator", &client, TOKENS);
    forged.sign_with(&key(160));
    insert(forged);
    // Not signed by the worker, or signed by someone else's key.
    let mut unsigned = pocc.receipt("no-worker-signature", &client, TOKENS);
    unsigned.worker_signature.clear();
    insert(unsigned);
    let mut impostor = pocc.receipt("impostor-worker", &client, TOKENS);
    impostor.sign_as_worker(&key(161));
    insert(impostor);
    // A worker that never registered (before this, settlement created it with a default
    // record and counted its tokens).
    let ghost = Pocc::new("ghost-worker", 162);
    insert(ghost.receipt("unregistered-worker", &client, TOKENS));
    // Cost that does not match tokens × price (overpaying the worker, underpaying).
    let mut inflated = pocc.receipt("inflated-reward", &client, TOKENS);
    inflated.worker_reward += 1;
    pocc.sign(&mut inflated);
    insert(inflated);
    let mut cheap = pocc.receipt("cheap-client", &client, TOKENS);
    cheap.client_cost -= 1;
    pocc.sign(&mut cheap);
    insert(cheap);
    // No client, unconfirmed delivery, and the worker paying itself.
    let mut no_client = pocc.receipt("no-client", "", TOKENS);
    no_client.client_address.clear();
    pocc.sign(&mut no_client);
    insert(no_client);
    let mut unconfirmed = pocc.receipt("unconfirmed", &client, TOKENS);
    unconfirmed.client_confirmed = false;
    pocc.sign(&mut unconfirmed);
    insert(unconfirmed);
    insert(pocc.receipt("wash-trade", &pocc.owner_address(), TOKENS));
    // Beyond what the client allowed (the allowance covers two valid receipts' cost).
    insert(pocc.receipt("over-allowance", &client, 3 * TOKENS));
    // The valid one.
    insert(pocc.receipt("valid", &client, TOKENS));

    wait("valid receipt settled", || node.is_task_settled("valid"));
    // A few more blocks give every other candidate its chance (and eviction).
    let height = node.highest_committed_height().unwrap();
    wait("more blocks", || {
        node.highest_committed_height().unwrap() >= height + 5
    });

    for task in [
        "forged-coordinator",
        "no-worker-signature",
        "impostor-worker",
        "unregistered-worker",
        "inflated-reward",
        "cheap-client",
        "no-client",
        "unconfirmed",
        "wash-trade",
        "over-allowance",
    ] {
        assert!(!node.is_task_settled(task), "{task} settled");
    }
    assert!(
        node.get_worker("ghost-worker").is_none(),
        "no worker is created by a receipt"
    );

    // Exactly one job paid: tokens × price from the client (minus its allowance fee) to the
    // worker's owner, out of the client's allowance.
    let cost = TOKENS * PRICE_PER_TOKEN;
    assert_eq!(
        node.get_account(&client).unwrap().balance,
        CLIENT_BALANCE - 1 - cost
    );
    assert_eq!(node.allowance(&client, &address(&pocc.coordinator)), cost);
    assert_eq!(
        node.get_account(&pocc.owner_address()).unwrap().balance,
        owner_before + cost
    );
    let worker = node.get_worker(&pocc.worker_id).unwrap();
    assert_eq!((worker.completed_tasks, worker.epoch_tokens), (1, TOKENS));
    let epoch = node.get_current_epoch().unwrap();
    assert_eq!((epoch.total_receipts, epoch.total_tokens), (1, TOKENS));
}

#[test]
fn test_a_settled_task_is_never_paid_twice() {
    let pocc = Pocc::new("worker-replay", 163);
    let client = address(&key(152));
    let node = start(&pocc, &client);
    assert!(node.submit_transaction(pocc.register(0)).is_ok());
    wait("worker registered", || {
        node.get_worker(&pocc.worker_id).is_some()
    });
    allow(&node, &pocc, &key(152), 1_000);

    // Two differently priced receipts for one task in the same mempool, and a third after the
    // first settles: one payment only.
    let mempool = node.mempool();
    assert!(
        mempool
            .insert_receipt(pocc.receipt("job-1", &client, TOKENS))
            .is_ok()
    );
    wait("first settlement", || node.is_task_settled("job-1"));
    assert!(
        mempool
            .insert_receipt(pocc.receipt("job-1", &client, TOKENS + 1))
            .is_ok()
    );
    let height = node.highest_committed_height().unwrap();
    wait("more blocks", || {
        node.highest_committed_height().unwrap() >= height + 5
    });

    assert_eq!(
        node.get_account(&client).unwrap().balance,
        CLIENT_BALANCE - 1 - TOKENS * PRICE_PER_TOKEN
    );
    assert_eq!(node.get_worker(&pocc.worker_id).unwrap().completed_tasks, 1);
    assert_eq!(mempool.receipt_count(), 0, "the replay was evicted");
}

#[test]
fn test_rewards_are_minted_only_by_governance_once_per_epoch() {
    let pocc = Pocc::new("worker-epoch", 164);
    let client = address(&key(153));
    let stranger: SigningKey = key(154);
    // Funded, so its rollover can only fail for lack of authority.
    let stranger_address = address(&stranger);
    let node = start_with(&pocc, &client, &[(stranger_address.as_str(), 1_000)]);

    // A stranger's and a worker owner's rollover never apply; neither does governance closing
    // an epoch that is not the current one.
    let mut rogue = miraset_chain::core::types::Transaction::new(
        address(&stranger),
        0,
        "system:epoch_manager",
        0,
        1,
    );
    rogue.payload = 0u64.to_be_bytes().to_vec();
    rogue.sign_with(&stranger);
    assert!(node.mempool().insert_tx(rogue).is_ok());
    assert!(node.mempool().insert_tx(pocc.close_epoch(0, 5)).is_ok());
    let height = node.highest_committed_height().unwrap_or(0);
    wait("blocks", || {
        node.highest_committed_height().unwrap_or(0) >= height + 5
    });
    let epoch = node.get_current_epoch().unwrap();
    assert_eq!(epoch.epoch_id, 0);

    // Governance closes epoch 0 with no tokens (its nonce 0 is unused: the wrong-epoch close
    // failed) once it has lasted EPOCH_SECONDS: nothing is paid, the whole budget rolls over.
    wait_epoch_over(&node);
    assert!(node.submit_transaction(pocc.close_epoch(0, 0)).is_ok());
    wait("epoch 1", || {
        node.get_current_epoch().map(|e| e.epoch_id) == Some(1)
    });
    let closed = node.get_epoch(0).unwrap().distribution.unwrap();
    assert!(closed.worker_rewards.values().all(|r| *r == 0));
    assert_eq!(closed.undistributed, common::pocc::EPOCH_REWARD_BUDGET);
    assert_eq!(
        node.get_current_epoch().unwrap().reward_budget,
        2 * common::pocc::EPOCH_REWARD_BUDGET
    );
}

/// Review H3: an epoch cannot close before `epoch_seconds` of block time — at admission, and in
/// execution for a close inserted straight into the mempool.
#[test]
fn test_an_epoch_cannot_close_early() {
    let pocc = Pocc::new("worker-early", 166);
    let client = address(&key(156));
    let params = miraset_chain::core::types::ProtocolParams {
        epoch_seconds: 3_600,
        ..pocc.params()
    };
    let node = start_params(&pocc, &client, &[], &params);
    let early = pocc.close_epoch(0, 0);
    assert!(node.submit_transaction(early.clone()).is_err());
    assert!(node.mempool().insert_tx(early).is_ok());
    let height = node.highest_committed_height().unwrap_or(0);
    wait("blocks", || {
        node.highest_committed_height().unwrap_or(0) >= height + 5
    });
    assert_eq!(node.get_current_epoch().unwrap().epoch_id, 0);
    assert_eq!(node.mempool().tx_count(), 0, "the early close was evicted");
    // Epoch 0 began at the first block's (real) time, not in 1970.
    let started = node.get_current_epoch().unwrap().start_timestamp;
    assert!(started > 1_700_000_000, "epoch 0 start {started}");
}

/// Review H2 (TODO 8.5.1): a coordinator can charge a client only what the client allowed.
#[test]
fn test_receipts_need_the_clients_allowance() {
    let pocc = Pocc::new("worker-allowance", 167);
    let client_key = key(157);
    let client = address(&client_key);
    let node = start(&pocc, &client);
    assert!(node.submit_transaction(pocc.register(0)).is_ok());
    wait("worker registered", || {
        node.get_worker(&pocc.worker_id).is_some()
    });

    // No allowance: refused at admission and by execution.
    let unallowed = pocc.receipt("no-allowance", &client, TOKENS);
    assert!(node.submit_receipt(unallowed.clone()).is_err());
    assert!(node.mempool().insert_receipt(unallowed).is_ok());
    // An allowance for a key that is not a genesis coordinator is refused at admission.
    let mut stray = pocc.allow(&client_key, 0, 1_000);
    let mut action: miraset_chain::core::types::AllowanceAction =
        serde_json::from_slice(&stray.payload).unwrap();
    action.coordinator = address(&key(168));
    stray.payload = action.to_bytes().unwrap();
    stray.sign_with(&client_key);
    assert!(node.submit_transaction(stray).is_err());

    let height = node.highest_committed_height().unwrap();
    wait("blocks", || {
        node.highest_committed_height().unwrap() >= height + 5
    });
    assert!(!node.is_task_settled("no-allowance"));
    assert_eq!(node.get_account(&client).unwrap().balance, CLIENT_BALANCE);

    // Allowed, then revoked (0) before the receipt arrives: nothing is charged.
    allow(&node, &pocc, &client_key, 1_000);
    assert!(
        node.submit_transaction(pocc.allow(&client_key, 1, 0))
            .is_ok()
    );
    let coordinator = address(&pocc.coordinator);
    wait("revoked", || node.allowance(&client, &coordinator) == 0);
    assert!(
        node.submit_receipt(pocc.receipt("after-revoke", &client, TOKENS))
            .is_err()
    );
    assert!(
        node.mempool()
            .insert_receipt(pocc.receipt("after-revoke", &client, TOKENS))
            .is_ok()
    );
    let height = node.highest_committed_height().unwrap();
    wait("blocks", || {
        node.highest_committed_height().unwrap() >= height + 5
    });
    assert!(!node.is_task_settled("after-revoke"));
    assert_eq!(
        node.get_account(&client).unwrap().balance,
        CLIENT_BALANCE - 2
    );
}

/// Review H1 (TODO 8.5.1): a worker that unbonds below the minimum settles no new receipts and
/// takes no compute reward for its earlier tokens; that share rolls over.
#[test]
fn test_unbonded_workers_settle_nothing_and_earn_no_compute_reward() {
    let pocc = Pocc::new("worker-leaving", 165);
    let client = address(&key(155));
    let node = start(&pocc, &client);
    assert!(node.submit_transaction(pocc.register(0)).is_ok());
    wait("worker registered", || {
        node.get_worker(&pocc.worker_id).is_some()
    });
    allow(&node, &pocc, &key(155), 1_000);
    assert!(
        node.submit_receipt(pocc.receipt("before-unbond", &client, TOKENS))
            .is_ok()
    );
    wait("receipt settled", || node.is_task_settled("before-unbond"));

    let unbond = miraset_chain::core::types::WorkerBondAction::Unbond {
        worker_id: pocc.worker_id.clone(),
        amount: miraset_chain::core::types::MIN_WORKER_BOND,
    };
    let mut tx = miraset_chain::core::types::Transaction::new_worker_bond(
        pocc.owner_address(),
        1,
        1,
        unbond,
    )
    .unwrap();
    tx.sign_with(&pocc.worker_owner);
    assert!(node.submit_transaction(tx).is_ok());
    wait("unbonded", || {
        node.get_worker(&pocc.worker_id).is_some_and(|w| !w.active)
    });

    // Straight into the mempool: execution must refuse a receipt for an unbonded worker.
    assert!(
        node.mempool()
            .insert_receipt(pocc.receipt("after-unbond", &client, TOKENS))
            .is_ok()
    );
    let height = node.highest_committed_height().unwrap();
    wait("more blocks", || {
        node.highest_committed_height().unwrap() >= height + 5
    });
    assert!(!node.is_task_settled("after-unbond"));

    wait_epoch_over(&node);
    let owner_before = node.get_account(&pocc.owner_address()).unwrap().balance;
    assert!(node.submit_transaction(pocc.close_epoch(0, 0)).is_ok());
    wait("epoch 1", || {
        node.get_current_epoch().map(|e| e.epoch_id) == Some(1)
    });
    let closed = node.get_epoch(0).unwrap().distribution.unwrap();
    assert_eq!(closed.worker_rewards.get(&pocc.worker_id), Some(&0));
    assert_eq!(closed.undistributed, common::pocc::EPOCH_REWARD_BUDGET);
    // The owner gets the unbonded bond back at the close, and no reward.
    assert_eq!(
        node.get_account(&pocc.owner_address()).unwrap().balance,
        owner_before + miraset_chain::core::types::MIN_WORKER_BOND
    );
}
