//! Incremental execution of one block's payload (PRD R04).
//!
//! `BlockExecution` holds the state a block has changed so far. Transactions and then receipts
//! are applied one at a time in payload order, exactly as a full re-execution would, so
//! block production can try each mempool candidate once (on a copy, kept only if it applies)
//! instead of re-executing the whole growing candidate list per candidate.

use super::MirasetApp;
use crate::core::types::{
    AccountState, ComputeReceipt, EpochInfo, EpochRewardConfig, EpochStatus, MIN_WORKER_BOND,
    ProtocolParams, Transaction, WorkerBondAction, WorkerInfo, compute_state_commitment,
};
use hotstuff_rs::block_tree::accessors::app::AppBlockTreeView;
use hotstuff_rs::block_tree::pluggables::KVStore;
use hotstuff_rs::types::update_sets::AppStateUpdates;
use std::collections::{BTreeMap, BTreeSet};

const WORKER_IDS_KEY: &[u8] = b"workers:all_ids";

/// State a block has changed so far. `None` from an `apply_*` method means the item is invalid
/// on top of this state; the execution must then be discarded (use a copy when trying items).
#[derive(Clone)]
pub(super) struct BlockExecution {
    updates: AppStateUpdates,
    committed_entries: BTreeMap<Vec<u8>, Vec<u8>>,
    local_accounts: BTreeMap<String, AccountState>,
    local_workers: BTreeMap<String, WorkerInfo>,
    local_tasks: BTreeSet<String>,
    worker_ids: Vec<String>,
    worker_ids_modified: bool,
    current_epoch: EpochInfo,
    epoch_modified: bool,
    /// Genesis protocol parameters (coordinators, governance, price, reward budget).
    params: ProtocolParams,
}

type Tree<'a, 'b, K> = AppBlockTreeView<'a, K>;

impl BlockExecution {
    pub(super) fn new<K: KVStore>(tree: &Tree<'_, '_, K>) -> Self {
        Self {
            updates: AppStateUpdates::new(),
            committed_entries: BTreeMap::new(),
            local_accounts: BTreeMap::new(),
            local_workers: BTreeMap::new(),
            local_tasks: BTreeSet::new(),
            worker_ids: tree
                .app_state(WORKER_IDS_KEY)
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .unwrap_or_default(),
            worker_ids_modified: false,
            current_epoch: MirasetApp::read_current_epoch(tree),
            epoch_modified: false,
            params: MirasetApp::read_protocol_params(tree),
        }
    }

    fn put(&mut self, key: Vec<u8>, value: Vec<u8>) {
        self.committed_entries.insert(key.clone(), value.clone());
        self.updates.insert(key, value);
    }

    fn take_account<K: KVStore>(&mut self, tree: &Tree<'_, '_, K>, address: &str) -> AccountState {
        self.local_accounts
            .remove(address)
            .unwrap_or_else(|| MirasetApp::read_account(tree, address))
    }

    /// The account nonce a transaction from `address` must carry next on top of this state.
    pub(super) fn account_nonce<K: KVStore>(&self, tree: &Tree<'_, '_, K>, address: &str) -> u64 {
        self.local_accounts.get(address).map_or_else(
            || MirasetApp::read_account(tree, address).nonce,
            |a| a.nonce,
        )
    }

    fn take_worker<K: KVStore>(&mut self, tree: &Tree<'_, '_, K>, id: &str) -> Option<WorkerInfo> {
        self.local_workers
            .remove(id)
            .or_else(|| MirasetApp::read_worker(tree, id))
    }

