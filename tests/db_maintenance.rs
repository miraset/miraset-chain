//! Offline database maintenance on a real committed chain (TODO 8.4.5): inspect, verify,
//! compact and backup refuse a database in use, never overwrite, and keep committed history.

use ed25519_dalek::SigningKey;
use hotstuff_rs::block_tree::accessors::public::BlockTreeCamera;
use hotstuff_rs::block_tree::pluggables::{KVStore, WriteBatch};
use hotstuff_rs::block_tree::variables;
use hotstuff_rs::types::data_types::BlockHeight;
use miraset_chain::core::types::BlockPayload;
use miraset_chain::mempool::Mempool;
use miraset_chain::network::{ChannelNetwork, ChannelNetworkHub};
use miraset_chain::node::{
    MirasetNode, NodeConfig, genesis_state_commitment, init_genesis_with_chain_id,
};
use miraset_chain::storage::RedbKVStore;
use miraset_chain::storage::maintenance::{self, ExpectedChain};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

const CHAIN_ID: u64 = 4242;
const MIN_HEIGHT: u64 = 4;
const COMMIT_TIMEOUT: Duration = Duration::from_secs(30);
const ACCOUNTS: [(&str, u64); 1] = [("alice", 1_000)];

/// A chain database with at least `MIN_HEIGHT` committed heights, written by a real node that
/// has since stopped.
fn committed_chain(dir: &Path) -> PathBuf {
    let path = dir.join("chain.redb");
    let key = SigningKey::from_bytes(&[33u8; 32]);
    let vk = key.verifying_key();
    let store = RedbKVStore::open(&path).expect("open chain db");
    init_genesis_with_chain_id(store.clone(), CHAIN_ID, &[(vk, 1)], &ACCOUNTS);
    let mut config = NodeConfig::devnet(key);
    config.chain_id = CHAIN_ID;
    config.max_view_time = Duration::from_millis(200);
    let node = MirasetNode::start(
        config,
        store,
        ChannelNetwork::new(vk, ChannelNetworkHub::new()),
        Mempool::new(),
    );
    let deadline = Instant::now() + COMMIT_TIMEOUT;
    while node.highest_committed_height().unwrap_or(0) < MIN_HEIGHT {
        assert!(Instant::now() < deadline, "node did not commit");
        thread::sleep(Duration::from_millis(50));
    }
    drop(node);
    path
}

fn expected() -> ExpectedChain {
    ExpectedChain {
        chain_id: CHAIN_ID,
        genesis_hash: genesis_state_commitment(&ACCOUNTS),
    }
}

#[test]
fn test_inspect_and_verify_report_the_committed_chain() {
    let dir = tempfile::tempdir().unwrap();
    let path = committed_chain(dir.path());

    let report = maintenance::inspect(&path).expect("inspect");
    assert_eq!(report.chain_id, CHAIN_ID);
    let height = report.committed_height.expect("committed height");
    assert!(height >= MIN_HEIGHT);
    assert!(report.commit_sequence > 0);

    let verified = maintenance::verify(&path, Some(expected())).expect("verify");
    assert_eq!(verified.heights_checked, height + 1);
    assert_eq!(verified.committed_block, report.committed_block);
    assert!(verified.genesis_checked);
    assert!(maintenance::verify(&path, None).is_ok());
}

#[test]
fn test_verify_rejects_another_chain_or_genesis() {
    let dir = tempfile::tempdir().unwrap();
    let path = committed_chain(dir.path());
    let wrong_chain = ExpectedChain {
        chain_id: CHAIN_ID + 1,
        ..expected()
    };
    let err = maintenance::verify(&path, Some(wrong_chain))
        .unwrap_err()
        .to_string();
    assert!(err.contains("chain_id mismatch"), "{err}");
    let wrong_genesis = ExpectedChain {
        genesis_hash: [7; 32],
        ..expected()
    };
    let err = maintenance::verify(&path, Some(wrong_genesis))
        .unwrap_err()
        .to_string();
    assert!(err.contains("genesis_hash mismatch"), "{err}");
}

/// Overwrite one key in the stopped node's database, as a corruption or a bad restore would.
fn tamper(path: &Path, key: Vec<u8>, value: &[u8]) {
    let mut store = RedbKVStore::open(path).expect("open for tampering");
    let mut batch = <RedbKVStore as KVStore>::WriteBatch::new();
    batch.set(&key, value);
    store.write(batch);
}

fn key(parts: &[&[u8]]) -> Vec<u8> {
    parts.concat()
}

fn committed_hash_at(path: &Path, height: u64) -> [u8; 32] {
    let store = RedbKVStore::open(path).unwrap();
    let camera = BlockTreeCamera::new(store.clone());
    camera
        .snapshot()
        .block_at_height(BlockHeight::new(height))
        .unwrap()
        .unwrap()
        .bytes()
}

#[test]
fn test_verify_detects_a_wrong_height_index() {
    let dir = tempfile::tempdir().unwrap();
    let path = committed_chain(dir.path());
    let later = committed_hash_at(&path, 3);
    // Height 2 now claims to be the block committed at height 3.
    tamper(
        &path,
        key(&[&variables::BLOCK_AT_HEIGHT, &2u64.to_le_bytes()]),
        &later,
    );
    let err = maintenance::verify(&path, Some(expected()))
        .unwrap_err()
        .to_string();
    assert!(err.contains("height 2:"), "{err}");
}

