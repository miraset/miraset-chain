use anyhow::{Context, Result};
use miraset_core::{Address, KeyPair};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use argon2::Argon2;
use rand::RngCore;

/// Salt length for argon2
const SALT_LEN: usize = 16;
/// Nonce length for AES-256-GCM
const NONCE_LEN: usize = 12;

#[derive(Debug, Serialize, Deserialize)]
struct WalletData {
    accounts: HashMap<String, [u8; 32]>, // name -> secret_key
}

/// Encrypted wallet envelope (stored on disk when password is set)
#[derive(Debug, Serialize, Deserialize)]
struct EncryptedWallet {
    version: u8,
    salt: String,       // hex-encoded argon2 salt
    nonce: String,      // hex-encoded AES-GCM nonce
    ciphertext: String, // base64-encoded encrypted WalletData JSON
}

#[derive(Debug)]
pub struct Wallet {
    path: PathBuf,
    data: WalletData,
    /// If Some, wallet is encrypted on disk with this password
    password: Option<String>,
}

impl Wallet {
    /// Open or create a wallet (plaintext, backward-compatible)
    pub fn new(path: PathBuf) -> Result<Self> {
        let data = if path.exists() {
            let content = fs::read_to_string(&path).context("Failed to read wallet file")?;
            // Try encrypted format first
            if let Ok(_envelope) = serde_json::from_str::<EncryptedWallet>(&content) {
                // Encrypted wallet — cannot open without password
                anyhow::bail!(
                    "Wallet is encrypted. Use Wallet::open_encrypted(path, password) instead."
                );
            }
            serde_json::from_str(&content).context("Failed to parse wallet file")?
        } else {
            WalletData {
                accounts: HashMap::new(),
            }
        };
        Ok(Self {
            path,
            data,
            password: None,
        })
    }

    /// Open an encrypted wallet with password
    pub fn open_encrypted(path: PathBuf, password: &str) -> Result<Self> {
        if !path.exists() {
            // New encrypted wallet
            return Ok(Self {
                path,
                data: WalletData {
                    accounts: HashMap::new(),
                },
                password: Some(password.to_string()),
            });
        }

        let content = fs::read_to_string(&path).context("Failed to read wallet file")?;

        // Try encrypted format
        if let Ok(envelope) = serde_json::from_str::<EncryptedWallet>(&content) {
            let data = decrypt_wallet_data(&envelope, password)?;
            return Ok(Self {
                path,
                data,
                password: Some(password.to_string()),
            });
        }

        // Fallback: plaintext wallet being upgraded to encrypted
        let data: WalletData =
            serde_json::from_str(&content).context("Failed to parse wallet file")?;
        let wallet = Self {
            path,
            data,
            password: Some(password.to_string()),
        };
        // Re-save encrypted
        wallet.save()?;
        Ok(wallet)
    }

    /// Set or change password (encrypts the wallet file on next save)
    pub fn set_password(&mut self, password: &str) -> Result<()> {
        self.password = Some(password.to_string());
        self.save()
    }

    /// Remove password (saves as plaintext)
    pub fn remove_password(&mut self) -> Result<()> {
        self.password = None;
        self.save()
    }

    /// Check if wallet is encrypted
    pub fn is_encrypted(&self) -> bool {
        self.password.is_some()
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
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }

        let content = if let Some(ref password) = self.password {
            // Encrypted save
            let envelope = encrypt_wallet_data(&self.data, password)?;
            serde_json::to_string_pretty(&envelope)?
        } else {
            // Plaintext save (backward-compatible)
            serde_json::to_string_pretty(&self.data)?
        };

        fs::write(&self.path, content)?;
        Ok(())
    }
}

/// Derive a 32-byte key from password + salt using Argon2id
fn derive_key(password: &str, salt: &[u8]) -> Result<[u8; 32]> {
    let mut key = [0u8; 32];
    Argon2::default()
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .map_err(|e| anyhow::anyhow!("Key derivation failed: {}", e))?;
    Ok(key)
}

/// Encrypt WalletData to EncryptedWallet envelope
fn encrypt_wallet_data(data: &WalletData, password: &str) -> Result<EncryptedWallet> {
    let plaintext = serde_json::to_vec(data)?;

    let mut salt = [0u8; SALT_LEN];
    rand::thread_rng().fill_bytes(&mut salt);

    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);

    let key = derive_key(password, &salt)?;
    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|e| anyhow::anyhow!("Cipher init failed: {}", e))?;
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_slice())
        .map_err(|e| anyhow::anyhow!("Encryption failed: {}", e))?;

    Ok(EncryptedWallet {
        version: 1,
        salt: hex::encode(salt),
        nonce: hex::encode(nonce_bytes),
        ciphertext: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &ciphertext),
    })
}

