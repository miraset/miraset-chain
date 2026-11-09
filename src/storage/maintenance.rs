//! Offline database maintenance: `db inspect`, `db verify`, `db compact`, `db backup`
//! (TODO 8.4.5).
//!
//! - `inspect` and `verify` open the chain database **read-only**: they never write the file,
//!   refuse a database that needs crash recovery (start the node once, or run `compact`), and
//!   refuse one that a running validator holds.
//! - `compact` opens it for writing (exclusive lock): redb first checks every page checksum and
//!   repairs if needed, then reclaims free pages. Records are not changed.
//! - `backup` reads a consistent snapshot read-only and writes a **logical** copy (every table,
//!   one transaction) into a new file that must not exist yet, closes it cleanly and verifies it
//!   like `verify`. A failed backup removes the file it created.
//!
//! No command creates or truncates the source database. Lock-based refusal relies on the
//! filesystem honouring `flock` (local disks and local Docker volumes do; some network
//! filesystems do not).
//!
//! The vote signing guard is deliberately **not** part of a backup: restoring an older guard would
//! roll back slashing protection and allow double signing.

use hotstuff_rs::block_tree::accessors::public::BlockTreeCamera;
use hotstuff_rs::block_tree::pluggables::KVStore;
use hotstuff_rs::types::block::Block;
use hotstuff_rs::types::data_types::{BlockHeight, CryptoHash};
use redb::backends::FileBackend;
use redb::{Builder, Database, DatabaseError};
use serde::Serialize;
use std::fs::{self, File, OpenOptions};
use std::os::unix::fs::OpenOptionsExt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

use crate::core::types::BlockPayload;
use crate::storage::readonly::ReadOnlyRedbStore;
use crate::storage::redb::copy_all_tables;
use crate::storage::{RecoveryMetadata, read_recovery_metadata, validate_recovery_state};

/// Mode of a backup file: the chain database holds no secrets, but it is not world data either.
const BACKUP_FILE_MODE: u32 = 0o600;

/// Why a maintenance command failed. [`Self::exit_code`] lets scripts tell the cases apart.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MaintenanceError {
    #[error("no chain database at {0}")]
    NotFound(String),
    #[error("{0} is in use (is the validator running?); stop it first")]
    InUse(String),
    #[error(
        "{0} needs crash recovery (unclean shutdown); start the validator once or run `db compact`"
    )]
    NeedsRecovery(String),
    #[error("{0} already exists; backups never overwrite")]
    Exists(String),
    /// The database is readable but its contents are wrong or corrupt.
    #[error("verification failed: {0}")]
    Invalid(String),
    #[error("{0}")]
    Io(String),
}

impl MaintenanceError {
    /// 4: in use; 5: needs recovery or failed verification; 1: anything else.
    pub fn exit_code(&self) -> i32 {
        match self {
            MaintenanceError::InUse(_) => 4,
            MaintenanceError::NeedsRecovery(_) | MaintenanceError::Invalid(_) => 5,
            MaintenanceError::NotFound(_)
            | MaintenanceError::Exists(_)
            | MaintenanceError::Io(_) => 1,
        }
    }
}

type Result<T> = std::result::Result<T, MaintenanceError>;

/// What the database says about itself (`db inspect`).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct InspectReport {
    pub schema_version: u32,
    pub chain_id: u64,
    pub genesis_hash: String,
    pub commit_sequence: u64,
    /// `None` before the first commit.
    pub committed_height: Option<u64>,
    pub committed_block: Option<String>,
    pub file_bytes: u64,
}

/// Result of a successful `db verify`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct VerifyReport {
    pub chain_id: u64,
    /// Number of committed heights checked (0 before the first commit).
    pub heights_checked: u64,
    pub committed_block: Option<String>,
    pub genesis_checked: bool,
}

/// Result of `db compact`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CompactReport {
    /// `true` if redb's integrity check had to repair pages before compacting.
    pub repaired: bool,
    pub bytes_before: u64,
    pub bytes_after: u64,
}

/// Result of `db backup`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct BackupReport {
    pub out: String,
    pub bytes: u64,
    pub entries: u64,
    pub committed_height: Option<u64>,
    /// Committed heights the backup passed `verify` for.
    pub heights_verified: u64,
}

