/// Miraset Worker Runtime
///
/// Ollama-like HTTP server that:
/// - Accepts job assignments
/// - Executes inference via Ollama/vLLM backend
/// - Generates signed receipts with canonical hashing
/// - Submits results to chain

use anyhow::{anyhow, Result};
use axum::{
    extract::Path,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use miraset_core::{KeyPair, ObjectId};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use parking_lot::RwLock;
use std::collections::HashMap;
use chrono::{DateTime, Utc};

mod receipt;
mod backend;

pub use receipt::{ReceiptPayload, ReceiptHash};
pub use backend::{InferenceBackend, OllamaBackend};

/// Worker configuration
#[derive(Debug, Clone)]
pub struct WorkerConfig {
    pub worker_id: ObjectId,
    pub keypair: KeyPair,
    pub endpoint: String,
    pub ollama_url: String,
    pub gpu_model: String,
    pub vram_total_gib: u32,
    pub supported_models: Vec<String>,
}

/// Worker state
pub struct Worker {
    config: WorkerConfig,
    jobs: Arc<RwLock<HashMap<ObjectId, JobExecution>>>,
    backend: Arc<dyn InferenceBackend>,
}

/// Job execution state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobExecution {
    pub job_id: ObjectId,
    pub epoch_id: u64,
    pub model_id: String,
    pub prompt: String,
    pub max_tokens: u64,
    pub status: JobStatus,
    pub assigned_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub output_tokens: u64,
    pub response: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum JobStatus {
    Accepted,
    Running,
    Completed,
    Failed,
}

/// Job acceptance request
#[derive(Debug, Deserialize)]
pub struct AcceptJobRequest {
    pub job_id: ObjectId,
    pub epoch_id: u64,
    pub model_id: String,
    pub max_tokens: u64,
    pub price_per_token: u64,
}

/// Job execution request
#[derive(Debug, Deserialize)]
pub struct RunJobRequest {
    pub job_id: String,  // Hex-encoded job ID
    pub prompt: String,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
}

/// Job report
#[derive(Debug, Serialize)]
pub struct JobReport {
    pub job_id: ObjectId,
    pub receipt_payload: ReceiptPayload,
    pub receipt_hash: ReceiptHash,
    #[serde(with = "signature_serde")]
    pub signature: [u8; 64],
}

// Helper module for serializing [u8; 64]
mod signature_serde {
    use serde::{Serializer, Deserializer, Deserialize};

    pub fn serialize<S>(bytes: &[u8; 64], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_bytes(bytes)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<[u8; 64], D::Error>
    where
        D: Deserializer<'de>,
    {
        let bytes: Vec<u8> = Vec::deserialize(deserializer)?;
        if bytes.len() != 64 {
            return Err(serde::de::Error::custom("signature must be 64 bytes"));
        }
        let mut arr = [0u8; 64];
        arr.copy_from_slice(&bytes);
        Ok(arr)
    }
}

impl Worker {
    pub fn new(config: WorkerConfig) -> Arc<Self> {
        let backend = Arc::new(OllamaBackend::new(config.ollama_url.clone()));

        Arc::new(Self {
            config,
            jobs: Arc::new(RwLock::new(HashMap::new())),
            backend,
        })
    }

    /// Create HTTP router
    pub fn router(self: Arc<Self>) -> Router {
        let accept_worker = Arc::clone(&self);
        let run_worker = Arc::clone(&self);
        let stream_worker = Arc::clone(&self);
        let report_worker = Arc::clone(&self);
        let status_worker = Arc::clone(&self);

        Router::new()
            .route("/health", get(health_handler))
            .route("/jobs/accept", post(move |Json(req): Json<AcceptJobRequest>| {
                let worker = Arc::clone(&accept_worker);
                async move {
                    match worker.accept_job(req) {
                        Ok(_) => (
                            StatusCode::OK,
                            Json(serde_json::json!({ "status": "accepted" })),
                        ).into_response(),
                        Err(e) => (
                            StatusCode::BAD_REQUEST,
                            Json(serde_json::json!({ "error": e.to_string() })),
                        ).into_response(),
                    }
                }
            }))
            .route("/jobs/run", post(move |Json(req): Json<RunJobRequest>| {
                let worker = Arc::clone(&run_worker);
                async move {
                    let job_id = match parse_object_id(&req.job_id) {
                        Ok(id) => id,
                        Err(e) => return (
                            StatusCode::BAD_REQUEST,
                            Json(serde_json::json!({ "error": format!("Invalid job_id: {}", e) })),
                        ).into_response(),
                    };

                    match worker.run_job(job_id, req).await {
                        Ok(_) => (
                            StatusCode::OK,
                            Json(serde_json::json!({ "status": "completed" })),
                        ).into_response(),
                        Err(e) => (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(serde_json::json!({ "error": e.to_string() })),
                        ).into_response(),
                    }
                }
            }))
            .route("/jobs/:id/stream", get(move |Path(job_id_hex): Path<String>| {
                let worker = Arc::clone(&stream_worker);
                async move {
                    let job_id = match parse_object_id(&job_id_hex) {
                        Ok(id) => id,
                        Err(e) => return (
                            StatusCode::BAD_REQUEST,
                            Json(serde_json::json!({ "error": e.to_string() })),
                        ).into_response(),
                    };

                    let jobs = worker.jobs.read();
                    let job = match jobs.get(&job_id) {
                        Some(j) => j,
                        None => return (
                            StatusCode::NOT_FOUND,
                            Json(serde_json::json!({ "error": "Job not found" })),
                        ).into_response(),
                    };

                    Json(serde_json::json!({
                        "job_id": hex::encode(job.job_id),
                        "status": job.status,
                        "response": job.response,
                        "output_tokens": job.output_tokens,
                    })).into_response()
                }
            }))
            .route("/jobs/:id/report", post(move |Path(job_id_hex): Path<String>| {
                let worker = Arc::clone(&report_worker);
                async move {
                    let job_id = match parse_object_id(&job_id_hex) {
                        Ok(id) => id,
                        Err(e) => return (
                            StatusCode::BAD_REQUEST,
                            Json(serde_json::json!({ "error": e.to_string() })),
                        ).into_response(),
                    };

                    match worker.generate_receipt(job_id) {
                        Ok(report) => Json(report).into_response(),
                        Err(e) => (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(serde_json::json!({ "error": e.to_string() })),
                        ).into_response(),
                    }
                }
            }))
            .route("/jobs/:id/status", get(move |Path(job_id_hex): Path<String>| {
                let worker = Arc::clone(&status_worker);
                async move {
                    let job_id = match parse_object_id(&job_id_hex) {
                        Ok(id) => id,
                        Err(e) => return (
                            StatusCode::BAD_REQUEST,
                            Json(serde_json::json!({ "error": e.to_string() })),
                        ).into_response(),
                    };

                    let jobs = worker.jobs.read();
                    match jobs.get(&job_id) {
                        Some(job) => Json(job.clone()).into_response(),
                        None => (
                            StatusCode::NOT_FOUND,
                            Json(serde_json::json!({ "error": "Job not found" })),
                        ).into_response(),
                    }
                }
            }))
    }

    /// Accept a job assignment
    pub fn accept_job(&self, req: AcceptJobRequest) -> Result<()> {
        let mut jobs = self.jobs.write();

        if jobs.contains_key(&req.job_id) {
            return Err(anyhow!("Job already accepted"));
        }

        // Verify we support this model
        if !self.config.supported_models.contains(&req.model_id) {
            return Err(anyhow!("Model not supported: {}", req.model_id));
        }

        let job = JobExecution {
            job_id: req.job_id,
            epoch_id: req.epoch_id,
            model_id: req.model_id,
            prompt: String::new(),
            max_tokens: req.max_tokens,
            status: JobStatus::Accepted,
            assigned_at: Utc::now(),
            started_at: None,
            completed_at: None,
            output_tokens: 0,
            response: Vec::new(),
        };

        jobs.insert(req.job_id, job);

        tracing::info!("Accepted job: {:?}", req.job_id);

        Ok(())
    }

    /// Execute a job
    pub async fn run_job(&self, job_id: ObjectId, req: RunJobRequest) -> Result<()> {
        // Take the lock, validate, and extract what we need before awaiting.
        let (model_id, max_tokens, prompt) = {
            let mut jobs = self.jobs.write();
            let job = jobs.get_mut(&job_id)
                .ok_or_else(|| anyhow!("Job not found"))?;

            if job.status != JobStatus::Accepted {
                return Err(anyhow!("Job already running or completed"));
            }

            job.status = JobStatus::Running;
            job.started_at = Some(Utc::now());
            job.prompt = req.prompt.clone();

            (job.model_id.clone(), job.max_tokens, req.prompt.clone())
        };

        // Execute inference via backend
        let response = self.backend.generate(
            &model_id,
            &prompt,
            max_tokens,
            req.temperature,
            req.top_p,
        ).await?;

        // Update job with results
        let mut jobs = self.jobs.write();
        let job = jobs.get_mut(&job_id)
            .ok_or_else(|| anyhow!("Job disappeared"))?;

        job.response = response.tokens.clone();
        job.output_tokens = response.token_count;
        job.status = JobStatus::Completed;
        job.completed_at = Some(Utc::now());

        tracing::info!("Completed job: {:?}, tokens: {}", job_id, response.token_count);

        Ok(())
    }

    /// Generate signed receipt for a job
    pub fn generate_receipt(&self, job_id: ObjectId) -> Result<JobReport> {
        let jobs = self.jobs.read();
        let job = jobs.get(&job_id)
            .ok_or_else(|| anyhow!("Job not found"))?;

        if job.status != JobStatus::Completed {
            return Err(anyhow!("Job not completed"));
        }

        // Create receipt payload
        let receipt_payload = ReceiptPayload::new(
            job.job_id,
            job.epoch_id,
            self.config.keypair.address(),
            job.model_id.clone(),
            job.prompt.clone(),
            job.response.clone(),
            job.output_tokens,
            job.started_at.unwrap(),
            job.completed_at.unwrap(),
        )?;

        // Compute canonical hash
        let receipt_hash = receipt_payload.compute_hash()?;

        // Sign the receipt hash (it's already [u8; 32])
        let signature = self.config.keypair.sign(&receipt_hash);

        Ok(JobReport {
            job_id: job.job_id,
            receipt_payload,
            receipt_hash,
            signature,
        })
    }
}

/// Health check handler
async fn health_handler() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "healthy",
        "timestamp": Utc::now(),
    }))
}

