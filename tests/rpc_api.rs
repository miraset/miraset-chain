use axum::body::Body;
use axum::http::{Request, StatusCode};
use ed25519_dalek::SigningKey;
use http_body_util::BodyExt;
use miraset_chain::core::types::{ComputeReceipt, Transaction};
use miraset_chain::mempool::Mempool;
use miraset_chain::network::{ChannelNetwork, ChannelNetworkHub};
use miraset_chain::node::{MirasetNode, NodeConfig, init_genesis};
use miraset_chain::rpc::{
    ApiResponse, BlockAtHeightResponse, HealthResponse, LatestBlockResponse, MempoolStatusResponse,
    RpcState, TaskSettlementResponse, create_router,
};
use miraset_chain::storage::MemKVStore;
use std::sync::Arc;
use tower::ServiceExt;

/// Funded account whose key signs the submissions in these tests.
fn payer() -> SigningKey {
    SigningKey::from_bytes(&[21u8; 32])
}

fn coordinator() -> SigningKey {
    SigningKey::from_bytes(&[22u8; 32])
}

fn address(key: &SigningKey) -> String {
    hex::encode(key.verifying_key().to_bytes())
}

fn signed_transfer(key: &SigningKey, nonce: u64, amount: u64) -> Transaction {
    let mut tx = Transaction::new(address(key), nonce, "bob", amount, 10);
    tx.sign_with(key);
    tx
}

fn signed_receipt(task_id: &str, client: &str) -> ComputeReceipt {
    let mut receipt = ComputeReceipt::new(
        task_id,
        client,
        "worker_h100",
        "coordinator_1",
        "model_hash_1",
        "proof_1",
        100,
        25,
        20,
    );
    receipt.sign_with(&coordinator());
    receipt
}

async fn post<T: serde::Serialize>(app: &axum::Router, path: &str, body: &T) -> StatusCode {
    let req = Request::builder()
        .uri(path)
        .method("POST")
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_vec(body).unwrap()))
        .unwrap();
    app.clone().oneshot(req).await.unwrap().status()
}

fn setup_test_node() -> (Arc<MirasetNode<MemKVStore, ChannelNetwork>>, u64) {
    let keypair = SigningKey::from_bytes(&[7u8; 32]);
    let vk = keypair.verifying_key();
    let hub = ChannelNetworkHub::new();
    let network = ChannelNetwork::new(vk, hub);
    let kv_store = MemKVStore::new();

    let validators = vec![(vk, 100)];
    let payer_address = address(&payer());
    let initial_accounts = vec![
        ("alice", 5000),
        ("bob", 100),
        (payer_address.as_str(), 5000),
    ];

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
async fn test_rpc_health_reports_safety_halt_as_503() {
    let (node, chain_id) = setup_test_node();
    assert!(node.safety_halt().trigger("signing guard: test fault"));
    let app = create_router(RpcState { node, chain_id });

    let req = Request::builder()
        .uri("/health")
        .method("GET")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let health: HealthResponse = serde_json::from_slice(&body).unwrap();
    assert_eq!(health.status, "halted");
    assert_eq!(health.reason.as_deref(), Some("signing guard: test fault"));
}

#[tokio::test]
async fn test_rpc_health_omits_reason_when_healthy() {
    let (node, chain_id) = setup_test_node();
    let app = create_router(RpcState { node, chain_id });
    let req = Request::builder()
        .uri("/health")
        .body(Body::empty())
        .unwrap();
    let body = app
        .oneshot(req)
        .await
        .unwrap()
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    let raw: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        raw.get("reason").is_none(),
        "healthy response has no reason: {raw}"
    );
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

    // Signed, funded transfer is admitted; the same one again conflicts.
    let tx = signed_transfer(&payer(), 0, 200);
    assert_eq!(post(&app, "/tx/submit", &tx).await, StatusCode::OK);
    assert_eq!(post(&app, "/tx/submit", &tx).await, StatusCode::CONFLICT);

    // Coordinator-signed receipt with a funded client is admitted; again conflicts.
    let receipt = signed_receipt("task_rpc_101", "alice");
    assert_eq!(
        post(&app, "/receipt/submit", &receipt).await,
        StatusCode::OK
    );
    assert_eq!(
        post(&app, "/receipt/submit", &receipt).await,
        StatusCode::CONFLICT
    );

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
    let mut tx = Transaction::new_worker_bond(address(&payer()), 0, 5, bond_action).unwrap();
    tx.sign_with(&payer());
    assert_eq!(post(&app, "/tx/submit", &tx).await, StatusCode::OK);
}

