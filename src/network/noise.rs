//! Authenticated validator sessions: Noise `XX` plus a validator-key attestation (TODO 8.4.1,
//! `docs/adr/proposal-transport-identity.md` T1–T5, approved 2026-11-12).
//!
//! - **T1.** `Noise_XX_25519_ChaChaPoly_SHA256`; the prologue binds chain ID and genesis hash, so
//!   a peer of another network fails the handshake before any consensus message is read.
//! - **T2.** Each side's handshake payload is `validator key ‖ not_after ‖ signature`, the
//!   signature being the validator key's `TransportIdentity` attestation of the Noise static key
//!   Noise authenticated. The receiver accepts only a configured validator, a valid signature
//!   over *that* static key, and an unexpired, not over-long attestation.
//! - **T3.** The X25519 static key is generated per process start and never written to disk.
//! - **T5.** After the handshake a frame is `u32 BE plaintext length ‖ encrypted chunks` (each at
//!   most 65 535 bytes). The length is checked before anything is allocated; a tampered, replayed
//!   or reordered chunk fails decryption and closes the connection.

use ed25519_dalek::{Signature, Verifier};
use hotstuff_rs::types::crypto_primitives::VerifyingKey;
use hotstuff_rs::types::data_types::ChainID;
use hotstuff_rs::types::signer::{LocalKeySigner, SignRequest, ValidatorSigner};
use snow::{HandshakeState, TransportState};
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use zeroize::Zeroizing;

const NOISE_PARAMS: &str = "Noise_XX_25519_ChaChaPoly_SHA256";
const PROLOGUE_DOMAIN: &[u8] = b"MIRASET:P2P:v1";
/// Noise's maximum message size and AEAD tag.
const NOISE_MAX_MESSAGE: usize = 65_535;
const NOISE_TAG: usize = 16;
/// Plaintext bytes per encrypted transport chunk.
pub(crate) const MAX_CHUNK_PLAINTEXT: usize = NOISE_MAX_MESSAGE - NOISE_TAG;
/// Pre-authentication handshake messages are small; anything larger is refused unread.
const MAX_HANDSHAKE_MESSAGE: usize = 1024;
const KEY_BYTES: usize = 32;
const SIGNATURE_BYTES: usize = 64;
const PAYLOAD_BYTES: usize = KEY_BYTES + 8 + SIGNATURE_BYTES;
/// How long one attestation is valid, and when to re-attest.
pub(crate) const ATTESTATION_LIFETIME: Duration = Duration::from_secs(600);
const ATTESTATION_REFRESH_BEFORE: Duration = Duration::from_secs(300);
/// Minimum interval between re-attestation attempts.
const ATTESTATION_RETRY: Duration = Duration::from_secs(10);
/// Clock difference tolerated between validators (they run NTP; see `docs/devnet.md`).
const CLOCK_TOLERANCE: Duration = Duration::from_secs(60);

/// Why a connection was refused or closed. Fixed set, used as a metrics label.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rejection {
    /// The handshake did not finish within the deadline (or the peer went away).
    HandshakeTimeout,
    /// Noise failed: wrong chain/genesis prologue, malformed or oversized handshake message.
    HandshakeFailed,
    /// The attestation is malformed, expired, too long-lived, or its signature does not cover
    /// the Noise static key that was authenticated.
    BadAttestation,
    /// The attested key is not a configured validator (or not the one we dialled).
    UnknownValidator,
    /// A frame length above the configured maximum.
    FrameTooLarge,
    /// An encrypted chunk failed authentication (tampered, replayed or reordered).
    DecryptFailed,
    /// No free pre-authentication slot.
    SlotsFull,
}

