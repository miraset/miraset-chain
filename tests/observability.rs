//! Probes, metrics and drain behaviour of the node RPC (TODO 8.4.4, PRD R12 / AT12).

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use ed25519_dalek::SigningKey;
use http_body_util::BodyExt;
use miraset_chain::core::types::Transaction;
use miraset_chain::mempool::Mempool;
use miraset_chain::network::{ChannelNetwork, ChannelNetworkHub};
use miraset_chain::node::{AdmissionError, MirasetNode, NodeConfig, init_genesis};
use miraset_chain::rpc::{ProbeResponse, RpcState, create_router};
use miraset_chain::storage::MemKVStore;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tower::ServiceExt;

type TestNode = Arc<MirasetNode<MemKVStore, ChannelNetwork>>;

const COMMIT_TIMEOUT: Duration = Duration::from_secs(20);
const POLL: Duration = Duration::from_millis(50);

fn start_node(ready_max_commit_age: Duration) -> TestNode {
    let key = SigningKey::from_bytes(&[11u8; 32]);
    let vk = key.verifying_key();
    let kv_store = MemKVStore::new();
    let funded = hex::encode(
        SigningKey::from_bytes(&[13u8; 32])
            .verifying_key()
            .to_bytes(),
    );
    init_genesis(
        kv_store.clone(),
        &[(vk, 100)],
        &[("alice", 5_000), (funded.as_str(), 5_000)],
    );
    let mut config = NodeConfig::devnet(key);
    config.max_view_time = Duration::from_millis(200);
    config.ready_max_commit_age = ready_max_commit_age;
    let network = ChannelNetwork::new(vk, ChannelNetworkHub::new());
    Arc::new(MirasetNode::start(
        config,
        kv_store,
        network,
        Mempool::new(),
    ))
}

fn router(node: &TestNode) -> Router {
    create_router(RpcState {
        node: Arc::clone(node),
        chain_id: 1337,
    })
}

async fn get(node: &TestNode, path: &str) -> (StatusCode, String) {
    let req = Request::builder().uri(path).body(Body::empty()).unwrap();
    let resp = router(node).oneshot(req).await.unwrap();
    let status = resp.status();
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8(body.to_vec()).unwrap())
}

async fn probe(node: &TestNode, path: &str) -> (StatusCode, ProbeResponse) {
    let (status, body) = get(node, path).await;
    (status, serde_json::from_str(&body).unwrap())
}

async fn wait_ready(node: &TestNode) {
    let deadline = Instant::now() + COMMIT_TIMEOUT;
    while node.readiness().is_err() {
        assert!(Instant::now() < deadline, "node never became ready");
        tokio::time::sleep(POLL).await;
    }
}

fn metric(text: &str, series: &str) -> f64 {
    text.lines()
        .find_map(|l| l.strip_prefix(series)?.strip_prefix(' '))
        .unwrap_or_else(|| panic!("{series} missing:\n{text}"))
        .parse()
        .unwrap()
}

#[tokio::test]
async fn test_livez_is_ok_while_running_and_503_after_safety_halt() {
    let node = start_node(Duration::from_secs(30));
    let (status, body) = probe(&node, "/livez").await;
    assert_eq!((status, body.status.as_str()), (StatusCode::OK, "alive"));

    node.safety_halt().trigger("signing guard: test fault");
    let (status, body) = probe(&node, "/livez").await;
    assert_eq!(
        (status, body.status.as_str()),
        (StatusCode::SERVICE_UNAVAILABLE, "halted")
    );
}

#[tokio::test]
async fn test_readyz_turns_ready_after_commits_and_reports_height() {
    let node = start_node(Duration::from_secs(30));
    wait_ready(&node).await;
    let (status, body) = probe(&node, "/readyz").await;
    assert_eq!((status, body.status.as_str()), (StatusCode::OK, "ready"));
    assert!(body.committed_height.is_some());
    assert!(body.last_commit_age_secs.unwrap() < 30.0);
}

#[tokio::test]
async fn test_readyz_reports_halt_before_other_reasons() {
    let node = start_node(Duration::from_secs(30));
    wait_ready(&node).await;
    node.lifecycle().begin_drain();
    node.safety_halt().trigger("test");
    let (status, body) = probe(&node, "/readyz").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body.status, "halted");
}

fn second_validator() -> ed25519_dalek::VerifyingKey {
    SigningKey::from_bytes(&[22u8; 32]).verifying_key()
}