#[tokio::test]
async fn test_rpc_rejects_items_that_can_never_apply() {
    let (node, chain_id) = setup_test_node();
    let app = create_router(RpcState { node, chain_id });

    // Unsigned sender.
    let unsigned = Transaction::new("alice", 0, "bob", 1, 10);
    assert_eq!(
        post(&app, "/tx/submit", &unsigned).await,
        StatusCode::BAD_REQUEST
    );
    // Signed by a key other than the sender's.
    let mut forged = Transaction::new(address(&payer()), 0, "bob", 1, 10);
    forged.sign_with(&coordinator());
    assert_eq!(
        post(&app, "/tx/submit", &forged).await,
        StatusCode::BAD_REQUEST
    );
    // Unfunded sender.
    let broke = SigningKey::from_bytes(&[23u8; 32]);
    assert_eq!(
        post(&app, "/tx/submit", &signed_transfer(&broke, 0, 1)).await,
        StatusCode::BAD_REQUEST
    );
    // Overspending the committed balance.
    assert_eq!(
        post(&app, "/tx/submit", &signed_transfer(&payer(), 0, 5_000)).await,
        StatusCode::BAD_REQUEST
    );
    // Receipt without a coordinator signature; receipt whose client cannot pay.
    let unsigned_receipt = ComputeReceipt::new("t-x", "alice", "w", "c", "m", "p", 10, 1, 1);
    assert_eq!(
        post(&app, "/receipt/submit", &unsigned_receipt).await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        post(&app, "/receipt/submit", &signed_receipt("t-y", "charlie")).await,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn test_rpc_rejects_replayed_nonce_and_settled_task() {
    let (node, chain_id) = setup_test_node();
    let app = create_router(RpcState {
        node: Arc::clone(&node),
        chain_id,
    });
    let payer_address = address(&payer());
    assert_eq!(
        post(&app, "/tx/submit", &signed_transfer(&payer(), 0, 1)).await,
        StatusCode::OK
    );
    assert_eq!(
        post(
            &app,
            "/receipt/submit",
            &signed_receipt("task-replay", "alice")
        )
        .await,
        StatusCode::OK
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while node.get_account(&payer_address).map(|a| a.nonce) != Some(1)
        || !node.is_task_settled("task-replay")
    {
        assert!(std::time::Instant::now() < deadline, "items not committed");
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    // Same nonce with a different payload: the nonce is already used.
    assert_eq!(
        post(&app, "/tx/submit", &signed_transfer(&payer(), 0, 2)).await,
        StatusCode::CONFLICT
    );
    // A new receipt for a settled task (different worker reward) is a replay.
    let mut replay = signed_receipt("task-replay", "alice");
    replay.worker_reward = 1;
    replay.sign_with(&coordinator());
    assert_eq!(
        post(&app, "/receipt/submit", &replay).await,
        StatusCode::CONFLICT
    );
    // The next nonce is still admitted.
    assert_eq!(
        post(&app, "/tx/submit", &signed_transfer(&payer(), 1, 1)).await,
        StatusCode::OK
    );
}

#[tokio::test]
async fn test_rpc_epoch_endpoints() {
    let (node, chain_id) = setup_test_node();
    let state = RpcState { node, chain_id };
    let app = create_router(state);

    // 1. Query current active epoch (epoch 0 from genesis)
    let req_epoch = Request::builder()
        .uri("/epoch/current")
        .method("GET")
        .body(Body::empty())
        .unwrap();

    let resp_epoch = app.clone().oneshot(req_epoch).await.unwrap();
    assert_eq!(resp_epoch.status(), StatusCode::OK);

    let body = resp_epoch.into_body().collect().await.unwrap().to_bytes();
    let epoch_resp: ApiResponse<miraset_chain::core::types::EpochInfo> =
        serde_json::from_slice(&body).unwrap();
    let current = epoch_resp.data.expect("epoch info expected");
    assert_eq!(current.epoch_id, 0);
    assert_eq!(
        current.status,
        miraset_chain::core::types::EpochStatus::Active
    );

    // 2. Query non-existent historical epoch
    let req_historical = Request::builder()
        .uri("/epoch/999")
        .method("GET")
        .body(Body::empty())
        .unwrap();

    let resp_historical = app.oneshot(req_historical).await.unwrap();
    assert_eq!(resp_historical.status(), StatusCode::NOT_FOUND);
}

async fn get_json<T: serde::de::DeserializeOwned>(
    app: &axum::Router,
    uri: &str,
) -> (StatusCode, ApiResponse<T>) {
    let req = Request::builder()
        .uri(uri)
        .method("GET")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&body).unwrap())
}

#[tokio::test]
async fn test_rpc_block_height_queries_match_latest() {
    let (node, chain_id) = setup_test_node();
    let app = create_router(RpcState {
        node: Arc::clone(&node),
        chain_id,
    });

    // Wait for the single validator to commit at least one block.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let latest = loop {
        let (_, resp) = get_json::<LatestBlockResponse>(&app, "/block/latest").await;
        let latest = resp.data.unwrap();
        if latest.height.is_some() || std::time::Instant::now() >= deadline {
            break latest;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    };
    let height = latest.height.expect("a block should be committed");
    let latest_hash = latest.highest_committed_block.expect("hash with height");

    // The committed hash at that height is stable and equals the latest committed hash.
    let (status, at) =
        get_json::<BlockAtHeightResponse>(&app, &format!("/block/height/{height}")).await;
    assert_eq!(status, StatusCode::OK);
    let at = at.data.unwrap();
    assert_eq!((at.height, at.hash), (height, latest_hash));

    let (status, genesis_block) = get_json::<BlockAtHeightResponse>(&app, "/block/height/0").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(genesis_block.data.unwrap().height, 0);

    // Heights above the committed tip are not served.
    let (status, missing) =
        get_json::<BlockAtHeightResponse>(&app, "/block/height/18446744073709551615").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(!missing.success);
}
