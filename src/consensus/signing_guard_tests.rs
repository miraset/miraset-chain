//! T1 (`docs/adr/proposal-vote-safety-state.md` §5): signing guard rules.

use super::*;
use borsh::BorshDeserialize;
use hotstuff_rs::types::data_types::{ChainID, CryptoHash, SignatureBytes, ViewNumber};
use tempfile::TempDir;

const CHAIN: u64 = 1337;

fn identity() -> GuardIdentity {
    GuardIdentity {
        chain_id: CHAIN,
        validator: [1u8; 32],
        genesis_hash: [2u8; 32],
    }
}

pub(crate) fn vote(chain: u64, view: u64, block: u8, phase: Phase) -> PhaseVote {
    PhaseVote {
        chain_id: ChainID::new(chain),
        view: ViewNumber::new(view),
        block: CryptoHash::new([block; 32]),
        phase,
        signature: SignatureBytes::try_from_slice(&[9u8; 64]).expect("64-byte signature"),
    }
}

fn open(dir: &TempDir, mode: OpenMode) -> Result<SigningGuard, SigningGuardError> {
    SigningGuard::open(&dir.path().join("guard.redb"), identity(), mode)
}

#[test]
fn test_guard_allows_increasing_views_and_identical_resend() {
    let dir = TempDir::new().unwrap();
    let guard = open(&dir, OpenMode::CreateIfMissing).unwrap();

    let first = vote(CHAIN, 5, 0xaa, Phase::Generic);
    assert_eq!(guard.authorize(&first).unwrap(), Authorization::Fresh);
    assert_eq!(
        guard.authorize(&first).unwrap(),
        Authorization::IdenticalResend
    );
    assert_eq!(
        guard
            .authorize(&vote(CHAIN, 7, 0xbb, Phase::Prepare))
            .unwrap(),
        Authorization::Fresh
    );
    assert_eq!(guard.last_vote().unwrap().map(|r| r.view), Some(7));
}

#[test]
fn test_guard_drops_stale_and_refuses_conflicting_votes() {
    let dir = TempDir::new().unwrap();
    let guard = open(&dir, OpenMode::CreateIfMissing).unwrap();
    guard
        .authorize(&vote(CHAIN, 10, 0xaa, Phase::Generic))
        .unwrap();

    assert_eq!(
        guard
            .authorize(&vote(CHAIN, 9, 0xcc, Phase::Generic))
            .unwrap(),
        Authorization::Stale { last_view: 10 }
    );
    for conflicting in [
        vote(CHAIN, 10, 0xbb, Phase::Generic),
        vote(CHAIN, 10, 0xaa, Phase::Precommit),
    ] {
        assert_eq!(
            guard.authorize(&conflicting).unwrap(),
            Authorization::Conflicting { view: 10 }
        );
    }
    // Refusals never move the record.
    assert_eq!(
        guard.last_vote().unwrap().map(|r| r.block),
        Some([0xaa; 32])
    );
}

#[test]
fn test_guard_record_survives_reopen() {
    let dir = TempDir::new().unwrap();
    {
        let guard = open(&dir, OpenMode::CreateIfMissing).unwrap();
        guard
            .authorize(&vote(CHAIN, 42, 0xaa, Phase::Generic))
            .unwrap();
    }
    let reopened = open(&dir, OpenMode::RequireExisting).unwrap();
    assert_eq!(
        reopened.last_vote().unwrap(),
        Some(VoteRecord {
            view: 42,
            block: [0xaa; 32],
            phase: 0
        })
    );
    assert_eq!(
        reopened
            .authorize(&vote(CHAIN, 42, 0xbb, Phase::Generic))
            .unwrap(),
        Authorization::Conflicting { view: 42 }
    );
}

#[test]
fn test_guard_refuses_missing_file_unless_initializing() {
    let dir = TempDir::new().unwrap();
    assert!(matches!(
        open(&dir, OpenMode::RequireExisting),
        Err(SigningGuardError::Missing(_))
    ));
    assert!(open(&dir, OpenMode::CreateIfMissing).is_ok());
}

