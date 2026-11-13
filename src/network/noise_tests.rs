//! Unit tests for the Noise session layer (TODO 8.4.1, proposal §6).

use super::*;
use ed25519_dalek::SigningKey;
use std::net::TcpListener;
use std::thread;

const CHAIN: u64 = 1337;
const GENESIS: [u8; 32] = [4u8; 32];

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn identity(seed: u8, genesis: [u8; 32]) -> Arc<TransportIdentity> {
    TransportIdentity::for_key(key(seed), CHAIN, genesis).unwrap()
}

fn peers(seeds: &[u8]) -> HashMap<VerifyingKey, String> {
    seeds
        .iter()
        .map(|s| (key(*s).verifying_key(), String::new()))
        .collect()
}

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(2)
}

/// Run a handshake: `dialer` initiates expecting `expected`; `listener` responds.
fn handshake(
    dialer: Arc<TransportIdentity>,
    dialer_peers: HashMap<VerifyingKey, String>,
    expected: VerifyingKey,
    listener: Arc<TransportIdentity>,
    listener_peers: HashMap<VerifyingKey, String>,
) -> (
    io::Result<Session>,
    io::Result<Session>,
    TcpStream,
    TcpStream,
) {
    let socket = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = socket.local_addr().unwrap();
    let responder = thread::spawn(move || {
        let (mut stream, _) = socket.accept().unwrap();
        let session = listener.respond(&mut stream, &listener_peers, deadline());
        (session, stream)
    });
    let mut stream = TcpStream::connect(addr).unwrap();
    let initiated = dialer.initiate(&mut stream, &dialer_peers, &expected, deadline());
    if initiated.is_err() {
        // Let the responder see the close instead of waiting for its deadline.
        let _ = stream.shutdown(std::net::Shutdown::Both);
    }
    let (responded, accepted) = responder.join().unwrap();
    (initiated, responded, stream, accepted)
}

fn reason(result: &io::Result<Session>) -> Option<Rejection> {
    result.as_ref().err().and_then(rejection_of)
}

#[test]
fn test_handshake_authenticates_both_validators_and_frames_round_trip() {
    let (a, b) = (identity(1, GENESIS), identity(2, GENESIS));
    let (initiated, responded, _s1, _s2) = handshake(
        a.clone(),
        peers(&[1, 2]),
        b.validator(),
        b.clone(),
        peers(&[1, 2]),
    );
    let (mut sender, mut receiver) = (initiated.unwrap(), responded.unwrap());
    assert_eq!(sender.peer(), b.validator());
    assert_eq!(receiver.peer(), a.validator());

    // A message spanning three chunks.
    let plaintext: Vec<u8> = (0..(2 * MAX_CHUNK_PLAINTEXT + 17))
        .map(|i| i as u8)
        .collect();
    let frame = sender.seal_frame(&plaintext).unwrap();
    let length = u32::from_be_bytes(frame[..4].try_into().unwrap()) as usize;
    assert_eq!(length, plaintext.len());
    let mut offset = 4;
    let mut opened = vec![0u8; length];
    for chunk in opened.chunks_mut(MAX_CHUNK_PLAINTEXT) {
        let sealed = &frame[offset..offset + chunk.len() + Session::TAG];
        receiver.open_chunk(sealed, chunk).unwrap();
        offset += chunk.len() + Session::TAG;
    }
    assert_eq!(offset, frame.len());
    assert_eq!(opened, plaintext);
}

