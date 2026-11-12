//! Experimental devnet tooling: multi-validator genesis and key generation.
//!
//! Key file handling lives in [`crate::keys`] (re-exported here for existing callers). With a
//! keys directory, each validator key and the faucet key get their own subdirectory so a
//! deployment can mount one key per container (key custody proposal D5).

use crate::core::types::ProtocolParams;
use crate::node::{GenesisAccount, GenesisConfig, GenesisValidator};
use std::path::{Path, PathBuf};

pub use crate::keys::{generate_signing_key, read_key_file, write_key_file};

pub const GENESIS_FILE: &str = "genesis.json";
pub const FAUCET_KEY_FILE: &str = "faucet.key";
pub const COORDINATOR_KEY_FILE: &str = "coordinator.key";
pub const GOVERNANCE_KEY_FILE: &str = "governance.key";
const VALIDATOR_POWER: u64 = 100;

/// Parameters for [`init_devnet`].
pub struct DevnetSpec<'a> {
    pub out_dir: &'a Path,
    pub validators: usize,
    pub chain_id: u64,
    /// `host:port` template; `{i}` is replaced with the validator index.
    pub p2p_addr_template: &'a str,
    pub faucet_balance: u64,
    /// When set, keys go to `<keys_dir>/node{i}/node{i}.key` and `<keys_dir>/faucet/faucet.key`
    /// instead of next to `genesis.json`.
    pub keys_dir: Option<&'a Path>,
}

/// Path of validator `index`'s key file inside a devnet directory (flat layout).
pub fn validator_key_path(out_dir: &Path, index: usize) -> PathBuf {
    out_dir.join(format!("node{index}.key"))
}

/// Validator key path for `spec`, honouring `keys_dir`.
pub fn spec_validator_key_path(spec: &DevnetSpec<'_>, index: usize) -> PathBuf {
    match spec.keys_dir {
        Some(root) => root
            .join(format!("node{index}"))
            .join(format!("node{index}.key")),
        None => validator_key_path(spec.out_dir, index),
    }
}

/// Faucet key path for `spec`, honouring `keys_dir`.
pub fn spec_faucet_key_path(spec: &DevnetSpec<'_>) -> PathBuf {
    spec_role_key_path(spec, "faucet", FAUCET_KEY_FILE)
}

/// Path of a role key (`faucet`, `coordinator`, `governance`), honouring `keys_dir`.
pub fn spec_role_key_path(spec: &DevnetSpec<'_>, role: &str, file: &str) -> PathBuf {
    match spec.keys_dir {
        Some(root) => root.join(role).join(file),
        None => spec.out_dir.join(file),
    }
}

fn create_role_key(
    spec: &DevnetSpec<'_>,
    role: &str,
    file: &str,
) -> Result<ed25519_dalek::SigningKey, String> {
    let key = generate_signing_key();
    let path = spec_role_key_path(spec, role, file);
    ensure_key_dir(&path)?;
    write_key_file(&path, &key)?;
    Ok(key)
}

/// Create the parent directory of a key file (owner-only when it is new).
fn ensure_key_dir(key_path: &Path) -> Result<(), String> {
    let Some(parent) = key_path.parent() else {
        return Ok(());
    };
    if parent.exists() {
        return Ok(());
    }
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(parent)
        .map_err(|e| format!("cannot create key directory {}: {e}", parent.display()))
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
        let key_path = spec_validator_key_path(spec, index);
        ensure_key_dir(&key_path)?;
        write_key_file(&key_path, &key)?;
        validators.push(GenesisValidator {
            pubkey: hex::encode(key.verifying_key().to_bytes()),
            power: VALIDATOR_POWER,
            p2p_addr: spec.p2p_addr_template.replace("{i}", &index.to_string()),
        });
    }

    let faucet = create_role_key(spec, "faucet", FAUCET_KEY_FILE)?;
    // TODO 8.5.1: one coordinator (receipt co-signer) and one governance key (slashing, epoch
    // close) for the devnet.
    let coordinator = create_role_key(spec, "coordinator", COORDINATOR_KEY_FILE)?;
    let governance = create_role_key(spec, "governance", GOVERNANCE_KEY_FILE)?;
    let genesis = GenesisConfig {
        chain_id: spec.chain_id,
        validators,
        accounts: vec![GenesisAccount {
            address: hex::encode(faucet.verifying_key().to_bytes()),
            balance: spec.faucet_balance,
        }],
        protocol: ProtocolParams {
            coordinators: vec![hex::encode(coordinator.verifying_key().to_bytes())],
            governance: vec![hex::encode(governance.verifying_key().to_bytes())],
            ..ProtocolParams::default()
        },
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
            keys_dir: None,
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
    fn test_init_devnet_keys_dir_puts_each_key_in_its_own_directory() {
        let dir = tempfile::tempdir().unwrap();
        let keys = dir.path().join("keys");
        let mut with_keys = spec(dir.path());
        with_keys.keys_dir = Some(&keys);
        assert!(init_devnet(&with_keys).unwrap());

        // genesis.json is the only file next to the config; no key leaks into it.
        let config_files: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .filter(|name| name != "keys")
            .collect();
        assert_eq!(config_files, vec![GENESIS_FILE.to_string()]);

        let genesis = GenesisConfig::load(&dir.path().join(GENESIS_FILE)).unwrap();
        for (index, validator) in genesis.validators.iter().enumerate() {
            let node_dir = keys.join(format!("node{index}"));
            let entries: Vec<_> = std::fs::read_dir(&node_dir).unwrap().collect();
            assert_eq!(entries.len(), 1, "exactly one key per node directory");
            let key = read_key_file(&spec_validator_key_path(&with_keys, index)).unwrap();
            assert_eq!(
                validator.pubkey,
                hex::encode(key.verifying_key().to_bytes())
            );
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = std::fs::metadata(&node_dir).unwrap().permissions().mode();
                assert_eq!(mode & 0o077, 0, "new key directories are owner-only");
            }
        }
        let faucet = read_key_file(&spec_faucet_key_path(&with_keys)).unwrap();
        assert_eq!(
            genesis.accounts[0].address,
            hex::encode(faucet.verifying_key().to_bytes())
        );
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
