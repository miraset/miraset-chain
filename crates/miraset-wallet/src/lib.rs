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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_temp_wallet() -> (Wallet, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let wallet_path = temp_dir.path().join("test_wallet.json");
        let wallet = Wallet::new(wallet_path).unwrap();
        (wallet, temp_dir)
    }

    #[test]
    fn test_new_wallet() {
        let (wallet, _temp_dir) = create_temp_wallet();
        assert_eq!(wallet.list_accounts().len(), 0);
    }

    #[test]
    fn test_create_account() {
        let (mut wallet, _temp_dir) = create_temp_wallet();

        let addr = wallet.create_account("alice".to_string()).unwrap();
        assert!(addr.to_hex().len() == 64);

        let accounts = wallet.list_accounts();
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].0, "alice");
        assert_eq!(accounts[0].1, addr);
    }

    #[test]
    fn test_create_duplicate_account() {
        let (mut wallet, _temp_dir) = create_temp_wallet();

        wallet.create_account("alice".to_string()).unwrap();
        let result = wallet.create_account("alice".to_string());

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("already exists"));
    }

    #[test]
    fn test_create_multiple_accounts() {
        let (mut wallet, _temp_dir) = create_temp_wallet();

        wallet.create_account("alice".to_string()).unwrap();
        wallet.create_account("bob".to_string()).unwrap();
        wallet.create_account("charlie".to_string()).unwrap();

        let accounts = wallet.list_accounts();
        assert_eq!(accounts.len(), 3);
    }

    #[test]
    fn test_import_account() {
        let (mut wallet, _temp_dir) = create_temp_wallet();

        let secret = "0101010101010101010101010101010101010101010101010101010101010101";
        let addr = wallet.import_account("imported".to_string(), secret).unwrap();

        assert!(addr.to_hex().len() == 64);

        let accounts = wallet.list_accounts();
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].0, "imported");
    }

    #[test]
    fn test_import_invalid_hex() {
        let (mut wallet, _temp_dir) = create_temp_wallet();

        let result = wallet.import_account("test".to_string(), "invalid_hex");
        assert!(result.is_err());
    }

    #[test]
    fn test_import_wrong_length() {
        let (mut wallet, _temp_dir) = create_temp_wallet();

        let result = wallet.import_account("test".to_string(), "0102"); // Too short
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(e.to_string().contains("32 bytes"));
        }
    }

    #[test]
    fn test_import_duplicate_name() {
        let (mut wallet, _temp_dir) = create_temp_wallet();

        let secret = "0101010101010101010101010101010101010101010101010101010101010101";
        wallet.import_account("alice".to_string(), secret).unwrap();

        let result = wallet.import_account("alice".to_string(), secret);
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(e.to_string().contains("already exists"));
        }
    }

    #[test]
    fn test_get_keypair() {
        let (mut wallet, _temp_dir) = create_temp_wallet();

        let addr = wallet.create_account("alice".to_string()).unwrap();
        let kp = wallet.get_keypair("alice").unwrap();

        assert_eq!(kp.address(), addr);
    }

    #[test]
    fn test_get_keypair_nonexistent() {
        let (wallet, _temp_dir) = create_temp_wallet();

        let result = wallet.get_keypair("nonexistent");
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(e.to_string().contains("not found"));
        }
    }

    #[test]
    fn test_list_accounts() {
        let (mut wallet, _temp_dir) = create_temp_wallet();

        wallet.create_account("alice".to_string()).unwrap();
        wallet.create_account("bob".to_string()).unwrap();

        let accounts = wallet.list_accounts();
        assert_eq!(accounts.len(), 2);

        let names: Vec<String> = accounts.iter().map(|(n, _)| n.clone()).collect();
        assert!(names.contains(&"alice".to_string()));
        assert!(names.contains(&"bob".to_string()));
    }

    #[test]
    fn test_export_secret() {
        let (mut wallet, _temp_dir) = create_temp_wallet();

        let secret = "0101010101010101010101010101010101010101010101010101010101010101";
        wallet.import_account("alice".to_string(), secret).unwrap();

        let exported = wallet.export_secret("alice").unwrap();
        assert_eq!(exported, secret);
    }

    #[test]
    fn test_export_secret_nonexistent() {
        let (wallet, _temp_dir) = create_temp_wallet();

        let result = wallet.export_secret("nonexistent");
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(e.to_string().contains("not found"));
        }
    }

    #[test]
    fn test_persistence() {
        let temp_dir = TempDir::new().unwrap();
        let wallet_path = temp_dir.path().join("test_wallet.json");

        // Create wallet and add account
        {
            let mut wallet = Wallet::new(wallet_path.clone()).unwrap();
            wallet.create_account("alice".to_string()).unwrap();
        }

        // Load wallet and verify account exists
        {
            let wallet = Wallet::new(wallet_path.clone()).unwrap();
            let accounts = wallet.list_accounts();
            assert_eq!(accounts.len(), 1);
            assert_eq!(accounts[0].0, "alice");
        }
    }

    #[test]
    fn test_keypair_consistency() {
        let (mut wallet, _temp_dir) = create_temp_wallet();

        let addr = wallet.create_account("alice".to_string()).unwrap();
        let kp1 = wallet.get_keypair("alice").unwrap();
        let kp2 = wallet.get_keypair("alice").unwrap();

        // Same secret should produce same keypair
        assert_eq!(kp1.address(), addr);
        assert_eq!(kp2.address(), addr);
        assert_eq!(kp1.secret_bytes(), kp2.secret_bytes());
    }

    #[test]
    fn test_sign_with_wallet_keypair() {
        let (mut wallet, _temp_dir) = create_temp_wallet();

        wallet.create_account("alice".to_string()).unwrap();
        let kp = wallet.get_keypair("alice").unwrap();

        let message = b"test message";
        let signature = kp.sign(message);

        // Verify signature works
        assert!(miraset_core::verify_signature(&kp.address(), message, &signature));
    }

    #[test]
    fn test_wallet_file_format() {
        let temp_dir = TempDir::new().unwrap();
        let wallet_path = temp_dir.path().join("test_wallet.json");

        {
            let mut wallet = Wallet::new(wallet_path.clone()).unwrap();
            wallet.create_account("alice".to_string()).unwrap();
        }

        // Read and verify JSON structure
        let content = fs::read_to_string(&wallet_path).unwrap();
        let json: serde_json::Value = serde_json::from_str(&content).unwrap();

        assert!(json.get("accounts").is_some());
        assert!(json["accounts"].is_object());
        assert!(json["accounts"].get("alice").is_some());
    }

    #[test]
    fn test_empty_wallet_persistence() {
        let temp_dir = TempDir::new().unwrap();
        let wallet_path = temp_dir.path().join("test_wallet.json");

        // Create empty wallet (should not save until first account)
        let wallet = Wallet::new(wallet_path.clone()).unwrap();
        assert_eq!(wallet.list_accounts().len(), 0);
    }
}
