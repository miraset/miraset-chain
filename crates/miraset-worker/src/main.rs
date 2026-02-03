/// Miraset Worker Binary
use anyhow::Result;
use miraset_worker::{Worker, WorkerConfig};
use miraset_core::KeyPair;
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().init();

    let config = WorkerConfig {
        worker_id: [1u8; 32],
        keypair: KeyPair::generate(),
        endpoint: "127.0.0.1:8080".to_string(),
        ollama_url: "http://localhost:11434".to_string(),
        gpu_model: "NVIDIA RTX 4090".to_string(),
        vram_total_gib: 24,
        supported_models: vec!["llama2".to_string()],
    };

    let worker = Worker::new(config.clone());
    let app = worker.router();

    tracing::info!("Worker listening on {}", config.endpoint);

    let listener = TcpListener::bind(&config.endpoint).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
