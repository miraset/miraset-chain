use crate::state::State;
use axum::{
    Json, Router,
    extract::State as AxumState,
    http::StatusCode,
    routing::{get, post},
};
use chrono::Utc;
use miraset_core::{Address, Block, Event, ObjectData, ObjectId, Transaction};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use tower_http::cors::{Any, CorsLayer};

#[derive(Clone)]
pub struct RpcState {
    pub state: State,
}

pub async fn serve_rpc(state: State, addr: SocketAddr) -> anyhow::Result<()> {
    let rpc_state = RpcState { state };

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        .route("/health", get(get_health))
        .route("/status", get(get_health))
        .route("/ping", get(ping))
        .route("/balance/{address}", get(get_balance))
        .route("/nonce/{address}", get(get_nonce))
        .route("/block/latest", get(get_latest_block))
        .route("/block/{height}", get(get_block_by_height))
        .route("/events", get(get_events))
        .route("/chat/messages", get(get_chat_messages))
        .route("/tx/submit", post(submit_transaction))
        // Job coordinator endpoints (D1)
        .route("/jobs", get(list_jobs))
        .route("/jobs/submit", post(submit_job))
        .route("/jobs/{id}", get(get_job))
        .route("/workers", get(list_workers))
        .route("/epoch", get(get_epoch))
        .with_state(rpc_state)
        .layer(cors);

    tracing::info!("RPC server listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

#[derive(Serialize)]
struct NodeStatus {
    status: String,
    timestamp: String,
    latest_block_height: u64,
}

#[derive(Serialize)]
struct PingResponse {
    status: String,
}

async fn get_health(
    AxumState(rpc): AxumState<RpcState>,
) -> Result<Json<NodeStatus>, crate::error::StateError> {
    let latest_block = rpc.state.get_latest_block()?;
    Ok(Json(NodeStatus {
        status: "healthy".to_string(),
        timestamp: Utc::now().to_rfc3339(),
        latest_block_height: latest_block.height,
    }))
}

async fn ping() -> Json<PingResponse> {
    Json(PingResponse {
        status: "ok".to_string(),
    })
}

async fn get_balance(
    axum::extract::Path(address): axum::extract::Path<String>,
    AxumState(rpc): AxumState<RpcState>,
) -> Result<Json<u64>, StatusCode> {
    let addr = Address::from_hex(&address).map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(rpc.state.get_balance(&addr)))
}

async fn get_nonce(
    axum::extract::Path(address): axum::extract::Path<String>,
    AxumState(rpc): AxumState<RpcState>,
) -> Result<Json<u64>, StatusCode> {
    let addr = Address::from_hex(&address).map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(rpc.state.get_nonce(&addr)))
}

async fn get_latest_block(
    AxumState(rpc): AxumState<RpcState>,
) -> Result<Json<Block>, crate::error::StateError> {
    Ok(Json(rpc.state.get_latest_block()?))
}

