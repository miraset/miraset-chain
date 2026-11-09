//! A validator signer *failure* (not a refusal) halts the node visibly (key custody D1, vendored
//! patch #9). Own test binary: the consensus panic hook is process-wide.

use ed25519_dalek::SigningKey;
use hotstuff_rs::types::crypto_primitives::VerifyingKey;
use hotstuff_rs::types::data_types::SignatureBytes;
use hotstuff_rs::types::signer::{LocalKeySigner, SignError, SignRequest, ValidatorSigner};
use miraset_chain::consensus::SafetyHalt;
use miraset_chain::mempool::Mempool;
use miraset_chain::network::{ChannelNetwork, ChannelNetworkHub};
use miraset_chain::node::{MirasetNode, NodeConfig, init_genesis};
use miraset_chain::storage::MemKVStore;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(20);

struct FailingSigner(LocalKeySigner);

impl ValidatorSigner for FailingSigner {
    fn public_key(&self) -> VerifyingKey {
        self.0.public_key()
    }

    fn sign(&self, request: &SignRequest) -> Result<SignatureBytes, SignError> {
        match request {
            SignRequest::PhaseVote { .. } => Err(SignError::Failed("hsm unreachable".into())),
            _ => self.0.sign(request),
        }
    }
}

#[test]
fn test_signer_failure_halts_the_node() {
    let keypair = SigningKey::from_bytes(&[62u8; 32]);
    let vk = keypair.verifying_key();
    let store = MemKVStore::new();
    init_genesis(store.clone(), &[(vk, 100)], &[("alice", 1_000)]);

    let halt = SafetyHalt::new();
    halt.install_consensus_panic_hook();
    let mut config =
        NodeConfig::devnet_with_signer(Arc::new(FailingSigner(LocalKeySigner::new(keypair))));
    config.safety_halt = halt.clone();
    let network = ChannelNetwork::new(vk, ChannelNetworkHub::new());
    let node = MirasetNode::start(config, store, network, Mempool::new());

    let deadline = Instant::now() + TIMEOUT;
    while !halt.is_halted() {
        assert!(
            Instant::now() < deadline,
            "signer failure did not halt the node"
        );
        thread::sleep(Duration::from_millis(25));
    }
    let reason = halt.reason().unwrap();
    assert!(reason.contains("hs-algorithm"), "{reason}");
    assert!(reason.contains("SignerFailed"), "{reason}");

    // The algorithm thread is dead; `Replica::drop` would re-panic joining it.
    std::mem::forget(node);
}
