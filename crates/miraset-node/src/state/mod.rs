mod execution;
mod jobs;

use crate::epoch::{Epoch, EpochStatus, JobResult as EpochJobResult, WorkerEpochStats};
use crate::error::{StateError, TxError};
use crate::gas::GasConfig;
use crate::storage::Storage;
use chrono::Utc;
use miraset_core::{Address, Block, Event, Object, ObjectData, ObjectId, Transaction, Version};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone)]
pub struct State {
    pub(crate) inner: Arc<RwLock<StateInner>>,
    pub(crate) storage: Option<Storage>,
    pub(crate) gas_config: Arc<GasConfig>,
}

pub(crate) struct StateInner {
    // Object storage (Sui-like)
    pub(crate) objects: HashMap<ObjectId, Object>,
    pub(crate) object_versions: HashMap<ObjectId, Version>,
    pub(crate) owned_objects: HashMap<Address, Vec<ObjectId>>,

    // Account state (for backward compatibility)
    pub(crate) balances: HashMap<Address, u64>,
    pub(crate) nonces: HashMap<Address, u64>,

    // Blockchain data
    pub(crate) blocks: Vec<Block>,
    pub(crate) pending_txs: Vec<Transaction>,
    pub(crate) events: Vec<Event>,

    // Epoch management
    pub(crate) current_epoch: Epoch,
    pub(crate) past_epochs: Vec<Epoch>,
}

impl State {
    pub fn new() -> Self {
        Self::new_with_storage(None)
    }

    pub fn new_with_storage(storage: Option<Storage>) -> Self {
        let genesis = Block::genesis();
        let now = Utc::now();

        // Try to load state from storage if available
        let (blocks, balances, nonces, events) = if let Some(ref store) = storage {
            // Load latest block or use genesis
            let blocks = if let Ok(Some(latest)) = store.get_latest_block() {
                // Load all blocks from 0 to latest height
                let mut loaded_blocks = Vec::new();
                for h in 0..=latest.height {
                    if let Ok(Some(block)) = store.get_block(h) {
                        loaded_blocks.push(block);
                    }
                }
                if loaded_blocks.is_empty() {
                    vec![genesis.clone()]
                } else {
                    loaded_blocks
                }
            } else {
                vec![genesis.clone()]
            };

            // For now, balances and nonces are loaded on-demand
            // Full state reconstruction would iterate all keys
            (blocks, HashMap::new(), HashMap::new(), Vec::new())
        } else {
            (vec![genesis], HashMap::new(), HashMap::new(), Vec::new())
        };

        Self {
            inner: Arc::new(RwLock::new(StateInner {
                objects: HashMap::new(),
                object_versions: HashMap::new(),
                owned_objects: HashMap::new(),
                balances,
                nonces,
                blocks,
                pending_txs: Vec::new(),
                events,
                current_epoch: Epoch::new(0, now),
                past_epochs: Vec::new(),
            })),
            storage,
            gas_config: Arc::new(GasConfig::default()),
        }
    }

    /// Get gas configuration
    pub fn gas_config(&self) -> Arc<GasConfig> {
        Arc::clone(&self.gas_config)
    }

    /// Set gas configuration (for governance)
    pub fn set_gas_config(&mut self, config: GasConfig) {
        self.gas_config = Arc::new(config);
    }

    pub fn get_balance(&self, addr: &Address) -> u64 {
        let balance = self.inner.read().balances.get(addr).copied();
        match balance {
            Some(b) => b,
            None => {
                // Try loading from storage
                if let Some(ref storage) = self.storage {
                    storage.get_balance(addr).unwrap_or(0)
                } else {
                    0
                }
            }
        }
    }

    pub fn get_nonce(&self, addr: &Address) -> u64 {
        let nonce = self.inner.read().nonces.get(addr).copied();
        match nonce {
            Some(n) => n,
            None => {
                // Try loading from storage
                if let Some(ref storage) = self.storage {
                    storage.get_nonce(addr).unwrap_or(0)
                } else {
                    0
                }
            }
        }
    }

