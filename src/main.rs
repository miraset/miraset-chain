use clap::{Parser, Subcommand};
use ed25519_dalek::{SigningKey, VerifyingKey};
use miraset_chain::core::types::{ComputeReceipt, Transaction};
use miraset_chain::mempool::Mempool;
use miraset_chain::network::{ChannelNetwork, ChannelNetworkHub, TcpNetwork};
use miraset_chain::node::{MirasetNode, NodeConfig, init_genesis};
use miraset_chain::rpc::start_rpc_server;
use miraset_chain::storage::{MemKVStore, RedbKVStore};
use rand::RngExt;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

#[derive(Parser, Debug)]
#[command(name = "miraset-chain")]
#[command(about = "MIRASET L1 blockchain node (HotStuff-rs consensus & PoCC settlement)")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Run with in-memory devnet demonstration
    #[arg(long, default_value_t = false)]
    dev: bool,

    /// RPC listen address
    #[arg(long, default_value = "127.0.0.1:8545")]
    rpc_addr: SocketAddr,

    /// P2P TCP listen address for HotStuff consensus
    #[arg(long)]
    p2p_addr: Option<SocketAddr>,

    /// Comma-separated list of validator peers in format <pubkey_hex>@<ip:port>
    #[arg(long)]
    peers: Option<String>,

    /// Path to persistent Redb database directory
    #[arg(long)]
    data_dir: Option<PathBuf>,

    /// Path to hex-encoded Ed25519 validator private key file
    #[arg(long)]
    key_file: Option<PathBuf>,

    /// Chain ID
    #[arg(long, default_value_t = 1337)]
    chain_id: u64,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Generate a fresh validator Ed25519 keypair
    Keygen {
        /// Optional path to save the private key hex
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Run the interactive in-memory devnet demo
    Demo,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Keygen { out }) => {
            let mut rng = rand::rng();
            let mut secret = [0u8; 32];
            rng.fill(&mut secret);
            let signing_key = SigningKey::from_bytes(&secret);
            let vk = signing_key.verifying_key();
            let priv_hex = hex::encode(signing_key.to_bytes());
            let pub_hex = hex::encode(vk.to_bytes());

            println!("Generated Ed25519 Keypair:");
            println!("  Public Key (VerifyingKey): {}", pub_hex);
            println!("  Private Key (Secret):       {}", priv_hex);

            if let Some(path) = out {
                std::fs::write(&path, &priv_hex)?;
                println!("Saved private key to {}", path.display());
            }
            return Ok(());
        }
        Some(Commands::Demo) => {
            run_devnet_demo().await;
            return Ok(());
        }
        None => {}
    }

    if cli.dev || (cli.p2p_addr.is_none() && cli.data_dir.is_none()) {
        run_devnet_demo().await;
        return Ok(());
    }

    // Persistent node mode
    let data_dir = cli
        .data_dir
        .unwrap_or_else(|| PathBuf::from(".miraset_data"));
    std::fs::create_dir_all(&data_dir)?;
    let db_path = data_dir.join("chain.redb");

    let signing_key = if let Some(key_path) = cli.key_file {
        let content = std::fs::read_to_string(key_path)?;
        let bytes = hex::decode(content.trim())?;
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        SigningKey::from_bytes(&arr)
    } else {
        // Fallback to dev key or generate
        SigningKey::from_bytes(&[42u8; 32])
    };

    let vk = signing_key.verifying_key();
    let p2p_addr = cli
        .p2p_addr
        .unwrap_or_else(|| "127.0.0.1:9948".parse().unwrap());

    println!("Starting MIRASET Production Node:");
    println!("  Public Key: {}", hex::encode(vk.to_bytes()));
    println!("  Data Dir:   {}", data_dir.display());
    println!("  P2P Addr:   {}", p2p_addr);
    println!("  RPC Addr:   {}", cli.rpc_addr);

    let mut peer_addresses = HashMap::new();
    let mut initial_validators = vec![(vk, 100)];

    if let Some(peers_str) = cli.peers {
        for entry in peers_str
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            if let Some((pk_hex, addr_str)) = entry.split_once('@') {
                let pk_bytes = hex::decode(pk_hex)?;
                let mut pk_arr = [0u8; 32];
                pk_arr.copy_from_slice(&pk_bytes);
                let peer_vk = VerifyingKey::from_bytes(&pk_arr)?;
                let peer_addr: SocketAddr = addr_str.parse()?;
                peer_addresses.insert(peer_vk, peer_addr);
                initial_validators.push((peer_vk, 100));
            }
        }
    }

    let kv_store = RedbKVStore::open(&db_path).map_err(|e| anyhow::anyhow!("{e}"))?;
    let (network, runtime) = TcpNetwork::bind(vk, p2p_addr, peer_addresses)?;

    // If database is freshly created, initialize genesis
    init_genesis(
        kv_store.clone(),
        &initial_validators,
        &[("alice", 1_000_000), ("bob", 500_000)],
    );

    let mut config = NodeConfig::devnet(signing_key);
    config.chain_id = cli.chain_id;

    let mempool = Mempool::new();
    let node = Arc::new(MirasetNode::start(config, kv_store, network, mempool));

    let rpc_node = Arc::clone(&node);
    let rpc_addr = cli.rpc_addr;
    tokio::spawn(async move {
        if let Err(e) = start_rpc_server(rpc_node, cli.chain_id, rpc_addr).await {
            eprintln!("RPC server error: {}", e);
        }
    });

    println!("Node initialized successfully. Press Ctrl+C to terminate.");
    tokio::signal::ctrl_c().await?;
    println!("Shutting down node...");
    drop(node);
    drop(runtime);

    Ok(())
}

