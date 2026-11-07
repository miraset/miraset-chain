use ed25519_dalek::SigningKey;
use miraset_chain::core::types::{ComputeReceipt, Transaction};
use miraset_chain::mempool::Mempool;
use miraset_chain::network::{ChannelNetwork, ChannelNetworkHub};
use miraset_chain::node::{MirasetNode, NodeConfig, init_genesis};
use miraset_chain::rpc::start_rpc_server;
use miraset_chain::storage::MemKVStore;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

#[tokio::main]
async fn main() {
    println!("=======================================================");
    println!("     MIRASET Chain L1 - HotStuff-rs Consensus Devnet   ");
    println!("=======================================================\n");

    // 1. Generate validator keys
    let validator_key = SigningKey::from_bytes(&[42u8; 32]);
    let vk = validator_key.verifying_key();
    let vk_hex = hex_encode(&vk.to_bytes());
    println!("[1] Initializing Validator Set:");
    println!("    - Validator 0: {} (Power: 100)", vk_hex);

    // 2. Set up Storage and Network
    let kv_store = MemKVStore::new();
    let hub = ChannelNetworkHub::new();
    let network = ChannelNetwork::new(vk, hub);

    // 3. Initialize Genesis State
    let validators = vec![(vk, 100)];
    let initial_accounts = vec![
        ("alice", 1_000_000),
        ("bob", 250_000),
        ("worker_4090", 10_000),
    ];

    println!("\n[2] Initializing Genesis Accounts:");
    for (acc, bal) in &initial_accounts {
        println!("    - Account '{}': {} MIRA", acc, bal);
    }
    init_genesis(kv_store.clone(), &validators, &initial_accounts);

    // 4. Start Node
    println!("\n[3] Starting MIRASET Replica Node (Chain ID: 1337)...");
    let mempool = Mempool::new();
    let config = NodeConfig::devnet(validator_key);
    let chain_id = config.chain_id;
    let node = Arc::new(MirasetNode::start(config, kv_store, network, mempool));

    // 5. Start Node RPC API Gateway
    let rpc_addr: SocketAddr = "127.0.0.1:8545".parse().unwrap();
    let rpc_node = Arc::clone(&node);
    tokio::spawn(async move {
        if let Err(e) = start_rpc_server(rpc_node, chain_id, rpc_addr).await {
            eprintln!("RPC server error: {}", e);
        }
    });
    println!("    ✓ Node RPC API listening on http://{}", rpc_addr);
    println!(
        "      Endpoints: /health, /account/:addr, /worker/:id, /tx/submit, /receipt/submit, /block/latest, /mempool/status"
    );

    // Verify initial account queries
    if let Some(alice) = node.get_account("alice") {
        println!(
            "    ✓ Alice initial balance verified: {} MIRA (nonce: {})",
            alice.balance, alice.nonce
        );
    }

    // 6. Submit sample transaction
    println!("\n[4] Submitting Transactions to Mempool:");
    let tx1 = Transaction::new("alice", 0, "bob", 75_000, 150);
    println!("    - Tx: Alice -> Bob (Amount: 75,000, Fee: 150)");
    node.submit_transaction(tx1).expect("Tx submission failed");

    // 7. Submit sample PoCC compute receipt
    println!("\n[5] Submitting PoCC Compute Receipt to Mempool:");
    let receipt1 = ComputeReceipt::new(
        "inference_task_gpt_oss_1",
        "alice",
        "worker_4090",
        "coordinator_east",
        "sha256:7f83b1657ff1fc53b92dc18148a1d65dfc2d4b1fa3d677284addd200126d9069",
        "zkp:0x892a71f0923058b73a4cf02",
        250,
        600,
        500,
    );
    println!(
        "    - Worker: {} completed task '{}' (Reward: 500 MIRA, Client Cost: 600 MIRA)",
        receipt1.worker_id, receipt1.task_id
    );
    node.submit_receipt(receipt1)
        .expect("Receipt submission failed");

    println!("\n[6] Waiting for HotStuff consensus round to propose and commit block...");
    tokio::time::sleep(Duration::from_millis(2000)).await;

    // 8. Check committed state
    println!("\n[7] Querying Node State after Consensus:");
    if let Some(block_hash) = node.highest_committed_block() {
        println!("    ✓ Highest Committed Block Hash: {}", block_hash);
    } else {
        println!("    ℹ Consensus in progress, block proposed.");
    }

    println!("\n[8] Shutting down node gracefully...");
    drop(node);
    println!("✓ Node shutdown complete.");
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}
