use anyhow::Context;
use miraset_core::{Address, Transaction};
use miraset_wallet::Wallet;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Serialize, Deserialize, Clone)]
struct AppConfig {
    rpc_url: String,
    wallet_path: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct AccountView {
    name: String,
    address: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct ConnectionStatus {
    state: String,
    detail: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct ConnectionSnapshot {
    rpc: ConnectionStatus,
    worker: ConnectionStatus,
    ollama: ConnectionStatus,
}

impl AppConfig {
    fn default_config() -> Self {
        Self {
            rpc_url: "http://127.0.0.1:9944".to_string(),
            wallet_path: default_wallet_path().to_string_lossy().to_string(),
        }
    }
}

fn config_file_path() -> PathBuf {
    let base = tauri::api::path::config_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join("miraset-wallet").join("config.json")
}

fn default_wallet_path() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".miraset").join("wallet.json")
}

fn load_config() -> anyhow::Result<AppConfig> {
    let path = config_file_path();
    if path.exists() {
        let content = fs::read_to_string(&path).context("Failed to read config")?;
        let config = serde_json::from_str(&content).context("Failed to parse config")?;
        Ok(config)
    } else {
        Ok(AppConfig::default_config())
    }
}

fn save_config(config: &AppConfig) -> anyhow::Result<()> {
    let path = config_file_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).context("Failed to create config folder")?;
    }
    let payload = serde_json::to_string_pretty(config).context("Failed to serialize config")?;
    fs::write(&path, payload).context("Failed to write config")?;
    Ok(())
}

fn open_wallet(path: &Path) -> anyhow::Result<Wallet> {
    Wallet::new(path.to_path_buf())
}

#[tauri::command]
fn get_config() -> Result<AppConfig, String> {
    load_config().map_err(|err| err.to_string())
}

#[tauri::command]
fn set_rpc_url(rpc_url: String) -> Result<AppConfig, String> {
    if rpc_url.trim().is_empty() {
        return Err("RPC URL cannot be empty".to_string());
    }
    let mut config = load_config().map_err(|err| err.to_string())?;
    config.rpc_url = rpc_url.trim().to_string();
    save_config(&config).map_err(|err| err.to_string())?;
    Ok(config)
}

#[tauri::command]
fn list_accounts() -> Result<Vec<AccountView>, String> {
    let config = load_config().map_err(|err| err.to_string())?;
    let wallet = open_wallet(Path::new(&config.wallet_path)).map_err(|err| err.to_string())?;
    let accounts = wallet
        .list_accounts()
        .into_iter()
        .map(|(name, address)| AccountView {
            name,
            address: address.to_hex(),
        })
        .collect();
    Ok(accounts)
}

#[tauri::command]
fn create_account(name: String) -> Result<String, String> {
    let config = load_config().map_err(|err| err.to_string())?;
    let mut wallet = open_wallet(Path::new(&config.wallet_path)).map_err(|err| err.to_string())?;
    let address = wallet
        .create_account(name)
        .map_err(|err| err.to_string())?;
    Ok(address.to_hex())
}

#[tauri::command]
fn import_account(name: String, secret_hex: String) -> Result<String, String> {
    let config = load_config().map_err(|err| err.to_string())?;
    let mut wallet = open_wallet(Path::new(&config.wallet_path)).map_err(|err| err.to_string())?;
    let address = wallet
        .import_account(name, &secret_hex)
        .map_err(|err| err.to_string())?;
    Ok(address.to_hex())
}

#[tauri::command]
fn export_secret(name: String) -> Result<String, String> {
    let config = load_config().map_err(|err| err.to_string())?;
    let wallet = open_wallet(Path::new(&config.wallet_path)).map_err(|err| err.to_string())?;
    wallet.export_secret(&name).map_err(|err| err.to_string())
}

#[tauri::command]
async fn get_balance(address: String) -> Result<u64, String> {
    let config = load_config().map_err(|err| err.to_string())?;
    let addr = Address::from_hex(&address).map_err(|err| err.to_string())?;
    let url = format!("{}/balance/{}", config.rpc_url, addr.to_hex());
    let resp = reqwest::get(&url)
        .await
        .map_err(|err| err.to_string())?;
    let value = resp.json::<u64>().await.map_err(|err| err.to_string())?;
    Ok(value)
}

#[tauri::command]
async fn transfer(from: String, to: String, amount: u64) -> Result<(), String> {
    let config = load_config().map_err(|err| err.to_string())?;
    let wallet = open_wallet(Path::new(&config.wallet_path)).map_err(|err| err.to_string())?;
    let kp = wallet.get_keypair(&from).map_err(|err| err.to_string())?;
    let to_addr = Address::from_hex(&to).map_err(|err| err.to_string())?;
    let nonce = get_nonce(&config.rpc_url, &kp.address())
        .await
        .map_err(|err| err.to_string())?;

    let mut tx = Transaction::Transfer {
        from: kp.address(),
        to: to_addr,
        amount,
        nonce,
        signature: [0; 64],
    };

    let msg = bincode::serialize(&tx).map_err(|err| err.to_string())?;
    let sig = kp.sign(&msg);
    if let Transaction::Transfer { signature, .. } = &mut tx {
        *signature = sig;
    }

    submit_tx(&config.rpc_url, &tx)
        .await
        .map_err(|err| err.to_string())?;

    Ok(())
}