    pub fn submit_transaction(&self, tx: Transaction) -> Result<(), TxError> {
        // Basic validation
        let from = tx.from();
        let nonce = tx.nonce();
        let signature = tx.signature();

        // D6: Verify signature for ALL transaction types using zero-sig canonical pattern
        {
            let mut tx_for_hash = tx.clone();
            // Zero out signature for canonical hashing
            match &mut tx_for_hash {
                Transaction::Transfer { signature, .. } => *signature = [0; 64],
                Transaction::ChatSend { signature, .. } => *signature = [0; 64],
                Transaction::CreateObject { signature, .. } => *signature = [0; 64],
                Transaction::MutateObject { signature, .. } => *signature = [0; 64],
                Transaction::TransferObject { signature, .. } => *signature = [0; 64],
                Transaction::RegisterWorker { signature, .. } => *signature = [0; 64],
                Transaction::SubmitResourceSnapshot { signature, .. } => *signature = [0; 64],
                Transaction::CreateJob { signature, .. } => *signature = [0; 64],
                Transaction::AssignJob { signature, .. } => *signature = [0; 64],
                Transaction::SubmitJobResult { signature, .. } => *signature = [0; 64],
                Transaction::AnchorReceipt { signature, .. } => *signature = [0; 64],
                Transaction::ChallengeJob { signature, .. } => *signature = [0; 64],
            }
            let msg = bincode::serialize(&tx_for_hash)?;
            if !miraset_core::verify_signature(from, &msg, signature) {
                return Err(TxError::InvalidSignature);
            }
        }

        let mut w = self.inner.write();

        // Check nonce
        let current_nonce = w.nonces.get(from).copied().unwrap_or(0);
        if nonce != current_nonce {
            return Err(TxError::InvalidNonce {
                expected: current_nonce,
                got: nonce,
            });
        }

        // Type-specific validation
        match &tx {
            Transaction::Transfer { amount, .. } => {
                let balance = w.balances.get(from).copied().unwrap_or(0);
                if balance < *amount {
                    return Err(TxError::InsufficientBalance);
                }
            }
            Transaction::ChatSend { message, .. } => {
                if message.is_empty() || message.len() > 1000 {
                    return Err(TxError::InvalidMessageLength);
                }
            }
            Transaction::MutateObject { object_id, .. } => {
                if !w.objects.contains_key(object_id) {
                    return Err(TxError::ObjectNotFound);
                }
            }
            Transaction::CreateJob { escrow_amount, .. } => {
                let balance = w.balances.get(from).copied().unwrap_or(0);
                if balance < *escrow_amount {
                    return Err(TxError::InsufficientEscrow);
                }
            }
            _ => {
                // Other transactions validated during execution
            }
        }

        w.pending_txs.push(tx);
        Ok(())
    }

    /// Compute a deterministic state root from the current in-memory state.
    ///
    /// Hashes sorted balances, nonces, object hashes, and object versions.
    /// This is a simple commitment suitable for a devnet; it replaces the
    /// previous placeholder `[0; 32]` state root.
    fn compute_state_root(w: &StateInner) -> Result<[u8; 32], StateError> {
        let mut hasher = blake3::Hasher::new();

        // Balances
        let mut balance_keys: Vec<&Address> = w.balances.keys().collect();
        balance_keys.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
        for addr in balance_keys {
            hasher.update(addr.as_bytes());
            hasher.update(&w.balances[addr].to_le_bytes());
        }

        // Nonces
        let mut nonce_keys: Vec<&Address> = w.nonces.keys().collect();
        nonce_keys.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
        for addr in nonce_keys {
            hasher.update(addr.as_bytes());
            hasher.update(&w.nonces[addr].to_le_bytes());
        }

        // Objects
        let mut object_keys: Vec<&ObjectId> = w.objects.keys().collect();
        object_keys.sort();
        for id in object_keys {
            hasher.update(id.as_ref());
            hasher.update(&w.objects[id].hash()?);
        }

        // Object versions
        let mut version_keys: Vec<&ObjectId> = w.object_versions.keys().collect();
        version_keys.sort();
        for id in version_keys {
            hasher.update(id.as_ref());
            hasher.update(&w.object_versions[id].to_le_bytes());
        }

        Ok(hasher.finalize().into())
    }