async fn run_devnet_demo() {
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

    // 3. Initialize Genesis State. Alice's account is keyed by her Ed25519 address
    // (not a free-text label) since transactions must be signed by the account they debit.
    let alice_key = SigningKey::from_bytes(&[7u8; 32]);
    let alice_addr = hex::encode(alice_key.verifying_key().to_bytes());
    let coordinator_key = SigningKey::from_bytes(&[8u8; 32]);

    let validators = vec![(vk, 100)];
    let initial_accounts = vec![
        (alice_addr.as_str(), 1_000_000),
        ("bob", 250_000),
        ("worker_4090", 10_000),
    ];

    println!("\n[2] Initializing Genesis Accounts:");
    for (acc, bal) in &initial_accounts {
        println!("    - Account '{}': {} MIRA", acc, bal);
    }
    println!("    (Account '{alice_addr}' is addressed below as 'alice')");
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
        "      Endpoints: /health, /account/:addr, /worker/:id, /task/:id, /tx/submit, /receipt/submit, /block/latest, /events/blocks"
    );

    // Verify initial account queries
    if let Some(alice) = node.get_account(&alice_addr) {
        println!(
            "    ✓ Alice initial balance verified: {} MIRA (nonce: {})",
            alice.balance, alice.nonce
        );
    }

    // 6. Submit sample transaction, signed by Alice's key
    println!("\n[4] Submitting Transactions to Mempool:");
    let mut tx1 = Transaction::new(&alice_addr, 0, "bob", 75_000, 150);
    tx1.sign_with(&alice_key);
    println!("    - Tx: Alice -> Bob (Amount: 75,000, Fee: 150)");
    node.submit_transaction(tx1).expect("Tx submission failed");

    // 7. Submit sample PoCC compute receipt, co-signed by the coordinator
    println!("\n[5] Submitting PoCC Compute Receipt to Mempool:");
    let mut receipt1 = ComputeReceipt::new(
        "inference_task_gpt_oss_1",
        &alice_addr,
        "worker_4090",
        "coordinator_east",
        "sha256:7f83b1657ff1fc53b92dc18148a1d65dfc2d4b1fa3d677284addd200126d9069",
        "zkp:0x892a71f0923058b73a4cf02",
        250,
        600,
        500,
    );
    receipt1.sign_with(&coordinator_key);
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