/// Chain identity a `verify` must match, taken from the genesis file.
#[derive(Debug, Clone, Copy)]
pub struct ExpectedChain {
    pub chain_id: u64,
    pub genesis_hash: [u8; 32],
}

fn shown(path: &Path) -> String {
    path.display().to_string()
}

fn require_file(path: &Path) -> Result<()> {
    if path.is_file() {
        Ok(())
    } else {
        Err(MaintenanceError::NotFound(shown(path)))
    }
}

fn open_error(path: &Path, error: DatabaseError) -> MaintenanceError {
    match error {
        DatabaseError::DatabaseAlreadyOpen => MaintenanceError::InUse(shown(path)),
        DatabaseError::RepairAborted => MaintenanceError::NeedsRecovery(shown(path)),
        other => MaintenanceError::Invalid(format!("cannot open {}: {other}", shown(path))),
    }
}

fn open_read_only(path: &Path) -> Result<ReadOnlyRedbStore> {
    require_file(path)?;
    ReadOnlyRedbStore::open(path).map_err(|e| open_error(path, e))
}

fn file_len(path: &Path) -> Result<u64> {
    fs::metadata(path)
        .map(|m| m.len())
        .map_err(|e| MaintenanceError::Io(format!("cannot stat {}: {e}", shown(path))))
}

/// Run a read of possibly corrupt data; a panic inside redb or the block tree becomes `Invalid`.
fn guarded<T>(what: &str, read: impl FnOnce() -> Result<T>) -> Result<T> {
    catch_unwind(AssertUnwindSafe(read)).unwrap_or_else(|panic| {
        let reason = panic
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| panic.downcast_ref::<&str>().copied())
            .unwrap_or("unknown panic");
        Err(MaintenanceError::Invalid(format!(
            "{what}: database unreadable ({reason})"
        )))
    })
}

fn metadata<K: KVStore>(store: &K) -> Result<RecoveryMetadata> {
    read_recovery_metadata(store)
        .map_err(MaintenanceError::Invalid)?
        .ok_or_else(|| {
            MaintenanceError::Invalid(
                "missing recovery metadata (not an initialized chain database)".to_string(),
            )
        })
}

fn committed_tip<K: KVStore>(store: &K) -> Result<Option<(u64, CryptoHash)>> {
    let invalid = |what: &str| MaintenanceError::Invalid(what.to_string());
    let camera = BlockTreeCamera::new(store.clone());
    let snapshot = camera.snapshot();
    let Some(hash) = snapshot
        .highest_committed_block()
        .map_err(|e| invalid(&format!("cannot read highest committed block: {e:?}")))?
    else {
        return Ok(None);
    };
    let height = snapshot
        .block_height(&hash)
        .map_err(|e| invalid(&format!("cannot read committed block height: {e:?}")))?
        .ok_or_else(|| invalid("highest committed block has no stored height"))?;
    Ok(Some((height.int(), hash)))
}

/// `db inspect`: read identity and the committed tip. Never writes the file.
pub fn inspect(path: &Path) -> Result<InspectReport> {
    let store = open_read_only(path)?;
    guarded("inspect", || {
        let meta = metadata(&store)?;
        let tip = committed_tip(&store)?;
        Ok(InspectReport {
            schema_version: meta.schema_version,
            chain_id: meta.chain_id,
            genesis_hash: hex::encode(meta.genesis_hash),
            commit_sequence: store.commit_sequence(),
            committed_height: tip.map(|(height, _)| height),
            committed_block: tip.map(|(_, hash)| hex::encode(hash.bytes())),
            file_bytes: file_len(path)?,
        })
    })
}

/// `db verify`: recovery metadata (against the genesis file when given) and every committed
/// height, read-only. Per height: the stored hash recomputes, the height and parent link are
/// consistent, the justify belongs to this chain with increasing views, and the payload is
/// canonical and matches the data hash; nothing is indexed above the committed tip.
/// Quorum-certificate signatures are not re-verified (they were checked on insert), and redb page
/// checksums are checked by `compact`, which needs write access.
pub fn verify(path: &Path, expected: Option<ExpectedChain>) -> Result<VerifyReport> {
    let store = open_read_only(path)?;
    guarded("verify", || verify_store(&store, expected))
}

