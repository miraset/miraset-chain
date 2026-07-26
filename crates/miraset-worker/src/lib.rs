#![warn(clippy::pedantic)]
#![deny(clippy::unwrap_used, clippy::expect_used)]

/// Miraset Worker Runtime
///
/// Ollama-like HTTP server that:
/// - Accepts job assignments
/// - Executes inference via Ollama/vLLM backend
/// - Generates signed receipts with canonical hashing
/// - Submits results to chain
use anyhow::{Result, anyhow};
use axum::{
    Json, Router,
    extract::DefaultBodyLimit,
    extract::Path,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use axum_governor::{GovernorConfigBuilder, GovernorLayer};
use chrono::{DateTime, Utc};
use miraset_core::{KeyPair, ObjectId};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceBuilder;
use tower_http::{limit::RequestBodyLimitLayer, timeout::TimeoutLayer};

mod auth;
mod backend;
mod node_client;
mod receipt;

pub use backend::{
    BackendType, InferenceBackend, LlamaCppBackend, MockBackend, OllamaBackend,
    OpenAiCompatibleBackend, OpenLlmBackend, TgiBackend,
};
pub use node_client::NodeClient;
pub use receipt::{ReceiptHash, ReceiptPayload};

/// Worker configuration
#[derive(Debug, Clone)]
pub struct WorkerConfig {
    pub worker_id: ObjectId,
    pub keypair: KeyPair,
    pub endpoint: String,
    pub node_url: String,
    /// Inference backend type to use (ollama/openai/llamacpp/tgi).
    pub backend_type: BackendType,
    /// Base URL of the inference backend (e.g. `http://localhost:11434` for
    /// Ollama, `http://localhost:1234/v1` for LM Studio, `https://api.openai.com/v1`
    /// for OpenAI cloud, `https://api.groq.com/openai/v1` for Groq).
    pub backend_url: String,
    /// Optional API key (Bearer auth). Required for cloud OpenAI-compatible
    /// providers (OpenAI, Groq, OpenRouter, ...); ignored by local
    /// backends that don't use auth (Ollama, llama.cpp, TGI).
    pub backend_api_key: Option<String>,
    pub gpu_model: String,
    pub vram_total_gib: u32,
    pub supported_models: Vec<String>,
    /// Shared secret for verifying node dispatch auth tags (hex-encoded 32 bytes).
    /// When set, `/jobs/accept` rejects requests without a valid tag (H4).
    pub dispatch_secret: Option<[u8; 32]>,
    /// Max request body size in bytes.
    pub max_body_size: usize,
    /// Request timeout.
    pub request_timeout: Duration,
    /// Per-IP rate limit burst size (requests).
    pub rate_limit_burst: u32,
    /// Per-IP rate limit replenish interval.
    pub rate_limit_period: Duration,
}

/// Worker state
pub struct Worker {
    config: WorkerConfig,
    jobs: Arc<RwLock<HashMap<ObjectId, JobExecution>>>,
    backend: Arc<dyn InferenceBackend>,
    node_client: NodeClient,
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
    pub job_id: String, // Hex-encoded job ID
    pub epoch_id: u64,
    pub model_id: String,
    pub max_tokens: u64,
    pub price_per_token: u64,
    /// Authentication tag from the node when dispatch auth is configured.
    pub auth_tag: Option<String>,
}

/// Job execution request
#[derive(Debug, Deserialize)]
pub struct RunJobRequest {
    pub job_id: String, // Hex-encoded job ID
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
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(bytes: &[u8; 64], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_bytes(bytes)
    }

    #[allow(dead_code)]
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
        let backend: Arc<dyn InferenceBackend> = match config.backend_type {
            BackendType::Ollama => Arc::new(OllamaBackend::new(config.backend_url.clone())),
            BackendType::OpenAi => Arc::new(OpenAiCompatibleBackend::new(
                config.backend_url.clone(),
                config.backend_api_key.clone(),
            )),
            BackendType::OpenLlm => Arc::new(OpenLlmBackend::new(config.backend_url.clone())),
            BackendType::LlamaCpp => Arc::new(LlamaCppBackend::new(config.backend_url.clone())),
            BackendType::Tgi => Arc::new(TgiBackend::new(config.backend_url.clone())),
        };
        let node_client = NodeClient::new(config.node_url.clone(), config.keypair.clone());

        Arc::new(Self {
            config,
            jobs: Arc::new(RwLock::new(HashMap::new())),
            backend,
            node_client,
        })
    }

    /// Verify a dispatch authentication tag against the configured secret.
    fn verify_dispatch_tag(&self, job_id: ObjectId, req: &AcceptJobRequest) -> Result<()> {
        if let Some(secret) = self.config.dispatch_secret {
            let tag_hex = req
                .auth_tag
                .as_ref()
                .ok_or_else(|| anyhow!("missing dispatch auth tag"))?;
            let tag = hex::decode(tag_hex)?;
            if tag.len() != 32 {
                return Err(anyhow!("invalid dispatch auth tag length"));
            }
            let mut tag_arr = [0u8; 32];
            tag_arr.copy_from_slice(&tag);

            let expected = auth::DispatchAuth::sign_dispatch(
                &secret,
                &job_id,
                &self.config.worker_id,
                req.epoch_id,
                &req.model_id,
                req.max_tokens,
            );
            if !auth::constant_time_eq(&expected, &tag_arr) {
                return Err(anyhow!("invalid dispatch auth tag"));
            }
        }
        Ok(())
    }

    /// Create HTTP router
    pub fn router(self: Arc<Self>) -> anyhow::Result<Router> {
        let accept_worker = Arc::clone(&self);
        let run_worker = Arc::clone(&self);
        let stream_worker = Arc::clone(&self);
        let report_worker = Arc::clone(&self);
        let status_worker = Arc::clone(&self);

        // M4 (residual): per-IP rate limiting on the worker surface. Defaults
        // are generous for local inference clusters; tighten in production.
        let rate_per_second = u32::try_from(
            1_000u64
                .checked_div(
                    u64::try_from(self.config.rate_limit_period.as_millis())
                        .unwrap_or(1_000)
                        .max(1),
                )
                .unwrap_or(1),
        )
        .unwrap_or(u32::MAX)
        .max(1);
        let non_zero_rate = std::num::NonZeroU32::new(rate_per_second)
            .ok_or_else(|| anyhow::anyhow!("rate_per_second must be non-zero"))?;
        let non_zero_burst = std::num::NonZeroU32::new(self.config.rate_limit_burst.max(1))
            .ok_or_else(|| anyhow::anyhow!("rate_limit_burst must be non-zero"))?;
        let governor_config = GovernorConfigBuilder::default()
            .with_extractor(axum_governor::extractor::PeerIp::default())
            .expect_connect_info()
            .quota_default(
                axum_governor::Quota::requests_per_second(non_zero_rate).burst(non_zero_burst),
            )
            .finish()
            .map_err(|_| anyhow::anyhow!("invalid rate limit configuration"))?;

        let app = Router::new()
            .route("/health", get(health_handler))
            .route("/status", get(health_handler))
            .route("/ping", get(ping_handler))
            .route(
                "/jobs/accept",
                post(move |Json(req): Json<AcceptJobRequest>| {
                    let worker = Arc::clone(&accept_worker);
                    async move {
                        match worker.accept_job(req) {
                            Ok(_) => (
                                StatusCode::OK,
                                Json(serde_json::json!({ "status": "accepted" })),
                            )
                                .into_response(),
                            Err(e) => (
                                StatusCode::BAD_REQUEST,
                                Json(serde_json::json!({ "error": e.to_string() })),
                            )
                                .into_response(),
                        }
                    }
                }),
            )
            .route(
                "/jobs/run",
                post(move |Json(req): Json<RunJobRequest>| {
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
                            )
                                .into_response(),
                            Err(e) => (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                Json(serde_json::json!({ "error": e.to_string() })),
                            )
                                .into_response(),
                        }
                    }
                }),
            )
            .route(
                "/jobs/{id}/stream",
                get(move |Path(job_id_hex): Path<String>| {
                    let worker = Arc::clone(&stream_worker);
                    async move {
                        let job_id = match parse_object_id(&job_id_hex) {
                            Ok(id) => id,
                            Err(e) => {
                                return (
                                    StatusCode::BAD_REQUEST,
                                    Json(serde_json::json!({ "error": e.to_string() })),
                                )
                                    .into_response();
                            }
                        };

                        let jobs = worker.jobs.read();
                        let job = match jobs.get(&job_id) {
                            Some(j) => j,
                            None => {
                                return (
                                    StatusCode::NOT_FOUND,
                                    Json(serde_json::json!({ "error": "Job not found" })),
                                )
                                    .into_response();
                            }
                        };

                        Json(serde_json::json!({
                            "job_id": hex::encode(job.job_id),
                            "status": job.status,
                            "response": job.response,
                            "output_tokens": job.output_tokens,
                        }))
                        .into_response()
                    }
                }),
            )
            .route(
                "/jobs/{id}/report",
                post(move |Path(job_id_hex): Path<String>| {
                    let worker = Arc::clone(&report_worker);
                    async move {
                        let job_id = match parse_object_id(&job_id_hex) {
                            Ok(id) => id,
                            Err(e) => {
                                return (
                                    StatusCode::BAD_REQUEST,
                                    Json(serde_json::json!({ "error": e.to_string() })),
                                )
                                    .into_response();
                            }
                        };

                        match worker.generate_receipt(job_id) {
                            Ok(report) => Json(report).into_response(),
                            Err(e) => (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                Json(serde_json::json!({ "error": e.to_string() })),
                            )
                                .into_response(),
                        }
                    }
                }),
            )
            .route(
                "/jobs/{id}/status",
                get(move |Path(job_id_hex): Path<String>| {
                    let worker = Arc::clone(&status_worker);
                    async move {
                        let job_id = match parse_object_id(&job_id_hex) {
                            Ok(id) => id,
                            Err(e) => {
                                return (
                                    StatusCode::BAD_REQUEST,
                                    Json(serde_json::json!({ "error": e.to_string() })),
                                )
                                    .into_response();
                            }
                        };

                        let jobs = worker.jobs.read();
                        match jobs.get(&job_id) {
                            Some(job) => Json(job.clone()).into_response(),
                            None => (
                                StatusCode::NOT_FOUND,
                                Json(serde_json::json!({ "error": "Job not found" })),
                            )
                                .into_response(),
                        }
                    }
                }),
            )
            .layer(
                ServiceBuilder::new()
                    .layer(DefaultBodyLimit::max(self.config.max_body_size))
                    .layer(RequestBodyLimitLayer::new(self.config.max_body_size))
                    .layer(TimeoutLayer::with_status_code(
                        StatusCode::REQUEST_TIMEOUT,
                        self.config.request_timeout,
                    )),
            )
            .layer(GovernorLayer::new(governor_config));

        Ok(app)
    }

    /// Accept a job assignment
    pub fn accept_job(&self, req: AcceptJobRequest) -> Result<()> {
        // Parse hex job_id
        let job_id = parse_object_id(&req.job_id)?;

        // H4: verify dispatch authentication tag when a secret is configured.
        self.verify_dispatch_tag(job_id, &req)?;

        let mut jobs = self.jobs.write();

        if jobs.contains_key(&job_id) {
            return Err(anyhow!("Job already accepted"));
        }

        // Verify we support this model
        if !self.config.supported_models.contains(&req.model_id) {
            return Err(anyhow!("Model not supported: {}", req.model_id));
        }

        let job = JobExecution {
            job_id,
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

        jobs.insert(job_id, job);

        tracing::info!("Accepted job: {}", hex::encode(job_id));

        Ok(())
    }

    /// Execute a job
    pub async fn run_job(&self, job_id: ObjectId, req: RunJobRequest) -> Result<()> {
        // Take the lock, validate, and extract what we need before awaiting.
        let (model_id, max_tokens, prompt) = {
            let mut jobs = self.jobs.write();
            let job = jobs
                .get_mut(&job_id)
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
        let response = self
            .backend
            .generate(&model_id, &prompt, max_tokens, req.temperature, req.top_p)
            .await?;

        // Update job with results
        let mut jobs = self.jobs.write();
        let job = jobs
            .get_mut(&job_id)
            .ok_or_else(|| anyhow!("Job disappeared"))?;

        job.response = response.tokens.clone();
        job.output_tokens = response.token_count;
        job.status = JobStatus::Completed;
        job.completed_at = Some(Utc::now());

        tracing::info!(
            "Completed job: {:?}, tokens: {}",
            job_id,
            response.token_count
        );

        Ok(())
    }

    /// Generate signed receipt for a job
    pub fn generate_receipt(&self, job_id: ObjectId) -> Result<JobReport> {
        let jobs = self.jobs.read();
        let job = jobs.get(&job_id).ok_or_else(|| anyhow!("Job not found"))?;

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
            job.started_at.ok_or_else(|| anyhow!("Job not started"))?,
            job.completed_at
                .ok_or_else(|| anyhow!("Job not completed"))?,
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

    /// Submit job result to chain (end-to-end flow)
    pub async fn submit_result_to_chain(&self, job_id: ObjectId) -> Result<()> {
        // Generate receipt
        let report = self.generate_receipt(job_id)?;

        // Submit to chain
        self.node_client
            .submit_job_result(
                report.job_id,
                self.config.worker_id,
                report.receipt_payload.output_tokens,
                report.receipt_hash,
            )
            .await?;

        // Optionally anchor the full receipt
        self.node_client
            .anchor_receipt(report.job_id, report.receipt_hash)
            .await?;

        tracing::info!("Submitted and anchored job result on-chain: {:?}", job_id);

        Ok(())
    }

    /// Register worker on-chain
    pub async fn register_on_chain(&self) -> Result<ObjectId> {
        self.node_client
            .register_worker(
                vec![format!("http://{}", self.config.endpoint)],
                self.config.gpu_model.clone(),
                self.config.vram_total_gib,
                self.config.supported_models.clone(),
                1000, // stake_bond
            )
            .await
    }

    /// Get worker config (for heartbeat loop)
    pub fn worker_id(&self) -> ObjectId {
        self.config.worker_id
    }

    pub fn vram_total_gib(&self) -> u32 {
        self.config.vram_total_gib
    }

    /// Start heartbeat loop — sends ResourceSnapshot TX every `interval` seconds
    pub fn start_heartbeat_loop(self: Arc<Self>, worker_id: ObjectId, interval_secs: u64) {
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
            loop {
                ticker.tick().await;

                // Ping node to check connectivity
                match self.node_client.ping().await {
                    Ok(true) => {
                        tracing::debug!("Heartbeat: node is reachable");
                    }
                    _ => {
                        tracing::warn!("Heartbeat: node is unreachable, skipping snapshot");
                        continue;
                    }
                }

                // Get current epoch id from node
                let epoch_id = match self.node_client.get_epoch().await {
                    Ok(epoch) => epoch["id"].as_u64().unwrap_or(0),
                    Err(_) => 0,
                };

                // Estimate available VRAM (for demo: total minus active jobs)
                let active_jobs = self
                    .jobs
                    .read()
                    .values()
                    .filter(|j| j.status == JobStatus::Running)
                    .count() as u32;
                let vram_used_per_job: u32 = 4; // estimate 4 GiB per job
                let vram_avail = self
                    .config
                    .vram_total_gib
                    .saturating_sub(active_jobs * vram_used_per_job);

                // Submit resource snapshot TX
                match self
                    .node_client
                    .submit_resource_snapshot(worker_id, epoch_id, vram_avail)
                    .await
                {
                    Ok(_) => {
                        tracing::info!(
                            "♥ Heartbeat sent: epoch={}, vram_avail={}GiB",
                            epoch_id,
                            vram_avail
                        );
                    }
                    Err(e) => {
                        tracing::warn!("Heartbeat failed: {}", e);
                    }
                }
            }
        });
    }
}

