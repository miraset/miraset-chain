use anyhow::{Context, Result};
use miraset_core::{Address, KeyPair};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize)]
struct WalletData {
    accounts: HashMap<String, [u8; 32]>, // name -> secret_key
}

pub struct Wallet {
    path: PathBuf,
    data: WalletData,
}

impl Wallet {
    pub fn new(path: PathBuf) -> Result<Self> {
        let data = if path.exists() {
            let content = fs::read_to_string(&path).context("Failed to read wallet file")?;
            serde_json::from_str(&content).context("Failed to parse wallet file")?
        } else {
            WalletData {
                accounts: HashMap::new(),
            }
        };
        Ok(Self { path, data })
    }

    pub fn create_account(&mut self, name: String) -> Result<Address> {
        if self.data.accounts.contains_key(&name) {
            anyhow::bail!("Account '{}' already exists", name);
        }
        let kp = KeyPair::generate();
        let addr = kp.address();
        self.data.accounts.insert(name, kp.secret_bytes());
        self.save()?;
        Ok(addr)
    }

    pub fn import_account(&mut self, name: String, secret_hex: &str) -> Result<Address> {
        if self.data.accounts.contains_key(&name) {
            anyhow::bail!("Account '{}' already exists", name);
        }
        let bytes = hex::decode(secret_hex).context("Invalid hex")?;
        if bytes.len() != 32 {
            anyhow::bail!("Secret key must be 32 bytes");
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        let kp = KeyPair::from_bytes(&arr);
        let addr = kp.address();
        self.data.accounts.insert(name, arr);
        self.save()?;
        Ok(addr)
    }

    pub fn get_keypair(&self, name: &str) -> Result<KeyPair> {
        let secret = self
            .data
            .accounts
            .get(name)
            .context(format!("Account '{}' not found", name))?;
        Ok(KeyPair::from_bytes(secret))
    }

    pub fn list_accounts(&self) -> Vec<(String, Address)> {
        self.data
            .accounts
            .iter()
            .map(|(name, secret)| {
                let kp = KeyPair::from_bytes(secret);
                (name.clone(), kp.address())
            })
            .collect()
    }

    pub fn export_secret(&self, name: &str) -> Result<String> {
        let secret = self
            .data
            .accounts
            .get(name)
            .context(format!("Account '{}' not found", name))?;
        Ok(hex::encode(secret))
    }

    fn save(&self) -> Result<()> {
        let content = serde_json::to_string_pretty(&self.data)?;
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&self.path, content)?;
        Ok(())
    }
}
