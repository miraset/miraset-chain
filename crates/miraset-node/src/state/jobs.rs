use crate::state::State;
use miraset_core::{JobStatus, ObjectData, ObjectId, WorkerStatus};

/// Find the first active worker that supports the requested model.
pub(crate) fn find_worker_for_model(
    objects: &std::collections::HashMap<ObjectId, miraset_core::Object>,
    model_id: &str,
) -> Option<ObjectId> {
    for (wid, obj) in objects {
        if let ObjectData::WorkerRegistration {
            supported_models,
            status,
            ..
        } = &obj.data
            && *status == WorkerStatus::Active
            && supported_models.iter().any(|m| m == model_id)
        {
            return Some(*wid);
        }
    }
    None
}

/// Assign an existing job object to a worker and emit a `JobAssigned` event.
///
/// The `tx_hash` is used for the assignment event so that event history stays
/// linked to the transaction that created the job.
pub(crate) fn assign_job_to_worker(
    w: &mut crate::state::StateInner,
    job_id: &ObjectId,
    worker_id: &ObjectId,
    height: u64,
    tx_hash: [u8; 32],
) {
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
        let new_version = obj.version;
        w.object_versions.insert(*job_id, new_version);

        let event = miraset_core::Event::JobAssigned {
            job_id: *job_id,
            worker_id: *worker_id,
            tx_hash,
            block_height: height,
        };
        crate::state::execution::emit_event(w, event);
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

/// Dispatch a job assignment notification to a worker endpoint.
///
/// This is a fire-and-forget HTTP call; failures are logged but do not block
/// block production.
pub(crate) fn dispatch_job_accept(
    endpoint: String,
    job_id: ObjectId,
    epoch_id: u64,
    model_id: String,
    max_tokens: u64,
    price_per_token: u64,
) {
    tokio::spawn(async move {
        let Ok(client) = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
        else {
            tracing::warn!("failed to build reqwest client for worker dispatch");
            return;
        };
        let accept_url = format!("{}/jobs/accept", endpoint.trim_end_matches('/'));
        let job_id_hex = hex::encode(job_id);
        let _ = client
            .post(&accept_url)
            .json(&serde_json::json!({
                "job_id": job_id_hex,
                "epoch_id": epoch_id,
                "model_id": model_id,
                "max_tokens": max_tokens,
                "price_per_token": price_per_token
            }))
            .send()
            .await;
    });
}

/// Scan events produced in a block and dispatch `/jobs/accept` notifications
/// to the assigned workers.
///
/// This is called by the block producer after the block has been persisted,
/// so it runs outside the state lock.
pub(crate) fn dispatch_new_job_assignments(state: &State, block_height: u64, price_per_token: u64) {
    let assignments: Vec<(ObjectId, ObjectId, u64, String, u64)> = {
        let r = state.inner.read();
        r.events
            .iter()
            .filter_map(|e| match e {
                miraset_core::Event::JobAssigned {
                    job_id,
                    worker_id,
                    block_height: h,
                    ..
                } if *h == block_height => r.objects.get(job_id).and_then(|obj| match &obj.data {
                    ObjectData::InferenceJob {
                        epoch_id,
                        model_id,
                        max_tokens,
                        ..
                    } => Some((
                        *job_id,
                        *worker_id,
                        *epoch_id,
                        model_id.clone(),
                        *max_tokens,
                    )),
                    _ => None,
                }),
                _ => None,
            })
            .collect()
    };

    for (job_id, worker_id, epoch_id, model_id, max_tokens) in assignments {
        if let Some(endpoint) = get_worker_endpoint(state, &worker_id) {
            dispatch_job_accept(
                endpoint,
                job_id,
                epoch_id,
                model_id,
                max_tokens,
                price_per_token,
            );
        }
    }
}
