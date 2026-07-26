#![warn(clippy::pedantic)]
#![deny(clippy::unwrap_used, clippy::expect_used)]

use clap::{Parser, Subcommand};
use miraset_core::{Address, KeyPair, Transaction};
use miraset_node::{State, Storage};
use miraset_wallet::Wallet;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Configuration file structure
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct Config {
    #[serde(default)]
    node: NodeConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct NodeConfig {
    #[serde(default = "default_rpc_addr")]
    rpc_addr: String,
    #[serde(default = "default_storage_path")]
    storage_path: String,
    #[serde(default = "default_block_interval")]
    block_interval: u64,
}

impl Default for NodeConfig {
    fn default() -> Self {
        Self {
            rpc_addr: default_rpc_addr(),
            storage_path: default_storage_path(),
            block_interval: default_block_interval(),
        }
    }
}

fn default_rpc_addr() -> String {
    "127.0.0.1:9944".to_string()
}

fn default_storage_path() -> String {
    ".data".to_string()
}

fn default_block_interval() -> u64 {
    300
}

/// Path for the persisted genesis key inside the node's storage directory.
fn genesis_key_path(storage_path: &str) -> PathBuf {
    Path::new(storage_path).join("genesis_key.json")
}

/// True if the supplied socket address is on a loopback interface.
fn is_loopback_bind(addr: &str) -> bool {
    addr.parse::<std::net::SocketAddr>()
        .map(|s| s.ip().is_loopback())
        .unwrap_or(false)
}

/// Load the genesis keypair, generating and persisting a random one if needed.
///
/// # Security
/// * Without `--dev`, a random Ed25519 key is generated and saved to a
///   permissions-restricted file inside `storage_path`. The secret is never
///   printed.
/// * With `--dev`, the fixed devnet key `[1u8; 32]` is used for reproducibility.
///   `--dev` is rejected unless the RPC bind address is loopback.
fn load_or_create_genesis_keypair(
    dev: bool,
    rpc_addr: &str,
    storage_path: &str,
) -> anyhow::Result<KeyPair> {
    if dev {
        if !is_loopback_bind(rpc_addr) {
            anyhow::bail!(
                "--dev is only allowed when binding to a loopback address (got {})",
                rpc_addr
            );
        }
        let fixed = [1u8; 32];
        return Ok(KeyPair::from_bytes(&fixed));
    }

    let path = genesis_key_path(storage_path);
    if path.exists() {
        let content = std::fs::read_to_string(&path)?;
        let hex_secret = content.trim();
        let bytes = hex::decode(hex_secret)?;
        if bytes.len() != 32 {
            anyhow::bail!("persisted genesis key has wrong length");
        }
        let mut secret = [0u8; 32];
        secret.copy_from_slice(&bytes);
        return Ok(KeyPair::from_bytes(&secret));
    }

    // Generate a fresh random key.
    let kp = KeyPair::generate();
    let secret_hex = hex::encode(kp.secret_bytes());

    // Ensure the storage directory exists.
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Write with restrictive permissions (0600).
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&path)?;
        std::io::Write::write_all(&mut file, secret_hex.as_bytes())?;
    }
    #[cfg(not(unix))]
    {
        std::fs::write(&path, secret_hex)?;
    }

    Ok(kp)
}

/// Path for the persisted dispatch secret inside the node's storage directory.
fn dispatch_secret_path(storage_path: &str) -> PathBuf {
    Path::new(storage_path).join("dispatch_secret.bin")
}

