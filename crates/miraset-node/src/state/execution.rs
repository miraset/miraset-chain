use crate::epoch::JobResult as EpochJobResult;
use crate::error::StateError;
use crate::state::{StateInner, jobs};
use chrono::Utc;
use miraset_core::{
    Address, Event, JobStatus, Object, ObjectData, Transaction, WorkerStatus, new_object_id,
};

/// Execute a single transaction (helper for produce_block).
///
/// Balance changes are collected in `balance_updates` so that the caller can
/// persist them to storage after releasing the state lock.
pub(crate) fn execute_transaction_inner(
    w: &mut StateInner,
    tx: &Transaction,
    height: u64,
    balance_updates: &mut Vec<(Address, u64)>,
) -> Result<(), StateError> {
    let tx_hash = tx.hash()?;

    match tx {
        Transaction::Transfer {
            from, to, amount, ..
        } => {
            let balance = w.balances.get(from).copied().unwrap_or(0);
            let new_from_balance = balance - amount;
            w.balances.insert(*from, new_from_balance);
            let new_to_balance = w.balances.entry(*to).or_insert(0);
            *new_to_balance += amount;

            balance_updates.push((*from, new_from_balance));
            balance_updates.push((*to, *new_to_balance));

            let event = Event::Transferred {
                from: *from,
                to: *to,
                amount: *amount,
                tx_hash,
                block_height: height,
            };
            emit_event(w, event);
        }

        Transaction::ChatSend { from, message, .. } => {
            let event = Event::ChatMessage {
                from: *from,
                message: message.clone(),
                tx_hash,
                block_height: height,
                timestamp: Utc::now(),
            };
            emit_event(w, event);
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
                worker_id: new_object_id(&bincode::serialize(&(owner, pubkey))?),
                pubkey: *pubkey,
                endpoints: endpoints.clone(),
                gpu_model: gpu_model.clone(),
                vram_total_gib: *vram_total_gib,
                supported_models: supported_models.clone(),
                stake_bond: *stake_bond,
                status: WorkerStatus::Active,
            };

            let obj = Object::new(*owner, data)?;
            let worker_id = obj.id;
            w.objects.insert(worker_id, obj);
            w.object_versions.insert(worker_id, 0);
            w.owned_objects.entry(*owner).or_default().push(worker_id);

            // Initialize worker stats in current epoch
            w.current_epoch.worker_stats.insert(
                worker_id,
                crate::epoch::WorkerEpochStats::new(worker_id, *owner),
            );

            let event = Event::WorkerRegistered {
                worker_id,
                owner: *owner,
                gpu_model: gpu_model.clone(),
                vram_gib: *vram_total_gib,
                tx_hash,
                block_height: height,
            };
            emit_event(w, event);
        }

        Transaction::SubmitResourceSnapshot {
            worker_id,
            epoch_id,
            vram_avail_gib,
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
            emit_event(w, event);
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

            balance_updates.push((*requester, new_balance));

            // Create job object. The job ID is derived from the transaction
            // hash so it is deterministic per tx while staying unique.
            let job_id = new_object_id(&bincode::serialize(&(requester, model_id, tx_hash))?);
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

            let obj = Object::new(*requester, data)?;
            w.objects.insert(job_id, obj);
            w.object_versions.insert(job_id, 0);
            w.owned_objects.entry(*requester).or_default().push(job_id);

            let event = Event::JobCreated {
                job_id,
                requester: *requester,
                model_id: model_id.clone(),
                max_tokens: *max_tokens,
                escrow_amount: *escrow_amount,
                tx_hash,
                block_height: height,
            };
            emit_event(w, event);

            // Auto-assign to the first active worker that supports the model.
            // The assignment happens inside the same block as job creation so
            // the coordinator path no longer mutates state outside block
            // boundaries.
            if let Some(worker_id) = jobs::find_worker_for_model(&w.objects, model_id) {
                jobs::assign_job_to_worker(w, &job_id, &worker_id, height, tx_hash);
            }
        }

        Transaction::AssignJob {
            job_id, worker_id, ..
        } => {
            // Update job object to assigned status
            if let Some(obj) = w.objects.get_mut(job_id)
                && let ObjectData::InferenceJob {
                    assigned_worker_id,
                    status,
                    ..
                } = &mut obj.data
            {
                *assigned_worker_id = Some(*worker_id);
                *status = JobStatus::Assigned;
                obj.version += 1;
                w.object_versions.insert(*job_id, obj.version);
            }

            let event = Event::JobAssigned {
                job_id: *job_id,
                worker_id: *worker_id,
                tx_hash,
                block_height: height,
            };
            emit_event(w, event);
        }

        Transaction::SubmitJobResult {
            job_id,
            worker_id,
            output_tokens,
            receipt_hash,
            ..
        } => {
            // Update job status to completed
            if let Some(obj) = w.objects.get_mut(job_id)
                && let ObjectData::InferenceJob {
                    status, requester, ..
                } = &mut obj.data
            {
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

            let event = Event::JobCompleted {
                job_id: *job_id,
                worker_id: *worker_id,
                output_tokens: *output_tokens,
                tx_hash,
                block_height: height,
            };
            emit_event(w, event);
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
            emit_event(w, event);
        }

        Transaction::ChallengeJob {
            job_id,
            challenger,
            reason,
            ..
        } => {
            // Update job status to challenged
            if let Some(obj) = w.objects.get_mut(job_id)
                && let ObjectData::InferenceJob { status, .. } = &mut obj.data
            {
                *status = JobStatus::Challenged;
                obj.version += 1;
                w.object_versions.insert(*job_id, obj.version);
            }

            let event = Event::JobChallenged {
                job_id: *job_id,
                challenger: *challenger,
                reason: reason.clone(),
                tx_hash,
                block_height: height,
            };
            emit_event(w, event);
        }

        Transaction::CreateObject { creator, data, .. } => {
            let obj = Object::new(*creator, data.clone())?;
            let obj_id = obj.id;
            let obj_type = format!("{:?}", data)
                .split('{')
                .next()
                .unwrap_or("Unknown")
                .to_string();

            w.objects.insert(obj_id, obj);
            w.object_versions.insert(obj_id, 0);
            w.owned_objects.entry(*creator).or_default().push(obj_id);

            let event = Event::ObjectCreated {
                object_id: obj_id,
                owner: *creator,
                object_type: obj_type,
                tx_hash,
                block_height: height,
            };
            emit_event(w, event);
        }

        Transaction::MutateObject {
            object_id,
            new_data,
            owner,
            version,
            ..
        } => {
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
                emit_event(w, event);
            }
        }

        Transaction::TransferObject {
            object_id,
            from,
            to,
            ..
        } => {
            if let Some(obj) = w.objects.get_mut(object_id) {
                obj.owner = *to;

                // Update ownership index
                if let Some(owned) = w.owned_objects.get_mut(from) {
                    owned.retain(|id| id != object_id);
                }
                w.owned_objects.entry(*to).or_default().push(*object_id);

                let event = Event::ObjectTransferred {
                    object_id: *object_id,
                    from: *from,
                    to: *to,
                    tx_hash,
                    block_height: height,
                };
                emit_event(w, event);
            }
        }
    }
    Ok(())
}

/// Helper to emit event.
///
/// Events are kept in memory only; the caller is responsible for persisting
/// them to storage after releasing the state lock.
pub(crate) fn emit_event(w: &mut StateInner, event: Event) {
    w.events.push(event);
}