impl Rejection {
    pub const ALL: [Rejection; 7] = [
        Rejection::HandshakeTimeout,
        Rejection::HandshakeFailed,
        Rejection::BadAttestation,
        Rejection::UnknownValidator,
        Rejection::FrameTooLarge,
        Rejection::DecryptFailed,
        Rejection::SlotsFull,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Rejection::HandshakeTimeout => "handshake_timeout",
            Rejection::HandshakeFailed => "handshake_failed",
            Rejection::BadAttestation => "bad_attestation",
            Rejection::UnknownValidator => "unknown_validator",
            Rejection::FrameTooLarge => "frame_too_large",
            Rejection::DecryptFailed => "decrypt_failed",
            Rejection::SlotsFull => "slots_full",
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// Rejection counters by reason. Process-local.
#[derive(Debug, Default)]
pub struct TransportRejections {
    counts: [AtomicU64; Rejection::ALL.len()],
}

impl TransportRejections {
    pub(crate) fn record(&self, reason: Rejection) {
        self.counts[reason.index()].fetch_add(1, Ordering::Relaxed);
    }

    pub fn get(&self, reason: Rejection) -> u64 {
        self.counts[reason.index()].load(Ordering::Relaxed)
    }
}

/// A refusal with its reason; converted to `io::Error` at the connection boundary.
#[derive(Debug)]
pub(crate) struct Refused(pub Rejection);

pub(crate) fn refused(reason: Rejection) -> io::Error {
    io::Error::other(RefusedError(reason))
}

#[derive(Debug)]
struct RefusedError(Rejection);

impl std::fmt::Display for RefusedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "transport refused: {}", self.0.as_str())
    }
}

impl std::error::Error for RefusedError {}

/// The rejection reason inside an `io::Error`, if it carries one.
pub(crate) fn rejection_of(error: &io::Error) -> Option<Rejection> {
    error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<RefusedError>())
        .map(|RefusedError(reason)| *reason)
}

#[derive(Clone, Copy)]
struct Attestation {
    not_after: u64,
    signature: [u8; SIGNATURE_BYTES],
}

/// Background re-attestation: at most one in flight, retried no more often than
/// `ATTESTATION_RETRY` (so a failing or slow signer is never called per connection).
#[derive(Default)]
struct Refresh {
    in_progress: AtomicBool,
    last_attempt: AtomicU64,
}

/// This node's transport identity: per-start Noise keypair, attested by the validator signer.
pub struct TransportIdentity {
    signer: Arc<dyn ValidatorSigner>,
    validator: VerifyingKey,
    chain_id: u64,
    prologue: Vec<u8>,
    static_private: Zeroizing<Vec<u8>>,
    static_public: [u8; KEY_BYTES],
    attestation: Arc<Mutex<Attestation>>,
    refresh: Arc<Refresh>,
}

impl TransportIdentity {
    /// Generate the per-start Noise key and obtain the first attestation (fails fast if the
    /// signer refuses or is unreachable).
    pub fn new(
        signer: Arc<dyn ValidatorSigner>,
        chain_id: u64,
        genesis_hash: [u8; 32],
    ) -> Result<Arc<Self>, String> {
        let builder = snow::Builder::new(noise_params());
        let keypair = builder
            .generate_keypair()
            .map_err(|e| format!("noise keypair: {e}"))?;
        let static_public: [u8; KEY_BYTES] = keypair
            .public
            .as_slice()
            .try_into()
            .map_err(|_| "noise static key is not 32 bytes".to_string())?;
        let mut prologue = PROLOGUE_DOMAIN.to_vec();
        prologue.extend_from_slice(&chain_id.to_be_bytes());
        prologue.extend_from_slice(&genesis_hash);
        let validator = signer.public_key();
        let attestation = attest(signer.as_ref(), chain_id, &static_public, unix_now())?;
        Ok(Arc::new(Self {
            signer,
            validator,
            chain_id,
            prologue,
            static_private: Zeroizing::new(keypair.private),
            static_public,
            attestation: Arc::new(Mutex::new(attestation)),
            refresh: Arc::default(),
        }))
    }

