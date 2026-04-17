// Example: How to use the new Sui-like object-centric API

use miraset_core::{KeyPair, Address, ObjectData, Transaction};
use miraset_node::{State, Epoch};
use chrono::Utc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize state with storage
    let state = State::new_with_storage(Some(
        miraset_node::Storage::open(".data")?
    ));

    println!("=== Sui-like Object-Centric Blockchain Demo ===\n");

    // 1. Register a GPU Worker
    println!("1. Registering GPU worker...");
    let worker_kp = KeyPair::generate();

    let register_tx = Transaction::RegisterWorker {
        owner: worker_kp.address(),
        pubkey: worker_kp.address(),
        endpoints: vec!["http://localhost:8080".to_string()],
        gpu_model: "NVIDIA RTX 4090".to_string(),
        vram_total_gib: 24,
        supported_models: vec![
            "llama-3-8b".to_string(),
            "llama-3-70b".to_string(),
        ],
        stake_bond: 10000,
        nonce: 0,
        signature: [0; 64], // Would be signed in production
    };

    state.submit_transaction(register_tx)?;
    state.produce_block();

    // Get all workers
    let workers = state.get_workers();
    println!("✓ Registered {} worker(s)", workers.len());
    for (worker_id, worker) in &workers {
        if let ObjectData::WorkerRegistration { gpu_model, vram_total_gib, .. } = &worker.data {
            println!("  Worker ID: {}", hex::encode(worker_id));
            println!("  GPU: {} ({} GiB VRAM)", gpu_model, vram_total_gib);
        }
    }

    // 2. Submit Resource Snapshot (VRAM availability)
    println!("\n2. Submitting VRAM snapshot...");
    let worker_id = workers[0].0;

    let snapshot_tx = Transaction::SubmitResourceSnapshot {
        worker_id,
        epoch_id: 0,
        vram_avail_gib: 22, // 22 GiB available
        owner: worker_kp.address(),
        nonce: 1,
        signature: [0; 64],
    };

    state.submit_transaction(snapshot_tx)?;
    state.produce_block();
    state.add_vram_snapshot(&worker_id, 22.0);
    println!("✓ VRAM snapshot submitted: 22 GiB available");

    // 3. Record Worker Heartbeats (for uptime scoring)
    println!("\n3. Recording worker heartbeats...");
    for i in 0..10 {
        state.record_worker_heartbeat(&worker_id, true); // All successful
    }
    println!("✓ Recorded 10/10 successful heartbeats (100% uptime)");

    // 4. Create an Inference Job
    println!("\n4. Creating inference job...");
    let requester_kp = KeyPair::generate();
    state.add_balance(&requester_kp.address(), 100000); // Fund requester

    let create_job_tx = Transaction::CreateJob {
        requester: requester_kp.address(),
        model_id: "llama-3-8b".to_string(),
        max_tokens: 1000,
        escrow_amount: 10000, // 1000 tokens * 10 units per token
        nonce: 0,
        signature: [0; 64],
    };

    state.submit_transaction(create_job_tx)?;
    state.produce_block();

    let jobs = state.get_jobs();
    println!("✓ Created job with {} tokens max", 1000);
    let job_id = jobs[0].0;

    // 5. Assign Job to Worker
    println!("\n5. Assigning job to worker...");
    let assign_tx = Transaction::AssignJob {
        job_id,
        worker_id,
        assigner: requester_kp.address(),
        nonce: 1,
        signature: [0; 64],
    };

    state.submit_transaction(assign_tx)?;
    state.produce_block();
    println!("✓ Job assigned to worker");

    // 6. Submit Job Result
    println!("\n6. Submitting job result...");
    let receipt_hash = [42u8; 32]; // Would be actual receipt hash in production

    let result_tx = Transaction::SubmitJobResult {
        job_id,
        worker_id,
        output_tokens: 850, // Actually generated 850 tokens
        receipt_hash,
        worker: worker_kp.address(),
        nonce: 2,
        signature: [0; 64],
    };

    state.submit_transaction(result_tx)?;
    state.produce_block();

    // Record completion for epoch settlement
    state.record_job_completion(
        &job_id,
        &worker_id,
        &requester_kp.address(),
        850,
        receipt_hash
    );
    println!("✓ Job completed: 850 tokens generated");

    // 7. Anchor Receipt Hash
    println!("\n7. Anchoring receipt hash on-chain...");
    let anchor_tx = Transaction::AnchorReceipt {
        job_id,
        receipt_hash,
        submitter: worker_kp.address(),
        nonce: 3,
        signature: [0; 64],
    };

    state.submit_transaction(anchor_tx)?;
    state.produce_block();
    println!("✓ Receipt hash anchored: {}", hex::encode(receipt_hash));

    // 8. Check Current Epoch
    println!("\n8. Current epoch status:");
    let epoch = state.get_current_epoch();
    println!("  Epoch ID: {}", epoch.id);
    println!("  Status: {:?}", epoch.status);
    println!("  Workers: {}", epoch.worker_stats.len());
    println!("  Jobs completed: {}", epoch.job_results.len());
    println!("  Verified tokens: {}", epoch.total_verified_tokens);

    // 9. Calculate Rewards (would happen at epoch end)
    println!("\n9. Calculating epoch rewards...");
    let rewards = epoch.calculate_rewards();
    println!("  Capacity pool: {} tokens", rewards.capacity_pool);
    println!("  Compute pool: {} tokens", rewards.compute_pool);

    for (wid, reward) in &rewards.worker_rewards {
        println!("\n  Worker {}:", hex::encode(&wid[..8]));
        println!("    Capacity reward: {}", reward.capacity_reward);
        println!("    Compute reward: {}", reward.compute_reward);
        println!("    Total reward: {}", reward.total_reward);
        println!("    Uptime: {:.1}%", reward.uptime_score * 100.0);
        println!("    VRAM avg: {:.1} GiB", reward.vram_avg_gib);
        println!("    Verified tokens: {}", reward.verified_tokens);
    }

    // 10. Query Objects
    println!("\n10. Querying objects by owner...");
    let owned_objects = state.get_owned_objects(&worker_kp.address());
    println!("  Worker owns {} object(s)", owned_objects.len());

    println!("\n=== Demo Complete ===");
    println!("\nKey Features Demonstrated:");
    println!("✓ Object-centric state management (Sui-like)");
    println!("✓ Worker registration as first-class objects");
    println!("✓ Job lifecycle with escrow");
    println!("✓ Resource snapshot tracking");
    println!("✓ Uptime monitoring via heartbeats");
    println!("✓ Receipt hash anchoring for proof");
    println!("✓ Epoch-based reward calculation");
    println!("✓ Capacity rewards (uptime + VRAM)");
    println!("✓ Compute rewards (verified tokens)");

    Ok(())
}