/// Two equal-power validators on one hub: finality needs both of them.
fn start_pair(
    ready_max_commit_age: Duration,
    isolate_second: bool,
) -> (ChannelNetworkHub, Vec<TestNode>) {
    let keys: Vec<SigningKey> = [21u8, 22].map(|b| SigningKey::from_bytes(&[b; 32])).into();
    let validators: Vec<_> = keys.iter().map(|k| (k.verifying_key(), 100)).collect();
    let hub = ChannelNetworkHub::new();
    if isolate_second {
        // Before start, so no block can be committed with both validators.
        hub.isolate_peer(second_validator());
    }
    let nodes = keys
        .into_iter()
        .map(|key| {
            let vk = key.verifying_key();
            let kv_store = MemKVStore::new();
            init_genesis(kv_store.clone(), &validators, &[("alice", 5_000)]);
            let mut config = NodeConfig::devnet(key);
            config.max_view_time = Duration::from_millis(200);
            config.ready_max_commit_age = ready_max_commit_age;
            let network = ChannelNetwork::new(vk, hub.clone());
            Arc::new(MirasetNode::start(
                config,
                kv_store,
                network,
                Mempool::new(),
            ))
        })
        .collect();
    (hub, nodes)
}

#[tokio::test]
async fn test_readyz_non_quorate_node_never_reports_ready() {
    let (_hub, nodes) = start_pair(Duration::from_secs(1), true);
    tokio::time::sleep(Duration::from_secs(2)).await;
    let (status, body) = probe(&nodes[0], "/readyz").await;
    assert_eq!(
        (status, body.status.as_str()),
        (StatusCode::SERVICE_UNAVAILABLE, "no_commit_yet")
    );
    assert_eq!(body.committed_height, None);
}

#[tokio::test]
async fn test_readyz_reports_commit_stale_after_losing_quorum() {
    let window = Duration::from_secs(1);
    let (hub, nodes) = start_pair(window, false);
    wait_ready(&nodes[0]).await;

    hub.isolate_peer(second_validator());
    let deadline = Instant::now() + COMMIT_TIMEOUT;
    loop {
        let (status, body) = probe(&nodes[0], "/readyz").await;
        if body.status == "commit_stale" {
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
            assert!(body.last_commit_age_secs.unwrap() > window.as_secs_f64());
            break;
        }
        assert!(Instant::now() < deadline, "never reported commit_stale");
        tokio::time::sleep(POLL).await;
    }

    hub.clear_faults();
    wait_ready(&nodes[0]).await;
}

#[tokio::test]
async fn test_draining_rejects_admission_with_503_and_withdraws_readiness() {
    let node = start_node(Duration::from_secs(30));
    wait_ready(&node).await;
    node.lifecycle().begin_drain();

    let key = SigningKey::from_bytes(&[12u8; 32]);
    let sender = hex::encode(key.verifying_key().to_bytes());
    let mut tx = Transaction::new(&sender, 0, "bob", 1, 1);
    tx.sign_with(&key);
    assert_eq!(
        node.submit_transaction(tx.clone()),
        Err(AdmissionError::Draining)
    );

    let req = Request::builder()
        .uri("/tx/submit")
        .method("POST")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&tx).unwrap()))
        .unwrap();
    let resp = router(&node).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);

    let (status, body) = probe(&node, "/readyz").await;
    assert_eq!(
        (status, body.status.as_str()),
        (StatusCode::SERVICE_UNAVAILABLE, "draining")
    );
    let (_, text) = get(&node, "/metrics").await;
    assert_eq!(metric(&text, "miraset_draining"), 1.0);
    assert_eq!(
        metric(
            &text,
            "miraset_admission_rejected_total{kind=\"transaction\",reason=\"draining\"}"
        ),
        2.0
    );
}

#[tokio::test]
async fn test_metrics_endpoint_exports_commits_admission_and_readiness() {
    let node = start_node(Duration::from_secs(30));
    wait_ready(&node).await;

    let key = SigningKey::from_bytes(&[13u8; 32]);
    let sender = hex::encode(key.verifying_key().to_bytes());
    let mut tx = Transaction::new(&sender, 0, "bob", 1, 1);
    tx.sign_with(&key);
    assert!(node.submit_transaction(tx.clone()).is_ok());
    assert_eq!(node.submit_transaction(tx), Err(AdmissionError::Duplicate));

    let req = Request::builder()
        .uri("/metrics")
        .body(Body::empty())
        .unwrap();
    let resp = router(&node).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let content_type = resp.headers()["content-type"].to_str().unwrap().to_string();
    assert!(content_type.starts_with("text/plain; version=0.0.4"));
    let text = String::from_utf8(
        resp.into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();

    assert!(metric(&text, "miraset_commits_total") >= 1.0);
    // Heights start at 0. Scraped before reading the store, so it can only lag the store.
    let store_height = node.highest_committed_height().unwrap() as f64;
    assert!(metric(&text, "miraset_committed_height") <= store_height);
    assert_eq!(metric(&text, "miraset_ready"), 1.0);
    assert_eq!(metric(&text, "miraset_safety_halted"), 0.0);
    assert_eq!(
        metric(
            &text,
            "miraset_admission_accepted_total{kind=\"transaction\"}"
        ),
        1.0
    );
    assert_eq!(
        metric(
            &text,
            "miraset_admission_rejected_total{kind=\"transaction\",reason=\"duplicate\"}"
        ),
        1.0
    );
    assert!(!text.contains("miraset_p2p_dropped_messages_total"));
}