    /// Apply one transaction: sender signature, nonce and funds, then its effect.
    pub(super) fn apply_transaction<K: KVStore>(
        &mut self,
        tree: &Tree<'_, '_, K>,
        tx: &Transaction,
    ) -> Option<()> {
        let sender_acc = self.take_account(tree, &tx.sender);
        // Reject an unsigned/forged sender, a replayed or out-of-order nonce, overflow, or
        // insufficient funds.
        let updated_sender = MirasetApp::apply_transaction_to_sender(tx, &sender_acc).ok()?;
        self.local_accounts
            .insert(tx.sender.clone(), updated_sender);

        if tx.recipient == "system:worker_registry" && !tx.payload.is_empty() {
            let action: WorkerBondAction = serde_json::from_slice(&tx.payload).ok()?;
            self.apply_worker_action(tree, tx, action)
        } else if tx.recipient == "system:epoch_manager" && !tx.payload.is_empty() {
            self.apply_epoch_rollover(tree, tx)
        } else {
            let mut recipient_acc = self.take_account(tree, &tx.recipient);
            recipient_acc.balance = recipient_acc.balance.checked_add(tx.amount)?;
            self.local_accounts
                .insert(tx.recipient.clone(), recipient_acc);
            Some(())
        }
    }

    fn apply_worker_action<K: KVStore>(
        &mut self,
        tree: &Tree<'_, '_, K>,
        tx: &Transaction,
        action: WorkerBondAction,
    ) -> Option<()> {
        match action {
            WorkerBondAction::Register {
                worker_id,
                bond_amount,
                vram_gib,
                endpoint,
            } => {
                if tx.amount != bond_amount {
                    return None;
                }
                let mut worker = self.take_worker(tree, &worker_id).unwrap_or(WorkerInfo {
                    worker_id: worker_id.clone(),
                    stake: 0,
                    completed_tasks: 0,
                    rewards_accumulated: 0,
                    active: true,
                    vram_gib,
                    uptime_bps: 10000,
                    epoch_tokens: 0,
                    owner_address: tx.sender.clone(),
                    endpoint: endpoint.clone(),
                    unbonding: 0,
                });
                // If already has an owner, only owner can add bond
                if !worker.owner_address.is_empty() && worker.owner_address != tx.sender {
                    return None;
                }
                worker.owner_address = tx.sender.clone();
                worker.stake = worker.stake.checked_add(bond_amount)?;
                if vram_gib > 0 {
                    worker.vram_gib = vram_gib;
                }
                if !endpoint.is_empty() {
                    worker.endpoint = endpoint;
                }
                worker.active = worker.stake >= MIN_WORKER_BOND;
                self.local_workers.insert(worker_id.clone(), worker);
                self.worker_ids_modified |=
                    MirasetApp::add_worker_id(&mut self.worker_ids, &worker_id);
            }
            WorkerBondAction::Unbond { worker_id, amount } => {
                if tx.amount != 0 {
                    return None; // nothing to pay into an unbond
                }
                let mut worker = self.take_worker(tree, &worker_id)?;
                // Only owner can unbond
                if worker.owner_address != tx.sender {
                    return None;
                }
                // Two steps (TODO 8.5.1): the amount leaves the active stake now but stays
                // slashable until the epoch closes, when it is paid to the owner.
                worker.stake = worker.stake.checked_sub(amount)?;
                worker.unbonding = worker.unbonding.checked_add(amount)?;
                if worker.stake < MIN_WORKER_BOND {
                    worker.active = false;
                }
                self.local_workers.insert(worker_id, worker);
            }
            WorkerBondAction::Slash {
                worker_id,
                slash_amount,
                ..
            } => {
                // Only a genesis governance key may slash (TODO 8.5.1); before this anyone could
                // burn any worker's bond.
                if !self.params.is_governance(&tx.sender) || tx.amount != 0 {
                    return None;
                }
                // Burn up to `slash_amount` from the active stake, then from bond still
                // unbonding; a worker cannot escape a slash by unbonding first.
                let mut worker = self.take_worker(tree, &worker_id)?;
                let from_stake = slash_amount.min(worker.stake);
                worker.stake -= from_stake;
                let from_unbonding = (slash_amount - from_stake).min(worker.unbonding);
                worker.unbonding -= from_unbonding;
                if worker.stake < MIN_WORKER_BOND {
                    worker.active = false;
                }
                self.local_workers.insert(worker_id, worker);
            }
        }
        Some(())
    }