    /// Convenience for tests and single-process setups: an in-process key.
    pub fn for_key(
        key: ed25519_dalek::SigningKey,
        chain_id: u64,
        genesis_hash: [u8; 32],
    ) -> Result<Arc<Self>, String> {
        Self::new(Arc::new(LocalKeySigner::new(key)), chain_id, genesis_hash)
    }

    pub fn validator(&self) -> VerifyingKey {
        self.validator
    }

    /// Handshake payload. Never waits for the signer: when the current attestation is close to
    /// expiry, a background refresh starts and the current one is used meanwhile.
    fn payload(&self) -> io::Result<Vec<u8>> {
        let now = unix_now();
        let current = *self
            .attestation
            .lock()
            .map_err(|_| io::Error::other("attestation lock poisoned"))?;
        if current.not_after.saturating_sub(now) < ATTESTATION_REFRESH_BEFORE.as_secs() {
            self.start_refresh(now);
        }
        let mut payload = Vec::with_capacity(PAYLOAD_BYTES);
        payload.extend_from_slice(&self.validator.to_bytes());
        payload.extend_from_slice(&current.not_after.to_be_bytes());
        payload.extend_from_slice(&current.signature);
        Ok(payload)
    }

    /// Re-attest on a background thread, signing outside the attestation lock.
    fn start_refresh(&self, now: u64) {
        let last = self.refresh.last_attempt.load(Ordering::Acquire);
        if now.saturating_sub(last) < ATTESTATION_RETRY.as_secs() && last != 0 {
            return;
        }
        if self.refresh.in_progress.swap(true, Ordering::AcqRel) {
            return;
        }
        self.refresh
            .last_attempt
            .store(now.max(1), Ordering::Release);
        let signer = Arc::clone(&self.signer);
        let attestation = Arc::clone(&self.attestation);
        let refresh = Arc::clone(&self.refresh);
        let (chain_id, static_public) = (self.chain_id, self.static_public);
        let spawned = std::thread::Builder::new()
            .name("transport-attest".to_string())
            .spawn(move || {
                match attest(signer.as_ref(), chain_id, &static_public, unix_now()) {
                    Ok(fresh) => {
                        if let Ok(mut current) = attestation.lock() {
                            *current = fresh;
                        }
                    }
                    Err(error) => tracing::warn!(%error, "transport re-attestation failed"),
                }
                refresh.in_progress.store(false, Ordering::Release);
            });
        if spawned.is_err() {
            self.refresh.in_progress.store(false, Ordering::Release);
        }
    }

    #[cfg(test)]
    fn expire_soon(&self, not_after: u64) {
        self.attestation.lock().unwrap().not_after = not_after;
    }

    #[cfg(test)]
    fn current_not_after(&self) -> u64 {
        self.attestation.lock().unwrap().not_after
    }