fn verify_store<K: KVStore>(store: &K, expected: Option<ExpectedChain>) -> Result<VerifyReport> {
    let meta = match expected {
        Some(chain) => validate_recovery_state(store, chain.chain_id, Some(chain.genesis_hash)),
        None => {
            let meta = metadata(store)?;
            validate_recovery_state(store, meta.chain_id, None)
        }
    }
    .map_err(MaintenanceError::Invalid)?;
    let tip = committed_tip(store)?;
    let heights_checked = match tip {
        Some((height, _)) => {
            verify_committed_chain(store, meta.chain_id, height)?;
            height + 1
        }
        None => 0,
    };
    Ok(VerifyReport {
        chain_id: meta.chain_id,
        heights_checked,
        committed_block: tip.map(|(_, hash)| hex::encode(hash.bytes())),
        genesis_checked: expected.is_some(),
    })
}

fn verify_committed_chain<K: KVStore>(store: &K, chain_id: u64, top: u64) -> Result<()> {
    let camera = BlockTreeCamera::new(store.clone());
    let snapshot = camera.snapshot();
    let mut parent: Option<Block> = None;
    for height in 0..=top {
        let at = |what: String| MaintenanceError::Invalid(format!("height {height}: {what}"));
        let hash = snapshot
            .block_at_height(BlockHeight::new(height))
            .map_err(|e| at(format!("cannot read block index: {e:?}")))?
            .ok_or_else(|| at("no committed block".to_string()))?;
        let block = snapshot
            .block(&hash)
            .map_err(|e| at(format!("cannot read block: {e:?}")))?
            .ok_or_else(|| at("committed block is missing".to_string()))?;
        verify_block(&block, &hash, height, parent.as_ref(), chain_id).map_err(at)?;
        parent = Some(block);
    }
    // `block_at_height` only answers for committed heights, so read the raw index entry.
    let above = top + 1;
    let index_key = [
        hotstuff_rs::block_tree::variables::BLOCK_AT_HEIGHT.as_slice(),
        &above.to_le_bytes(),
    ]
    .concat();
    if store.get(&index_key).is_some() {
        return Err(MaintenanceError::Invalid(format!(
            "height {above}: indexed above the committed tip {top}"
        )));
    }
    Ok(())
}

fn verify_block(
    block: &Block,
    hash: &CryptoHash,
    height: u64,
    parent: Option<&Block>,
    chain_id: u64,
) -> std::result::Result<(), String> {
    if block.hash != *hash || Block::hash(block.height, &block.justify, &block.data_hash) != *hash {
        return Err("block hash does not match its contents".to_string());
    }
    if block.height.int() != height {
        return Err(format!("stored height is {}", block.height.int()));
    }
    match parent {
        None if !block.justify.is_genesis_pc() => {
            return Err("first block does not extend genesis".to_string());
        }
        Some(parent) => {
            if block.justify.is_genesis_pc() || block.justify.block != parent.hash {
                return Err("parent link does not point at the block one height below".to_string());
            }
            if block.justify.chain_id.int() != chain_id {
                return Err(format!(
                    "justify is for chain {}, database is chain {chain_id}",
                    block.justify.chain_id.int()
                ));
            }
            if !parent.justify.is_genesis_pc() && block.justify.view <= parent.justify.view {
                return Err("justify view does not increase along the chain".to_string());
            }
        }
        None => {}
    }
    let data = block.data.vec();
    if data.len() != 1 {
        return Err(format!("expected one payload datum, found {}", data.len()));
    }
    let payload =
        BlockPayload::from_bytes(data[0].bytes()).map_err(|e| format!("bad payload: {e}"))?;
    let canonical = payload
        .to_canonical_bytes()
        .map_err(|e| format!("payload not encodable: {e}"))?;
    if canonical.as_slice() != data[0].bytes() {
        return Err("payload is not canonically encoded".to_string());
    }
    let commitment = payload
        .commitment_hash()
        .map_err(|e| format!("payload commitment: {e}"))?;
    if commitment != block.data_hash.bytes() {
        return Err("payload does not match the block's data hash".to_string());
    }
    Ok(())
}

