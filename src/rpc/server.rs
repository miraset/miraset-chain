use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::StatusCode,
    middleware,
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
use std::time::Duration;
use tokio_stream::Stream;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
use tower_http::cors::{Any, CorsLayer};
use tower_http::timeout::TimeoutLayer;

use super::access::{RpcAccess, guard_submission};
use crate::core::types::{CommitEvent, ComputeReceipt, Transaction};

/// Largest request body accepted (a transaction or receipt is well below this); larger ones
/// get 413.
pub const MAX_REQUEST_BODY_BYTES: usize = 64 * 1024;

/// Longest a request (other than the event stream) may take, including reading its body;
/// slower ones get 408.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
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
    /// Freshness of the committed state a read was served from (reads only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<ReadMeta>,
}

/// How fresh this node's committed state is. Taken before the data is read, so the data is
/// at least as new as `committed_height`. Each node serves its own committed state; a node can
/// lag the chain, which `stale` reports.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ReadMeta {
    /// Highest committed height in this node's store; absent before the first commit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub committed_height: Option<u64>,
    /// Seconds since this node last committed a block through consensus; absent since the
    /// process started or since a block-sync session began, until the next such commit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_commit_age_secs: Option<f64>,
    /// True when the node may be behind the chain: `halted`, `syncing`, `no_commit_yet` or
    /// `commit_stale` (see `stale_reason`).
    pub stale: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stale_reason: Option<String>,
}

impl<T> ApiResponse<T> {
    pub fn ok(data: T) -> Self {
        Self {
            success: true,
            data: Some(data),
            error: None,
            meta: None,
        }
    }

    pub fn err(msg: impl Into<String>) -> Self {
        Self {
            success: false,
            data: None,
            error: Some(msg.into()),
            meta: None,
        }
    }

    /// Label a read with the freshness of the state it came from.
    pub fn with_meta(mut self, meta: ReadMeta) -> Self {
        self.meta = Some(meta);
        self
    }
}

fn read_meta<K, N>(state: &RpcState<K, N>) -> ReadMeta
where
    K: KVStore + Send + Sync + 'static,
    N: Network + Send + Sync + 'static,
{
    let (age, staleness) = state.node.freshness();
    ReadMeta {
        committed_height: state.node.highest_committed_height(),
        last_commit_age_secs: age.map(|age| age.as_secs_f64()),
        stale: staleness.is_some(),
        stale_reason: staleness.map(|reason| reason.as_str().to_string()),
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
    create_router_with_access(state, Arc::new(RpcAccess::open()))
}

/// Build the router with an access policy for the submission endpoints.
pub fn create_router_with_access<K, N>(state: RpcState<K, N>, access: Arc<RpcAccess>) -> Router
where
    K: KVStore + Send + Sync + 'static,
    N: Network + Send + Sync + 'static,
{
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let submissions = Router::new()
        .route("/v1/transactions", post(submit_tx_handler))
        .route("/v1/receipts", post(submit_receipt_handler))
        .route_layer(middleware::from_fn_with_state(access, guard_submission));

    Router::new()
        // Operational probes and metrics: unversioned, for orchestrators and scrapers.
        .route("/livez", get(livez_handler))
        .route("/readyz", get(readyz_handler))
        .route("/metrics", get(metrics_handler))
        // Client contract, version 1 (TODO 8.4.3, docs/rpc-v1.md).
        .route("/v1/health", get(health_handler))
        .route("/v1/accounts/{address}", get(get_account_handler))
        .route("/v1/workers/{worker_id}", get(get_worker_handler))
        .route("/v1/epochs/current", get(get_current_epoch_handler))
        .route("/v1/epochs/{epoch_id}", get(get_epoch_handler))
        .route("/v1/tasks/{task_id}", get(get_task_handler))
        .route("/v1/blocks/latest", get(latest_block_handler))
        .route("/v1/blocks/{height}", get(block_at_height_handler))
        .route("/v1/mempool", get(mempool_status_handler))
        .merge(submissions)
        // Bounded request time for everything except the event stream, added below.
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            REQUEST_TIMEOUT,
        ))
        .route("/v1/events/blocks", get(sse_block_events_handler))
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BODY_BYTES))
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
    let meta = read_meta(&state);
    match state.node.get_account(&address) {
        Some(account) => (
            StatusCode::OK,
            Json(ApiResponse::ok(account).with_meta(meta)),
        ),
        None => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::err("Account not found in committed state").with_meta(meta)),
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
    let meta = read_meta(&state);
    match state.node.get_worker(&worker_id) {
        Some(worker) => (
            StatusCode::OK,
            Json(ApiResponse::ok(worker).with_meta(meta)),
        ),
        None => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::err("Worker not found in committed state").with_meta(meta)),
        ),
    }
}