/// Load or create the shared secret used for node->worker dispatch auth (H4).
///
/// In `--dev` mode a fixed all-zeros secret is used for convenience. In
/// normal mode a random 32-byte secret is generated and persisted with
/// restrictive permissions. The secret can also be supplied via the
/// `MIRASET_DISPATCH_SECRET` environment variable (hex-encoded 32 bytes).
fn load_or_create_dispatch_secret(
    dev: bool,
    rpc_addr: &str,
    storage_path: &str,
) -> anyhow::Result<[u8; 32]> {
    if let Ok(hex_secret) = std::env::var("MIRASET_DISPATCH_SECRET") {
        let bytes = hex::decode(hex_secret.trim())?;
        if bytes.len() != 32 {
            anyhow::bail!("MIRASET_DISPATCH_SECRET must be 32 hex-encoded bytes");
        }
        let mut secret = [0u8; 32];
        secret.copy_from_slice(&bytes);
        return Ok(secret);
    }

    if dev {
        if !is_loopback_bind(rpc_addr) {
            anyhow::bail!(
                "dev-mode dispatch secret is only allowed on loopback binds (got {})",
                rpc_addr
            );
        }
        return Ok([0u8; 32]);
    }

    let path = dispatch_secret_path(storage_path);
    if path.exists() {
        let bytes = std::fs::read(&path)?;
        if bytes.len() != 32 {
            anyhow::bail!("persisted dispatch secret has wrong length");
        }
        let mut secret = [0u8; 32];
        secret.copy_from_slice(&bytes);
        return Ok(secret);
    }

    let secret = miraset_node::auth::DispatchAuth::generate_secret();

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&path)?;
        std::io::Write::write_all(&mut file, &secret)?;
    }
    #[cfg(not(unix))]
    {
        std::fs::write(&path, &secret)?;
    }

    Ok(secret)
}

/// Load configuration with precedence: CLI flags > Env vars > Config file > Defaults
fn load_config() -> Config {
    load_config_from_path(Path::new("miraset.toml"))
}

fn load_config_from_path(path: &std::path::Path) -> Config {
    if path.exists()
        && let Ok(content) = std::fs::read_to_string(path)
        && let Ok(config) = toml::from_str::<Config>(&content)
    {
        return config;
    }

    // Return defaults if config file not found or invalid
    Config::default()
}

/// Apply environment variable overrides (MIRASET_* prefix)
fn apply_env_overrides(config: Config) -> Config {
    let vars: std::collections::HashMap<String, String> = std::env::vars().collect();
    apply_env_overrides_from_vars(config, &vars)
}

fn apply_env_overrides_from_vars(
    mut config: Config,
    vars: &std::collections::HashMap<String, String>,
) -> Config {
    if let Some(val) = vars.get("MIRASET_RPC_ADDR") {
        config.node.rpc_addr = val.clone();
    }
    if let Some(val) = vars.get("MIRASET_STORAGE_PATH") {
        config.node.storage_path = val.clone();
    }
    if let Some(val) = vars.get("MIRASET_BLOCK_INTERVAL")
        && let Ok(interval) = val.parse::<u64>()
    {
        config.node.block_interval = interval;
    }
    config
}

#[derive(Parser)]
#[command(name = "miraset")]
#[command(about = "Miraset Chain CLI", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Node operations
    Node {
        #[command(subcommand)]
        cmd: NodeCommands,
    },
    /// Wallet operations
    Wallet {
        #[command(subcommand)]
        cmd: WalletCommands,
    },
    /// Chat operations
    Chat {
        #[command(subcommand)]
        cmd: ChatCommands,
    },
}

#[derive(Subcommand)]
enum NodeCommands {
    /// Start local devnet node
    Start {
        #[arg(long, default_value = "127.0.0.1:9944")]
        rpc_addr: String,
        #[arg(long)]
        storage_path: Option<String>,
        #[arg(long)]
        block_interval: Option<u64>,
        /// Run in devnet mode (default true). This enables the single-author
        /// block producer without consensus. Production/multi-node operation is
        /// not supported until PoCC consensus is wired into block production.
        /// Pass `--dev=false` to opt out; the node will refuse to start.
        #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
        dev: bool,
        /// Allow worker endpoints that resolve to loopback/private addresses.
        /// This weakens SSRF protections and is only intended for local testing.
        #[arg(long)]
        allow_private_endpoints: bool,
        /// Allow any origin for CORS. Only safe on loopback devnets.
        #[arg(long)]
        cors_any: bool,
    },
}