async fn get_block_by_height(
    axum::extract::Path(height): axum::extract::Path<u64>,
    AxumState(rpc): AxumState<RpcState>,
) -> Result<Json<Block>, StatusCode> {
    rpc.state
        .get_block(height)
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

#[derive(Deserialize)]
struct EventsQuery {
    from_height: Option<u64>,
    limit: Option<usize>,
}

async fn get_events(
    axum::extract::Query(q): axum::extract::Query<EventsQuery>,
    AxumState(rpc): AxumState<RpcState>,
) -> Json<Vec<Event>> {
    let from = q.from_height.unwrap_or(0);
    let limit = q.limit.unwrap_or(100).min(1000);
    Json(rpc.state.get_events(from, limit))
}

#[derive(Deserialize)]
struct ChatQuery {
    limit: Option<usize>,
}

#[derive(Serialize)]
struct ChatMessage {
    from: String,
    message: String,
    timestamp: String,
}

async fn get_chat_messages(
    axum::extract::Query(q): axum::extract::Query<ChatQuery>,
    AxumState(rpc): AxumState<RpcState>,
) -> Json<Vec<ChatMessage>> {
    let limit = q.limit.unwrap_or(50).min(500);
    let messages = rpc.state.get_chat_messages(limit);
    Json(
        messages
            .into_iter()
            .map(|(from, msg, ts)| ChatMessage {
                from: from.to_hex(),
                message: msg,
                timestamp: ts.to_rfc3339(),
            })
            .collect(),
    )
}

async fn submit_transaction(
    AxumState(rpc): AxumState<RpcState>,
    Json(tx): Json<Transaction>,
) -> Result<Json<serde_json::Value>, crate::error::TxError> {
    rpc.state.submit_transaction(tx)?;
    Ok(Json(serde_json::json!({ "status": "accepted" })))
}

// ======= Job coordinator (D1) =======

#[derive(Serialize)]
struct JobView {
    job_id: String,
    epoch_id: u64,
    requester: String,
    model_id: String,
    max_tokens: u64,
    escrow_amount: u64,
    status: String,
    assigned_worker: Option<String>,
    created_at: String,
}

#[derive(Serialize)]
struct WorkerView {
    worker_id: String,
    owner: String,
    gpu_model: String,
    vram_gib: u32,
    status: String,
    endpoints: Vec<String>,
    supported_models: Vec<String>,
}

async fn list_jobs(AxumState(rpc): AxumState<RpcState>) -> Json<Vec<JobView>> {
    let jobs = rpc.state.get_jobs();
    let views: Vec<JobView> = jobs
        .iter()
        .filter_map(|(id, obj)| {
            if let ObjectData::InferenceJob {
                epoch_id,
                requester,
                model_id,
                max_tokens,
                escrow_amount,
                status,
                assigned_worker_id,
                created_at,
                ..
            } = &obj.data
            {
                Some(JobView {
                    job_id: hex::encode(id),
                    epoch_id: *epoch_id,
                    requester: requester.to_hex(),
                    model_id: model_id.clone(),
                    max_tokens: *max_tokens,
                    escrow_amount: *escrow_amount,
                    status: format!("{:?}", status),
                    assigned_worker: assigned_worker_id.map(hex::encode),
                    created_at: created_at.to_rfc3339(),
                })
            } else {
                None
            }
        })
        .collect();
    Json(views)
}

async fn get_job(
    axum::extract::Path(job_id_hex): axum::extract::Path<String>,
    AxumState(rpc): AxumState<RpcState>,
) -> Result<Json<JobView>, StatusCode> {
    let job_id = parse_object_id(&job_id_hex).map_err(|_| StatusCode::BAD_REQUEST)?;
    let obj = rpc.state.get_object(&job_id).ok_or(StatusCode::NOT_FOUND)?;
    if let ObjectData::InferenceJob {
        epoch_id,
        requester,
        model_id,
        max_tokens,
        escrow_amount,
        status,
        assigned_worker_id,
        created_at,
        ..
    } = &obj.data
    {
        Ok(Json(JobView {
            job_id: hex::encode(job_id),
            epoch_id: *epoch_id,
            requester: requester.to_hex(),
            model_id: model_id.clone(),
            max_tokens: *max_tokens,
            escrow_amount: *escrow_amount,
            status: format!("{:?}", status),
            assigned_worker: assigned_worker_id.map(hex::encode),
            created_at: created_at.to_rfc3339(),
        }))
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

/// Submit a signed `Transaction::CreateJob` to the transaction pipeline.
///
/// The coordinator no longer mutates state directly. Instead, the job is
/// enqueued like any other transaction, included in the next block, and
/// auto-assigned to a worker that supports the requested model during block
/// execution. The HTTP dispatch to the worker happens after the block is
/// produced.
async fn submit_job(
    AxumState(rpc): AxumState<RpcState>,
    Json(tx): Json<Transaction>,
) -> Result<Json<serde_json::Value>, crate::error::TxError> {
    // Ensure only CreateJob transactions are accepted on this endpoint.
    let tx_hash = tx.hash().map_err(crate::error::TxError::Serialization)?;

    let (requester, model_id) = match &tx {
        Transaction::CreateJob {
            requester,
            model_id,
            ..
        } => (*requester, model_id.clone()),
        _ => {
            return Err(crate::error::TxError::Other(
                "only Transaction::CreateJob is accepted on /jobs/submit".to_string(),
            ));
        }
    };

    rpc.state.submit_transaction(tx)?;

    Ok(Json(serde_json::json!({
        "status": "accepted",
        "tx_hash": hex::encode(tx_hash),
        "requester": requester.to_hex(),
        "model_id": model_id,
    })))
}

async fn list_workers(AxumState(rpc): AxumState<RpcState>) -> Json<Vec<WorkerView>> {
    let workers = rpc.state.get_workers();
    let views: Vec<WorkerView> = workers
        .iter()
        .filter_map(|(id, obj)| {
            if let ObjectData::WorkerRegistration {
                gpu_model,
                vram_total_gib,
                status,
                endpoints,
                supported_models,
                ..
            } = &obj.data
            {
                Some(WorkerView {
                    worker_id: hex::encode(id),
                    owner: obj.owner.to_hex(),
                    gpu_model: gpu_model.clone(),
                    vram_gib: *vram_total_gib,
                    status: format!("{:?}", status),
                    endpoints: endpoints.clone(),
                    supported_models: supported_models.clone(),
                })
            } else {
                None
            }
        })
        .collect();
    Json(views)
}

#[derive(Serialize)]
struct EpochView {
    id: u64,
    status: String,
    start_time: String,
    end_time: String,
    total_verified_tokens: u64,
    workers_count: usize,
    jobs_count: usize,
}

async fn get_epoch(AxumState(rpc): AxumState<RpcState>) -> Json<EpochView> {
    let epoch = rpc.state.get_current_epoch();
    Json(EpochView {
        id: epoch.id,
        status: format!("{:?}", epoch.status),
        start_time: epoch.start_time.to_rfc3339(),
        end_time: epoch.end_time.to_rfc3339(),
        total_verified_tokens: epoch.total_verified_tokens,
        workers_count: epoch.worker_stats.len(),
        jobs_count: epoch.job_results.len(),
    })
}

fn parse_object_id(hex_str: &str) -> Result<ObjectId, String> {
    let bytes = hex::decode(hex_str).map_err(|e| e.to_string())?;
    if bytes.len() != 32 {
        return Err("ObjectId must be 32 bytes".to_string());
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    Ok(arr)
}