#[tauri::command]
async fn check_connections(
    rpc_url: String,
    worker_url: String,
    ollama_url: String,
) -> Result<ConnectionSnapshot, String> {
    let rpc = probe_endpoint(&rpc_url, "/health").await;
    let worker = probe_endpoint(&worker_url, "/health").await;
    let ollama = probe_endpoint(&ollama_url, "/api/tags").await;

    Ok(ConnectionSnapshot { rpc, worker, ollama })
}

async fn probe_endpoint(base_url: &str, path: &str) -> ConnectionStatus {
    let url = format!("{}{}", base_url.trim_end_matches('/'), path);
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
    {
        Ok(client) => client,
        Err(err) => {
            return ConnectionStatus {
                state: "offline".to_string(),
                detail: Some(err.to_string()),
            }
        }
    };

    match client.get(&url).send().await {
        Ok(resp) if resp.status().is_success() => ConnectionStatus {
            state: "online".to_string(),
            detail: None,
        },
        Ok(resp) => ConnectionStatus {
            state: "offline".to_string(),
            detail: Some(format!("HTTP {}", resp.status().as_u16())),
        },
        Err(err) => ConnectionStatus {
            state: "offline".to_string(),
            detail: Some(err.to_string()),
        },
    }
}

// D2: Fetch transaction events from node
#[tauri::command]
async fn get_events(from_height: Option<u64>, limit: Option<usize>) -> Result<Vec<serde_json::Value>, String> {
    let config = load_config().map_err(|err| err.to_string())?;
    let from = from_height.unwrap_or(0);
    let lim = limit.unwrap_or(100);
    let url = format!("{}/events?from_height={}&limit={}", config.rpc_url, from, lim);
    let resp = reqwest::get(&url).await.map_err(|err| err.to_string())?;
    let events: Vec<serde_json::Value> = resp.json().await.map_err(|err| err.to_string())?;
    Ok(events)
}

// D10: List registered workers from node
#[tauri::command]
async fn get_workers() -> Result<Vec<serde_json::Value>, String> {
    let config = load_config().map_err(|err| err.to_string())?;
    let url = format!("{}/workers", config.rpc_url);
    let resp = reqwest::get(&url).await.map_err(|err| err.to_string())?;
    let workers: Vec<serde_json::Value> = resp.json().await.map_err(|err| err.to_string())?;
    Ok(workers)
}

// D10: Submit inference job via coordinator
#[tauri::command]
async fn submit_job(
    from: String,
    model_id: String,
    max_tokens: u64,
    escrow_amount: u64,
) -> Result<serde_json::Value, String> {
    let config = load_config().map_err(|err| err.to_string())?;
    let wallet = open_wallet(Path::new(&config.wallet_path)).map_err(|err| err.to_string())?;
    let kp = wallet.get_keypair(&from).map_err(|err| err.to_string())?;

    let url = format!("{}/jobs/submit", config.rpc_url);
    let client = reqwest::Client::new();
    let resp = client
        .post(&url)
        .json(&serde_json::json!({
            "requester": kp.address().to_hex(),
            "model_id": model_id,
            "max_tokens": max_tokens,
            "escrow_amount": escrow_amount
        }))
        .send()
        .await
        .map_err(|err| err.to_string())?;

    if !resp.status().is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("Job submission failed: {}", text));
    }

    let result: serde_json::Value = resp.json().await.map_err(|err| err.to_string())?;
    Ok(result)
}

// D10: Get current epoch info
#[tauri::command]
async fn get_epoch() -> Result<serde_json::Value, String> {
    let config = load_config().map_err(|err| err.to_string())?;
    let url = format!("{}/epoch", config.rpc_url);
    let resp = reqwest::get(&url).await.map_err(|err| err.to_string())?;
    let epoch: serde_json::Value = resp.json().await.map_err(|err| err.to_string())?;
    Ok(epoch)
}

// D10: List jobs
#[tauri::command]
async fn get_jobs() -> Result<Vec<serde_json::Value>, String> {
    let config = load_config().map_err(|err| err.to_string())?;
    let url = format!("{}/jobs", config.rpc_url);
    let resp = reqwest::get(&url).await.map_err(|err| err.to_string())?;
    let jobs: Vec<serde_json::Value> = resp.json().await.map_err(|err| err.to_string())?;
    Ok(jobs)
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

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            get_config,
            set_rpc_url,
            list_accounts,
            create_account,
            import_account,
            export_secret,
            get_balance,
            transfer,
            check_connections,
            get_events,
            get_workers,
            submit_job,
            get_epoch,
            get_jobs,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

