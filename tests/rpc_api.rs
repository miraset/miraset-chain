use axum::body::Body;
use axum::http::{Request, StatusCode};
use ed25519_dalek::SigningKey;
use http_body_util::BodyExt;
use miraset_chain::core::types::{ComputeReceipt, Transaction};
use miraset_chain::mempool::Mempool;
use miraset_chain::network::{ChannelNetwork, ChannelNetworkHub};
use miraset_chain::node::{MirasetNode, NodeConfig, init_genesis};
use miraset_chain::rpc::{
    ApiResponse, HealthResponse, MempoolStatusResponse, RpcState, TaskSettlementResponse,
    create_router,
};
use miraset_chain::storage::MemKVStore;
use std::sync::Arc;
use tower::ServiceExt;

fn setup_test_node() -> (Arc<MirasetNode<MemKVStore, ChannelNetwork>>, u64) {
    let keypair = SigningKey::from_bytes(&[7u8; 32]);
    let vk = keypair.verifying_key();
    let hub = ChannelNetworkHub::new();
    let network = ChannelNetwork::new(vk, hub);
    let kv_store = MemKVStore::new();

    let validators = vec![(vk, 100)];
    let initial_accounts = vec![("alice", 5000), ("bob", 100)];

    init_genesis(kv_store.clone(), &validators, &initial_accounts);

    let mempool = Mempool::new();
    let config = NodeConfig::devnet(keypair);
    let chain_id = config.chain_id;

    let node = Arc::new(MirasetNode::start(config, kv_store, network, mempool));
    (node, chain_id)
}

#[tokio::test]
async fn test_rpc_health() {
    let (node, chain_id) = setup_test_node();
    let state = RpcState { node, chain_id };
    let app = create_router(state);

    let req = Request::builder()
        .uri("/health")
        .method("GET")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let health: HealthResponse = serde_json::from_slice(&body).unwrap();
    assert_eq!(health.status, "ok");
    assert_eq!(health.chain_id, chain_id);
}

#[tokio::test]
async fn test_rpc_get_account() {
    let (node, chain_id) = setup_test_node();
    let state = RpcState { node, chain_id };
    let app = create_router(state);

    // Existing account
    let req = Request::builder()
        .uri("/account/alice")
        .method("GET")
        .body(Body::empty())
        .unwrap();

    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let result: ApiResponse<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    assert!(result.success);
    let data = result.data.unwrap();
    assert_eq!(data["balance"], 5000);
    assert_eq!(data["nonce"], 0);

    // Non-existent account
    let req_nonexistent = Request::builder()
        .uri("/account/charlie")
        .method("GET")
        .body(Body::empty())
        .unwrap();

    let resp_nonexistent = app.oneshot(req_nonexistent).await.unwrap();
    assert_eq!(resp_nonexistent.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_rpc_tx_and_receipt_submission() {
    let (node, chain_id) = setup_test_node();
    let state = RpcState { node, chain_id };
    let app = create_router(state);

    // Initial mempool status
    let req = Request::builder()
        .uri("/mempool/status")
        .method("GET")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let status: ApiResponse<MempoolStatusResponse> = serde_json::from_slice(&body).unwrap();
    assert_eq!(status.data.unwrap().pending_txs, 0);

    // Submit valid transaction
    let tx = Transaction::new("alice", 0, "bob", 200, 10);
    let req = Request::builder()
        .uri("/tx/submit")
        .method("POST")
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_vec(&tx).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Duplicate transaction submission should be rejected
    let req_dup = Request::builder()
        .uri("/tx/submit")
        .method("POST")
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_vec(&tx).unwrap()))
        .unwrap();
    let resp_dup = app.clone().oneshot(req_dup).await.unwrap();
    assert_eq!(resp_dup.status(), StatusCode::BAD_REQUEST);

    // Submit compute receipt
    let receipt = ComputeReceipt::new(
        "task_rpc_101",
        "alice",
        "worker_h100",
        "coordinator_1",
        "model_hash_1",
        "proof_1",
        100,
        25,
        20,
    );
    let req_rcpt = Request::builder()
        .uri("/receipt/submit")
        .method("POST")
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_vec(&receipt).unwrap()))
        .unwrap();
    let resp_rcpt = app.clone().oneshot(req_rcpt).await.unwrap();
    assert_eq!(resp_rcpt.status(), StatusCode::OK);

    // Check mempool status has updated
    let req = Request::builder()
        .uri("/mempool/status")
        .method("GET")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let status: ApiResponse<MempoolStatusResponse> = serde_json::from_slice(&body).unwrap();
    let data = status.data.unwrap();
    assert_eq!(data.pending_txs, 1);
    assert_eq!(data.pending_receipts, 1);
}

#[tokio::test]
async fn test_rpc_worker_and_block_query() {
    let (node, chain_id) = setup_test_node();
    let state = RpcState { node, chain_id };
    let app = create_router(state);

    // Query non-existent worker
    let req = Request::builder()
        .uri("/worker/unknown_worker")
        .method("GET")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // Query latest block
    let req = Request::builder()
        .uri("/block/latest")
        .method("GET")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Query task settlement status
    let req_task = Request::builder()
        .uri("/task/unsettled_task_123")
        .method("GET")
        .body(Body::empty())
        .unwrap();
    let resp_task = app.clone().oneshot(req_task).await.unwrap();
    assert_eq!(resp_task.status(), StatusCode::OK);
    let body = resp_task.into_body().collect().await.unwrap().to_bytes();
    let task_resp: ApiResponse<TaskSettlementResponse> = serde_json::from_slice(&body).unwrap();
    assert!(!task_resp.data.unwrap().settled);

    // Test SSE block events endpoint connects with 200 OK and text/event-stream content type
    let req_events = Request::builder()
        .uri("/events/blocks")
        .method("GET")
        .body(Body::empty())
        .unwrap();
    let resp_events = app.oneshot(req_events).await.unwrap();
    assert_eq!(resp_events.status(), StatusCode::OK);
    assert_eq!(
        resp_events.headers().get("content-type").unwrap(),
        "text/event-stream"
    );
}

#[tokio::test]
async fn test_rpc_worker_bond_submission() {
    let (node, chain_id) = setup_test_node();
    let state = RpcState { node, chain_id };
    let app = create_router(state);

    let bond_action = miraset_chain::core::types::WorkerBondAction::Register {
        worker_id: "rpc_worker_1".to_string(),
        bond_amount: 300,
        vram_gib: 24,
        endpoint: "127.0.0.1:8080".to_string(),
    };
    let tx = Transaction::new_worker_bond("alice", 0, 5, bond_action).unwrap();

    let req = Request::builder()
        .uri("/tx/submit")
        .method("POST")
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_vec(&tx).unwrap()))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}
