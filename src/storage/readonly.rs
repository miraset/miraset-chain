//! Read-only access to a chain database for offline maintenance (TODO 8.4.5).
//!
//! Opening never writes the file: redb's read-only open neither sets the recovery flag nor
//! repairs, and a database that needs crash recovery is refused. It takes a shared lock, so a
//! database a running validator holds (exclusive lock) is refused too.

use hotstuff_rs::block_tree::pluggables::{KVGet, KVStore};
use redb::{Builder, DatabaseError, ReadOnlyDatabase, ReadTransaction, ReadableDatabase};
use std::path::Path;
use std::sync::Arc;

use super::redb::{COMMIT_SEQ_KEY, RedbSnapshot, RedbWriteBatch, get_in};

/// A chain database opened read-only. It implements `KVStore` only so block-tree accessors can
/// read it; its write operations are never valid and panic.
#[derive(Clone)]
pub struct ReadOnlyRedbStore {
    db: Arc<ReadOnlyDatabase>,
}

impl ReadOnlyRedbStore {
    /// Open an existing database without modifying it.
    pub fn open(path: &Path) -> Result<Self, DatabaseError> {
        Builder::new()
            .open_read_only(path)
            .map(|db| Self { db: Arc::new(db) })
    }

    pub(crate) fn begin_read(&self) -> Result<ReadTransaction, String> {
        self.db
            .begin_read()
            .map_err(|e| format!("cannot start read transaction: {e}"))
    }

    /// Durable commit sequence counter (as `RedbKVStore::commit_sequence`).
    pub fn commit_sequence(&self) -> u64 {
        self.get(COMMIT_SEQ_KEY)
            .and_then(|bytes| Some(u64::from_be_bytes(bytes.try_into().ok()?)))
            .unwrap_or(0)
    }
}

impl KVGet for ReadOnlyRedbStore {
    fn get(&self, key: &[u8]) -> Option<Vec<u8>> {
        get_in(&self.db.begin_read().ok()?, key)
    }
}

impl KVStore for ReadOnlyRedbStore {
    type WriteBatch = RedbWriteBatch;
    type Snapshot<'a>
        = RedbSnapshot
    where
        Self: 'a;

    fn write(&mut self, _write_batch: Self::WriteBatch) {
        panic!("read-only maintenance store: write is not allowed");
    }

    fn clear(&mut self) {
        panic!("read-only maintenance store: clear is not allowed");
    }

    fn snapshot<'a>(&'a self) -> Self::Snapshot<'a> {
        RedbSnapshot::from_read_txn(
            self.db
                .begin_read()
                .expect("failed to open read transaction"),
        )
    }
}
