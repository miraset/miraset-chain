/*
    Copyright © 2024, ParallelChain Lab
    Licensed under the Apache License, Version 2.0: http://www.apache.org/licenses/LICENSE-2.0
*/

//! Cryptographic primitives.
//!
//! The definitions and re-exports in this module provide two categories of cryptographic primitives:
//! 1. **Cryptographic Hashes**: provided by the [`sha2`] crate.
//! 2. **Digital Signatures**: provided by the [`ed25519_dalek`] crate.

use std::sync::Arc;

use super::data_types::SignatureBytes;
use super::signer::{LocalKeySigner, SignError, SignRequest, ValidatorSigner};

// re-exports below.
pub use sha2::Digest;
pub use sha2::Sha256 as CryptoHasher;

pub use ed25519_dalek::{Signature, SignatureError, Signer, SigningKey, Verifier, VerifyingKey};

/// MIRASET PATCH #9: the replica's signing handle. Upstream wrapped a [`SigningKey`] and signed
/// raw bytes; it now delegates every signature to a [`ValidatorSigner`] as a typed
/// [`SignRequest`]. There is no raw-bytes signing path.
#[derive(Clone)]
pub(crate) struct Keypair {
    signer: Arc<dyn ValidatorSigner>,
    public: VerifyingKey,
}

impl Keypair {
    /// A `Keypair` backed by an in-process [`LocalKeySigner`].
    pub(crate) fn new(signing_key: SigningKey) -> Keypair {
        Self::from_signer(Arc::new(LocalKeySigner::new(signing_key)))
    }

    /// A `Keypair` backed by any [`ValidatorSigner`].
    pub(crate) fn from_signer(signer: Arc<dyn ValidatorSigner>) -> Keypair {
        let public = signer.public_key();
        Keypair { signer, public }
    }

    /// Sign a typed request.
    pub(crate) fn sign(&self, request: &SignRequest) -> Result<SignatureBytes, SignError> {
        self.signer.sign(request)
    }

    /// Get the `VerifyingKey` of this `Keypair`.
    pub(crate) fn public(&self) -> VerifyingKey {
        self.public
    }
}
