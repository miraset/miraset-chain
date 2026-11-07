use hotstuff_rs::block_tree::pluggables::{KVGet, KVStore, WriteBatch};
use redb::{Database, ReadTransaction, ReadableDatabase, ReadableTable, TableDefinition};
use std::path::Path;
use std::sync::Arc;

const BLOCKTREE_TABLE: TableDefinition<&[u8], &[u8]> = TableDefinition::new("blocktree");

/// WriteBatch implementation for Redb.
#[derive(Debug, Default, Clone)]
pub struct RedbWriteBatch {
    operations: Vec<(Vec<u8>, Option<Vec<u8>>)>,
}

impl WriteBatch for RedbWriteBatch {
    fn new() -> Self {
        Self {
            operations: Vec::new(),
        }
    }

    fn set(&mut self, key: &[u8], value: &[u8]) {
        self.operations.push((key.to_vec(), Some(value.to_vec())));
    }

    fn delete(&mut self, key: &[u8]) {
        self.operations.push((key.to_vec(), None));
    }
}

/// Point-in-time snapshot of Redb table.
pub struct RedbSnapshot {
    read_txn: ReadTransaction,
}

impl KVGet for RedbSnapshot {
    fn get(&self, key: &[u8]) -> Option<Vec<u8>> {
        let table = self.read_txn.open_table(BLOCKTREE_TABLE).ok()?;
        let val = table.get(key).ok()??;
        Some(val.value().to_vec())
    }
}

/// Durable, crash-consistent embedded KVStore powered by Redb.
#[derive(Clone)]
pub struct RedbKVStore {
    db: Arc<Database>,
}

impl RedbKVStore {
    /// Open or create a Redb database at the specified path.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let db = Database::create(path)?;
        // Ensure table exists
        let write_txn = db.begin_write()?;
        {
            let _ = write_txn.open_table(BLOCKTREE_TABLE)?;
        }
        write_txn.commit()?;
        Ok(Self { db: Arc::new(db) })
    }

    /// Access the underlying Redb Database instance.
    pub fn db(&self) -> &Database {
        &self.db
    }
}

impl KVGet for RedbKVStore {
    fn get(&self, key: &[u8]) -> Option<Vec<u8>> {
        let read_txn = self.db.begin_read().ok()?;
        let table = read_txn.open_table(BLOCKTREE_TABLE).ok()?;
        let val = table.get(key).ok()??;
        Some(val.value().to_vec())
    }
}

impl KVStore for RedbKVStore {
    type WriteBatch = RedbWriteBatch;
    type Snapshot<'a>
        = RedbSnapshot
    where
        Self: 'a;

    fn clear(&mut self) {
        if let Ok(write_txn) = self.db.begin_write() {
            if let Ok(mut table) = write_txn.open_table(BLOCKTREE_TABLE) {
                // Collect keys to delete
                let mut keys = Vec::new();
                if let Ok(iter) = table.iter() {
                    for entry in iter.flatten() {
                        keys.push(entry.0.value().to_vec());
                    }
                }
                for k in keys {
                    let _ = table.remove(k.as_slice());
                }
            }
            let _ = write_txn.commit();
        }
    }

    fn snapshot<'a>(&'a self) -> Self::Snapshot<'a> {
        let read_txn = self
            .db
            .begin_read()
            .expect("failed to open read transaction");
        RedbSnapshot { read_txn }
    }

    fn write(&mut self, write_batch: Self::WriteBatch) {
        let write_txn = self
            .db
            .begin_write()
            .expect("failed to begin write transaction");
        {
            let mut table = write_txn
                .open_table(BLOCKTREE_TABLE)
                .expect("failed to open blocktree table");
            for (key, val_opt) in write_batch.operations {
                match val_opt {
                    Some(val) => {
                        table
                            .insert(key.as_slice(), val.as_slice())
                            .expect("insert failed");
                    }
                    None => {
                        let _ = table.remove(key.as_slice());
                    }
                }
            }
        }
        write_txn
            .commit()
            .expect("failed to commit write transaction");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_redb_kv_store_basic_and_persistence() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let db_path = tmp.path().to_path_buf();

        // 1. Initial write
        {
            let mut store = RedbKVStore::open(&db_path).unwrap();
            let mut batch = RedbWriteBatch::new();
            batch.set(b"key1", b"val1");
            batch.set(b"key2", b"val2");
            store.write(batch);

            assert_eq!(store.get(b"key1"), Some(b"val1".to_vec()));
            assert_eq!(store.get(b"key2"), Some(b"val2".to_vec()));
        }

        // 2. Re-open and verify persistence (crash/restart simulation)
        {
            let mut store = RedbKVStore::open(&db_path).unwrap();
            assert_eq!(store.get(b"key1"), Some(b"val1".to_vec()));
            assert_eq!(store.get(b"key2"), Some(b"val2".to_vec()));
            assert_eq!(store.get(b"key3"), None);

            // Delete key1
            let mut batch_del = RedbWriteBatch::new();
            batch_del.delete(b"key1");
            store.write(batch_del);
            assert_eq!(store.get(b"key1"), None);
        }

        // 3. Verify deletion persisted
        {
            let store = RedbKVStore::open(&db_path).unwrap();
            assert_eq!(store.get(b"key1"), None);
            assert_eq!(store.get(b"key2"), Some(b"val2".to_vec()));
        }
    }

    #[test]
    fn test_redb_snapshot_isolation() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut store = RedbKVStore::open(tmp.path()).unwrap();

        let mut batch = RedbWriteBatch::new();
        batch.set(b"immutable", b"initial");
        store.write(batch);

        let snap = store.snapshot();
        assert_eq!(snap.get(b"immutable"), Some(b"initial".to_vec()));

        // Mutate store
        let mut batch2 = RedbWriteBatch::new();
        batch2.set(b"immutable", b"modified");
        batch2.set(b"new_key", b"hello");
        store.write(batch2);

        // Snapshot still sees original state
        assert_eq!(snap.get(b"immutable"), Some(b"initial".to_vec()));
        assert_eq!(snap.get(b"new_key"), None);

        // Store sees modified state
        assert_eq!(store.get(b"immutable"), Some(b"modified".to_vec()));
        assert_eq!(store.get(b"new_key"), Some(b"hello".to_vec()));
    }
}
