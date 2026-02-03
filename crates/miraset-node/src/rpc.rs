use crate::state::State;
use axum::{
    extract::State as AxumState,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use miraset_core::{Address, Block, Event, Transaction};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;

#[derive(Clone)]
pub struct RpcState {
    pub state: State,
}

pub async fn serve_rpc(state: State, addr: SocketAddr) -> anyhow::Result<()> {
    let rpc_state = RpcState { state };

    let app = Router::new()
        .route("/balance/{address}", get(get_balance))
        .route("/nonce/{address}", get(get_nonce))
        .route("/block/latest", get(get_latest_block))
        .route("/block/{height}", get(get_block_by_height))
        .route("/events", get(get_events))
        .route("/chat/messages", get(get_chat_messages))
        .route("/tx/submit", post(submit_transaction))
        .with_state(rpc_state);

    tracing::info!("RPC server listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
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
) -> Json<Block> {
    Json(rpc.state.get_latest_block())
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
) -> Result<StatusCode, (StatusCode, String)> {
    rpc.state
        .submit_transaction(tx)
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    Ok(StatusCode::OK)
}