/// `db compact`: check page checksums (repairing if needed), then reclaim free pages. Records are
/// not changed.
pub fn compact(path: &Path) -> Result<CompactReport> {
    require_file(path)?;
    let mut db = Database::open(path).map_err(|e| open_error(path, e))?;
    let bytes_before = file_len(path)?;
    let repaired = !db
        .check_integrity()
        .map_err(|e| MaintenanceError::Invalid(format!("integrity check failed: {e}")))?;
    while db
        .compact()
        .map_err(|e| MaintenanceError::Io(format!("compaction failed: {e}")))?
    {}
    drop(db);
    Ok(CompactReport {
        repaired,
        bytes_before,
        bytes_after: file_len(path)?,
    })
}

/// `db backup`: logical copy of a read-only snapshot into the new file `out`, then a full
/// `verify` of the copy against the source's identity and tip. On failure the new file is removed.
pub fn backup(path: &Path, out: &Path) -> Result<BackupReport> {
    let source = open_read_only(path)?;
    let (meta, tip) = guarded("backup", || {
        Ok((metadata(&source)?, committed_tip(&source)?))
    })?;
    let entries = write_new_database(&source, out)?;
    let checked = verify(
        out,
        Some(ExpectedChain {
            chain_id: meta.chain_id,
            genesis_hash: meta.genesis_hash,
        }),
    )
    .and_then(|report| {
        let expected_tip = tip.map(|(_, hash)| hex::encode(hash.bytes()));
        if report.committed_block == expected_tip {
            Ok(report)
        } else {
            Err(MaintenanceError::Invalid(
                "backup tip differs from the source".to_string(),
            ))
        }
    });
    let report = match checked {
        Ok(report) => report,
        Err(error) => {
            let _ = fs::remove_file(out);
            return Err(error);
        }
    };
    Ok(BackupReport {
        out: shown(out),
        bytes: file_len(out)?,
        entries,
        committed_height: tip.map(|(height, _)| height),
        heights_verified: report.heights_checked,
    })
}

/// Create `out` (must not exist; never follows a symlink there), copy every table of one
/// snapshot of `source` into it, close it cleanly and flush its directory entry. Removes `out`
/// if anything fails after creating it.
fn write_new_database(source: &ReadOnlyRedbStore, out: &Path) -> Result<u64> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(BACKUP_FILE_MODE)
        .open(out)
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::AlreadyExists => MaintenanceError::Exists(shown(out)),
            _ => MaintenanceError::Io(format!("cannot create {}: {e}", shown(out))),
        })?;
    let copied = copy_into(source, file, out);
    if copied.is_err() {
        let _ = fs::remove_file(out);
    }
    copied
}

