/*
    MIRASET PATCH #9 (not upstream): purpose-bound validator signing.
    Licensed under the Apache License, Version 2.0, like the rest of this crate.
*/

//! Purpose-bound signing for validator messages (MIRASET TODO 8.3.4, key custody D1).
//!
//! Upstream signed raw bytes with an in-process [`SigningKey`]. Every signature the replica
//! produces now goes through a [`ValidatorSigner`] as a typed [`SignRequest`], so a signer can
//! see *what* it signs: it can apply slashing protection before a phase-vote signature exists,
//! and it never signs arbitrary bytes for the node.
//!
//! [`SignRequest::signing_bytes`] reproduces the exact borsh encodings upstream signed, so
//! signatures and the wire format are unchanged.

use std::fmt;

use borsh::BorshSerialize;
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};

use crate::hotstuff::types::Phase;
use crate::types::data_types::{BlockHeight, ChainID, CryptoHash, SignatureBytes, ViewNumber};

/// Everything a replica signs.
#[derive(Clone, PartialEq, Eq)]
pub enum SignRequest {
    /// A HotStuff phase vote: the safety-critical signature (at most one per view).
    ///
    /// `justify_view` and `locked_view` are **context, not signed content** (MIRASET TODO 8.5.2):
    /// the view of the certificate the vote builds on (the proposed block's `justify`, or the
    /// nudge's `justify`) and the replica's locked certificate view after processing it. A signer
    /// can keep its own durable lock floor from them and refuse a vote whose `justify_view` is
    /// below a lock it held before, even if the chain database (and with it the replica's lock)
    /// was wiped or restored.
    PhaseVote {
        chain_id: ChainID,
        view: ViewNumber,
        block: CryptoHash,
        phase: Phase,
        justify_view: ViewNumber,
        locked_view: ViewNumber,
    },
    /// A pacemaker timeout vote. Deterministic Ed25519 makes re-signing byte-identical.
    TimeoutVote { chain_id: ChainID, view: ViewNumber },
    /// A block sync server's advertisement of its highest committed height.
    AdvertiseBlock {
        chain_id: ChainID,
        highest_committed_block_height: BlockHeight,
    },
}

impl SignRequest {
    /// The bytes that are signed, identical to upstream `hotstuff_rs` 0.4.0.
    pub fn signing_bytes(&self) -> Vec<u8> {
        let encoded = match self {
            SignRequest::PhaseVote {
                chain_id,
                view,
                block,
                phase,
                ..
            } => (*chain_id, *view, *block, *phase).try_to_vec(),
            SignRequest::TimeoutVote { chain_id, view } => (*chain_id, *view).try_to_vec(),
            SignRequest::AdvertiseBlock {
                chain_id,
                highest_committed_block_height,
            } => (*chain_id, *highest_committed_block_height).try_to_vec(),
        };
        encoded.expect("borsh encoding of fixed-size values cannot fail")
    }

    /// Chain the request is for.
    pub fn chain_id(&self) -> ChainID {
        match self {
            SignRequest::PhaseVote { chain_id, .. }
            | SignRequest::TimeoutVote { chain_id, .. }
            | SignRequest::AdvertiseBlock { chain_id, .. } => *chain_id,
        }
    }
}

/// Why a signature was not produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SignError {
    /// The signer declined on purpose (e.g. slashing protection). The replica skips the message
    /// and keeps running.
    Refused(String),
    /// The signer could not decide or sign (storage, transport, key failure). The replica
    /// treats this as fatal.
    Failed(String),
}

impl fmt::Display for SignError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SignError::Refused(reason) => write!(f, "signer refused: {reason}"),
            SignError::Failed(reason) => write!(f, "signer failed: {reason}"),
        }
    }
}

impl std::error::Error for SignError {}

/// A validator's signing authority.
pub trait ValidatorSigner: Send + Sync {
    /// The validator's public key. Must not change for the lifetime of the signer.
    fn public_key(&self) -> VerifyingKey;

    /// Sign `request`, or explain why not.
    fn sign(&self, request: &SignRequest) -> Result<SignatureBytes, SignError>;
}

/// In-process signer holding the secret key (`ed25519-dalek` zeroizes it on drop).
pub struct LocalKeySigner(SigningKey);

impl LocalKeySigner {
    pub fn new(key: SigningKey) -> Self {
        Self(key)
    }
}

impl ValidatorSigner for LocalKeySigner {
    fn public_key(&self) -> VerifyingKey {
        self.0.verifying_key()
    }

    fn sign(&self, request: &SignRequest) -> Result<SignatureBytes, SignError> {
        Ok(SignatureBytes::new(
            self.0.sign(&request.signing_bytes()).to_bytes(),
        ))
    }
}

#[cfg(test)]
#[path = "signer_tests.rs"]
mod tests;
