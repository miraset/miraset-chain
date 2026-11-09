//! Remote signer client: a `hotstuff_rs` `ValidatorSigner` backed by the gRPC signer process
//! (key custody D3). The node holds no key.
//!
//! - At connect time the signer's public key and chain are fetched and checked against what the
//!   node expects (chain id, and optionally a pinned validator key).
//! - Every returned signature is verified against that public key over the exact signing bytes;
//!   a wrong-key or corrupted signature is a failure, never forwarded.
//! - Each request has a deadline. Unreachable or slow signers are failures, which halt the node
//!   (fail closed).
//!
//! `sign` blocks on an internal runtime, so it must be called from a plain thread (as the
//! `hotstuff_rs` threads are), not from inside an async runtime. `connect` likewise.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use borsh::BorshDeserialize;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use hotstuff_rs::types::data_types::SignatureBytes;
use hotstuff_rs::types::signer::{SignError, SignRequest, ValidatorSigner};
use hyper_util::rt::TokioIo;
use tokio::net::UnixStream;
use tokio::runtime::Runtime;
use tonic::transport::{Channel, Endpoint, Uri};

use super::convert::to_wire;
use super::proto;
use super::proto::validator_signer_client::ValidatorSignerClient;

/// Deadline for one signing round trip (local socket; views are ~1s).
pub const SIGN_TIMEOUT: Duration = Duration::from_secs(2);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const SIGNATURE_BYTES: usize = 64;
const PUBLIC_KEY_BYTES: usize = 32;

/// What the node expects of the signer it connects to.
#[derive(Clone, Debug)]
pub struct RemoteSignerExpectation {
    pub chain_id: u64,
    /// When set, the signer must serve exactly this validator key.
    pub validator: Option<[u8; PUBLIC_KEY_BYTES]>,
}

struct Connection {
    runtime: Runtime,
    client: ValidatorSignerClient<Channel>,
}

pub struct RemoteSigner {
    socket: PathBuf,
    public_key: VerifyingKey,
    connection: Mutex<Connection>,
}

impl RemoteSigner {
    /// Connect over the Unix socket at `socket` and check the signer's identity.
    pub fn connect(socket: &Path, expect: &RemoteSignerExpectation) -> Result<Self, String> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| format!("cannot start signer client runtime: {e}"))?;
        let path = socket.to_path_buf();
        let mut client = runtime.block_on(async {
            let channel = Endpoint::from_static("http://signer.local")
                .connect_timeout(CONNECT_TIMEOUT)
                .timeout(SIGN_TIMEOUT)
                .connect_with_connector(tower::service_fn(move |_: Uri| {
                    let path = path.clone();
                    async move {
                        Ok::<_, std::io::Error>(TokioIo::new(UnixStream::connect(path).await?))
                    }
                }))
                .await
                .map_err(|e| format!("cannot connect to signer at {}: {e}", socket.display()))?;
            Ok::<_, String>(ValidatorSignerClient::new(channel))
        })?;
        let identity = runtime
            .block_on(client.get_public_key(proto::GetPublicKeyRequest {}))
            .map_err(|status| format!("signer identity request failed: {}", status.message()))?
            .into_inner();
        let public_key = check_identity(&identity, expect)?;
        Ok(Self {
            socket: socket.to_path_buf(),
            public_key,
            connection: Mutex::new(Connection { runtime, client }),
        })
    }

    fn call(&self, request: &SignRequest) -> Result<proto::SignResponse, SignError> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| SignError::Failed("signer client lock poisoned".into()))?;
        let Connection { runtime, client } = &mut *connection;
        runtime
            .block_on(client.sign(to_wire(request)))
            .map(tonic::Response::into_inner)
            .map_err(|status| {
                SignError::Failed(format!(
                    "remote signer at {}: {} ({:?})",
                    self.socket.display(),
                    status.message(),
                    status.code()
                ))
            })
    }
}

