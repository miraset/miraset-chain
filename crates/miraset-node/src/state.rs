use miraset_core::{Address, Block, Event, Transaction, Object, ObjectData, ObjectId, Version,
                   WorkerStatus, JobStatus, new_object_id};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use crate::storage::Storage;
use crate::epoch::{Epoch, EpochStatus, WorkerEpochStats, JobResult as EpochJobResult};
use crate::gas::GasConfig;
use chrono::Utc;

#[derive(Clone)]
pub struct State {
    inner: Arc<RwLock<StateInner>>,
    storage: Option<Storage>,
    gas_config: Arc<GasConfig>,
}

struct StateInner {
    // Object storage (Sui-like)
    objects: HashMap<ObjectId, Object>,
    object_versions: HashMap<ObjectId, Version>,
    owned_objects: HashMap<Address, Vec<ObjectId>>,

    // Account state (for backward compatibility)
    balances: HashMap<Address, u64>,
    nonces: HashMap<Address, u64>,

    // Blockchain data
    blocks: Vec<Block>,
    pending_txs: Vec<Transaction>,
    events: Vec<Event>,

    // Epoch management
    current_epoch: Epoch,
    past_epochs: Vec<Epoch>,
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

    pub fn submit_transaction(&self, tx: Transaction) -> Result<(), String> {
        // Basic validation
        let from = tx.from();
        let nonce = tx.nonce();
        let signature = tx.signature();

        // Verify signature (simplified for MVP - only for legacy transactions)
        let needs_sig_verify = matches!(tx,
            Transaction::Transfer { .. } |
            Transaction::ChatSend { .. });

        if needs_sig_verify {
            let tx_copy = tx.clone();
            let mut tx_for_hash = tx_copy;
            // Remove signature for hashing
            match &mut tx_for_hash {
                Transaction::Transfer { signature, .. } => *signature = [0; 64],
                Transaction::ChatSend { signature, .. } => *signature = [0; 64],
                _ => {}
            }
            let msg = bincode::serialize(&tx_for_hash).unwrap();
            if !miraset_core::verify_signature(from, &msg, signature) {
                return Err("Invalid signature".into());
            }
        }

        let mut w = self.inner.write();

        // Check nonce
        let current_nonce = w.nonces.get(from).copied().unwrap_or(0);
        if nonce != current_nonce {
            return Err(format!(
                "Invalid nonce: expected {}, got {}",
                current_nonce, nonce
            ));
        }

        // Type-specific validation
        match &tx {
            Transaction::Transfer { amount, .. } => {
                let balance = w.balances.get(from).copied().unwrap_or(0);
                if balance < *amount {
                    return Err("Insufficient balance".into());
                }
            }
            Transaction::ChatSend { message, .. } => {
                if message.is_empty() || message.len() > 1000 {
                    return Err("Invalid message length".into());
                }
            }
            Transaction::MutateObject { object_id, .. } => {
                if !w.objects.contains_key(object_id) {
                    return Err("Object not found".into());
                }
            }
            Transaction::CreateJob { escrow_amount, .. } => {
                let balance = w.balances.get(from).copied().unwrap_or(0);
                if balance < *escrow_amount {
                    return Err("Insufficient balance for escrow".into());
                }
            }
            _ => {
                // Other transactions validated during execution
            }
        }

        w.pending_txs.push(tx);
        Ok(())
    }

    pub fn produce_block(&self) -> Block {
        let mut w = self.inner.write();
        let prev = w.blocks.last().unwrap();
        let height = prev.height + 1;
        let prev_hash = prev.hash();

        let transactions = std::mem::take(&mut w.pending_txs);

        // Execute transactions
        for tx in &transactions {
            // Update nonce
            let from = tx.from();
            let new_nonce = w.nonces.entry(*from).or_insert(0);
            *new_nonce += 1;
            
            // Persist nonce to storage
            if let Some(ref storage) = self.storage {
                let _ = storage.save_nonce(from, *new_nonce);
            }

            // Execute transaction
            self.execute_transaction_inner(&mut w, tx, height);
        }

        let block = Block {
            height,
            timestamp: chrono::Utc::now(),
            prev_hash,
            transactions,
            state_root: [0; 32], // Simplified for MVP
        };

        w.blocks.push(block.clone());
        
        // Persist block to storage
        if let Some(ref storage) = self.storage {
            let _ = storage.save_block(&block);
            let _ = storage.flush();
        }
        
        block
    }

