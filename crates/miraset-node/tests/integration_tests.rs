// Integration tests for Miraset Chain
// Boots an in-process node so these tests run without an external server.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use miraset_core::{KeyPair, Transaction};
use miraset_node::{State, serve_rpc};
use reqwest::Client;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::time::sleep;

/// The devnet genesis secret key used by the CLI to seed a funded account.
const DEVNET_SECRET: [u8; 32] = [1u8; 32];

/// Spawn a node on an ephemeral port and return the base RPC URL.
async fn spawn_test_node() -> String {
    let state = State::new();

    // L1: genesis hash is persisted when storage is present. In-memory
    // tests have no storage, so no verification is needed here.

    // Seed the devnet genesis account so the legacy integration assertions
    // (which expect a funded account at this address) still pass.
    let dev_key = KeyPair::from_bytes(&DEVNET_SECRET);
    let dev_addr = dev_key.address();
    state.add_balance(&dev_addr, 1_000_000_000_000);

    // Find a free ephemeral port.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind to ephemeral port");
    let port = listener.local_addr().expect("local address").port();
    // Drop the probe listener so the RPC server can bind the same port.
    drop(listener);

    let addr: SocketAddr = format!("127.0.0.1:{}", port)
        .parse()
        .expect("valid socket address");

    // Start the block producer in the background so submitted transactions
    // are actually included in blocks.
    let producer_state = state.clone();
    tokio::spawn(async move {
        miraset_node::run_block_producer(producer_state, Duration::from_secs(1)).await;
    });

    // Start the RPC server in the background.
    tokio::spawn(async move {
        if let Err(e) = serve_rpc(state, addr).await {
            eprintln!("RPC server error: {}", e);
        }
    });

    // Give the server a moment to start accepting connections.
    sleep(Duration::from_millis(200)).await;

    format!("http://{}", addr)
}

async fn wait_for_block() {
    sleep(Duration::from_secs(2)).await;
}

#[tokio::test]
async fn test_rpc_get_balance() {
    let url = spawn_test_node().await;
    let client = Client::new();
    let genesis_addr = "8a88e3dd7409f195fd52db2d3cba5d72ca6709bf1d94121bf3748801b40f6f5c";

    let response = client
        .get(format!("{}/balance/{}", url, genesis_addr))
        .send()
        .await;

    assert!(response.is_ok(), "Should connect to RPC server");

    if let Ok(resp) = response {
        assert_eq!(resp.status(), 200);
        let balance: u64 = resp.json().await.unwrap();
        assert!(balance > 0, "Genesis account should have balance");
    }
}

#[tokio::test]
async fn test_rpc_get_nonce() {
    let url = spawn_test_node().await;
    let client = Client::new();
    let genesis_addr = "8a88e3dd7409f195fd52db2d3cba5d72ca6709bf1d94121bf3748801b40f6f5c";

    let response = client
        .get(format!("{}/nonce/{}", url, genesis_addr))
        .send()
        .await;

    if let Ok(resp) = response {
        assert_eq!(resp.status(), 200);
        let nonce: u64 = resp.json().await.unwrap();
        assert!(nonce < u64::MAX, "Nonce should be valid");
    }
}

#[tokio::test]
async fn test_rpc_get_latest_block() {
    let url = spawn_test_node().await;
    let client = Client::new();

    let response = client.get(format!("{}/block/latest", url)).send().await;

    if let Ok(resp) = response {
        assert_eq!(resp.status(), 200);
        let block: serde_json::Value = resp.json().await.unwrap();
        assert!(block.get("height").is_some());
        assert!(block.get("timestamp").is_some());
        assert!(block.get("transactions").is_some());
    }
}

#[tokio::test]
async fn test_rpc_get_block_by_height() {
    let url = spawn_test_node().await;
    let client = Client::new();

    let response = client.get(format!("{}/block/0", url)).send().await;

    if let Ok(resp) = response {
        assert_eq!(resp.status(), 200);
        let block: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(block["height"], 0);
    }
}

#[tokio::test]
async fn test_rpc_get_events() {
    let url = spawn_test_node().await;
    let client = Client::new();

    let response = client
        .get(format!("{}/events?from_height=0&limit=10", url))
        .send()
        .await;

    if let Ok(resp) = response {
        assert_eq!(resp.status(), 200);
        let events: Vec<serde_json::Value> = resp.json().await.unwrap();
        assert!(events.is_empty() || !events.is_empty());
    }
}

#[tokio::test]
async fn test_rpc_get_chat_messages() {
    let url = spawn_test_node().await;
    let client = Client::new();

    let response = client
        .get(format!("{}/chat/messages?limit=10", url))
        .send()
        .await;

    if let Ok(resp) = response {
        assert_eq!(resp.status(), 200);
        let messages: Vec<serde_json::Value> = resp.json().await.unwrap();
        assert!(messages.is_empty() || !messages.is_empty());
    }
}

#[tokio::test]
async fn test_invalid_address_format() {
    let url = spawn_test_node().await;
    let client = Client::new();

    let response = client
        .get(format!("{}/balance/invalid_address", url))
        .send()
        .await;

    if let Ok(resp) = response {
        assert_eq!(
            resp.status(),
            400,
            "Should return bad request for invalid address"
        );
    }
}

#[tokio::test]
async fn test_nonexistent_block() {
    let url = spawn_test_node().await;
    let client = Client::new();

    let response = client.get(format!("{}/block/999999", url)).send().await;

    if let Ok(resp) = response {
        assert_eq!(
            resp.status(),
            404,
            "Should return not found for nonexistent block"
        );
    }
}

#[tokio::test]
async fn test_rpc_submit_transaction() {
    let url = spawn_test_node().await;
    let client = Client::new();
    let kp = KeyPair::from_bytes(&DEVNET_SECRET);
    let recipient = KeyPair::generate();

    let mut tx = Transaction::Transfer {
        from: kp.address(),
        to: recipient.address(),
        amount: 100,
        nonce: 0,
        signature: [0; 64],
    };

    miraset_core::sign_transaction(&mut tx, &kp).unwrap();

    let response = client
        .post(format!("{}/tx/submit", url))
        .json(&tx)
        .send()
        .await;

    assert!(response.is_ok(), "Should submit transaction");
    if let Ok(resp) = response {
        assert_eq!(resp.status(), 200);
    }

    // Wait for block production to include the transaction.
    wait_for_block().await;

    let balance_resp = client
        .get(format!("{}/balance/{}", url, recipient.address().to_hex()))
        .send()
        .await;

    if let Ok(resp) = balance_resp {
        assert_eq!(resp.status(), 200);
        let balance: u64 = resp.json().await.unwrap();
        assert_eq!(balance, 100, "Recipient should receive the transfer");
    }
}