/// Health check handler
async fn health_handler() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "healthy",
        "timestamp": Utc::now(),
    }))
}

/// Ping handler
async fn ping_handler() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "ok",
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
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn test_worker_creation() {
        let config = WorkerConfig {
            worker_id: [1u8; 32],
            keypair: KeyPair::generate(),
            endpoint: "http://localhost:8080".to_string(),
            node_url: "http://127.0.0.1:9944".to_string(),
            backend_type: BackendType::Ollama,
            backend_url: "http://localhost:11434".to_string(),
            backend_api_key: None,
            gpu_model: "NVIDIA RTX 4090".to_string(),
            vram_total_gib: 24,
            supported_models: vec!["llama2".to_string(), "mistral".to_string()],
            dispatch_secret: None,
            max_body_size: 2 * 1024 * 1024,
            request_timeout: Duration::from_secs(30),
            rate_limit_burst: 100,
            rate_limit_period: Duration::from_secs(1),
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
            node_url: "http://127.0.0.1:9944".to_string(),
            backend_type: BackendType::Ollama,
            backend_url: "http://localhost:11434".to_string(),
            backend_api_key: None,
            gpu_model: "NVIDIA RTX 4090".to_string(),
            vram_total_gib: 24,
            supported_models: vec!["llama2".to_string()],
            dispatch_secret: None,
            max_body_size: 2 * 1024 * 1024,
            request_timeout: Duration::from_secs(30),
            rate_limit_burst: 100,
            rate_limit_period: Duration::from_secs(1),
        };

        let worker = Worker::new(config);

        let req = AcceptJobRequest {
            job_id: "0202020202020202020202020202020202020202020202020202020202020202".to_string(),
            epoch_id: 1,
            model_id: "llama2".to_string(),
            max_tokens: 1000,
            price_per_token: 10,
            auth_tag: None,
        };

        let result = worker.accept_job(req);
        assert!(result.is_ok());

        let jobs = worker.jobs.read();
        assert_eq!(jobs.len(), 1);
    }
}