#[test]
fn test_tampered_or_replayed_chunk_fails_decryption() {
    let (a, b) = (identity(1, GENESIS), identity(2, GENESIS));
    let (initiated, responded, _s1, _s2) =
        handshake(a, peers(&[1, 2]), b.validator(), b, peers(&[1, 2]));
    let (mut sender, mut receiver) = (initiated.unwrap(), responded.unwrap());
    let frame = sender.seal_frame(b"hello").unwrap();
    let mut tampered = frame[4..].to_vec();
    tampered[0] ^= 1;
    let mut out = [0u8; 5];
    let error = receiver.open_chunk(&tampered, &mut out).unwrap_err();
    assert_eq!(rejection_of(&error), Some(Rejection::DecryptFailed));

    // A fresh receiver state cannot accept the same chunk twice either.
    let (a, b) = (identity(1, GENESIS), identity(2, GENESIS));
    let (initiated, responded, _s1, _s2) =
        handshake(a, peers(&[1, 2]), b.validator(), b, peers(&[1, 2]));
    let (mut sender, mut receiver) = (initiated.unwrap(), responded.unwrap());
    let chunk = sender.seal_frame(b"hello").unwrap()[4..].to_vec();
    receiver.open_chunk(&chunk, &mut out).unwrap();
    let replay = receiver.open_chunk(&chunk, &mut out).unwrap_err();
    assert_eq!(rejection_of(&replay), Some(Rejection::DecryptFailed));
}

#[test]
fn test_other_genesis_fails_the_handshake() {
    let (a, b) = (identity(1, GENESIS), identity(2, [5u8; 32]));
    let (initiated, responded, _s1, _s2) =
        handshake(a, peers(&[1, 2]), b.validator(), b, peers(&[1, 2]));
    assert!(initiated.is_err());
    assert!(responded.is_err());
    assert_eq!(reason(&initiated), Some(Rejection::HandshakeFailed));
}

#[test]
fn test_unknown_or_unexpected_validator_is_refused() {
    // The responder is a valid Noise peer but not in the dialer's validator set.
    let (a, outsider) = (identity(1, GENESIS), identity(9, GENESIS));
    let (initiated, _, _s1, _s2) = handshake(
        a,
        peers(&[1, 2]),
        key(2).verifying_key(),
        outsider,
        peers(&[1, 2, 9]),
    );
    assert_eq!(reason(&initiated), Some(Rejection::UnknownValidator));

    // The responder is a validator, but not the one we dialled (DNS/IP redirect).
    let (a, c) = (identity(1, GENESIS), identity(3, GENESIS));
    let (initiated, _, _s1, _s2) = handshake(
        a,
        peers(&[1, 2, 3]),
        key(2).verifying_key(),
        c,
        peers(&[1, 2, 3]),
    );
    assert_eq!(reason(&initiated), Some(Rejection::UnknownValidator));

    // The dialer is not a validator for the responder.
    let (outsider, b) = (identity(9, GENESIS), identity(2, GENESIS));
    let (_, responded, _s1, _s2) = handshake(
        outsider,
        peers(&[1, 2, 9]),
        b.validator(),
        b,
        peers(&[1, 2]),
    );
    assert_eq!(reason(&responded), Some(Rejection::UnknownValidator));
}

fn payload_for(signer: &SigningKey, chain: u64, static_key: [u8; 32], not_after: u64) -> Vec<u8> {
    let signature = LocalKeySigner::new(signer.clone())
        .sign(&SignRequest::TransportIdentity {
            chain_id: ChainID::new(chain),
            noise_static_pubkey: static_key,
            not_after,
        })
        .unwrap();
    let mut payload = signer.verifying_key().to_bytes().to_vec();
    payload.extend_from_slice(&not_after.to_be_bytes());
    payload.extend_from_slice(&signature.bytes());
    payload
}