async fn get_current_epoch_handler<K, N>(State(state): State<RpcState<K, N>>) -> impl IntoResponse
where
    K: KVStore + Send + Sync + 'static,
    N: Network + Send + Sync + 'static,
{
    let meta = read_meta(&state);
    match state.node.get_current_epoch() {
        Some(epoch) => (StatusCode::OK, Json(ApiResponse::ok(epoch).with_meta(meta))),
        None => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::err("Current epoch not found in committed state").with_meta(meta)),
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
    let meta = read_meta(&state);
    match state.node.get_epoch(epoch_id) {
        Some(epoch) => (StatusCode::OK, Json(ApiResponse::ok(epoch).with_meta(meta))),
        None => (
            StatusCode::NOT_FOUND,
            Json(
                ApiResponse::err(format!("Epoch {} not found in committed state", epoch_id))
                    .with_meta(meta),
            ),
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
    let meta = read_meta(&state);
    let settled = state.node.is_task_settled(&task_id);
    (
        StatusCode::OK,
        Json(ApiResponse::ok(TaskSettlementResponse { task_id, settled }).with_meta(meta)),
    )
}

async fn latest_block_handler<K, N>(State(state): State<RpcState<K, N>>) -> impl IntoResponse
where
    K: KVStore + Send + Sync + 'static,
    N: Network + Send + Sync + 'static,
{
    let meta = read_meta(&state);
    let highest = state.node.highest_committed();
    Json(
        ApiResponse::ok(LatestBlockResponse {
            highest_committed_block: highest.map(|(hash, _)| hex::encode(hash.bytes())),
            height: highest.map(|(_, height)| height),
        })
        .with_meta(meta),
    )
}

async fn block_at_height_handler<K, N>(
    State(state): State<RpcState<K, N>>,
    Path(height): Path<u64>,
) -> impl IntoResponse
where
    K: KVStore + Send + Sync + 'static,
    N: Network + Send + Sync + 'static,
{
    let meta = read_meta(&state);
    match state.node.committed_block_at_height(height) {
        Some(hash) => (
            StatusCode::OK,
            Json(
                ApiResponse::ok(BlockAtHeightResponse {
                    height,
                    hash: hex::encode(hash.bytes()),
                })
                .with_meta(meta),
            ),
        ),
        None => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::err("no committed block at this height").with_meta(meta)),
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
    let stream = BroadcastStream::new(receiver).map(|item| Ok::<_, Infallible>(sse_event(item)));
    Sse::new(stream).keep_alive(KeepAlive::default())
}

/// One commit as a `commit_block` event. A subscriber too slow for the node's event buffer gets
/// a `lagged` event with the number of commits it missed instead of a silent gap; it should
/// re-read state (`/v1/blocks/latest`) before relying on the stream again.
fn sse_event(item: Result<CommitEvent, BroadcastStreamRecvError>) -> SseEvent {
    let (name, data) = sse_payload(item);
    SseEvent::default().event(name).data(data)
}

fn sse_payload(item: Result<CommitEvent, BroadcastStreamRecvError>) -> (&'static str, String) {
    match item {
        Ok(event) => (
            "commit_block",
            serde_json::to_string(&event).unwrap_or_default(),
        ),
        Err(BroadcastStreamRecvError::Lagged(missed)) => (
            "lagged",
            serde_json::json!({ "missed": missed }).to_string(),
        ),
    }
}

/// Start an RPC server on the specified address.
pub async fn start_rpc_server<K, N>(
    node: Arc<MirasetNode<K, N>>,
    chain_id: u64,
    addr: SocketAddr,
    access: RpcAccess,
) -> Result<(), std::io::Error>
where
    K: KVStore + Send + Sync + 'static,
    N: Network + Send + Sync + 'static,
{
    let access = Arc::new(access.with_metrics(Arc::clone(node.metrics())));
    let state = RpcState { node, chain_id };
    let app = create_router_with_access(state, access);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    // Client addresses key the rate limit when no API keys are configured.
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_commit_events_carry_height_and_lag_is_reported_not_skipped() {
        let event = CommitEvent {
            block_hash: "ab".into(),
            timestamp_secs: 1,
            tx_count: 2,
            receipt_count: 0,
            height: Some(7),
        };
        let (name, data) = sse_payload(Ok(event.clone()));
        assert_eq!(name, "commit_block");
        assert_eq!(serde_json::from_str::<CommitEvent>(&data).unwrap(), event);

        let (name, data) = sse_payload(Err(BroadcastStreamRecvError::Lagged(42)));
        assert_eq!(name, "lagged");
        assert_eq!(data, r#"{"missed":42}"#);
    }
}