/// Parse ObjectId from hex string
fn parse_object_id(hex: &str) -> Result<ObjectId> {
    let bytes = hex::decode(hex)?;
    if bytes.len() != 32 {
        return Err(anyhow!("Invalid ObjectId length"));
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    Ok(arr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_worker_creation() {
        let config = WorkerConfig {
            worker_id: [1u8; 32],
            keypair: KeyPair::generate(),
            endpoint: "http://localhost:8080".to_string(),
            ollama_url: "http://localhost:11434".to_string(),
            gpu_model: "NVIDIA RTX 4090".to_string(),
            vram_total_gib: 24,
            supported_models: vec![
                "llama2".to_string(),
                "mistral".to_string(),
            ],
        };

        let worker = Worker::new(config);
        assert!(worker.jobs.read().is_empty());
    }

    #[test]
    fn test_accept_job() {
        let config = WorkerConfig {
            worker_id: [1u8; 32],
            keypair: KeyPair::generate(),
            endpoint: "http://localhost:8080".to_string(),
            ollama_url: "http://localhost:11434".to_string(),
            gpu_model: "NVIDIA RTX 4090".to_string(),
            vram_total_gib: 24,
            supported_models: vec!["llama2".to_string()],
        };

        let worker = Worker::new(config);

        let req = AcceptJobRequest {
            job_id: [2u8; 32],
            epoch_id: 1,
            model_id: "llama2".to_string(),
            max_tokens: 1000,
            price_per_token: 10,
        };

        let result = worker.accept_job(req);
        assert!(result.is_ok());

        let jobs = worker.jobs.read();
        assert_eq!(jobs.len(), 1);
    }
}
