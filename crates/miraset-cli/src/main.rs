use clap::{Parser, Subcommand};
use miraset_core::{Address, KeyPair, Transaction};
use miraset_node::State;
use miraset_wallet::Wallet;
use std::path::PathBuf;
use std::time::Duration;

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
        NodeCommands::Start { rpc_addr } => {
            println!("Starting Miraset devnet node...");
            let state = State::new();

            // Fund genesis account for testing (fixed for devnet)
            // Using a fixed secret for reproducibility in devnet
            let genesis_secret = [1u8; 32]; // Fixed devnet genesis key
            let genesis_kp = KeyPair::from_bytes(&genesis_secret);
            state.add_balance(&genesis_kp.address(), 1_000_000_000_000); // 1 trillion tokens
            println!("Genesis account: {}", genesis_kp.address().to_hex());
            println!("Genesis secret: {}", hex::encode(genesis_kp.secret_bytes()));

            // Start block producer
            let producer_state = state.clone();
            tokio::spawn(async move {
                miraset_node::run_block_producer(producer_state, Duration::from_secs(5)).await;
            });

            // Start RPC
            let addr: std::net::SocketAddr = rpc_addr.parse()?;
            println!("RPC listening on http://{}", addr);
            miraset_node::serve_rpc(state, addr).await?;
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
        WalletCommands::Transfer { from, to, amount, rpc } => {
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

            // Sign
            let msg = bincode::serialize(&tx)?;
            let sig = kp.sign(&msg);
            if let Transaction::Transfer { signature, .. } = &mut tx {
                *signature = sig;
            }

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

            // Sign
            let msg = bincode::serialize(&tx)?;
            let sig = kp.sign(&msg);
            if let Transaction::ChatSend { signature, .. } = &mut tx {
                *signature = sig;
            }

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