    /// Epoch rollover and reward distribution (`system:epoch_manager`).
    fn apply_epoch_rollover<K: KVStore>(
        &mut self,
        tree: &Tree<'_, '_, K>,
        tx: &Transaction,
    ) -> Option<()> {
        // Only a genesis governance key may close an epoch: closing mints the epoch's reward
        // budget (TODO 8.5.1; before this anyone could mint by repeating rollovers). Epoch
        // timing is the governance key's duty until blocks carry a timestamp.
        if !self.params.is_governance(&tx.sender) || tx.amount != 0 {
            return None;
        }
        let epoch_bytes: [u8; 8] = tx.payload.as_slice().try_into().ok()?;
        // Target epoch must match current active epoch
        if u64::from_be_bytes(epoch_bytes) != self.current_epoch.epoch_id {
            return None;
        }

        // Load all registered workers from state, augmented by locally updated workers
        let mut all_workers = MirasetApp::read_all_registered_workers(tree);
        for local_w in self.local_workers.values() {
            match all_workers
                .iter()
                .position(|w| w.worker_id == local_w.worker_id)
            {
                Some(pos) => all_workers[pos] = local_w.clone(),
                None => all_workers.push(local_w.clone()),
            }
        }
        all_workers.sort_by(|a, b| a.worker_id.cmp(&b.worker_id));

        // Compute canonical PoCC reward distribution across all registered workers
        let distribution = EpochRewardConfig::default()
            .try_distribute(self.current_epoch.reward_budget, &all_workers)
            .ok()?;

        // Credit distributed rewards to worker owners / balances
        for (w_id, reward_amount) in &distribution.worker_rewards {
            if *reward_amount == 0 {
                continue;
            }
            if let Some(mut worker) = self.take_worker(tree, w_id) {
                worker.rewards_accumulated =
                    worker.rewards_accumulated.checked_add(*reward_amount)?;
                if !worker.owner_address.is_empty() {
                    let mut owner_acc = self.take_account(tree, &worker.owner_address);
                    owner_acc.balance = owner_acc.balance.checked_add(*reward_amount)?;
                    self.local_accounts
                        .insert(worker.owner_address.clone(), owner_acc);
                }
                self.local_workers.insert(w_id.clone(), worker);
            }
        }

        // Finalize current epoch and write it to the historical index: epoch:info:{epoch_id}
        // Unpaid budget (no tokens, unmeasured capacity) rolls over (REWARDS §5.2-5.3, MVP).
        let carried = distribution.undistributed;
        self.current_epoch.status = EpochStatus::Finalized;
        self.current_epoch.distribution = Some(distribution);
        let hist_key = MirasetApp::epoch_key(self.current_epoch.epoch_id);
        let hist_val = self.current_epoch.to_bytes().ok()?;
        self.put(hist_key, hist_val);

        // Reset epoch tokens and pay out bond that finished unbonding this epoch.
        for worker in all_workers {
            let mut worker = self
                .local_workers
                .remove(&worker.worker_id)
                .unwrap_or(worker);
            worker.epoch_tokens = 0;
            let released = std::mem::take(&mut worker.unbonding);
            if released > 0 {
                let mut owner = self.take_account(tree, &worker.owner_address);
                owner.balance = owner.balance.checked_add(released)?;
                self.local_accounts
                    .insert(worker.owner_address.clone(), owner);
            }
            self.local_workers.insert(worker.worker_id.clone(), worker);
        }

        // Advance to next active epoch: a fresh budget plus what this epoch did not pay out.
        self.current_epoch = EpochInfo::new(
            self.current_epoch.epoch_id.checked_add(1)?,
            self.current_epoch.start_timestamp.checked_add(3600)?,
            self.params.epoch_reward_budget.checked_add(carried)?,
        );
        self.epoch_modified = true;
        Some(())
    }

