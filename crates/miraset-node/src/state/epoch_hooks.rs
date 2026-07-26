use crate::epoch::{Epoch, EpochStatus, JobResult as EpochJobResult, WorkerEpochStats};
use crate::state::State;
use chrono::Utc;
use miraset_core::{Address, ObjectId};

/// Advance the current epoch status and, if it has just settled, distribute
/// rewards and start a new epoch.
pub(crate) fn update_epoch(state: &State) {
    let now = Utc::now();
    let mut w = state.inner.write();

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
            if let Some(ref storage) = state.storage {
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

/// Record a worker heartbeat in the current epoch stats.
pub(crate) fn record_worker_heartbeat(state: &State, worker_id: &ObjectId, success: bool) {
    let mut w = state.inner.write();

    // Get worker owner from object
    let owner = if let Some(obj) = w.objects.get(worker_id) {
        obj.owner
    } else {
        return;
    };

    let worker_stats = w
        .current_epoch
        .worker_stats
        .entry(*worker_id)
        .or_insert_with(|| WorkerEpochStats::new(*worker_id, owner));

    worker_stats.record_heartbeat(success);
}

/// Add a VRAM snapshot for a worker in the current epoch.
pub(crate) fn add_vram_snapshot(state: &State, worker_id: &ObjectId, vram_gib: f64) {
    let mut w = state.inner.write();

    if let Some(stats) = w.current_epoch.worker_stats.get_mut(worker_id) {
        stats.add_vram_snapshot(vram_gib);
    }
}

/// Record a completed job result for epoch settlement.
pub(crate) fn record_job_completion(
    state: &State,
    job_id: &ObjectId,
    worker_id: &ObjectId,
    requester: &Address,
    output_tokens: u64,
    receipt_hash: [u8; 32],
) {
    let mut w = state.inner.write();

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
