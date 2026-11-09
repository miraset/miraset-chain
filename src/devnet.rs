//! Experimental devnet tooling: validator key files and multi-validator genesis generation.
//!
//! Key files hold a hex-encoded Ed25519 secret, are created with owner-only permissions, and
//! are refused when group/world accessible. Secrets are never printed by this module.

use crate::node::{GenesisAccount, GenesisConfig, GenesisValidator};
use ed25519_dalek::SigningKey;
use rand::RngExt;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

pub const GENESIS_FILE: &str = "genesis.json";
pub const FAUCET_KEY_FILE: &str = "faucet.key";
const SECRET_KEY_BYTES: usize = 32;
const VALIDATOR_POWER: u64 = 100;

/// Generate a fresh random Ed25519 signing key.
pub fn generate_signing_key() -> SigningKey {
    let mut secret = [0u8; SECRET_KEY_BYTES];
    rand::rng().fill(&mut secret);
    SigningKey::from_bytes(&secret)
}

/// Write `key` as hex to a new file with mode 0600. Refuses to overwrite an existing file.
pub fn write_key_file(path: &Path, key: &SigningKey) -> Result<(), String> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|e| format!("cannot create key file {}: {e}", path.display()))?;
    file.write_all(hex::encode(key.to_bytes()).as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|e| format!("cannot write key file {}: {e}", path.display()))
}

/// Read a hex-encoded Ed25519 secret key file, failing closed on bad permissions or contents.
pub fn read_key_file(path: &Path) -> Result<SigningKey, String> {
    check_key_permissions(path)?;
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read key file {}: {e}", path.display()))?;
    let bytes: [u8; SECRET_KEY_BYTES] = hex::decode(content.trim())
        .ok()
        .and_then(|b| b.try_into().ok())
        .ok_or_else(|| {
            format!(
                "key file {} must contain 32 hex-encoded bytes",
                path.display()
            )
        })?;
    Ok(SigningKey::from_bytes(&bytes))
}

