use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{
        IntoResponse,
        sse::{Event as SseEvent, KeepAlive, Sse},
    },
    routing::{get, post},
};
use hotstuff_rs::block_tree::pluggables::KVStore;
use hotstuff_rs::networking::network::Network;
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio_stream::Stream;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;
use tower_http::cors::{Any, CorsLayer};

use crate::core::types::{ComputeReceipt, Transaction};
use crate::node::{AdmissionError, MirasetNode};

/// Response for health check.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct HealthResponse {
    pub status: String,
    pub chain_id: u64,
    /// Present when consensus is halted for safety (TODO 8.3.2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Response for `/livez` and `/readyz` (TODO 8.4.4).
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ProbeResponse {
    /// `alive`, `ready`, or the not-ready/halted reason label.
    pub status: String,
    /// Highest committed height this process has observed; absent before the first commit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub committed_height: Option<u64>,
    /// Seconds since this process last observed a commit; absent before the first commit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_commit_age_secs: Option<f64>,
}

/// Generic success or rejection status.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ApiResponse<T> {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl<T> ApiResponse<T> {
    pub fn ok(data: T) -> Self {
        Self {
            success: true,
            data: Some(data),
            error: None,
        }
    }

    pub fn err(msg: impl Into<String>) -> Self {
        Self {
            success: false,
            data: None,
            error: Some(msg.into()),
        }
    }
}

/// Answer to an admitted submission. Admission is not finality: the item is pending in this
/// node's mempool and is final only once a committed block includes it (watch the account
/// nonce, `/v1/tasks/{id}`, or `/v1/events/blocks`).
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SubmissionResponse {
    /// Always `pending`.
    pub status: String,
    /// `sender:nonce` for a transaction, the task id for a receipt.
    pub id: String,
}

impl SubmissionResponse {
    fn pending(id: String) -> Self {
        Self {
            status: "pending".to_string(),
            id,
        }
    }
}

/// Status of the mempool.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MempoolStatusResponse {
    pub pending_txs: usize,
    pub pending_receipts: usize,
}

/// Latest block response.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LatestBlockResponse {
    pub highest_committed_block: Option<String>,
    /// Height of `highest_committed_block` (additive field; absent before the first commit).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u64>,
}

/// Committed block at a specific height.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BlockAtHeightResponse {
    pub height: u64,
    pub hash: String,
}

/// Task settlement status response for Coordinator / client tracking.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TaskSettlementResponse {
    pub task_id: String,
    pub settled: bool,
}

/// Shared RPC state wrapping MirasetNode.
pub struct RpcState<K: KVStore, N: Network + 'static> {
    pub node: Arc<MirasetNode<K, N>>,
    pub chain_id: u64,
}

impl<K: KVStore, N: Network + 'static> Clone for RpcState<K, N> {
    fn clone(&self) -> Self {
        Self {
            node: Arc::clone(&self.node),
            chain_id: self.chain_id,
        }
    }
}

/// Build the Axum router for the node RPC.
pub fn create_router<K, N>(state: RpcState<K, N>) -> Router
where
    K: KVStore + Send + Sync + 'static,
    N: Network + Send + Sync + 'static,
{
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        // Operational probes and metrics: unversioned, for orchestrators and scrapers.
        .route("/livez", get(livez_handler))
        .route("/readyz", get(readyz_handler))
        .route("/metrics", get(metrics_handler))
        // Client contract, version 1 (TODO 8.4.3, docs/rpc-v1.md).
        .route("/v1/health", get(health_handler))
        .route("/v1/transactions", post(submit_tx_handler))
        .route("/v1/receipts", post(submit_receipt_handler))
        .route("/v1/accounts/{address}", get(get_account_handler))
        .route("/v1/workers/{worker_id}", get(get_worker_handler))
        .route("/v1/epochs/current", get(get_current_epoch_handler))
        .route("/v1/epochs/{epoch_id}", get(get_epoch_handler))
        .route("/v1/tasks/{task_id}", get(get_task_handler))
        .route("/v1/blocks/latest", get(latest_block_handler))
        .route("/v1/blocks/{height}", get(block_at_height_handler))
        .route("/v1/mempool", get(mempool_status_handler))
        .route("/v1/events/blocks", get(sse_block_events_handler))
        .layer(cors)
        .with_state(state)
}