    fn builder(&self) -> io::Result<snow::Builder<'_>> {
        snow::Builder::new(noise_params())
            .local_private_key(&self.static_private)
            .and_then(|b| b.prologue(&self.prologue))
            .map_err(|_| refused(Rejection::HandshakeFailed))
    }

    /// Dial side. Returns the authenticated session; the responder must be `expected`.
    pub(crate) fn initiate(
        &self,
        stream: &mut TcpStream,
        peers: &HashMap<VerifyingKey, String>,
        expected: &VerifyingKey,
        deadline: Instant,
    ) -> io::Result<Session> {
        let mut hs = self
            .builder()?
            .build_initiator()
            .map_err(|_| refused(Rejection::HandshakeFailed))?;
        send_handshake(stream, &mut hs, &[])?;
        let payload = receive_handshake(stream, &mut hs, deadline)?;
        let peer = self.verify(&hs, &payload, peers)?;
        if &peer != expected {
            return Err(refused(Rejection::UnknownValidator));
        }
        send_handshake(stream, &mut hs, &self.payload()?)?;
        Session::from(hs, peer)
    }

    /// Accept side. Returns the authenticated session and the proven validator key.
    pub(crate) fn respond(
        &self,
        stream: &mut TcpStream,
        peers: &HashMap<VerifyingKey, String>,
        deadline: Instant,
    ) -> io::Result<Session> {
        let mut hs = self
            .builder()?
            .build_responder()
            .map_err(|_| refused(Rejection::HandshakeFailed))?;
        receive_handshake(stream, &mut hs, deadline)?;
        send_handshake(stream, &mut hs, &self.payload()?)?;
        let payload = receive_handshake(stream, &mut hs, deadline)?;
        let peer = self.verify(&hs, &payload, peers)?;
        Session::from(hs, peer)
    }

    fn verify(
        &self,
        hs: &HandshakeState,
        payload: &[u8],
        peers: &HashMap<VerifyingKey, String>,
    ) -> io::Result<VerifyingKey> {
        let remote_static: [u8; KEY_BYTES] = hs
            .get_remote_static()
            .and_then(|key| key.try_into().ok())
            .ok_or_else(|| refused(Rejection::HandshakeFailed))?;
        verify_attestation(payload, &remote_static, self.chain_id, peers, unix_now())
            .map_err(|Refused(reason)| refused(reason))
    }
}

