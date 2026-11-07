use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use hotstuff_rs::block_tree::pluggables::KVStore;
use hotstuff_rs::networking::network::Network;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};

use crate::core::types::{ComputeReceipt, Transaction};
use crate::node::MirasetNode;

/// Response for health check.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct HealthResponse {
    pub status: String,
    pub chain_id: u64,
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
        .route("/health", get(health_handler))
        .route("/tx/submit", post(submit_tx_handler))
        .route("/receipt/submit", post(submit_receipt_handler))
        .route("/account/{address}", get(get_account_handler))
        .route("/worker/{worker_id}", get(get_worker_handler))
        .route("/task/{task_id}", get(get_task_handler))
        .route("/block/latest", get(latest_block_handler))
        .route("/mempool/status", get(mempool_status_handler))
        .layer(cors)
        .with_state(state)
}

async fn health_handler<K, N>(State(state): State<RpcState<K, N>>) -> impl IntoResponse
where
    K: KVStore + Send + Sync + 'static,
    N: Network + Send + Sync + 'static,
{
    Json(HealthResponse {
        status: "ok".to_string(),
        chain_id: state.chain_id,
    })
}

async fn submit_tx_handler<K, N>(
    State(state): State<RpcState<K, N>>,
    Json(tx): Json<Transaction>,
) -> impl IntoResponse
where
    K: KVStore + Send + Sync + 'static,
    N: Network + Send + Sync + 'static,
{
    match state.node.submit_transaction(tx.clone()) {
        Ok(()) => (
            StatusCode::OK,
            Json(ApiResponse::ok(format!(
                "Tx from {} with nonce {} accepted into mempool",
                tx.sender, tx.nonce
            ))),
        ),
        Err(err) => (StatusCode::BAD_REQUEST, Json(ApiResponse::err(err))),
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
    match state.node.submit_receipt(receipt.clone()) {
        Ok(()) => (
            StatusCode::OK,
            Json(ApiResponse::ok(format!(
                "Receipt for task {} accepted into mempool",
                receipt.task_id
            ))),
        ),
        Err(err) => (StatusCode::BAD_REQUEST, Json(ApiResponse::err(err))),
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
    let highest_block = state
        .node
        .highest_committed_block()
        .map(|hash| hex::encode(hash.bytes()));

    Json(ApiResponse::ok(LatestBlockResponse {
        highest_committed_block: highest_block,
    }))
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