async fn health_handler<K, N>(State(state): State<RpcState<K, N>>) -> impl IntoResponse
where
    K: KVStore + Send + Sync + 'static,
    N: Network + Send + Sync + 'static,
{
    let halt = state.node.safety_halt();
    let (code, status, reason) = match halt.reason() {
        Some(reason) => (
            StatusCode::SERVICE_UNAVAILABLE,
            "halted",
            Some(reason.to_string()),
        ),
        None => (StatusCode::OK, "ok", None),
    };
    (
        code,
        Json(HealthResponse {
            status: status.to_string(),
            chain_id: state.chain_id,
            reason,
        }),
    )
}

/// Liveness: the process serves requests and consensus has not halted for safety.
async fn livez_handler<K, N>(State(state): State<RpcState<K, N>>) -> impl IntoResponse
where
    K: KVStore + Send + Sync + 'static,
    N: Network + Send + Sync + 'static,
{
    let halted = state.node.safety_halt().is_halted();
    let (code, status) = if halted {
        (StatusCode::SERVICE_UNAVAILABLE, "halted")
    } else {
        (StatusCode::OK, "alive")
    };
    (code, Json(probe(&state, status)))
}

/// Readiness to finalize: alive, not draining, and a block committed recently.
async fn readyz_handler<K, N>(State(state): State<RpcState<K, N>>) -> impl IntoResponse
where
    K: KVStore + Send + Sync + 'static,
    N: Network + Send + Sync + 'static,
{
    match state.node.readiness() {
        Ok(()) => (StatusCode::OK, Json(probe(&state, "ready"))),
        Err(reason) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(probe(&state, reason.as_str())),
        ),
    }
}

fn probe<K, N>(state: &RpcState<K, N>, status: &str) -> ProbeResponse
where
    K: KVStore + Send + Sync + 'static,
    N: Network + Send + Sync + 'static,
{
    let metrics = state.node.metrics();
    let age = metrics.last_commit_age();
    ProbeResponse {
        status: status.to_string(),
        committed_height: age.map(|_| metrics.committed_height()),
        last_commit_age_secs: age.map(|d| d.as_secs_f64()),
    }
}

/// Prometheus text exposition format 0.0.4.
async fn metrics_handler<K, N>(State(state): State<RpcState<K, N>>) -> impl IntoResponse
where
    K: KVStore + Send + Sync + 'static,
    N: Network + Send + Sync + 'static,
{
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        state.node.render_metrics(),
    )
}

/// Map an admission refusal to an HTTP status: 400 for items that can never apply, 409 for
/// items already pending or committed, 429 for a sender over its quota, 503 when the node cannot
/// take work (full mempool, draining) and the client should retry later or elsewhere.
fn admission_status(err: AdmissionError) -> StatusCode {
    match err {
        AdmissionError::Invalid(_) => StatusCode::BAD_REQUEST,
        AdmissionError::Duplicate | AdmissionError::Replay(_) => StatusCode::CONFLICT,
        AdmissionError::SenderLimit => StatusCode::TOO_MANY_REQUESTS,
        AdmissionError::MempoolFull | AdmissionError::Draining => StatusCode::SERVICE_UNAVAILABLE,
    }
}

async fn submit_tx_handler<K, N>(
    State(state): State<RpcState<K, N>>,
    Json(tx): Json<Transaction>,
) -> impl IntoResponse
where
    K: KVStore + Send + Sync + 'static,
    N: Network + Send + Sync + 'static,
{
    let id = format!("{}:{}", tx.sender, tx.nonce);
    match state.node.submit_transaction(tx) {
        Ok(()) => (
            StatusCode::ACCEPTED,
            Json(ApiResponse::ok(SubmissionResponse::pending(id))),
        ),
        Err(err) => (
            admission_status(err),
            Json(ApiResponse::err(err.to_string())),
        ),
    }
}

async fn submit_receipt_handler<K, N>(
    State(state): State<RpcState<K, N>>,
    Json(receipt): Json<ComputeReceipt>,
) -> impl IntoResponse
where
    K: KVStore + Send + Sync + 'static,
    N: Network + Send + Sync + 'static,
{
    let id = receipt.task_id.clone();
    match state.node.submit_receipt(receipt) {
        Ok(()) => (
            StatusCode::ACCEPTED,
            Json(ApiResponse::ok(SubmissionResponse::pending(id))),
        ),
        Err(err) => (
            admission_status(err),
            Json(ApiResponse::err(err.to_string())),
        ),
    }
}

async fn get_account_handler<K, N>(
    State(state): State<RpcState<K, N>>,
    Path(address): Path<String>,
) -> impl IntoResponse
where
    K: KVStore + Send + Sync + 'static,
    N: Network + Send + Sync + 'static,
{
    match state.node.get_account(&address) {
        Some(account) => (StatusCode::OK, Json(ApiResponse::ok(account))),
        None => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::err("Account not found in committed state")),
        ),
    }
}