    pub fn produce_block(&self) -> Result<Block, StateError> {
        let mut w = self.inner.write();
        let prev = w.blocks.last().ok_or(StateError::NoGenesis)?;
        let height = prev.height + 1;
        let prev_hash = prev.hash()?;

        let transactions = std::mem::take(&mut w.pending_txs);
        let event_index_start = w.events.len();
        let mut nonce_updates: Vec<(Address, u64)> = Vec::with_capacity(transactions.len());
        let mut balance_updates: Vec<(Address, u64)> = Vec::new();

        // Execute transactions
        for tx in &transactions {
            // Update nonce
            let from = tx.from();
            let new_nonce = w.nonces.entry(*from).or_insert(0);
            *new_nonce += 1;
            nonce_updates.push((*from, *new_nonce));

            // Execute transaction
            execution::execute_transaction_inner(&mut w, tx, height, &mut balance_updates)?;
        }

        // Compute deterministic state root after all mutations
        let state_root = Self::compute_state_root(&w)?;

        let block = Block {
            height,
            timestamp: chrono::Utc::now(),
            prev_hash,
            transactions,
            state_root,
        };

        w.blocks.push(block.clone());
        // Move new events out of the in-memory vec so we can persist them after
        // dropping the lock. They are restored below once storage I/O completes.
        let new_events: Vec<Event> = w.events.split_off(event_index_start);

        // Release write lock before doing storage I/O
        drop(w);

        // Persist block, balances, nonces, and events outside the lock
        if let Some(ref storage) = self.storage {
            let _ = storage.save_block(&block);
            for (addr, balance) in balance_updates {
                let _ = storage.save_balance(&addr, balance);
            }
            for (addr, nonce) in nonce_updates {
                let _ = storage.save_nonce(&addr, nonce);
            }
            for (i, event) in new_events.iter().enumerate() {
                let event_index = (event_index_start + i) as u64;
                let _ = storage.save_event(event_index, event);
            }
            let _ = storage.flush();
        }

        let event_count = new_events.len();

        // Re-append the new events to the in-memory history now that storage I/O
        // is complete. They were removed with `split_off` above to allow safe
        // persistence outside the lock.
        {
            let mut w = self.inner.write();
            w.events.extend(new_events);
        }

        tracing::debug!(
            height = block.height,
            txs = block.transactions.len(),
            events = event_count,
            state_root = hex::encode(state_root),
            "produced block",
        );

        Ok(block)
    }

    pub fn get_latest_block(&self) -> Result<Block, StateError> {
        self.inner
            .read()
            .blocks
            .last()
            .cloned()
            .ok_or(StateError::NoGenesis)
    }

    pub fn get_block(&self, height: u64) -> Option<Block> {
        self.inner
            .read()
            .blocks
            .iter()
            .find(|b| b.height == height)
            .cloned()
    }

    pub fn get_events(&self, from_height: u64, limit: usize) -> Vec<Event> {
        self.inner
            .read()
            .events
            .iter()
            .filter(|e| {
                let block_height = match e {
                    Event::Transferred { block_height, .. } => *block_height,
                    Event::ChatMessage { block_height, .. } => *block_height,
                    Event::WorkerRegistered { block_height, .. } => *block_height,
                    Event::ObjectCreated { block_height, .. } => *block_height,
                    Event::ObjectMutated { block_height, .. } => *block_height,
                    Event::ObjectTransferred { block_height, .. } => *block_height,
                    Event::ResourceSnapshotSubmitted { block_height, .. } => *block_height,
                    Event::JobCreated { block_height, .. } => *block_height,
                    Event::JobAssigned { block_height, .. } => *block_height,
                    Event::JobCompleted { block_height, .. } => *block_height,
                    Event::ReceiptAnchored { block_height, .. } => *block_height,
                    Event::JobChallenged { block_height, .. } => *block_height,
                    Event::EpochSettled { block_height, .. } => *block_height,
                    Event::RewardsDistributed { block_height, .. } => *block_height,
                };
                block_height >= from_height
            })
            .take(limit)
            .cloned()
            .collect()
    }

    pub fn get_chat_messages(
        &self,
        limit: usize,
    ) -> Vec<(Address, String, chrono::DateTime<chrono::Utc>)> {
        self.inner
            .read()
            .events
            .iter()
            .filter_map(|e| match e {
                Event::ChatMessage {
                    from,
                    message,
                    timestamp,
                    ..
                } => Some((*from, message.clone(), *timestamp)),
                _ => None,
            })
            .rev()
            .take(limit)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    }