#[derive(Subcommand)]
enum WalletCommands {
    /// Create new account
    New {
        /// Account name
        name: String,
    },
    /// List all accounts
    List,
    /// Show balance
    Balance {
        /// Account name
        name: String,
        #[arg(long, default_value = "http://127.0.0.1:9944")]
        rpc: String,
    },
    /// Transfer tokens
    Transfer {
        /// From account name
        from: String,
        /// To address (hex)
        to: String,
        /// Amount
        amount: u64,
        #[arg(long, default_value = "http://127.0.0.1:9944")]
        rpc: String,
    },
    /// Export secret key
    Export {
        /// Account name
        name: String,
    },
    /// Import account from secret key
    Import {
        /// Account name
        name: String,
        /// Secret key (hex)
        secret: String,
    },
}

#[derive(Subcommand)]
enum ChatCommands {
    /// Send chat message
    Send {
        /// From account name
        from: String,
        /// Message text
        message: String,
        #[arg(long, default_value = "http://127.0.0.1:9944")]
        rpc: String,
    },
    /// List recent messages
    List {
        #[arg(long, default_value = "50")]
        limit: usize,
        #[arg(long, default_value = "http://127.0.0.1:9944")]
        rpc: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Node { cmd } => handle_node(cmd).await?,
        Commands::Wallet { cmd } => handle_wallet(cmd).await?,
        Commands::Chat { cmd } => handle_chat(cmd).await?,
    }

    Ok(())
}

async fn handle_node(cmd: NodeCommands) -> anyhow::Result<()> {
    match cmd {
        NodeCommands::Start {
            rpc_addr,
            storage_path,
            block_interval,
            dev,
            allow_private_endpoints,
            cors_any,
        } => {
            // Load config with precedence: CLI > Env > File > Defaults
            let mut config = load_config();
            config = apply_env_overrides(config);

            // CLI flags override everything
            let final_rpc_addr = rpc_addr;
            let final_storage_path = storage_path.unwrap_or(config.node.storage_path);
            let final_block_interval = block_interval.unwrap_or(config.node.block_interval);

            // H5: devnet gate. Consensus is not wired; production mode is
            // explicitly unsupported.
            if !dev {
                anyhow::bail!(
                    "Non-dev mode is not supported: PoCC consensus is not yet wired into block production. Use --dev for local devnet only."
                );
            }

            println!("Starting Miraset devnet node...");
            println!("RPC address: {}", final_rpc_addr);
            println!("Storage path: {}", final_storage_path);
            println!("Block interval: {}s", final_block_interval);

            // H2: secure genesis key handling.
            let genesis_kp =
                load_or_create_genesis_keypair(dev, &final_rpc_addr, &final_storage_path)?;
            println!("Genesis account: {}", genesis_kp.address().to_hex());
            if dev {
                tracing::warn!(
                    "Using fixed devnet genesis key because --dev was set. Never use this on a public network."
                );
            } else {
                tracing::info!("Using persisted random genesis key.");
            }

            // H4: load or create the shared secret used to authenticate
            // node->worker job dispatches.
            let dispatch_secret =
                load_or_create_dispatch_secret(dev, &final_rpc_addr, &final_storage_path)?;
            if dev {
                tracing::warn!(
                    "A dispatch secret has been generated/loaded. Workers must be started with the same secret to accept job assignments."
                );
            }

            // Open persistent storage
            let storage = Storage::open(&final_storage_path)?;
            println!("Storage opened at: {}", final_storage_path);

            // Initialize state with persistent storage
            let mut state = State::new_with_storage(Some(storage.clone()))?;
            state.set_dispatch_secret(dispatch_secret);
            state.set_allow_private_endpoints(allow_private_endpoints);

            // Fund genesis account for devnet operation.
            state.add_balance(&genesis_kp.address(), 1_000_000_000_000); // 1 trillion tokens

            // Start block producer
            let producer_state = state.clone();
            tokio::spawn(async move {
                miraset_node::run_block_producer(
                    producer_state,
                    Duration::from_secs(final_block_interval),
                )
                .await;
            });

            // Start RPC
            let addr: std::net::SocketAddr = final_rpc_addr.parse()?;
            println!("RPC listening on http://{}", addr);
            let rpc_config = miraset_node::rpc::RpcConfig {
                cors_any,
                ..miraset_node::rpc::RpcConfig::default()
            };
            miraset_node::serve_rpc_with_config(state, addr, rpc_config).await?;
        }
    }
    Ok(())
}