async fn get_worker_handler<K, N>(
    State(state): State<RpcState<K, N>>,
    Path(worker_id): Path<String>,
) -> impl IntoResponse
where
    K: KVStore + Send + Sync + 'static,
    N: Network + Send + Sync + 'static,
{
    match state.node.get_worker(&worker_id) {
        Some(worker) => (StatusCode::OK, Json(ApiResponse::ok(worker))),
        None => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::err("Worker not found in committed state")),
        ),
    }
}

async fn get_current_epoch_handler<K, N>(State(state): State<RpcState<K, N>>) -> impl IntoResponse
where
    K: KVStore + Send + Sync + 'static,
    N: Network + Send + Sync + 'static,
{
    match state.node.get_current_epoch() {
        Some(epoch) => (StatusCode::OK, Json(ApiResponse::ok(epoch))),
        None => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::err(
                "Current epoch not found in committed state",
            )),
        ),
    }
}

async fn get_epoch_handler<K, N>(
    State(state): State<RpcState<K, N>>,
    Path(epoch_id): Path<u64>,
) -> impl IntoResponse
where
    K: KVStore + Send + Sync + 'static,
    N: Network + Send + Sync + 'static,
{
    match state.node.get_epoch(epoch_id) {
        Some(epoch) => (StatusCode::OK, Json(ApiResponse::ok(epoch))),
        None => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::err(format!(
                "Epoch {} not found in committed state",
                epoch_id
            ))),
        ),
    }
}

async fn get_task_handler<K, N>(
    State(state): State<RpcState<K, N>>,
    Path(task_id): Path<String>,
) -> impl IntoResponse
where
    K: KVStore + Send + Sync + 'static,
    N: Network + Send + Sync + 'static,
{
    let settled = state.node.is_task_settled(&task_id);
    (
        StatusCode::OK,
        Json(ApiResponse::ok(TaskSettlementResponse { task_id, settled })),
    )
}

async fn latest_block_handler<K, N>(State(state): State<RpcState<K, N>>) -> impl IntoResponse
where
    K: KVStore + Send + Sync + 'static,
    N: Network + Send + Sync + 'static,
{
    let highest = state.node.highest_committed();
    Json(ApiResponse::ok(LatestBlockResponse {
        highest_committed_block: highest.map(|(hash, _)| hex::encode(hash.bytes())),
        height: highest.map(|(_, height)| height),
    }))
}

async fn block_at_height_handler<K, N>(
    State(state): State<RpcState<K, N>>,
    Path(height): Path<u64>,
) -> impl IntoResponse
where
    K: KVStore + Send + Sync + 'static,
    N: Network + Send + Sync + 'static,
{
    match state.node.committed_block_at_height(height) {
        Some(hash) => (
            StatusCode::OK,
            Json(ApiResponse::ok(BlockAtHeightResponse {
                height,
                hash: hex::encode(hash.bytes()),
            })),
        ),
        None => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::err("no committed block at this height")),
        ),
    }
}

async fn mempool_status_handler<K, N>(State(state): State<RpcState<K, N>>) -> impl IntoResponse
where
    K: KVStore + Send + Sync + 'static,
    N: Network + Send + Sync + 'static,
{
    let (txs, receipts) = {
        let pool = state.node.mempool();
        (pool.tx_count(), pool.receipt_count())
    };

    Json(ApiResponse::ok(MempoolStatusResponse {
        pending_txs: txs,
        pending_receipts: receipts,
    }))
}

async fn sse_block_events_handler<K, N>(
    State(state): State<RpcState<K, N>>,
) -> Sse<impl Stream<Item = Result<SseEvent, Infallible>>>
where
    K: KVStore + Send + Sync + 'static,
    N: Network + Send + Sync + 'static,
{
    let receiver = state.node.subscribe_commits();
    let stream = BroadcastStream::new(receiver).filter_map(|res| match res {
        Ok(event) => {
            let json = serde_json::to_string(&event).unwrap_or_default();
            Some(Ok(SseEvent::default().event("commit_block").data(json)))
        }
        Err(_) => None,
    });

    Sse::new(stream).keep_alive(KeepAlive::default())
}

/// Start an RPC server on the specified address.
pub async fn start_rpc_server<K, N>(
    node: Arc<MirasetNode<K, N>>,
    chain_id: u64,
    addr: SocketAddr,
) -> Result<(), std::io::Error>
where
    K: KVStore + Send + Sync + 'static,
    N: Network + Send + Sync + 'static,
{
    let state = RpcState { node, chain_id };
    let app = create_router(state);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await
}
