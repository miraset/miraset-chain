use crate::error::StateError;
use crate::state::State;
use chrono::Utc;
use miraset_core::{Address, JobStatus, Object, ObjectData, ObjectId, WorkerStatus, new_object_id};

/// Create a job directly (coordinator path, no TX needed for demo).
pub(crate) fn create_job(
    state: &State,
    requester: &Address,
    model_id: String,
    max_tokens: u64,
    escrow_amount: u64,
) -> Result<ObjectId, StateError> {
    let mut w = state.inner.write();

    let balance = w.balances.get(requester).copied().unwrap_or(0);
    if balance < escrow_amount {
        return Err(StateError::Other(
            "Insufficient balance for escrow".to_string(),
        ));
    }

    // Deduct escrow
    let new_balance = balance - escrow_amount;
    w.balances.insert(*requester, new_balance);
    if let Some(ref storage) = state.storage {
        let _ = storage.save_balance(requester, new_balance);
    }

    let job_id = new_object_id(&bincode::serialize(&(
        requester,
        &model_id,
        Utc::now().timestamp_nanos_opt(),
    ))?);

    let data = ObjectData::InferenceJob {
        job_id,
        epoch_id: w.current_epoch.id,
        requester: *requester,
        model_id,
        max_tokens,
        assigned_worker_id: None,
        fixed_price_per_token: crate::epoch::PRICE_PER_TOKEN,
        escrow_amount,
        status: JobStatus::Created,
        created_at: Utc::now(),
    };

    let obj = Object::new(*requester, data)?;
    w.objects.insert(job_id, obj);
    w.object_versions.insert(job_id, 0);
    w.owned_objects.entry(*requester).or_default().push(job_id);

    Ok(job_id)
}

/// Auto-assign a job to the first available worker that supports the model.
pub(crate) fn auto_assign_job(
    state: &State,
    job_id: &ObjectId,
    model_id: &str,
) -> Option<ObjectId> {
    let mut w = state.inner.write();

    // Find a worker that supports this model and is Active
    let worker_id = {
        let mut found = None;
        for (wid, obj) in &w.objects {
            if let ObjectData::WorkerRegistration {
                supported_models,
                status,
                ..
            } = &obj.data
                && *status == WorkerStatus::Active
                && supported_models.iter().any(|m| m == model_id)
            {
                found = Some(*wid);
                break;
            }
        }
        found
    };

    if let Some(worker_id) = worker_id {
        // Update job with assignment
        if let Some(obj) = w.objects.get_mut(job_id) {
            if let ObjectData::InferenceJob {
                assigned_worker_id,
                status,
                ..
            } = &mut obj.data
            {
                *assigned_worker_id = Some(worker_id);
                *status = JobStatus::Assigned;
                obj.version += 1;
            }
            let new_version = obj.version;
            w.object_versions.insert(*job_id, new_version);
        }
        Some(worker_id)
    } else {
        None
    }
}

/// Get the HTTP endpoint of a registered worker.
pub(crate) fn get_worker_endpoint(state: &State, worker_id: &ObjectId) -> Option<String> {
    let r = state.inner.read();
    if let Some(obj) = r.objects.get(worker_id)
        && let ObjectData::WorkerRegistration { endpoints, .. } = &obj.data
    {
        return endpoints.first().cloned();
    }
    None
}