fn noise_params() -> snow::params::NoiseParams {
    NOISE_PARAMS.parse().expect("valid noise parameter string")
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn attest(
    signer: &dyn ValidatorSigner,
    chain_id: u64,
    static_public: &[u8; KEY_BYTES],
    now: u64,
) -> Result<Attestation, String> {
    let not_after = now.saturating_add(ATTESTATION_LIFETIME.as_secs());
    let signature = signer
        .sign(&SignRequest::TransportIdentity {
            chain_id: ChainID::new(chain_id),
            noise_static_pubkey: *static_public,
            not_after,
        })
        .map_err(|e| format!("transport attestation: {e}"))?;
    tracing::info!(not_after, "transport identity attested");
    Ok(Attestation {
        not_after,
        signature: signature.bytes(),
    })
}

/// Check a peer's handshake payload against the Noise static key it authenticated.
pub(crate) fn verify_attestation(
    payload: &[u8],
    remote_static: &[u8; KEY_BYTES],
    chain_id: u64,
    peers: &HashMap<VerifyingKey, String>,
    now: u64,
) -> Result<VerifyingKey, Refused> {
    if payload.len() != PAYLOAD_BYTES {
        return Err(Refused(Rejection::BadAttestation));
    }
    let (key_bytes, rest) = payload.split_at(KEY_BYTES);
    let (not_after_bytes, signature_bytes) = rest.split_at(8);
    let validator = VerifyingKey::from_bytes(key_bytes.try_into().expect("32 bytes"))
        .map_err(|_| Refused(Rejection::BadAttestation))?;
    if !peers.contains_key(&validator) {
        return Err(Refused(Rejection::UnknownValidator));
    }
    let not_after = u64::from_be_bytes(not_after_bytes.try_into().expect("8 bytes"));
    let longest = now
        .saturating_add(ATTESTATION_LIFETIME.as_secs())
        .saturating_add(CLOCK_TOLERANCE.as_secs());
    if not_after.saturating_add(CLOCK_TOLERANCE.as_secs()) < now || not_after > longest {
        return Err(Refused(Rejection::BadAttestation));
    }
    let signed = SignRequest::TransportIdentity {
        chain_id: ChainID::new(chain_id),
        noise_static_pubkey: *remote_static,
        not_after,
    }
    .signing_bytes();
    let signature = Signature::from_bytes(signature_bytes.try_into().expect("64 bytes"));
    validator
        .verify(&signed, &signature)
        .map_err(|_| Refused(Rejection::BadAttestation))?;
    Ok(validator)
}

fn send_handshake(
    stream: &mut TcpStream,
    hs: &mut HandshakeState,
    payload: &[u8],
) -> io::Result<()> {
    let mut message = [0u8; MAX_HANDSHAKE_MESSAGE];
    let len = hs
        .write_message(payload, &mut message)
        .map_err(|_| refused(Rejection::HandshakeFailed))?;
    let mut framed = (len as u16).to_be_bytes().to_vec();
    framed.extend_from_slice(&message[..len]);
    stream.write_all(&framed)
}

fn receive_handshake(
    stream: &mut TcpStream,
    hs: &mut HandshakeState,
    deadline: Instant,
) -> io::Result<Vec<u8>> {
    let mut len_bytes = [0u8; 2];
    read_exact_by(stream, &mut len_bytes, deadline)?;
    let len = u16::from_be_bytes(len_bytes) as usize;
    if len > MAX_HANDSHAKE_MESSAGE {
        return Err(refused(Rejection::HandshakeFailed));
    }
    let mut message = vec![0u8; len];
    read_exact_by(stream, &mut message, deadline)?;
    let mut payload = [0u8; MAX_HANDSHAKE_MESSAGE];
    let n = hs
        .read_message(&message, &mut payload)
        .map_err(|_| refused(Rejection::HandshakeFailed))?;
    Ok(payload[..n].to_vec())
}

/// `read_exact` that gives up at `deadline` (handshakes run before any peer is trusted).
fn read_exact_by(stream: &mut TcpStream, buf: &mut [u8], deadline: Instant) -> io::Result<()> {
    let mut filled = 0;
    while filled < buf.len() {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(refused(Rejection::HandshakeTimeout));
        }
        stream.set_read_timeout(Some(left))?;
        match stream.read(&mut buf[filled..]) {
            Ok(0) => return Err(refused(Rejection::HandshakeTimeout)),
            Ok(n) => filled += n,
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock
                        | io::ErrorKind::TimedOut
                        | io::ErrorKind::Interrupted
                ) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// An authenticated Noise session with a known validator.
pub(crate) struct Session {
    transport: TransportState,
    peer: VerifyingKey,
}

impl Session {
    fn from(hs: HandshakeState, peer: VerifyingKey) -> io::Result<Self> {
        let transport = hs
            .into_transport_mode()
            .map_err(|_| refused(Rejection::HandshakeFailed))?;
        Ok(Self { transport, peer })
    }

    pub(crate) fn peer(&self) -> VerifyingKey {
        self.peer
    }

    /// Encrypt `plaintext` as one frame: `u32 BE length ‖ chunks`.
    pub(crate) fn seal_frame(&mut self, plaintext: &[u8]) -> io::Result<Vec<u8>> {
        let length = u32::try_from(plaintext.len())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "frame length overflow"))?;
        let chunks = plaintext.len().div_ceil(MAX_CHUNK_PLAINTEXT);
        let mut frame = Vec::with_capacity(4 + plaintext.len() + chunks * NOISE_TAG);
        frame.extend_from_slice(&length.to_be_bytes());
        let mut sealed = vec![0u8; NOISE_MAX_MESSAGE];
        for chunk in plaintext.chunks(MAX_CHUNK_PLAINTEXT) {
            let n = self
                .transport
                .write_message(chunk, &mut sealed)
                .map_err(|_| io::Error::other("noise encrypt failed"))?;
            frame.extend_from_slice(&sealed[..n]);
        }
        Ok(frame)
    }

    /// Decrypt one chunk (`plaintext_len` + tag bytes) into `out`.
    pub(crate) fn open_chunk(&mut self, sealed: &[u8], out: &mut [u8]) -> io::Result<()> {
        let n = self
            .transport
            .read_message(sealed, out)
            .map_err(|_| refused(Rejection::DecryptFailed))?;
        if n != out.len() {
            return Err(refused(Rejection::DecryptFailed));
        }
        Ok(())
    }

    pub(crate) const TAG: usize = NOISE_TAG;
}

#[cfg(test)]
#[path = "noise_tests.rs"]
mod tests;