fn copy_into(source: &ReadOnlyRedbStore, file: File, out: &Path) -> Result<u64> {
    let io = |what: String| MaintenanceError::Io(format!("backup {}: {what}", shown(out)));
    let backend = FileBackend::new(file).map_err(|e| io(e.to_string()))?;
    let target = Builder::new()
        .create_with_backend(backend)
        .map_err(|e| io(e.to_string()))?;
    let snapshot = source.begin_read().map_err(io)?;
    let entries = copy_all_tables(&snapshot, &target).map_err(io)?;
    drop(snapshot);
    target.close().map_err(|e| io(format!("close: {e}")))?;
    if let Some(dir) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
        File::open(dir)
            .and_then(|d| d.sync_all())
            .map_err(|e| io(format!("flush directory: {e}")))?;
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hotstuff_rs::hotstuff::types::{Phase, PhaseCertificate};
    use hotstuff_rs::types::data_types::{ChainID, Data, Datum, SignatureSet, ViewNumber};

    fn payload_block(height: u64, justify: PhaseCertificate, payload: &BlockPayload) -> Block {
        let hash = CryptoHash::new(payload.commitment_hash().unwrap());
        let data = Data::new(vec![Datum::new(payload.to_bytes())]);
        Block::new(BlockHeight::new(height), justify, hash, data)
    }

    const CHAIN: u64 = 1;

    fn justify_for(parent: &CryptoHash, view: u64) -> PhaseCertificate {
        PhaseCertificate {
            chain_id: ChainID::new(CHAIN),
            view: ViewNumber::new(view),
            block: *parent,
            phase: Phase::Generic,
            signatures: SignatureSet::genesis(),
        }
    }

    fn first() -> Block {
        payload_block(0, PhaseCertificate::genesis_pc(), &BlockPayload::default())
    }

    #[test]
    fn test_verify_block_accepts_a_consistent_chain() {
        let first = first();
        assert_eq!(verify_block(&first, &first.hash, 0, None, CHAIN), Ok(()));
        let second = payload_block(1, justify_for(&first.hash, 1), &BlockPayload::default());
        assert_eq!(
            verify_block(&second, &second.hash, 1, Some(&first), CHAIN),
            Ok(())
        );
        let third = payload_block(2, justify_for(&second.hash, 2), &BlockPayload::default());
        assert_eq!(
            verify_block(&third, &third.hash, 2, Some(&second), CHAIN),
            Ok(())
        );
    }

    #[test]
    fn test_verify_block_rejects_inconsistent_blocks() {
        let first = first();
        let second = payload_block(1, justify_for(&first.hash, 1), &BlockPayload::default());
        let other = CryptoHash::new([9; 32]);
        let orphan = payload_block(0, justify_for(&other, 1), &BlockPayload::default());
        let wrong_parent = payload_block(1, justify_for(&other, 1), &BlockPayload::default());
        let mut foreign_justify = justify_for(&first.hash, 1);
        foreign_justify.chain_id = ChainID::new(CHAIN + 1);
        let foreign = payload_block(1, foreign_justify, &BlockPayload::default());
        let stale_view = payload_block(2, justify_for(&second.hash, 1), &BlockPayload::default());
        let genesis_link =
            payload_block(1, PhaseCertificate::genesis_pc(), &BlockPayload::default());
        let cases = [
            (
                "index points elsewhere",
                verify_block(&first, &other, 0, None, CHAIN),
            ),
            (
                "wrong height",
                verify_block(&first, &first.hash, 1, None, CHAIN),
            ),
            (
                "not extending genesis",
                verify_block(&orphan, &orphan.hash, 0, None, CHAIN),
            ),
            (
                "wrong parent",
                verify_block(&wrong_parent, &wrong_parent.hash, 1, Some(&first), CHAIN),
            ),
            (
                "another chain",
                verify_block(&foreign, &foreign.hash, 1, Some(&first), CHAIN),
            ),
            (
                "view not increasing",
                verify_block(&stale_view, &stale_view.hash, 2, Some(&second), CHAIN),
            ),
            (
                "genesis link above height 0",
                verify_block(&genesis_link, &genesis_link.hash, 1, Some(&first), CHAIN),
            ),
        ];
        for (name, result) in cases {
            assert!(result.is_err(), "{name} accepted");
        }
    }

    #[test]
    fn test_verify_block_rejects_payload_problems() {
        let mut tampered = first();
        tampered.hash = Block::hash(
            tampered.height,
            &tampered.justify,
            &CryptoHash::new([1; 32]),
        );
        tampered.data_hash = CryptoHash::new([1; 32]);
        let err = verify_block(&tampered, &tampered.hash, 0, None, CHAIN).unwrap_err();
        assert!(err.contains("data hash"), "{err}");

        let mut two_datums = first();
        two_datums.data = Data::new(vec![Datum::new(vec![1]), Datum::new(vec![2])]);
        let err = verify_block(&two_datums, &two_datums.hash, 0, None, CHAIN).unwrap_err();
        assert!(err.contains("one payload datum"), "{err}");

        let mut garbage = first();
        garbage.data = Data::new(vec![Datum::new(b"not a payload".to_vec())]);
        let err = verify_block(&garbage, &garbage.hash, 0, None, CHAIN).unwrap_err();
        assert!(err.contains("bad payload"), "{err}");
    }

    #[test]
    fn test_commands_refuse_a_missing_database_without_creating_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("chain.redb");
        for result in [
            inspect(&path).map(|_| ()),
            verify(&path, None).map(|_| ()),
            compact(&path).map(|_| ()),
            backup(&path, &dir.path().join("copy.redb")).map(|_| ()),
        ] {
            assert!(matches!(result, Err(MaintenanceError::NotFound(_))));
        }
        assert!(!path.exists());
        assert!(!dir.path().join("copy.redb").exists());
    }
}