#[test]
fn test_guard_refuses_other_identity_and_chain() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("guard.redb");
    let guard = SigningGuard::open(&path, identity(), OpenMode::CreateIfMissing).unwrap();
    assert!(matches!(
        guard.authorize(&vote(CHAIN + 1, 1, 0xaa, Phase::Generic)),
        Err(SigningGuardError::WrongChain { .. })
    ));
    drop(guard);

    let mutations: [fn(&mut GuardIdentity); 3] = [
        |id| id.chain_id += 1,
        |id| id.validator = [7u8; 32],
        |id| id.genesis_hash = [7u8; 32],
    ];
    for mutate in mutations {
        let mut other = identity();
        mutate(&mut other);
        assert!(matches!(
            SigningGuard::open(&path, other, OpenMode::RequireExisting),
            Err(SigningGuardError::IdentityMismatch)
        ));
    }
}

#[test]
fn test_guard_rejects_second_open_in_use() {
    let dir = TempDir::new().unwrap();
    let _first = open(&dir, OpenMode::CreateIfMissing).unwrap();
    assert!(matches!(
        open(&dir, OpenMode::RequireExisting),
        Err(SigningGuardError::InUse)
    ));
}

#[test]
fn test_guard_rejects_corrupt_records() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("guard.redb");
    drop(SigningGuard::open(&path, identity(), OpenMode::CreateIfMissing).unwrap());

    let db = Database::create(&path).unwrap();
    write_record(&db, LAST_VOTE_KEY, &[FORMAT_VERSION, 1, 2, 3]).unwrap();
    drop(db);
    assert!(matches!(
        SigningGuard::open(&path, identity(), OpenMode::RequireExisting),
        Err(SigningGuardError::Corrupt(_))
    ));

    let db = Database::create(&path).unwrap();
    let mut bad_phase = VoteRecord {
        view: 1,
        block: [0; 32],
        phase: 0,
    }
    .encode();
    bad_phase[VOTE_RECORD_LEN - 1] = MAX_PHASE_CODE + 1;
    write_record(&db, LAST_VOTE_KEY, &bad_phase).unwrap();
    drop(db);
    assert!(matches!(
        SigningGuard::open(&path, identity(), OpenMode::RequireExisting),
        Err(SigningGuardError::Corrupt(_))
    ));
}

#[test]
fn test_guard_errors_contain_no_key_material() {
    let dir = TempDir::new().unwrap();
    let guard = open(&dir, OpenMode::CreateIfMissing).unwrap();
    let wrong_chain = guard
        .authorize(&vote(CHAIN + 1, 3, 0xbb, Phase::Generic))
        .unwrap_err()
        .to_string();
    drop(guard);
    let mut other = identity();
    other.validator = [7u8; 32];
    let mismatch = SigningGuard::open(
        &dir.path().join("guard.redb"),
        other,
        OpenMode::RequireExisting,
    )
    .err()
    .expect("identity mismatch")
    .to_string();
    let validator_hex = hex::encode([1u8; 32]);
    let other_hex = hex::encode([7u8; 32]);
    let signature_hex = hex::encode([9u8; 64]);
    for err in [wrong_chain, mismatch] {
        for secret_like in [&validator_hex, &other_hex, &signature_hex] {
            assert!(!err.contains(secret_like.as_str()), "{err}");
        }
    }
}

#[test]
fn test_startup_requires_existing_guard_once_chain_has_data() {
    assert_eq!(
        OpenMode::for_startup(false, false),
        OpenMode::CreateIfMissing
    );
    assert_eq!(
        OpenMode::for_startup(false, true),
        OpenMode::CreateIfMissing
    );
    assert_eq!(
        OpenMode::for_startup(true, false),
        OpenMode::RequireExisting
    );
    assert_eq!(OpenMode::for_startup(true, true), OpenMode::CreateIfMissing);
}

#[test]
fn test_identical_resend_after_reopen_is_not_a_conflict() {
    let dir = TempDir::new().unwrap();
    let signed = vote(CHAIN, 8, 0xaa, Phase::Prepare);
    open(&dir, OpenMode::CreateIfMissing)
        .unwrap()
        .authorize(&signed)
        .unwrap();
    let reopened = open(&dir, OpenMode::RequireExisting).unwrap();
    assert_eq!(
        reopened.authorize(&signed).unwrap(),
        Authorization::IdenticalResend
    );
}