#[cfg(unix)]
fn check_key_permissions(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(path)
        .map_err(|e| format!("cannot stat key file {}: {e}", path.display()))?
        .permissions()
        .mode();
    if mode & 0o077 != 0 {
        return Err(format!(
            "key file {} is accessible by group/others (mode {:o}); run chmod 600",
            path.display(),
            mode & 0o777
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
fn check_key_permissions(_path: &Path) -> Result<(), String> {
    Ok(())
}

/// Parameters for [`init_devnet`].
pub struct DevnetSpec<'a> {
    pub out_dir: &'a Path,
    pub validators: usize,
    pub chain_id: u64,
    /// `host:port` template; `{i}` is replaced with the validator index.
    pub p2p_addr_template: &'a str,
    pub faucet_balance: u64,
}

/// Path of validator `index`'s key file inside a devnet directory.
pub fn validator_key_path(out_dir: &Path, index: usize) -> PathBuf {
    out_dir.join(format!("node{index}.key"))
}

/// Create validator keys, a funded faucet key, and `genesis.json` in `spec.out_dir`.
/// Returns `Ok(false)` without touching anything if a genesis file already exists.
pub fn init_devnet(spec: &DevnetSpec<'_>) -> Result<bool, String> {
    let genesis_path = spec.out_dir.join(GENESIS_FILE);
    if genesis_path.exists() {
        return Ok(false);
    }
    if spec.validators == 0 {
        return Err("devnet needs at least one validator".into());
    }
    if !spec.p2p_addr_template.contains("{i}") {
        return Err("p2p address template must contain {i}".into());
    }
    std::fs::create_dir_all(spec.out_dir)
        .map_err(|e| format!("cannot create {}: {e}", spec.out_dir.display()))?;

    let mut validators = Vec::with_capacity(spec.validators);
    for index in 0..spec.validators {
        let key = generate_signing_key();
        write_key_file(&validator_key_path(spec.out_dir, index), &key)?;
        validators.push(GenesisValidator {
            pubkey: hex::encode(key.verifying_key().to_bytes()),
            power: VALIDATOR_POWER,
            p2p_addr: spec.p2p_addr_template.replace("{i}", &index.to_string()),
        });
    }

    let faucet = generate_signing_key();
    write_key_file(&spec.out_dir.join(FAUCET_KEY_FILE), &faucet)?;
    let genesis = GenesisConfig {
        chain_id: spec.chain_id,
        validators,
        accounts: vec![GenesisAccount {
            address: hex::encode(faucet.verifying_key().to_bytes()),
            balance: spec.faucet_balance,
        }],
    };
    genesis.validate()?;

    // Write genesis last and atomically so a partial run never looks initialized.
    let json = serde_json::to_string_pretty(&genesis).map_err(|e| e.to_string())?;
    let tmp = spec.out_dir.join(format!("{GENESIS_FILE}.tmp"));
    std::fs::write(&tmp, json).map_err(|e| format!("cannot write genesis: {e}"))?;
    std::fs::rename(&tmp, &genesis_path).map_err(|e| format!("cannot finalize genesis: {e}"))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(dir: &Path) -> DevnetSpec<'_> {
        DevnetSpec {
            out_dir: dir,
            validators: 5,
            chain_id: 4242,
            p2p_addr_template: "node{i}:9948",
            faucet_balance: 1_000_000,
        }
    }

    #[test]
    fn test_init_devnet_writes_consistent_keys_and_genesis() {
        let dir = tempfile::tempdir().unwrap();
        assert!(init_devnet(&spec(dir.path())).unwrap());

        let genesis = GenesisConfig::load(&dir.path().join(GENESIS_FILE)).unwrap();
        assert_eq!(genesis.chain_id, 4242);
        assert_eq!(genesis.validators.len(), 5);
        for (index, validator) in genesis.validators.iter().enumerate() {
            let key = read_key_file(&validator_key_path(dir.path(), index)).unwrap();
            assert_eq!(
                validator.pubkey,
                hex::encode(key.verifying_key().to_bytes())
            );
            assert_eq!(validator.p2p_addr, format!("node{index}:9948"));
        }
        let faucet = read_key_file(&dir.path().join(FAUCET_KEY_FILE)).unwrap();
        assert_eq!(
            genesis.accounts[0].address,
            hex::encode(faucet.verifying_key().to_bytes())
        );

        // Idempotent: a second run leaves the existing devnet untouched.
        let before = std::fs::read(dir.path().join(GENESIS_FILE)).unwrap();
        assert!(!init_devnet(&spec(dir.path())).unwrap());
        assert_eq!(
            std::fs::read(dir.path().join(GENESIS_FILE)).unwrap(),
            before
        );
    }

    #[test]
    fn test_key_files_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("k.key");
        let key = generate_signing_key();
        write_key_file(&path, &key).unwrap();
        assert_eq!(read_key_file(&path).unwrap().to_bytes(), key.to_bytes());
        assert!(write_key_file(&path, &key).is_err(), "must not overwrite");

        let short = dir.path().join("short.key");
        std::fs::write(&short, "abcd").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&short, std::fs::Permissions::from_mode(0o600)).unwrap();
            assert!(read_key_file(&short).is_err(), "wrong length");

            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
            assert!(read_key_file(&path).is_err(), "world-readable key refused");
        }
    }

    #[test]
    fn test_init_devnet_rejects_bad_spec() {
        let dir = tempfile::tempdir().unwrap();
        let mut bad = spec(dir.path());
        bad.p2p_addr_template = "node:9948";
        assert!(init_devnet(&bad).is_err());
        let mut none = spec(dir.path());
        none.validators = 0;
        assert!(init_devnet(&none).is_err());
        assert!(!dir.path().join(GENESIS_FILE).exists());
    }
}