/// Decrypt EncryptedWallet envelope to WalletData
fn decrypt_wallet_data(envelope: &EncryptedWallet, password: &str) -> Result<WalletData> {
    let salt = hex::decode(&envelope.salt).context("Invalid salt hex")?;
    let nonce_bytes = hex::decode(&envelope.nonce).context("Invalid nonce hex")?;
    let ciphertext = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        &envelope.ciphertext,
    )
    .context("Invalid ciphertext base64")?;

    if nonce_bytes.len() != NONCE_LEN {
        anyhow::bail!("Invalid nonce length");
    }

    let key = derive_key(password, &salt)?;
    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|e| anyhow::anyhow!("Cipher init failed: {}", e))?;
    let nonce = Nonce::from_slice(&nonce_bytes);

    let plaintext = cipher
        .decrypt(nonce, ciphertext.as_slice())
        .map_err(|_| anyhow::anyhow!("Decryption failed: wrong password or corrupted file"))?;

    let data: WalletData = serde_json::from_slice(&plaintext)?;
    Ok(data)
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

    // ===== D9: Encryption tests =====

    #[test]
    fn test_encrypted_wallet_create_and_open() {
        let temp_dir = TempDir::new().unwrap();
        let wallet_path = temp_dir.path().join("encrypted_wallet.json");

        // Create encrypted wallet
        {
            let mut wallet =
                Wallet::open_encrypted(wallet_path.clone(), "mypassword123").unwrap();
            assert!(wallet.is_encrypted());
            wallet.create_account("alice".to_string()).unwrap();
            wallet.create_account("bob".to_string()).unwrap();
        }

        // Re-open with correct password
        {
            let wallet =
                Wallet::open_encrypted(wallet_path.clone(), "mypassword123").unwrap();
            let accounts = wallet.list_accounts();
            assert_eq!(accounts.len(), 2);
        }

        // Fail with wrong password
        {
            let result = Wallet::open_encrypted(wallet_path.clone(), "wrongpassword");
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("wrong password"));
        }
    }

    #[test]
    fn test_encrypted_wallet_roundtrip() {
        let temp_dir = TempDir::new().unwrap();
        let wallet_path = temp_dir.path().join("enc_rt.json");

        let secret = "0101010101010101010101010101010101010101010101010101010101010101";

        {
            let mut wallet =
                Wallet::open_encrypted(wallet_path.clone(), "pass").unwrap();
            wallet.import_account("test".to_string(), secret).unwrap();
        }

        {
            let wallet = Wallet::open_encrypted(wallet_path.clone(), "pass").unwrap();
            let exported = wallet.export_secret("test").unwrap();
            assert_eq!(exported, secret);
        }
    }

    #[test]
    fn test_plaintext_cannot_open_encrypted() {
        let temp_dir = TempDir::new().unwrap();
        let wallet_path = temp_dir.path().join("enc_noopen.json");

        // Create encrypted
        {
            let mut wallet =
                Wallet::open_encrypted(wallet_path.clone(), "secret").unwrap();
            wallet.create_account("alice".to_string()).unwrap();
        }

        // Try opening as plaintext
        let result = Wallet::new(wallet_path.clone());
        assert!(result.is_err());
    }

    #[test]
    fn test_upgrade_plaintext_to_encrypted() {
        let temp_dir = TempDir::new().unwrap();
        let wallet_path = temp_dir.path().join("upgrade.json");

        // Create plaintext wallet
        {
            let mut wallet = Wallet::new(wallet_path.clone()).unwrap();
            wallet.create_account("alice".to_string()).unwrap();
        }

        // Re-open with password → auto-upgrade
        {
            let wallet =
                Wallet::open_encrypted(wallet_path.clone(), "newpass").unwrap();
            assert!(wallet.is_encrypted());
            assert_eq!(wallet.list_accounts().len(), 1);
        }

        // Verify it's now encrypted
        {
            let wallet = Wallet::open_encrypted(wallet_path.clone(), "newpass").unwrap();
            assert_eq!(wallet.list_accounts().len(), 1);
        }
    }
}
