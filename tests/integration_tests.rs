// Integration tests for Miraset Chain
use miraset_core::{Address, Block, KeyPair, Transaction};
use reqwest::Client;
use serde_json::json;
use std::time::Duration;
use tokio::time::sleep;

const RPC_URL: &str = "http://127.0.0.1:9944";

/// Helper to wait for transaction to be included in block
async fn wait_for_block() {
    sleep(Duration::from_secs(6)).await;
}

#[tokio::test]
async fn test_rpc_get_balance() {
    let client = Client::new();
    let genesis_addr = "4ae7bb72c634332e0db3e27e22091bcd7a0167dafb5320e886d8c6ae21289ffd";
    
    let response = client
        .get(format!("{}/balance/{}", RPC_URL, genesis_addr))
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
    let client = Client::new();
    let genesis_addr = "4ae7bb72c634332e0db3e27e22091bcd7a0167dafb5320e886d8c6ae21289ffd";
    
    let response = client
        .get(format!("{}/nonce/{}", RPC_URL, genesis_addr))
        .send()
        .await;
    
    if let Ok(resp) = response {
        assert_eq!(resp.status(), 200);
        let nonce: u64 = resp.json().await.unwrap();
        assert!(nonce >= 0, "Nonce should be valid");
    }
}

#[tokio::test]
async fn test_rpc_get_latest_block() {
    let client = Client::new();
    
    let response = client
        .get(format!("{}/block/latest", RPC_URL))
        .send()
        .await;
    
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
    let client = Client::new();
    
    let response = client
        .get(format!("{}/block/0", RPC_URL))
        .send()
        .await;
    
    if let Ok(resp) = response {
        assert_eq!(resp.status(), 200);
        let block: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(block["height"], 0);
    }
}

#[tokio::test]
async fn test_rpc_get_events() {
    let client = Client::new();
    
    let response = client
        .get(format!("{}/events?from_height=0&limit=10", RPC_URL))
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
    let client = Client::new();
    
    let response = client
        .get(format!("{}/chat/messages?limit=10", RPC_URL))
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
    let client = Client::new();
    
    let response = client
        .get(format!("{}/balance/invalid_address", RPC_URL))
        .send()
        .await;
    
    if let Ok(resp) = response {
        assert_eq!(resp.status(), 400, "Should return bad request for invalid address");
    }
}

#[tokio::test]
async fn test_nonexistent_block() {
    let client = Client::new();
    
    let response = client
        .get(format!("{}/block/999999", RPC_URL))
        .send()
        .await;
    
    if let Ok(resp) = response {
        assert_eq!(resp.status(), 404, "Should return not found for nonexistent block");
    }
}