    /// Execute a single transaction (helper for produce_block)
    fn execute_transaction_inner(&self, w: &mut StateInner, tx: &Transaction, height: u64) {
        let tx_hash = tx.hash();

        match tx {
            Transaction::Transfer { from, to, amount, .. } => {
                let balance = w.balances.get(from).copied().unwrap_or(0);
                let new_from_balance = balance - amount;
                w.balances.insert(*from, new_from_balance);
                let new_to_balance = w.balances.entry(*to).or_insert(0);
                *new_to_balance += amount;

                if let Some(ref storage) = self.storage {
                    let _ = storage.save_balance(from, new_from_balance);
                    let _ = storage.save_balance(to, *new_to_balance);
                }

                let event = Event::Transferred {
                    from: *from,
                    to: *to,
                    amount: *amount,
                    tx_hash,
                    block_height: height,
                };
                self.emit_event(w, event);
            }

            Transaction::ChatSend { from, message, .. } => {
                let event = Event::ChatMessage {
                    from: *from,
                    message: message.clone(),
                    tx_hash,
                    block_height: height,
                    timestamp: Utc::now(),
                };
                self.emit_event(w, event);
            }

            Transaction::MoveCall { sender, .. } => {
                // Move calls are handled by the executor
                // This is just a placeholder for block inclusion
                tracing::info!("MoveCall transaction from {:?} included in block {}", sender, height);
            }

            Transaction::PublishModule { sender, .. } => {
                // Module publishing is handled by the executor
                // This is just a placeholder for block inclusion
                tracing::info!("PublishModule transaction from {:?} included in block {}", sender, height);
            }

            Transaction::RegisterWorker {
                owner,
                pubkey,
                endpoints,
                gpu_model,
                vram_total_gib,
                supported_models,
                stake_bond,
                ..
            } => {
                let data = ObjectData::WorkerRegistration {
                    worker_id: new_object_id(&bincode::serialize(&(owner, pubkey)).unwrap()),
                    pubkey: *pubkey,
                    endpoints: endpoints.clone(),
                    gpu_model: gpu_model.clone(),
                    vram_total_gib: *vram_total_gib,
                    supported_models: supported_models.clone(),
                    stake_bond: *stake_bond,
                    status: WorkerStatus::Active,
                };

                let obj = Object::new(*owner, data);
                let worker_id = obj.id;
                w.objects.insert(worker_id, obj);
                w.object_versions.insert(worker_id, 0);
                w.owned_objects.entry(*owner).or_insert_with(Vec::new).push(worker_id);

                // Initialize worker stats in current epoch
                w.current_epoch.worker_stats.insert(
                    worker_id,
                    WorkerEpochStats::new(worker_id, *owner)
                );

                let event = Event::WorkerRegistered {
                    worker_id,
                    owner: *owner,
                    gpu_model: gpu_model.clone(),
                    vram_gib: *vram_total_gib,
                    tx_hash,
                    block_height: height,
                };
                self.emit_event(w, event);
            }

            Transaction::SubmitResourceSnapshot {
                worker_id,
                epoch_id,
                vram_avail_gib,
                owner: _,
                ..
            } => {
                // Add VRAM snapshot to epoch stats
                if let Some(stats) = w.current_epoch.worker_stats.get_mut(worker_id) {
                    stats.add_vram_snapshot(*vram_avail_gib as f64);
                }

                let event = Event::ResourceSnapshotSubmitted {
                    worker_id: *worker_id,
                    epoch_id: *epoch_id,
                    vram_avail_gib: *vram_avail_gib,
                    tx_hash,
                    block_height: height,
                };
                self.emit_event(w, event);
            }

            Transaction::CreateJob {
                requester,
                model_id,
                max_tokens,
                escrow_amount,
                ..
            } => {
                // Deduct escrow from requester
                let balance = w.balances.get(requester).copied().unwrap_or(0);
                let new_balance = balance - escrow_amount;
                w.balances.insert(*requester, new_balance);

                if let Some(ref storage) = self.storage {
                    let _ = storage.save_balance(requester, new_balance);
                }

                // Create job object
                let job_id = new_object_id(&bincode::serialize(&(requester, model_id, Utc::now())).unwrap());
                let data = ObjectData::InferenceJob {
                    job_id,
                    epoch_id: w.current_epoch.id,
                    requester: *requester,
                    model_id: model_id.clone(),
                    max_tokens: *max_tokens,
                    assigned_worker_id: None,
                    fixed_price_per_token: crate::epoch::PRICE_PER_TOKEN,
                    escrow_amount: *escrow_amount,
                    status: JobStatus::Created,
                    created_at: Utc::now(),
                };

                let obj = Object::new(*requester, data);
                w.objects.insert(job_id, obj);
                w.object_versions.insert(job_id, 0);
                w.owned_objects.entry(*requester).or_insert_with(Vec::new).push(job_id);

                let event = Event::JobCreated {
                    job_id,
                    requester: *requester,
                    model_id: model_id.clone(),
                    max_tokens: *max_tokens,
                    escrow_amount: *escrow_amount,
                    tx_hash,
                    block_height: height,
                };
                self.emit_event(w, event);
            }

            Transaction::AssignJob {
                job_id,
                worker_id,
                ..
            } => {
                // Update job object to assigned status
                if let Some(obj) = w.objects.get_mut(job_id) {
                    if let ObjectData::InferenceJob { assigned_worker_id, status, .. } = &mut obj.data {
                        *assigned_worker_id = Some(*worker_id);
                        *status = JobStatus::Assigned;
                        obj.version += 1;
                        w.object_versions.insert(*job_id, obj.version);
                    }
                }

                let event = Event::JobAssigned {
                    job_id: *job_id,
                    worker_id: *worker_id,
                    tx_hash,
                    block_height: height,
                };
                self.emit_event(w, event);
            }

            Transaction::SubmitJobResult {
                job_id,
                worker_id,
                output_tokens,
                receipt_hash,
                ..
            } => {
                // Update job status to completed
                if let Some(obj) = w.objects.get_mut(job_id) {
                    if let ObjectData::InferenceJob { status, requester, .. } = &mut obj.data {
                        *status = JobStatus::Completed;
                        obj.version += 1;
                        w.object_versions.insert(*job_id, obj.version);

                        // Record completion in epoch
                        let cost = output_tokens * crate::epoch::PRICE_PER_TOKEN;
                        let result = EpochJobResult {
                            job_id: *job_id,
                            worker_id: *worker_id,
                            requester: *requester,
                            output_tokens: *output_tokens,
                            receipt_hash: *receipt_hash,
                            cost,
                        };
                        w.current_epoch.add_job_result(result);
                    }
                }

                let event = Event::JobCompleted {
                    job_id: *job_id,
                    worker_id: *worker_id,
                    output_tokens: *output_tokens,
                    tx_hash,
                    block_height: height,
                };
                self.emit_event(w, event);
            }

            Transaction::AnchorReceipt {
                job_id,
                receipt_hash,
                ..
            } => {
                let event = Event::ReceiptAnchored {
                    job_id: *job_id,
                    receipt_hash: *receipt_hash,
                    tx_hash,
                    block_height: height,
                };
                self.emit_event(w, event);
            }

            Transaction::ChallengeJob {
                job_id,
                challenger,
                reason,
                ..
            } => {
                // Update job status to challenged
                if let Some(obj) = w.objects.get_mut(job_id) {
                    if let ObjectData::InferenceJob { status, .. } = &mut obj.data {
                        *status = JobStatus::Challenged;
                        obj.version += 1;
                        w.object_versions.insert(*job_id, obj.version);
                    }
                }

                let event = Event::JobChallenged {
                    job_id: *job_id,
                    challenger: *challenger,
                    reason: reason.clone(),
                    tx_hash,
                    block_height: height,
                };
                self.emit_event(w, event);
            }

            Transaction::CreateObject { creator, data, .. } => {
                let obj = Object::new(*creator, data.clone());
                let obj_id = obj.id;
                let obj_type = format!("{:?}", data).split('{').next().unwrap_or("Unknown").to_string();

                w.objects.insert(obj_id, obj);
                w.object_versions.insert(obj_id, 0);
                w.owned_objects.entry(*creator).or_insert_with(Vec::new).push(obj_id);

                let event = Event::ObjectCreated {
                    object_id: obj_id,
                    owner: *creator,
                    object_type: obj_type,
                    tx_hash,
                    block_height: height,
                };
                self.emit_event(w, event);
            }

            Transaction::MutateObject { object_id, new_data, owner, version, .. } => {
                if let Some(obj) = w.objects.get_mut(object_id) {
                    obj.data = new_data.clone();
                    obj.version = version + 1;
                    w.object_versions.insert(*object_id, obj.version);

                    let event = Event::ObjectMutated {
                        object_id: *object_id,
                        version: obj.version,
                        owner: *owner,
                        tx_hash,
                        block_height: height,
                    };
                    self.emit_event(w, event);
                }
            }

            Transaction::TransferObject { object_id, from, to, .. } => {
                if let Some(obj) = w.objects.get_mut(object_id) {
                    obj.owner = *to;

                    // Update ownership index
                    if let Some(owned) = w.owned_objects.get_mut(from) {
                        owned.retain(|id| id != object_id);
                    }
                    w.owned_objects.entry(*to).or_insert_with(Vec::new).push(*object_id);

                    let event = Event::ObjectTransferred {
                        object_id: *object_id,
                        from: *from,
                        to: *to,
                        tx_hash,
                        block_height: height,
                    };
                    self.emit_event(w, event);
                }
            }
        }
    }