async fn handle_wallet(cmd: WalletCommands) -> anyhow::Result<()> {
    let wallet_path = get_wallet_path();
    let mut wallet = Wallet::new(wallet_path)?;

    match cmd {
        WalletCommands::New { name } => {
            let addr = wallet.create_account(name.clone())?;
            println!("Created account '{}': {}", name, addr.to_hex());
        }
        WalletCommands::List => {
            let accounts = wallet.list_accounts();
            if accounts.is_empty() {
                println!("No accounts found. Create one with: miraset wallet new <name>");
            } else {
                println!("Accounts:");
                for (name, addr) in accounts {
                    println!("  {} -> {}", name, addr.to_hex());
                }
            }
        }
        WalletCommands::Balance { name, rpc } => {
            let kp = wallet.get_keypair(&name)?;
            let balance = get_balance(&rpc, &kp.address()).await?;
            println!("Balance for '{}': {}", name, balance);
        }
        WalletCommands::Transfer {
            from,
            to,
            amount,
            rpc,
        } => {
            let kp = wallet.get_keypair(&from)?;
            let to_addr = Address::from_hex(&to)?;
            let nonce = get_nonce(&rpc, &kp.address()).await?;

            let mut tx = Transaction::Transfer {
                from: kp.address(),
                to: to_addr,
                amount,
                nonce,
                signature: [0; 64],
            };

            sign_transaction(&mut tx, &kp)?;

            submit_tx(&rpc, &tx).await?;
            println!("Transfer submitted: {} -> {}, amount: {}", from, to, amount);
        }
        WalletCommands::Export { name } => {
            let secret = wallet.export_secret(&name)?;
            println!("Secret key for '{}': {}", name, secret);
            println!("WARNING: Keep this secret safe!");
        }
        WalletCommands::Import { name, secret } => {
            let addr = wallet.import_account(name.clone(), &secret)?;
            println!("Imported account '{}': {}", name, addr.to_hex());
        }
    }

    Ok(())
}

async fn handle_chat(cmd: ChatCommands) -> anyhow::Result<()> {
    let wallet_path = get_wallet_path();
    let wallet = Wallet::new(wallet_path)?;

    match cmd {
        ChatCommands::Send { from, message, rpc } => {
            let kp = wallet.get_keypair(&from)?;
            let nonce = get_nonce(&rpc, &kp.address()).await?;

            let mut tx = Transaction::ChatSend {
                from: kp.address(),
                message: message.clone(),
                nonce,
                signature: [0; 64],
            };

            sign_transaction(&mut tx, &kp)?;

            submit_tx(&rpc, &tx).await?;
            println!("Message sent!");
        }
        ChatCommands::List { limit, rpc } => {
            let messages = get_chat_messages(&rpc, limit).await?;
            if messages.is_empty() {
                println!("No messages yet.");
            } else {
                println!("Recent messages:");
                for msg in messages {
                    let timestamp = msg["timestamp"].as_str().unwrap_or("");
                    let from = msg["from"].as_str().unwrap_or("");
                    let message = msg["message"].as_str().unwrap_or("");
                    let from_short = if from.len() >= 8 { &from[..8] } else { from };
                    println!("[{}] {}: {}", timestamp, from_short, message);
                }
            }
        }
    }

    Ok(())
}

fn get_wallet_path() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".miraset").join("wallet.json")
}

async fn get_balance(rpc: &str, addr: &Address) -> anyhow::Result<u64> {
    let url = format!("{}/balance/{}", rpc, addr.to_hex());
    let resp = reqwest::get(&url).await?;
    Ok(resp.json().await?)
}

async fn get_nonce(rpc: &str, addr: &Address) -> anyhow::Result<u64> {
    let url = format!("{}/nonce/{}", rpc, addr.to_hex());
    let resp = reqwest::get(&url).await?;
    Ok(resp.json().await?)
}

