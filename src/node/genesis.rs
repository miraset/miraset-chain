//! Devnet genesis file: chain identity, static validator set with peer addresses, and funded
//! accounts. Every validator must start from a byte-identical genesis file.

use crate::core::types::ProtocolParams;
use ed25519_dalek::VerifyingKey;
use hotstuff_rs::block_tree::pluggables::KVGet;
use hotstuff_rs::types::data_types::Power;
use hotstuff_rs::types::validator_set::ValidatorSet;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::path::Path;

/// Upper bound that keeps total power far below `hotstuff_rs`'s `u128::MAX / 2` limit and
/// leaves room for future reconfiguration.
const MAX_VALIDATOR_POWER: u64 = 1_000_000;
const MAX_VALIDATORS: usize = 128;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GenesisValidator {
    /// Hex-encoded Ed25519 verifying key.
    pub pubkey: String,
    pub power: u64,
    /// `host:port` of the validator's P2P listener, resolvable by every other validator.
    pub p2p_addr: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GenesisAccount {
    pub address: String,
    pub balance: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GenesisConfig {
    pub chain_id: u64,
    pub validators: Vec<GenesisValidator>,
    pub accounts: Vec<GenesisAccount>,
    /// Receipt coordinators, governance keys, price and reward budget (TODO 8.5.1). Absent:
    /// no coordinator or governance key, so no receipt settles and no epoch closes.
    #[serde(default)]
    pub protocol: ProtocolParams,
}

impl GenesisConfig {
    /// Read and validate a genesis file.
    pub fn load(path: &Path) -> Result<Self, String> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read genesis {}: {e}", path.display()))?;
        let genesis: Self = serde_json::from_str(&raw)
            .map_err(|e| format!("invalid genesis {}: {e}", path.display()))?;
        genesis.validate()?;
        Ok(genesis)
    }

    /// Structural validation; fails closed on anything ambiguous.
    pub fn validate(&self) -> Result<(), String> {
        if self.chain_id == 0 {
            return Err("genesis chain_id must be non-zero".into());
        }
        if self.validators.is_empty() || self.validators.len() > MAX_VALIDATORS {
            return Err(format!("genesis must list 1..={MAX_VALIDATORS} validators"));
        }
        let mut keys = BTreeSet::new();
        for validator in &self.validators {
            let key = parse_pubkey(&validator.pubkey)?;
            if !keys.insert(key.to_bytes()) {
                return Err(format!("duplicate validator {}", validator.pubkey));
            }
            if validator.power == 0 || validator.power > MAX_VALIDATOR_POWER {
                return Err(format!(
                    "validator {} power must be in 1..={MAX_VALIDATOR_POWER}",
                    validator.pubkey
                ));
            }
            validate_host_port(&validator.p2p_addr)?;
        }
        self.protocol.validate()?;
        // Separate roles (ADR-0007): a validator key is never a coordinator or governance key.
        for key in self
            .protocol
            .coordinators
            .iter()
            .chain(&self.protocol.governance)
        {
            if self
                .validators
                .iter()
                .any(|v| v.pubkey.eq_ignore_ascii_case(key))
            {
                return Err(format!(
                    "validator key {key} cannot also be a protocol authority"
                ));
            }
        }
        let mut addresses = BTreeSet::new();
        for account in &self.accounts {
            if account.address.trim().is_empty() || account.address.trim() != account.address {
                return Err(format!("invalid account address '{}'", account.address));
            }
            if !addresses.insert(account.address.as_str()) {
                return Err(format!("duplicate account {}", account.address));
            }
        }
        Ok(())
    }

    /// `(verifying key, power)` pairs for `init_genesis`.
    pub fn validator_powers(&self) -> Result<Vec<(VerifyingKey, u64)>, String> {
        self.validators
            .iter()
            .map(|v| Ok((parse_pubkey(&v.pubkey)?, v.power)))
            .collect()
    }

    /// `(address, balance)` pairs for `init_genesis`.
    pub fn initial_accounts(&self) -> Vec<(&str, u64)> {
        self.accounts
            .iter()
            .map(|a| (a.address.as_str(), a.balance))
            .collect()
    }

    /// Genesis state commitment (accounts, protocol parameters, epoch 0).
    pub fn state_commitment(&self) -> [u8; 32] {
        crate::node::genesis_state_commitment_with_params(&self.initial_accounts(), &self.protocol)
    }

    /// P2P address table for every validator other than `local`.
    pub fn peer_addresses(
        &self,
        local: &VerifyingKey,
    ) -> Result<HashMap<VerifyingKey, String>, String> {
        let mut peers = HashMap::new();
        for validator in &self.validators {
            let key = parse_pubkey(&validator.pubkey)?;
            if key != *local {
                peers.insert(key, validator.p2p_addr.clone());
            }
        }
        Ok(peers)
    }

    /// Whether `key` is a genesis validator.
    pub fn is_validator(&self, key: &VerifyingKey) -> bool {
        self.validator_powers()
            .map(|set| set.iter().any(|(k, _)| k == key))
            .unwrap_or(false)
    }

    /// Fail closed if an existing store's committed validator set differs from this genesis.
    /// Only meaningful while validator reconfiguration is disabled (TODO 8.3.3).
    pub fn verify_store_validator_set<G: KVGet>(&self, store: &G) -> Result<(), String> {
        let stored = store
            .committed_validator_set()
            .map_err(|e| format!("cannot read stored validator set: {e:?}"))?;
        let mut expected = ValidatorSet::new();
        for (key, power) in self.validator_powers()? {
            expected.put(&key, Power::new(power));
        }
        if stored != expected {
            return Err("stored validator set does not match the genesis file".into());
        }
        Ok(())
    }
}

/// Parse a hex-encoded Ed25519 verifying key.
pub fn parse_pubkey(hex_key: &str) -> Result<VerifyingKey, String> {
    let bytes: [u8; 32] = hex::decode(hex_key)
        .ok()
        .and_then(|b| b.try_into().ok())
        .ok_or_else(|| format!("invalid validator pubkey '{hex_key}'"))?;
    VerifyingKey::from_bytes(&bytes).map_err(|_| format!("invalid validator pubkey '{hex_key}'"))
}

/// Validate `host:port` syntax without resolving it (resolution happens at connect time).
pub fn validate_host_port(address: &str) -> Result<(), String> {
    match address.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() && port.parse::<u16>().is_ok_and(|p| p != 0) => {
            Ok(())
        }
        _ => Err(format!("invalid address '{address}', expected host:port")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::init_genesis_with_chain_id;
    use crate::storage::MemKVStore;
    use ed25519_dalek::SigningKey;

    fn pubkey(seed: u8) -> String {
        hex::encode(
            SigningKey::from_bytes(&[seed; 32])
                .verifying_key()
                .to_bytes(),
        )
    }

    fn sample() -> GenesisConfig {
        GenesisConfig {
            chain_id: 1337,
            validators: (1..=3)
                .map(|i| GenesisValidator {
                    pubkey: pubkey(i),
                    power: 100,
                    p2p_addr: format!("node{i}:9948"),
                })
                .collect(),
            accounts: vec![GenesisAccount {
                address: pubkey(9),
                balance: 1_000,
            }],
            protocol: ProtocolParams::default(),
        }
    }

    #[test]
    fn test_genesis_accepts_valid_and_round_trips() {
        let genesis = sample();
        assert!(genesis.validate().is_ok());
        let json = serde_json::to_string(&genesis).unwrap();
        assert_eq!(
            serde_json::from_str::<GenesisConfig>(&json).unwrap(),
            genesis
        );

        let local = parse_pubkey(&pubkey(1)).unwrap();
        let peers = genesis.peer_addresses(&local).unwrap();
        assert_eq!(peers.len(), 2);
        assert!(!peers.contains_key(&local));
        assert!(genesis.is_validator(&local));
        assert!(!genesis.is_validator(&parse_pubkey(&pubkey(9)).unwrap()));
    }

    #[test]
    fn test_genesis_rejects_invalid_configurations() {
        let mutations: Vec<fn(&mut GenesisConfig)> = vec![
            |g| g.chain_id = 0,
            |g| g.validators.clear(),
            |g| g.validators[1].pubkey = g.validators[0].pubkey.clone(),
            |g| g.validators[0].pubkey = "zz".into(),
            |g| g.validators[0].power = 0,
            |g| g.validators[0].power = MAX_VALIDATOR_POWER + 1,
            |g| g.validators[0].p2p_addr = "node1".into(),
            |g| g.validators[0].p2p_addr = ":9948".into(),
            |g| g.validators[0].p2p_addr = "node1:0".into(),
            |g| g.accounts.push(g.accounts[0].clone()),
            |g| g.accounts[0].address = " padded".into(),
        ];
        for (index, mutate) in mutations.into_iter().enumerate() {
            let mut genesis = sample();
            mutate(&mut genesis);
            assert!(
                genesis.validate().is_err(),
                "mutation {index} should be rejected"
            );
        }
        assert!(
            serde_json::from_str::<GenesisConfig>(
                r#"{"chain_id":1,"validators":[],"accounts":[],"extra":1}"#
            )
            .is_err()
        );
    }

    #[test]
    fn test_genesis_detects_validator_set_mismatch_in_store() {
        let genesis = sample();
        let store = MemKVStore::new();
        init_genesis_with_chain_id(
            store.clone(),
            genesis.chain_id,
            &genesis.validator_powers().unwrap(),
            &genesis.initial_accounts(),
        );
        assert!(genesis.verify_store_validator_set(&store).is_ok());

        let mut other = genesis.clone();
        other.validators[0].power = 7;
        assert!(other.verify_store_validator_set(&store).is_err());
    }

    #[test]
    fn test_protocol_section_is_validated_and_part_of_the_genesis_hash() {
        let mut genesis = sample();
        let without = genesis.state_commitment();
        // A genesis file without a `protocol` section still loads (no authorities).
        let mut json: serde_json::Value = serde_json::to_value(&genesis).unwrap();
        json.as_object_mut().unwrap().remove("protocol");
        let parsed: GenesisConfig = serde_json::from_value(json).unwrap();
        assert_eq!(parsed.protocol, ProtocolParams::default());

        genesis.protocol.coordinators = vec![pubkey(7)];
        assert!(genesis.validate().is_ok());
        assert_ne!(genesis.state_commitment(), without, "params are committed");
        // A validator key cannot double as a protocol authority.
        genesis.protocol.governance = vec![pubkey(1)];
        assert!(genesis.validate().is_err());
    }

    /// TODO 8.5.2 F2: the pacemaker epoch length is a genesis parameter, so validators of one
    /// chain share it and a different value is a different genesis commitment.
    #[test]
    fn test_consensus_epoch_length_is_part_of_the_genesis_hash() {
        let mut genesis = sample();
        assert_eq!(
            genesis.protocol.consensus_epoch_length,
            crate::node::DEFAULT_EPOCH_LENGTH
        );
        let default_hash = genesis.state_commitment();
        genesis.protocol.consensus_epoch_length = 10;
        assert!(genesis.validate().is_ok());
        assert_ne!(genesis.state_commitment(), default_hash);
        genesis.protocol.consensus_epoch_length = 0;
        assert!(genesis.validate().is_err());
        genesis.protocol.consensus_epoch_length =
            crate::core::types::MAX_CONSENSUS_EPOCH_LENGTH + 1;
        assert!(genesis.validate().is_err());

        // A genesis file written before the field existed loads with the default.
        let mut json: serde_json::Value = serde_json::to_value(sample()).unwrap();
        json["protocol"]
            .as_object_mut()
            .unwrap()
            .remove("consensus_epoch_length");
        let parsed: GenesisConfig = serde_json::from_value(json).unwrap();
        assert_eq!(
            parsed.protocol.consensus_epoch_length,
            crate::node::DEFAULT_EPOCH_LENGTH
        );
    }
}
