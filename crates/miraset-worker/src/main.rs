/// Miraset Worker Binary
use anyhow::Result;
use miraset_worker::{Worker, WorkerConfig};
use miraset_core::KeyPair;
use std::sync::Arc;
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().init();

    let config = WorkerConfig {
        worker_id: [1u8; 32],
        keypair: KeyPair::generate(),
        endpoint: "127.0.0.1:8080".to_string(),
        node_url: "http://127.0.0.1:9944".to_string(),  // Node RPC port
        ollama_url: "http://localhost:11434".to_string(),
        gpu_model: "NVIDIA RTX 4090".to_string(),
        vram_total_gib: 24,
        supported_models: vec![
            "gemma3:latest".to_string(),
            "llama3.3:latest".to_string(),
            "deepseek-r1:8b".to_string(),
            "llama2".to_string(),  // Fallback to mock if not installed
        ],
    };

    let worker = Worker::new(config.clone());

    // Register worker on-chain
    let registered_worker_id = match worker.register_on_chain().await {
        Ok(worker_id) => {
            tracing::info!("✓ Worker registered on-chain with ID: {}", hex::encode(worker_id));
            Some(worker_id)
        }
        Err(e) => {
            tracing::warn!("Failed to register on-chain (node may not be running): {}", e);
            None
        }
    };

    // D5: Start heartbeat loop (every 30 seconds)
    if let Some(worker_id) = registered_worker_id {
        tracing::info!("♥ Starting heartbeat loop (30s interval)");
        Arc::clone(&worker).start_heartbeat_loop(worker_id, 30);
    }

    let app = worker.router();

    tracing::info!("Worker listening on {}", config.endpoint);
    tracing::info!("Connecting to node at {}", config.node_url);

    let listener = TcpListener::bind(&config.endpoint).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
