//! Unit tests for `GuardedSigner` (key custody D2).

use super::*;
use crate::consensus::signing_guard::{GuardIdentity, OpenMode};
use ed25519_dalek::{SigningKey, Verifier};
use hotstuff_rs::hotstuff::types::Phase;
use hotstuff_rs::types::data_types::{BlockHeight, ChainID, CryptoHash, ViewNumber};
use tempfile::TempDir;

const CHAIN: u64 = 1337;

fn key() -> SigningKey {
    SigningKey::from_bytes(&[21u8; 32])
}

fn guard(dir: &TempDir, validator: [u8; 32]) -> SigningGuard {
    SigningGuard::open(
        &dir.path().join("guard.redb"),
        GuardIdentity {
            chain_id: CHAIN,
            validator,
            genesis_hash: [0u8; 32],
        },
        OpenMode::CreateIfMissing,
    )
    .unwrap()
}

fn signer(dir: &TempDir) -> Arc<GuardedSigner> {
    GuardedSigner::new(key(), guard(dir, key().verifying_key().to_bytes())).unwrap()
}

fn phase_vote(chain: u64, view: u64, block: u8) -> SignRequest {
    SignRequest::PhaseVote {
        chain_id: ChainID::new(chain),
        view: ViewNumber::new(view),
        block: CryptoHash::new([block; 32]),
        phase: Phase::Generic,
    }
}

#[test]
fn test_fresh_vote_is_signed_with_the_validator_key() {
    let dir = TempDir::new().unwrap();
    let request = phase_vote(CHAIN, 4, 0xaa);
    let signature = signer(&dir).sign(&request).unwrap();
    let signature = ed25519_dalek::Signature::from_bytes(&signature.bytes());
    assert!(
        key()
            .verifying_key()
            .verify(&request.signing_bytes(), &signature)
            .is_ok()
    );
}

#[test]
fn test_guard_runs_before_signing() {
    let dir = TempDir::new().unwrap();
    let signer = signer(&dir);
    signer.sign(&phase_vote(CHAIN, 10, 0xaa)).unwrap();
    assert!(
        signer.sign(&phase_vote(CHAIN, 10, 0xaa)).is_ok(),
        "identical re-send"
    );

    assert!(matches!(
        signer.sign(&phase_vote(CHAIN, 9, 0xbb)),
        Err(SignError::Refused(_))
    ));
    assert!(matches!(
        signer.sign(&phase_vote(CHAIN, 10, 0xbb)),
        Err(SignError::Refused(_))
    ));
    assert_eq!(
        (signer.stale_refused(), signer.conflicting_refused()),
        (1, 1)
    );
}

#[test]
fn test_guard_fault_is_a_signer_failure() {
    let dir = TempDir::new().unwrap();
    let result = signer(&dir).sign(&phase_vote(CHAIN + 1, 1, 0xaa));
    assert!(matches!(result, Err(SignError::Failed(reason)) if reason.contains("chain")));
}

#[test]
fn test_non_vote_requests_bypass_the_guard() {
    let dir = TempDir::new().unwrap();
    let signer = signer(&dir);
    signer.sign(&phase_vote(CHAIN, 50, 0xaa)).unwrap();
    let timeout = SignRequest::TimeoutVote {
        chain_id: ChainID::new(CHAIN),
        view: ViewNumber::new(1),
    };
    let advertise = SignRequest::AdvertiseBlock {
        chain_id: ChainID::new(CHAIN),
        highest_committed_block_height: BlockHeight::new(3),
    };
    assert!(signer.sign(&timeout).is_ok());
    assert!(signer.sign(&advertise).is_ok());
}

#[test]
fn test_guard_for_another_key_is_rejected() {
    let dir = TempDir::new().unwrap();
    assert!(GuardedSigner::new(key(), guard(&dir, [9u8; 32])).is_err());
}
