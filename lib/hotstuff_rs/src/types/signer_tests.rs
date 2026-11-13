//! MIRASET PATCH #9 tests: signing bytes and signatures are identical to upstream.

use super::*;
use crate::hotstuff::messages::PhaseVote;
use crate::pacemaker::messages::PacemakerMessage;
use crate::types::crypto_primitives::Keypair;
use ed25519_dalek::Verifier;
use std::sync::Arc;

const CHAIN: ChainID = ChainID::new(1337);

fn key() -> SigningKey {
    SigningKey::from_bytes(&[3u8; 32])
}

fn requests() -> Vec<(SignRequest, Vec<u8>)> {
    let view = ViewNumber::new(42);
    let block = CryptoHash::new([7u8; 32]);
    let height = BlockHeight::new(9);
    vec![
        (
            SignRequest::PhaseVote {
                chain_id: CHAIN,
                view,
                block,
                phase: Phase::Precommit,
                justify_view: ViewNumber::new(41),
                locked_view: ViewNumber::new(40),
            },
            (CHAIN, view, block, Phase::Precommit).try_to_vec().unwrap(),
        ),
        (
            SignRequest::TimeoutVote {
                chain_id: CHAIN,
                view,
            },
            (CHAIN, view).try_to_vec().unwrap(),
        ),
        (
            SignRequest::AdvertiseBlock {
                chain_id: CHAIN,
                highest_committed_block_height: height,
            },
            (CHAIN, height).try_to_vec().unwrap(),
        ),
    ]
}

#[test]
fn test_signing_bytes_match_upstream_encodings() {
    for (request, upstream) in requests() {
        assert_eq!(request.signing_bytes(), upstream);
    }
}

#[test]
fn test_local_signer_signatures_equal_upstream_raw_signatures() {
    let signer = LocalKeySigner::new(key());
    for (request, upstream) in requests() {
        let ours = signer.sign(&request).unwrap();
        let theirs = key().sign(&upstream).to_bytes();
        assert_eq!(ours.bytes(), theirs);
        let signature = ed25519_dalek::Signature::from_bytes(&ours.bytes());
        assert!(signer.public_key().verify(&upstream, &signature).is_ok());
    }
}

struct Refusing;

impl ValidatorSigner for Refusing {
    fn public_key(&self) -> VerifyingKey {
        key().verifying_key()
    }
    fn sign(&self, _: &SignRequest) -> Result<SignatureBytes, SignError> {
        Err(SignError::Refused("test".into()))
    }
}

#[test]
fn test_message_constructors_propagate_signer_errors() {
    let keypair = Keypair::from_signer(Arc::new(Refusing));
    let vote = PhaseVote::new(
        &keypair,
        CHAIN,
        ViewNumber::new(1),
        CryptoHash::new([0u8; 32]),
        Phase::Generic,
        ViewNumber::new(0),
        ViewNumber::new(0),
    );
    assert!(matches!(vote, Err(SignError::Refused(_))));
    let timeout = PacemakerMessage::timeout_vote(&keypair, CHAIN, ViewNumber::new(1), None);
    assert!(matches!(timeout, Err(SignError::Refused(_))));
}

#[test]
fn test_phase_vote_built_through_signer_verifies() {
    let keypair = Keypair::new(key());
    let vote = PhaseVote::new(
        &keypair,
        CHAIN,
        ViewNumber::new(5),
        CryptoHash::new([1u8; 32]),
        Phase::Generic,
        ViewNumber::new(4),
        ViewNumber::new(3),
    )
    .unwrap();
    let signature = ed25519_dalek::Signature::from_bytes(&vote.signature.bytes());
    let bytes = (
        CHAIN,
        ViewNumber::new(5),
        CryptoHash::new([1u8; 32]),
        Phase::Generic,
    )
        .try_to_vec()
        .unwrap();
    assert!(key().verifying_key().verify(&bytes, &signature).is_ok());
}

/// MIRASET PATCH #11: the lock context is not signed, so votes stay byte-identical to upstream.
#[test]
fn test_phase_vote_lock_context_is_not_signed() {
    let request = |justify_view, locked_view| SignRequest::PhaseVote {
        chain_id: CHAIN,
        view: ViewNumber::new(9),
        block: CryptoHash::new([2u8; 32]),
        phase: Phase::Generic,
        justify_view: ViewNumber::new(justify_view),
        locked_view: ViewNumber::new(locked_view),
    };
    assert_eq!(request(8, 7).signing_bytes(), request(0, 0).signing_bytes());
    let signer = LocalKeySigner::new(key());
    assert_eq!(
        signer.sign(&request(8, 7)).unwrap().bytes(),
        signer.sign(&request(1, 0)).unwrap().bytes()
    );
}

/// TODO 8.4.1: the transport attestation has a fixed golden encoding, and its length differs from
/// every consensus signature's, so no attestation can ever be the same signed message.
#[test]
fn test_transport_identity_signing_bytes_are_domain_separated() {
    let request = SignRequest::TransportIdentity {
        chain_id: CHAIN,
        noise_static_pubkey: [0xab; 32],
        not_after: 1_800_000_000,
    };
    let bytes = request.signing_bytes();
    let mut golden = b"MIRASET:P2P-ID:v1".to_vec();
    golden.extend_from_slice(&1337u64.to_be_bytes());
    golden.extend_from_slice(&[0xab; 32]);
    golden.extend_from_slice(&1_800_000_000u64.to_be_bytes());
    assert_eq!(bytes, golden);
    assert_eq!(bytes.len(), 65);
    for (other, _) in requests() {
        assert_ne!(other.signing_bytes().len(), bytes.len());
    }
    assert!(request.chain_id() == CHAIN);
}