    /// Apply one PoCC compute receipt (TODO 8.5.1; see `ComputeReceipt` for the rules):
    /// authorized coordinator, registered and bonded worker that signed it, no replay, client
    /// confirmation, and payment `tokens × price` from the client to the worker's owner, all
    /// with checked arithmetic.
    pub(super) fn apply_receipt<K: KVStore>(
        &mut self,
        tree: &Tree<'_, '_, K>,
        receipt: &ComputeReceipt,
    ) -> Option<()> {
        if !self.params.is_coordinator(&receipt.coordinator_pubkey) {
            return None;
        }
        receipt.verify_signature().ok()?;

        let t_key = MirasetApp::task_key(&receipt.task_id);
        if tree.app_state(&t_key).is_some() || self.local_tasks.contains(&receipt.task_id) {
            return None; // duplicate task replay
        }

        // The worker must be registered with a bond (ADR-0007) and must have signed.
        let mut worker = self.take_worker(tree, &receipt.worker_id)?;
        if !worker.active || worker.stake < MIN_WORKER_BOND || worker.owner_address.is_empty() {
            return None;
        }
        receipt
            .verify_worker_signature(&worker.owner_address)
            .ok()?;

        // Every settled job is paid: JobCost = output tokens × PRICE_PER_TOKEN (REWARDS §6),
        // moved from the client to the worker's owner, nothing minted or burned.
        if receipt.client_address.is_empty() || !receipt.client_confirmed {
            return None;
        }
        // One key, one account: addresses are lowercase hex (as transaction senders are).
        if receipt.tokens_computed == 0 || has_uppercase(&receipt.client_address) {
            return None;
        }
        // A worker paying itself moves nothing but would earn compute-pool tokens.
        if receipt.client_address == worker.owner_address {
            return None;
        }
        let cost = receipt
            .tokens_computed
            .checked_mul(self.params.price_per_token)?;
        if receipt.client_cost != cost || receipt.worker_reward != cost {
            return None;
        }
        let mut client = self.take_account(tree, &receipt.client_address);
        client.balance = client.balance.checked_sub(cost)?;
        self.local_accounts
            .insert(receipt.client_address.clone(), client);
        let mut owner = self.take_account(tree, &worker.owner_address);
        owner.balance = owner.balance.checked_add(cost)?;
        self.local_accounts
            .insert(worker.owner_address.clone(), owner);

        self.local_tasks.insert(receipt.task_id.clone());
        self.put(t_key, b"settled".to_vec());

        // Verified output tokens feed the epoch's compute reward pool (REWARDS §3.3).
        worker.completed_tasks = worker.completed_tasks.checked_add(1)?;
        worker.epoch_tokens = worker.epoch_tokens.checked_add(receipt.tokens_computed)?;
        self.local_workers.insert(receipt.worker_id.clone(), worker);
        self.current_epoch.total_tokens = self
            .current_epoch
            .total_tokens
            .checked_add(receipt.tokens_computed)?;
        self.current_epoch.total_receipts = self.current_epoch.total_receipts.checked_add(1)?;
        self.epoch_modified = true;
        Some(())
    }

    /// Write the changed state in deterministic order and compute the state commitment root.
    pub(super) fn finish(mut self) -> Option<(AppStateUpdates, [u8; 32])> {
        if self.worker_ids_modified {
            self.worker_ids.sort();
            let val = serde_json::to_vec(&self.worker_ids).ok()?;
            self.put(WORKER_IDS_KEY.to_vec(), val);
        }

        if self.epoch_modified {
            let val = self.current_epoch.to_bytes().ok()?;
            self.put(MirasetApp::current_epoch_key(), val.clone());
            // Also keep current epoch synced under its indexed key epoch:info:{epoch_id}
            self.put(MirasetApp::epoch_key(self.current_epoch.epoch_id), val);
        }

        // Apply worker and then account updates in deterministic BTreeMap order
        for (w_id, worker) in std::mem::take(&mut self.local_workers) {
            let val = serde_json::to_vec(&worker).ok()?;
            self.put(MirasetApp::worker_key(&w_id), val);
        }
        for (addr, acc) in std::mem::take(&mut self.local_accounts) {
            let val = serde_json::to_vec(&acc).ok()?;
            self.put(MirasetApp::account_key(&addr), val);
        }

        let refs: Vec<(&[u8], &[u8])> = self
            .committed_entries
            .iter()
            .map(|(k, v)| (k.as_slice(), v.as_slice()))
            .collect();
        let state_root = compute_state_commitment(&refs);
        self.updates
            .insert(MirasetApp::state_commitment_key(), state_root.to_vec());
        Some((self.updates, state_root))
    }
}

fn has_uppercase(address: &str) -> bool {
    address.bytes().any(|b| b.is_ascii_uppercase())
}