    pub fn height(&self) -> Result<u64, StateError> {
        self.inner
            .read()
            .blocks
            .last()
            .map(|b| b.height)
            .ok_or(StateError::NoGenesis)
    }

    // ===== Object-centric methods (Sui-like) =====

    /// Create a new object from data (convenience method)
    pub fn create_object_from_data(
        &self,
        owner: Address,
        data: ObjectData,
    ) -> Result<ObjectId, StateError> {
        let mut w = self.inner.write();
        let obj = Object::new(owner, data)?;
        let obj_id = obj.id;

        w.objects.insert(obj_id, obj);
        w.object_versions.insert(obj_id, 0);
        w.owned_objects.entry(owner).or_default().push(obj_id);

        Ok(obj_id)
    }

    /// Get an object by ID
    pub fn get_object(&self, object_id: &ObjectId) -> Option<Object> {
        self.inner.read().objects.get(object_id).cloned()
    }

    /// Get all objects owned by an address
    pub fn get_owned_objects(&self, owner: &Address) -> Vec<Object> {
        let r = self.inner.read();
        if let Some(obj_ids) = r.owned_objects.get(owner) {
            obj_ids
                .iter()
                .filter_map(|id| r.objects.get(id).cloned())
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Mutate an object (owner-only, with version check)
    pub fn mutate_object(
        &self,
        object_id: &ObjectId,
        expected_version: Version,
        new_data: ObjectData,
        owner: &Address,
    ) -> Result<(), StateError> {
        let mut w = self.inner.write();

        // Check version first
        let current_version = w.object_versions.get(object_id).copied().unwrap_or(0);
        if current_version != expected_version {
            return Err(StateError::VersionMismatch {
                expected: expected_version,
                got: current_version,
            });
        }

        let obj = w
            .objects
            .get_mut(object_id)
            .ok_or_else(|| StateError::ObjectNotFound(hex::encode(object_id)))?;

        if obj.owner != *owner {
            return Err(StateError::NotOwner);
        }

        obj.data = new_data;
        let new_version = current_version + 1;
        obj.version = new_version;
        w.object_versions.insert(*object_id, new_version);

        Ok(())
    }

    /// Transfer object ownership
    pub fn transfer_object(
        &self,
        object_id: &ObjectId,
        from: &Address,
        to: &Address,
    ) -> Result<(), StateError> {
        let mut w = self.inner.write();

        let obj = w
            .objects
            .get_mut(object_id)
            .ok_or_else(|| StateError::ObjectNotFound(hex::encode(object_id)))?;

        if obj.owner != *from {
            return Err(StateError::NotOwner);
        }

        obj.owner = *to;

        // Update ownership index
        if let Some(owned) = w.owned_objects.get_mut(from) {
            owned.retain(|id| id != object_id);
        }
        w.owned_objects.entry(*to).or_default().push(*object_id);

        Ok(())
    }

    /// Create a new object and store it
    pub fn create_object(&self, object: Object) -> Result<(), StateError> {
        let mut w = self.inner.write();

        let object_id = object.id;
        let owner = object.owner;

        // Store object
        w.objects.insert(object_id, object);
        w.object_versions.insert(object_id, 0);

        // Update ownership index
        w.owned_objects.entry(owner).or_default().push(object_id);

        Ok(())
    }

    /// Update an existing object (used by executor)
    pub fn update_object(&self, object: Object) -> Result<(), StateError> {
        let mut w = self.inner.write();

        let object_id = object.id;

        if !w.objects.contains_key(&object_id) {
            return Err(StateError::ObjectNotFound(hex::encode(object_id)));
        }

        // Update version
        w.object_versions.insert(object_id, object.version);

        // Update object
        w.objects.insert(object_id, object);

        Ok(())
    }

    /// Add balance (can be negative for deductions)
    pub fn add_balance(&self, addr: &Address, amount: i64) {
        let mut w = self.inner.write();
        let balance = w.balances.entry(*addr).or_insert(0);
        if amount < 0 {
            *balance = balance.saturating_sub(amount.unsigned_abs());
        } else {
            *balance = balance.saturating_add(amount as u64);
        }

        // Persist to storage
        if let Some(ref storage) = self.storage {
            let _ = storage.save_balance(addr, *balance);
        }
    }

    /// Get all workers
    pub fn get_workers(&self) -> Vec<(ObjectId, Object)> {
        let r = self.inner.read();
        r.objects
            .iter()
            .filter(|(_, obj)| matches!(obj.data, ObjectData::WorkerRegistration { .. }))
            .map(|(id, obj)| (*id, obj.clone()))
            .collect()
    }

    /// Get all jobs
    pub fn get_jobs(&self) -> Vec<(ObjectId, Object)> {
        let r = self.inner.read();
        r.objects
            .iter()
            .filter(|(_, obj)| matches!(obj.data, ObjectData::InferenceJob { .. }))
            .map(|(id, obj)| (*id, obj.clone()))
            .collect()
    }

    /// Get current epoch
    pub fn get_current_epoch(&self) -> Epoch {
        self.inner.read().current_epoch.clone()
    }

    /// Update epoch state
    pub fn update_epoch(&self) {
        let now = Utc::now();
        let mut w = self.inner.write();

        let old_status = w.current_epoch.status.clone();
        w.current_epoch.update_status(now);

        // If epoch is settled, start new epoch
        if w.current_epoch.status == EpochStatus::Settled && old_status != EpochStatus::Settled {
            tracing::info!("Epoch {} settled, starting new epoch", w.current_epoch.id);

            // Calculate and distribute rewards
            let rewards = w.current_epoch.calculate_rewards();
            for (_, reward) in rewards.worker_rewards {
                let balance = w.balances.entry(reward.owner).or_insert(0);
                *balance += reward.total_reward;

                // Persist to storage
                if let Some(ref storage) = self.storage {
                    let _ = storage.save_balance(&reward.owner, *balance);
                }
            }

            // Archive current epoch and start new one
            let next_epoch_id = w.current_epoch.id + 1;
            let finished_epoch =
                std::mem::replace(&mut w.current_epoch, Epoch::new(next_epoch_id, now));
            w.past_epochs.push(finished_epoch);
        }
    }

    /// Record worker heartbeat
    pub fn record_worker_heartbeat(&self, worker_id: &ObjectId, success: bool) {
        let mut w = self.inner.write();

        // Get worker owner from object
        let owner = if let Some(obj) = w.objects.get(worker_id) {
            obj.owner
        } else {
            return;
        };

        let stats = w
            .current_epoch
            .worker_stats
            .entry(*worker_id)
            .or_insert_with(|| WorkerEpochStats::new(*worker_id, owner));

        stats.record_heartbeat(success);
    }

    /// Add VRAM snapshot for worker
    pub fn add_vram_snapshot(&self, worker_id: &ObjectId, vram_gib: f64) {
        let mut w = self.inner.write();

        if let Some(stats) = w.current_epoch.worker_stats.get_mut(worker_id) {
            stats.add_vram_snapshot(vram_gib);
        }
    }

    /// Record job completion for epoch settlement
    pub fn record_job_completion(
        &self,
        job_id: &ObjectId,
        worker_id: &ObjectId,
        requester: &Address,
        output_tokens: u64,
        receipt_hash: [u8; 32],
    ) {
        let mut w = self.inner.write();

        let cost = output_tokens * crate::epoch::PRICE_PER_TOKEN;
        let result = EpochJobResult {
            job_id: *job_id,
            worker_id: *worker_id,
            requester: *requester,
            output_tokens,
            receipt_hash,
            cost,
        };

        w.current_epoch.add_job_result(result);
    }

    /// Get the HTTP endpoint of a registered worker.
    pub fn get_worker_endpoint(&self, worker_id: &ObjectId) -> Option<String> {
        jobs::get_worker_endpoint(self, worker_id)
    }

    /// Dispatch `/jobs/accept` notifications for new job assignments in a block.
    pub fn dispatch_job_assignments(&self, block_height: u64) {
        jobs::dispatch_new_job_assignments(self, block_height, crate::epoch::PRICE_PER_TOKEN);
    }
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}