async fn submit_tx(rpc: &str, tx: &Transaction) -> anyhow::Result<()> {
    let url = format!("{}/tx/submit", rpc);
    let client = reqwest::Client::new();
    let resp = client.post(&url).json(tx).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("TX failed: {}", resp.text().await?);
    }
    Ok(())
}

async fn get_chat_messages(rpc: &str, limit: usize) -> anyhow::Result<Vec<serde_json::Value>> {
    let url = format!("{}/chat/messages?limit={}", rpc, limit);
    let resp = reqwest::get(&url).await?;
    Ok(resp.json().await?)
}

/// Sign a transaction in-place using the chain-scoped canonical message.
fn sign_transaction(tx: &mut Transaction, kp: &KeyPair) -> anyhow::Result<()> {
    miraset_core::sign_transaction(tx, kp)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use std::collections::HashMap;
    use std::io::Write;

    #[test]
    fn test_load_config_defaults() {
        let config = load_config_from_path(Path::new("nonexistent_config.toml"));
        assert_eq!(config.node.rpc_addr, "127.0.0.1:9944");
        assert_eq!(config.node.storage_path, ".data");
        assert_eq!(config.node.block_interval, 300);
    }

    #[test]
    fn test_load_config_from_file() {
        let mut temp = tempfile::NamedTempFile::new().unwrap();
        write!(
            temp,
            r#"
[node]
rpc_addr = "0.0.0.0:19944"
storage_path = "/tmp/miraset"
block_interval = 60
"#
        )
        .unwrap();
        temp.flush().unwrap();

        let config = load_config_from_path(temp.path());
        assert_eq!(config.node.rpc_addr, "0.0.0.0:19944");
        assert_eq!(config.node.storage_path, "/tmp/miraset");
        assert_eq!(config.node.block_interval, 60);
    }

    #[test]
    fn test_apply_env_overrides() {
        let config = Config::default();
        let mut vars = HashMap::new();
        vars.insert("MIRASET_RPC_ADDR".to_string(), "0.0.0.0:19944".to_string());
        vars.insert("MIRASET_STORAGE_PATH".to_string(), "/tmp/store".to_string());
        vars.insert("MIRASET_BLOCK_INTERVAL".to_string(), "120".to_string());

        let config = apply_env_overrides_from_vars(config, &vars);
        assert_eq!(config.node.rpc_addr, "0.0.0.0:19944");
        assert_eq!(config.node.storage_path, "/tmp/store");
        assert_eq!(config.node.block_interval, 120);
    }

    #[test]
    fn test_apply_env_overrides_ignores_invalid_interval() {
        let config = Config::default();
        let mut vars = HashMap::new();
        vars.insert(
            "MIRASET_BLOCK_INTERVAL".to_string(),
            "not_a_number".to_string(),
        );

        let config = apply_env_overrides_from_vars(config, &vars);
        assert_eq!(config.node.block_interval, 300);
    }

    #[test]
    fn test_sign_transaction_transfer() {
        let kp = KeyPair::from_bytes(&[1u8; 32]);
        let recipient = KeyPair::generate();
        let mut tx = Transaction::Transfer {
            from: kp.address(),
            to: recipient.address(),
            amount: 42,
            nonce: 7,
            signature: [0; 64],
        };

        sign_transaction(&mut tx, &kp).unwrap();
        assert!(
            miraset_core::verify_transaction_signature(&tx),
            "signature should verify against the chain-scoped canonical transaction hash"
        );
    }

    #[test]
    fn test_sign_transaction_chat() {
        let kp = KeyPair::from_bytes(&[2u8; 32]);
        let mut tx = Transaction::ChatSend {
            from: kp.address(),
            message: "hello".to_string(),
            nonce: 1,
            signature: [0; 64],
        };

        sign_transaction(&mut tx, &kp).unwrap();
        assert!(
            miraset_core::verify_transaction_signature(&tx),
            "signature should verify against the chain-scoped canonical transaction hash"
        );
    }
}