fn check_identity(
    identity: &proto::GetPublicKeyResponse,
    expect: &RemoteSignerExpectation,
) -> Result<VerifyingKey, String> {
    if identity.chain_id != expect.chain_id {
        return Err(format!(
            "signer is bound to chain {}, node runs chain {}",
            identity.chain_id, expect.chain_id
        ));
    }
    let bytes: [u8; PUBLIC_KEY_BYTES] = identity
        .public_key
        .as_slice()
        .try_into()
        .map_err(|_| "signer returned a malformed public key".to_string())?;
    if let Some(pinned) = expect.validator
        && pinned != bytes
    {
        return Err(format!(
            "signer serves validator {}, expected {}",
            hex::encode(bytes),
            hex::encode(pinned)
        ));
    }
    VerifyingKey::from_bytes(&bytes).map_err(|_| "signer returned an invalid public key".into())
}

/// Accept a returned signature only if it verifies for `request` under `public_key`.
pub fn verified_signature(
    public_key: &VerifyingKey,
    request: &SignRequest,
    raw: &[u8],
) -> Result<SignatureBytes, SignError> {
    let bytes: [u8; SIGNATURE_BYTES] = raw
        .try_into()
        .map_err(|_| SignError::Failed("signer returned a malformed signature".into()))?;
    public_key
        .verify(&request.signing_bytes(), &Signature::from_bytes(&bytes))
        .map_err(|_| {
            SignError::Failed("signer returned a signature that does not verify".into())
        })?;
    SignatureBytes::try_from_slice(&bytes)
        .map_err(|_| SignError::Failed("cannot decode signature".into()))
}

impl ValidatorSigner for RemoteSigner {
    fn public_key(&self) -> VerifyingKey {
        self.public_key
    }

    fn sign(&self, request: &SignRequest) -> Result<SignatureBytes, SignError> {
        match self.call(request)?.result {
            Some(proto::sign_response::Result::Signature(raw)) => {
                verified_signature(&self.public_key, request, &raw)
            }
            Some(proto::sign_response::Result::Refused(reason)) => Err(SignError::Refused(reason)),
            None => Err(SignError::Failed(
                "signer returned an empty response".into(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use hotstuff_rs::types::data_types::{ChainID, ViewNumber};

    fn identity(chain_id: u64, key: &SigningKey) -> proto::GetPublicKeyResponse {
        proto::GetPublicKeyResponse {
            public_key: key.verifying_key().to_bytes().to_vec(),
            chain_id,
        }
    }

    #[test]
    fn test_identity_checks_chain_pin_and_key_shape() {
        let key = SigningKey::from_bytes(&[5u8; 32]);
        let expect = RemoteSignerExpectation {
            chain_id: 7,
            validator: None,
        };
        assert!(check_identity(&identity(7, &key), &expect).is_ok());
        assert!(check_identity(&identity(8, &key), &expect).is_err());

        let pinned = RemoteSignerExpectation {
            chain_id: 7,
            validator: Some([9u8; 32]),
        };
        assert!(check_identity(&identity(7, &key), &pinned).is_err());

        let malformed = proto::GetPublicKeyResponse {
            public_key: vec![1, 2, 3],
            chain_id: 7,
        };
        assert!(check_identity(&malformed, &expect).is_err());
    }

    #[test]
    fn test_only_signatures_that_verify_are_accepted() {
        let key = SigningKey::from_bytes(&[5u8; 32]);
        let other = SigningKey::from_bytes(&[6u8; 32]);
        let request = SignRequest::TimeoutVote {
            chain_id: ChainID::new(7),
            view: ViewNumber::new(3),
        };
        let good = key.sign(&request.signing_bytes()).to_bytes();
        let wrong_key = other.sign(&request.signing_bytes()).to_bytes();

        assert!(verified_signature(&key.verifying_key(), &request, &good).is_ok());
        for bad in [&wrong_key[..], &good[..63], &[0u8; 64][..]] {
            assert!(matches!(
                verified_signature(&key.verifying_key(), &request, bad),
                Err(SignError::Failed(_))
            ));
        }
    }
}