#[test]
fn test_attestation_checks() {
    let now = 1_800_000_000;
    let validators = peers(&[1, 2]);
    let static_key = [7u8; 32];
    let check = |payload: &[u8], static_key: &[u8; 32]| {
        verify_attestation(payload, static_key, CHAIN, &validators, now).map_err(|Refused(r)| r)
    };
    let fresh = payload_for(&key(1), CHAIN, static_key, now + 300);
    assert_eq!(check(&fresh, &static_key).unwrap(), key(1).verifying_key());

    let lifetime = ATTESTATION_LIFETIME.as_secs();
    let tolerance = CLOCK_TOLERANCE.as_secs();
    let cases = [
        (
            "expired",
            payload_for(&key(1), CHAIN, static_key, now - tolerance - 1),
            static_key,
        ),
        (
            "too long-lived",
            payload_for(&key(1), CHAIN, static_key, now + lifetime + tolerance + 1),
            static_key,
        ),
        (
            "another chain",
            payload_for(&key(1), CHAIN + 1, static_key, now + 300),
            static_key,
        ),
        // Replaying an attestation with a different Noise key.
        ("other noise key", fresh.clone(), [8u8; 32]),
        ("truncated", fresh[..fresh.len() - 1].to_vec(), static_key),
    ];
    for (what, payload, static_key) in cases {
        assert_eq!(
            check(&payload, &static_key),
            Err(Rejection::BadAttestation),
            "{what}"
        );
    }
    let outsider = payload_for(&key(9), CHAIN, static_key, now + 300);
    assert_eq!(
        check(&outsider, &static_key),
        Err(Rejection::UnknownValidator)
    );
}

#[test]
fn test_rejection_labels_are_fixed() {
    let labels: Vec<_> = Rejection::ALL.iter().map(|r| r.as_str()).collect();
    assert_eq!(
        labels,
        [
            "handshake_timeout",
            "handshake_failed",
            "bad_attestation",
            "unknown_validator",
            "frame_too_large",
            "decrypt_failed",
            "slots_full"
        ]
    );
    let counters = TransportRejections::default();
    counters.record(Rejection::DecryptFailed);
    assert_eq!(counters.get(Rejection::DecryptFailed), 1);
}

/// A signer that counts calls, can be slowed down, and can be made to fail.
struct SlowSigner {
    inner: LocalKeySigner,
    calls: AtomicU64,
    delay: Duration,
    fail: AtomicBool,
}

impl ValidatorSigner for SlowSigner {
    fn public_key(&self) -> VerifyingKey {
        self.inner.public_key()
    }
    fn sign(
        &self,
        request: &SignRequest,
    ) -> Result<hotstuff_rs::types::data_types::SignatureBytes, hotstuff_rs::types::signer::SignError>
    {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.calls.load(Ordering::SeqCst) > 1 {
            thread::sleep(self.delay);
            if self.fail.load(Ordering::SeqCst) {
                return Err(hotstuff_rs::types::signer::SignError::Failed("down".into()));
            }
        }
        self.inner.sign(request)
    }
}

/// Re-attestation never blocks a handshake payload, happens once, and is not retried per
/// connection when the signer fails.
#[test]
fn test_reattestation_runs_in_the_background_with_backoff() {
    let signer = Arc::new(SlowSigner {
        inner: LocalKeySigner::new(key(1)),
        calls: AtomicU64::new(0),
        delay: Duration::from_millis(500),
        fail: AtomicBool::new(false),
    });
    let identity = TransportIdentity::new(signer.clone(), CHAIN, GENESIS).unwrap();
    let soon = unix_now() + 10;
    identity.expire_soon(soon);

    let started = Instant::now();
    for _ in 0..20 {
        identity.payload().unwrap();
    }
    assert!(
        started.elapsed() < Duration::from_millis(400),
        "payload waited for the signer"
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while identity.current_not_after() == soon {
        assert!(Instant::now() < deadline, "attestation never refreshed");
        thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        signer.calls.load(Ordering::SeqCst),
        2,
        "one refresh for many handshakes"
    );

    // A failing signer: the current attestation keeps being used, and new connections do not
    // each call the signer again (retry interval).
    signer.fail.store(true, Ordering::SeqCst);
    identity.expire_soon(soon);
    identity.refresh.last_attempt.store(0, Ordering::SeqCst);
    for _ in 0..20 {
        identity.payload().unwrap();
    }
    thread::sleep(Duration::from_millis(800));
    for _ in 0..20 {
        identity.payload().unwrap();
    }
    assert_eq!(signer.calls.load(Ordering::SeqCst), 3);
    assert_eq!(
        identity.current_not_after(),
        soon,
        "failed refresh keeps the old one"
    );
}