#[test]
fn test_verify_detects_a_replaced_block_payload() {
    let dir = tempfile::tempdir().unwrap();
    let path = committed_chain(dir.path());
    let block = committed_hash_at(&path, 1);
    // A well-formed payload that is not the one the block committed to.
    let mut other = BlockPayload::new(Vec::new(), Vec::new());
    other.state_root = [5; 32];
    tamper(
        &path,
        key(&[
            &variables::BLOCKS,
            &block,
            &variables::BLOCK_DATA,
            &0u32.to_le_bytes(),
        ]),
        &other.to_bytes(),
    );
    let err = maintenance::verify(&path, Some(expected()))
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("height 1:") && err.contains("data hash"),
        "{err}"
    );
}

#[test]
fn test_commands_refuse_a_database_held_by_a_node() {
    let dir = tempfile::tempdir().unwrap();
    let path = committed_chain(dir.path());
    let held = RedbKVStore::open(&path).expect("node-like open");
    let backup = dir.path().join("backup.redb");
    for result in [
        maintenance::inspect(&path).map(|_| ()),
        maintenance::verify(&path, None).map(|_| ()),
        maintenance::compact(&path).map(|_| ()),
        maintenance::backup(&path, &backup).map(|_| ()),
    ] {
        let err = result.unwrap_err().to_string();
        assert!(err.contains("in use"), "{err}");
    }
    assert!(!backup.exists(), "nothing written while in use");
    drop(held);
    assert!(maintenance::inspect(&path).is_ok());
}

#[test]
fn test_backup_is_a_verified_new_file_and_never_overwrites() {
    let dir = tempfile::tempdir().unwrap();
    let path = committed_chain(dir.path());
    let out = dir.path().join("backup.redb");

    let report = maintenance::backup(&path, &out).expect("backup");
    assert!(report.heights_verified > MIN_HEIGHT);
    assert!(report.entries > 0);
    assert_eq!(
        report.committed_height,
        maintenance::inspect(&path).unwrap().committed_height
    );
    let mode = fs::metadata(&out).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    assert!(maintenance::verify(&out, Some(expected())).is_ok());

    let before = fs::read(&out).unwrap();
    let err = maintenance::backup(&path, &out).unwrap_err().to_string();
    assert!(err.contains("never overwrite"), "{err}");
    assert_eq!(fs::read(&out).unwrap(), before, "existing backup untouched");
}

#[test]
fn test_compact_keeps_committed_history() {
    let dir = tempfile::tempdir().unwrap();
    let path = committed_chain(dir.path());
    let before = maintenance::inspect(&path).unwrap();

    let report = maintenance::compact(&path).expect("compact");
    assert!(report.bytes_after <= report.bytes_before);

    let after = maintenance::inspect(&path).unwrap();
    assert_eq!(after.committed_block, before.committed_block);
    assert_eq!(after.commit_sequence, before.commit_sequence);
    assert!(maintenance::verify(&path, Some(expected())).is_ok());
}

#[test]
fn test_inspect_and_verify_never_write_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = committed_chain(dir.path());
    let before = fs::read(&path).unwrap();
    maintenance::inspect(&path).expect("inspect");
    maintenance::verify(&path, Some(expected())).expect("verify");
    assert_eq!(fs::read(&path).unwrap(), before, "file bytes changed");
}

#[test]
fn test_unclean_database_is_refused_read_only_and_repaired_by_compact() {
    let dir = tempfile::tempdir().unwrap();
    let path = committed_chain(dir.path());
    // A copy taken while a writer holds the file has redb's recovery flag set, exactly like the
    // file of a validator that was SIGKILLed.
    let unclean = dir.path().join("unclean.redb");
    {
        let _held = RedbKVStore::open(&path).unwrap();
        fs::copy(&path, &unclean).unwrap();
    }
    let before = fs::read(&unclean).unwrap();
    for result in [
        maintenance::inspect(&unclean).map(|_| ()),
        maintenance::verify(&unclean, None).map(|_| ()),
    ] {
        let err = result.unwrap_err();
        assert_eq!(err.exit_code(), 5, "{err}");
        assert!(err.to_string().contains("needs crash recovery"), "{err}");
    }
    assert_eq!(
        fs::read(&unclean).unwrap(),
        before,
        "read-only commands wrote"
    );

    maintenance::compact(&unclean).expect("compact recovers");
    assert!(maintenance::verify(&unclean, Some(expected())).is_ok());
}

#[test]
fn test_damaged_files_fail_verification_without_panicking() {
    let dir = tempfile::tempdir().unwrap();
    let path = committed_chain(dir.path());
    let bytes = fs::read(&path).unwrap();

    let truncated = dir.path().join("truncated.redb");
    fs::write(&truncated, &bytes[..bytes.len() / 3]).unwrap();
    let garbage = dir.path().join("garbage.redb");
    fs::write(&garbage, vec![0xa5; bytes.len()]).unwrap();

    for damaged in [&truncated, &garbage] {
        let err = maintenance::verify(damaged, Some(expected())).unwrap_err();
        assert!(matches!(err.exit_code(), 1 | 5), "{damaged:?}: {err}");
    }
}

#[test]
fn test_compact_on_a_clean_database_reports_no_repair() {
    let dir = tempfile::tempdir().unwrap();
    let path = committed_chain(dir.path());
    let report = maintenance::compact(&path).expect("compact");
    assert!(!report.repaired);
}