    /// Helper to emit event
    fn emit_event(&self, w: &mut StateInner, event: Event) {
        let event_index = w.events.len() as u64;
        w.events.push(event.clone());

        if let Some(ref storage) = self.storage {
            let _ = storage.save_event(event_index, &event);
        }
    }

    pub fn get_latest_block(&self) -> Block {
        self.inner.read().blocks.last().unwrap().clone()
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

    pub fn get_chat_messages(&self, limit: usize) -> Vec<(Address, String, chrono::DateTime<chrono::Utc>)> {
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

    pub fn height(&self) -> u64 {
        self.inner.read().blocks.last().unwrap().height
    }

    // ===== Object-centric methods (Sui-like) =====

    /// Create a new object from data (convenience method)
    pub fn create_object_from_data(&self, owner: Address, data: ObjectData) -> ObjectId {
        let mut w = self.inner.write();
        let obj = Object::new(owner, data);
        let obj_id = obj.id;

        w.objects.insert(obj_id, obj);
        w.object_versions.insert(obj_id, 0);
        w.owned_objects.entry(owner).or_insert_with(Vec::new).push(obj_id);

        obj_id
    }

    /// Get an object by ID
    pub fn get_object(&self, object_id: &ObjectId) -> Option<Object> {
        self.inner.read().objects.get(object_id).cloned()
    }

    /// Get all objects owned by an address
    pub fn get_owned_objects(&self, owner: &Address) -> Vec<Object> {
        let r = self.inner.read();
        if let Some(obj_ids) = r.owned_objects.get(owner) {
            obj_ids.iter()
                .filter_map(|id| r.objects.get(id).cloned())
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Mutate an object (owner-only, with version check)
    pub fn mutate_object(&self, object_id: &ObjectId, expected_version: Version, new_data: ObjectData, owner: &Address) -> Result<(), String> {
        let mut w = self.inner.write();

        // Check version first
        let current_version = w.object_versions.get(object_id).copied().unwrap_or(0);
        if current_version != expected_version {
            return Err(format!("Version mismatch: expected {}, got {}", expected_version, current_version));
        }

        let obj = w.objects.get_mut(object_id)
            .ok_or("Object not found")?;

        if obj.owner != *owner {
            return Err("Not owner".into());
        }

        obj.data = new_data;
        let new_version = current_version + 1;
        obj.version = new_version;
        w.object_versions.insert(*object_id, new_version);

        Ok(())
    }

    /// Transfer object ownership
    pub fn transfer_object(&self, object_id: &ObjectId, from: &Address, to: &Address) -> Result<(), String> {
        let mut w = self.inner.write();

        let obj = w.objects.get_mut(object_id)
            .ok_or("Object not found")?;

        if obj.owner != *from {
            return Err("Not owner".into());
        }

        obj.owner = *to;

        // Update ownership index
        if let Some(owned) = w.owned_objects.get_mut(from) {
            owned.retain(|id| id != object_id);
        }
        w.owned_objects.entry(*to).or_insert_with(Vec::new).push(*object_id);

        Ok(())
    }

    /// Create a new object and store it
    pub fn create_object(&self, object: Object) -> Result<(), String> {
        let mut w = self.inner.write();

        let object_id = object.id;
        let owner = object.owner;

        // Store object
        w.objects.insert(object_id, object);
        w.object_versions.insert(object_id, 0);

        // Update ownership index
        w.owned_objects.entry(owner).or_insert_with(Vec::new).push(object_id);

        Ok(())
    }

    /// Update an existing object (used by executor)
    pub fn update_object(&self, object: Object) -> Result<(), String> {
        let mut w = self.inner.write();

        let object_id = object.id;

        if !w.objects.contains_key(&object_id) {
            return Err("Object not found".into());
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
        r.objects.iter()
            .filter(|(_, obj)| matches!(obj.data, ObjectData::WorkerRegistration { .. }))
            .map(|(id, obj)| (*id, obj.clone()))
            .collect()
    }

    /// Get all jobs
    pub fn get_jobs(&self) -> Vec<(ObjectId, Object)> {
        let r = self.inner.read();
        r.objects.iter()
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
            let finished_epoch = std::mem::replace(
                &mut w.current_epoch,
                Epoch::new(next_epoch_id, now)
            );
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

        let stats = w.current_epoch.worker_stats
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
    pub fn record_job_completion(&self, job_id: &ObjectId, worker_id: &ObjectId, requester: &Address, output_tokens: u64, receipt_hash: [u8; 32]) {
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
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use miraset_core::KeyPair;

    #[test]
    fn test_state_new() {
        let state = State::new();
        assert_eq!(state.height(), 0);
    }

    #[test]
    fn test_get_balance_zero() {
        let state = State::new();
        let kp = KeyPair::generate();
        assert_eq!(state.get_balance(&kp.address()), 0);
    }

    #[test]
    fn test_add_balance() {
        let state = State::new();
        let kp = KeyPair::generate();

        state.add_balance(&kp.address(), 1000);
        assert_eq!(state.get_balance(&kp.address()), 1000);

        state.add_balance(&kp.address(), 500);
        assert_eq!(state.get_balance(&kp.address()), 1500);
    }

    #[test]
    fn test_get_nonce_initial() {
        let state = State::new();
        let kp = KeyPair::generate();
        assert_eq!(state.get_nonce(&kp.address()), 0);
    }

    #[test]
    fn test_submit_transfer_valid() {
        let state = State::new();
        let kp = KeyPair::generate();
        let recipient = KeyPair::generate();

        state.add_balance(&kp.address(), 2000);

        let mut tx = Transaction::Transfer {
            from: kp.address(),
            to: recipient.address(),
            amount: 500,
            nonce: 0,
            signature: [0; 64],
        };

        // Sign transaction
        let mut tx_for_hash = tx.clone();
        match &mut tx_for_hash {
            Transaction::Transfer { signature, .. } => *signature = [0; 64],
            _ => {}
        }
        let msg = bincode::serialize(&tx_for_hash).unwrap();
        let sig = kp.sign(&msg);

        match &mut tx {
            Transaction::Transfer { signature, .. } => *signature = sig,
            _ => {}
        }

        let result = state.submit_transaction(tx);
        assert!(result.is_ok());
    }

    #[test]
    fn test_submit_transfer_insufficient_balance() {
        let state = State::new();
        let kp = KeyPair::generate();
        let recipient = KeyPair::generate();

        state.add_balance(&kp.address(), 100);

        let mut tx = Transaction::Transfer {
            from: kp.address(),
            to: recipient.address(),
            amount: 500,
            nonce: 0,
            signature: [0; 64],
        };

        // Sign transaction
        let mut tx_for_hash = tx.clone();
        match &mut tx_for_hash {
            Transaction::Transfer { signature, .. } => *signature = [0; 64],
            _ => {}
        }
        let msg = bincode::serialize(&tx_for_hash).unwrap();
        let sig = kp.sign(&msg);

        match &mut tx {
            Transaction::Transfer { signature, .. } => *signature = sig,
            _ => {}
        }

        let result = state.submit_transaction(tx);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Insufficient balance"));
    }

    #[test]
    fn test_submit_transfer_invalid_nonce() {
        let state = State::new();
        let kp = KeyPair::generate();
        let recipient = KeyPair::generate();

        state.add_balance(&kp.address(), 2000);

        let mut tx = Transaction::Transfer {
            from: kp.address(),
            to: recipient.address(),
            amount: 500,
            nonce: 5, // Wrong nonce
            signature: [0; 64],
        };

        // Sign transaction
        let mut tx_for_hash = tx.clone();
        match &mut tx_for_hash {
            Transaction::Transfer { signature, .. } => *signature = [0; 64],
            _ => {}
        }
        let msg = bincode::serialize(&tx_for_hash).unwrap();
        let sig = kp.sign(&msg);

        match &mut tx {
            Transaction::Transfer { signature, .. } => *signature = sig,
            _ => {}
        }

        let result = state.submit_transaction(tx);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid nonce"));
    }

    #[test]
    fn test_submit_transfer_invalid_signature() {
        let state = State::new();
        let kp = KeyPair::generate();
        let recipient = KeyPair::generate();

        state.add_balance(&kp.address(), 2000);

        let tx = Transaction::Transfer {
            from: kp.address(),
            to: recipient.address(),
            amount: 500,
            nonce: 0,
            signature: [0; 64], // Invalid signature
        };

        let result = state.submit_transaction(tx);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid signature"));
    }

    #[test]
    fn test_submit_chat_valid() {
        let state = State::new();
        let kp = KeyPair::generate();

        let mut tx = Transaction::ChatSend {
            from: kp.address(),
            message: "Hello, blockchain!".to_string(),
            nonce: 0,
            signature: [0; 64],
        };

        // Sign transaction
        let mut tx_for_hash = tx.clone();
        match &mut tx_for_hash {
            Transaction::ChatSend { signature, .. } => *signature = [0; 64],
            _ => {}
        }
        let msg = bincode::serialize(&tx_for_hash).unwrap();
        let sig = kp.sign(&msg);

        match &mut tx {
            Transaction::ChatSend { signature, .. } => *signature = sig,
            _ => {}
        }

        let result = state.submit_transaction(tx);
        assert!(result.is_ok());
    }

    #[test]
    fn test_submit_chat_empty_message() {
        let state = State::new();
        let kp = KeyPair::generate();

        let mut tx = Transaction::ChatSend {
            from: kp.address(),
            message: "".to_string(),
            nonce: 0,
            signature: [0; 64],
        };

        // Sign transaction
        let mut tx_for_hash = tx.clone();
        match &mut tx_for_hash {
            Transaction::ChatSend { signature, .. } => *signature = [0; 64],
            _ => {}
        }
        let msg = bincode::serialize(&tx_for_hash).unwrap();
        let sig = kp.sign(&msg);

        match &mut tx {
            Transaction::ChatSend { signature, .. } => *signature = sig,
            _ => {}
        }

        let result = state.submit_transaction(tx);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid message length"));
    }

    #[test]
    fn test_submit_chat_message_too_long() {
        let state = State::new();
        let kp = KeyPair::generate();

        let long_message = "x".repeat(1001);
        let mut tx = Transaction::ChatSend {
            from: kp.address(),
            message: long_message,
            nonce: 0,
            signature: [0; 64],
        };

        // Sign transaction
        let mut tx_for_hash = tx.clone();
        match &mut tx_for_hash {
            Transaction::ChatSend { signature, .. } => *signature = [0; 64],
            _ => {}
        }
        let msg = bincode::serialize(&tx_for_hash).unwrap();
        let sig = kp.sign(&msg);

        match &mut tx {
            Transaction::ChatSend { signature, .. } => *signature = sig,
            _ => {}
        }

        let result = state.submit_transaction(tx);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid message length"));
    }

    #[test]
    fn test_produce_block() {
        let state = State::new();
        let kp = KeyPair::generate();
        let recipient = KeyPair::generate();

        state.add_balance(&kp.address(), 1000);

        // Submit transaction
        let mut tx = Transaction::Transfer {
            from: kp.address(),
            to: recipient.address(),
            amount: 300,
            nonce: 0,
            signature: [0; 64],
        };

        let mut tx_for_hash = tx.clone();
        match &mut tx_for_hash {
            Transaction::Transfer { signature, .. } => *signature = [0; 64],
            _ => {}
        }
        let msg = bincode::serialize(&tx_for_hash).unwrap();
        let sig = kp.sign(&msg);

        match &mut tx {
            Transaction::Transfer { signature, .. } => *signature = sig,
            _ => {}
        }

        state.submit_transaction(tx).unwrap();

        // Produce block
        let block = state.produce_block();

        assert_eq!(block.height, 1);
        assert_eq!(block.transactions.len(), 1);

        // Check balances updated
        assert_eq!(state.get_balance(&kp.address()), 700);
        assert_eq!(state.get_balance(&recipient.address()), 300);

        // Check nonce incremented
        assert_eq!(state.get_nonce(&kp.address()), 1);
    }

    #[test]
    fn test_produce_block_multiple_transactions() {
        let state = State::new();
        let kp1 = KeyPair::generate();
        let kp2 = KeyPair::generate();

        state.add_balance(&kp1.address(), 1000);
        state.add_balance(&kp2.address(), 1000);

        // Submit two transactions
        for (kp, amount) in [(kp1, 100), (kp2, 200)] {
            let mut tx = Transaction::ChatSend {
                from: kp.address(),
                message: format!("Message {}", amount),
                nonce: 0,
                signature: [0; 64],
            };

            let mut tx_for_hash = tx.clone();
            match &mut tx_for_hash {
                Transaction::ChatSend { signature, .. } => *signature = [0; 64],
                _ => {}
            }
            let msg = bincode::serialize(&tx_for_hash).unwrap();
            let sig = kp.sign(&msg);

            match &mut tx {
                Transaction::ChatSend { signature, .. } => *signature = sig,
                _ => {}
            }

            state.submit_transaction(tx).unwrap();
        }

        let block = state.produce_block();
        assert_eq!(block.transactions.len(), 2);
        assert_eq!(state.height(), 1);
    }

    #[test]
    fn test_get_latest_block() {
        let state = State::new();
        let latest = state.get_latest_block();
        assert_eq!(latest.height, 0);
    }

    #[test]
    fn test_get_block_by_height() {
        let state = State::new();

        let genesis = state.get_block(0);
        assert!(genesis.is_some());
        assert_eq!(genesis.unwrap().height, 0);

        let nonexistent = state.get_block(999);
        assert!(nonexistent.is_none());
    }

    #[test]
    fn test_get_events() {
        let state = State::new();
        let kp = KeyPair::generate();
        let recipient = KeyPair::generate();

        state.add_balance(&kp.address(), 1000);

        // Submit and produce block
        let mut tx = Transaction::Transfer {
            from: kp.address(),
            to: recipient.address(),
            amount: 100,
            nonce: 0,
            signature: [0; 64],
        };

        let mut tx_for_hash = tx.clone();
        match &mut tx_for_hash {
            Transaction::Transfer { signature, .. } => *signature = [0; 64],
            _ => {}
        }
        let msg = bincode::serialize(&tx_for_hash).unwrap();
        let sig = kp.sign(&msg);

        match &mut tx {
            Transaction::Transfer { signature, .. } => *signature = sig,
            _ => {}
        }

        state.submit_transaction(tx).unwrap();
        state.produce_block();

        let events = state.get_events(0, 10);
        assert_eq!(events.len(), 1);

        match &events[0] {
            Event::Transferred { from, to, amount, .. } => {
                assert_eq!(*from, kp.address());
                assert_eq!(*to, recipient.address());
                assert_eq!(*amount, 100);
            }
            _ => panic!("Expected Transferred event"),
        }
    }

    #[test]
    fn test_get_chat_messages() {
        let state = State::new();
        let kp = KeyPair::generate();

        // Submit chat messages and produce block after each
        for i in 1..=3 {
            let mut tx = Transaction::ChatSend {
                from: kp.address(),
                message: format!("Message {}", i),
                nonce: i - 1,
                signature: [0; 64],
            };

            let mut tx_for_hash = tx.clone();
            match &mut tx_for_hash {
                Transaction::ChatSend { signature, .. } => *signature = [0; 64],
                _ => {}
            }
            let msg = bincode::serialize(&tx_for_hash).unwrap();
            let sig = kp.sign(&msg);

            match &mut tx {
                Transaction::ChatSend { signature, .. } => *signature = sig,
                _ => {}
            }

            state.submit_transaction(tx).unwrap();
            state.produce_block(); // Produce block to increment nonce
        }

        let messages = state.get_chat_messages(10);
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].1, "Message 1");
        assert_eq!(messages[1].1, "Message 2");
        assert_eq!(messages[2].1, "Message 3");
    }

    #[test]
    fn test_worker_register() {
        let state = State::new();
        let kp = KeyPair::generate();

        let mut tx = Transaction::RegisterWorker {
            owner: kp.address(),
            pubkey: kp.address(),
            endpoints: vec!["http://localhost:8080".to_string()],
            gpu_model: "NVIDIA RTX 4090".to_string(),
            vram_total_gib: 24,
            supported_models: vec!["llama-3-8b".to_string()],
            stake_bond: 1000,
            nonce: 0,
            signature: [0; 64],
        };

        let mut tx_for_hash = tx.clone();
        match &mut tx_for_hash {
            Transaction::RegisterWorker { signature, .. } => *signature = [0; 64],
            _ => {}
        }
        let msg = bincode::serialize(&tx_for_hash).unwrap();
        let sig = kp.sign(&msg);

        match &mut tx {
            Transaction::RegisterWorker { signature, .. } => *signature = sig,
            _ => {}
        }

        state.submit_transaction(tx).unwrap();
        state.produce_block();

        let events = state.get_events(0, 10);
        assert_eq!(events.len(), 1);

        match &events[0] {
            Event::WorkerRegistered { owner, gpu_model, vram_gib, .. } => {
                assert_eq!(*owner, kp.address());
                assert_eq!(gpu_model, "NVIDIA RTX 4090");
                assert_eq!(*vram_gib, 24);
            }
            _ => panic!("Expected WorkerRegistered event"),
        }
    }

    #[test]
    fn test_height() {
        let state = State::new();
        assert_eq!(state.height(), 0);

        state.produce_block();
        assert_eq!(state.height(), 1);

        state.produce_block();
        assert_eq!(state.height(), 2);
    }
}
