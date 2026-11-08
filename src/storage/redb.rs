use hotstuff_rs::block_tree::pluggables::{KVGet, KVStore, WriteBatch};
use redb::{Database, ReadTransaction, ReadableDatabase, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

const BLOCKTREE_TABLE: TableDefinition<&[u8], &[u8]> = TableDefinition::new("blocktree");
const RECOVERY_META_KEY: &[u8] = b"meta:recovery_v1";
const COMMIT_SEQ_KEY: &[u8] = b"meta:commit_seq_v1";

/// Current durable storage schema version (PRD R05).
pub const STORAGE_SCHEMA_VERSION: u32 = 1;

/// Versioned recovery metadata stored atomically in the KV store (PRD R05).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryMetadata {
    pub schema_version: u32,
    pub chain_id: u64,
    pub genesis_hash: [u8; 32],
}

impl RecoveryMetadata {
    pub fn to_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }
}

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
    fail_next_write: Arc<AtomicBool>,
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
        Ok(Self {
            db: Arc::new(db),
            fail_next_write: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Access the underlying Redb Database instance.
    pub fn db(&self) -> &Database {
        &self.db
    }

    /// Inject a simulated write/flush failure on the next `try_write` / `write` call (for AT05 fault testing).
    pub fn set_fail_next_write(&self, fail: bool) {
        self.fail_next_write.store(fail, Ordering::SeqCst);
    }

    /// Atomically apply a `RedbWriteBatch` and increment the durable commit sequence number.
    /// If fault injection is armed or the commit fails, no partial writes are persisted.
    pub fn try_write(&mut self, write_batch: RedbWriteBatch) -> Result<u64, String> {
        let write_txn = self
            .db
            .begin_write()
            .map_err(|e| format!("failed to begin write transaction: {e}"))?;
        let next_seq = {
            let mut table = write_txn
                .open_table(BLOCKTREE_TABLE)
                .map_err(|e| format!("failed to open blocktree table: {e}"))?;

            let prev_seq = table
                .get(COMMIT_SEQ_KEY)
                .map_err(|e| format!("failed to read commit seq: {e}"))?
                .and_then(|v| {
                    let b: [u8; 8] = v.value().try_into().ok()?;
                    Some(u64::from_be_bytes(b))
                })
                .unwrap_or(0);

            for (key, val_opt) in write_batch.operations {
                match val_opt {
                    Some(val) => {
                        table
                            .insert(key.as_slice(), val.as_slice())
                            .map_err(|e| format!("insert failed: {e}"))?;
                    }
                    None => {
                        let _ = table
                            .remove(key.as_slice())
                            .map_err(|e| format!("remove failed: {e}"))?;
                    }
                }
            }

            if self.fail_next_write.swap(false, Ordering::SeqCst) {
                // Abort transaction before commit to simulate crash/flush failure at commit boundary
                drop(table);
                drop(write_txn);
                return Err("injected durable write/flush fault before commit".to_string());
            }

            let next_seq = prev_seq
                .checked_add(1)
                .ok_or_else(|| "commit sequence overflow".to_string())?;
            table
                .insert(COMMIT_SEQ_KEY, next_seq.to_be_bytes().as_slice())
                .map_err(|e| format!("failed to persist commit sequence: {e}"))?;
            next_seq
        };
        write_txn
            .commit()
            .map_err(|e| format!("failed to commit write transaction: {e}"))?;
        Ok(next_seq)
    }

    /// Read the current durable commit sequence counter.
    pub fn commit_sequence(&self) -> u64 {
        self.get(COMMIT_SEQ_KEY)
            .and_then(|b| {
                let arr: [u8; 8] = b.try_into().ok()?;
                Some(u64::from_be_bytes(arr))
            })
            .unwrap_or(0)
    }
}

/// Write recovery metadata into any `KVStore`.
pub fn write_recovery_metadata<K: KVStore>(store: &mut K, meta: &RecoveryMetadata) -> Result<(), String> {
    let bytes = meta
        .to_bytes()
        .map_err(|e| format!("failed to encode recovery metadata: {e}"))?;
    let mut batch = K::WriteBatch::new();
    batch.set(RECOVERY_META_KEY, &bytes);
    store.write(batch);
    Ok(())
}

/// Read recovery metadata from any `KVStore`.
pub fn read_recovery_metadata<K: KVStore>(store: &K) -> Result<Option<RecoveryMetadata>, String> {
    let Some(raw) = store.get(RECOVERY_META_KEY) else {
        return Ok(None);
    };
    RecoveryMetadata::from_bytes(&raw)
        .map(Some)
        .map_err(|e| format!("corrupt recovery metadata: {e}"))
}

/// Fail-closed validation of durable storage state on startup/recovery (PRD R05).
/// Verifies schema version, chain identity, genesis commitment, and committed block payload integrity.
pub fn validate_recovery_state<K: KVStore>(
    store: &K,
    expected_chain_id: u64,
    expected_genesis_hash: Option<[u8; 32]>,
) -> Result<RecoveryMetadata, String> {
    let meta = read_recovery_metadata(store)?
        .ok_or_else(|| "missing recovery metadata in store (uninitialized or incomplete genesis)".to_string())?;

    if meta.schema_version != STORAGE_SCHEMA_VERSION {
        return Err(format!(
            "unsupported storage schema version: found {}, expected {}",
            meta.schema_version, STORAGE_SCHEMA_VERSION
        ));
    }

    if meta.chain_id != expected_chain_id {
        return Err(format!(
            "chain_id mismatch on recovery: store has {}, node configured for {}",
            meta.chain_id, expected_chain_id
        ));
    }

    if let Some(expected_genesis) = expected_genesis_hash
        && meta.genesis_hash != expected_genesis
    {
        return Err("genesis_hash mismatch on recovery".to_string());
    }

    // Validate block tree camera and highest committed block integrity if present
    let camera = hotstuff_rs::block_tree::accessors::public::BlockTreeCamera::new(store.clone());
    let snapshot = camera.snapshot();
    if let Some(committed_block) = snapshot
        .highest_committed_block()
        .map_err(|e| format!("corrupt block tree highest_committed_block: {e:?}"))?
        && let Some(data) = snapshot
            .block_data(&committed_block)
            .map_err(|e| format!("corrupt block data for committed block: {e:?}"))?
        && !data.vec().is_empty()
    {
        if data.vec().len() != 1 {
            return Err("corrupt committed block datum count".to_string());
        }
        let payload = crate::core::types::BlockPayload::from_bytes(data.vec()[0].bytes())
            .map_err(|e| format!("corrupt committed block payload encoding: {e}"))?;
        payload
            .commitment_hash()
            .map_err(|e| format!("corrupt committed block commitment: {e}"))?;
    }

    Ok(meta)
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
        self.try_write(write_batch)
            .expect("failed to commit durable write batch");
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
            assert_eq!(store.commit_sequence(), 1);
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
            assert_eq!(store.commit_sequence(), 2);
        }

        // 3. Verify deletion persisted
        {
            let store = RedbKVStore::open(&db_path).unwrap();
            assert_eq!(store.get(b"key1"), None);
            assert_eq!(store.get(b"key2"), Some(b"val2".to_vec()));
            assert_eq!(store.commit_sequence(), 2);
        }
    }

    #[test]
    fn test_redb_fault_injection_preserves_acknowledged_commits() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut store = RedbKVStore::open(tmp.path()).unwrap();

        let mut b1 = RedbWriteBatch::new();
        b1.set(b"committed_key", b"good_value");
        assert_eq!(store.try_write(b1).unwrap(), 1);

        // Arm fault injection before commit
        store.set_fail_next_write(true);
        let mut b2 = RedbWriteBatch::new();
        b2.set(b"committed_key", b"corrupted_partial");
        b2.set(b"uncommitted_key", b"should_not_persist");
        assert!(store.try_write(b2).is_err());

        // Acknowledged commit remains intact; aborted batch never became visible
        assert_eq!(store.get(b"committed_key"), Some(b"good_value".to_vec()));
        assert_eq!(store.get(b"uncommitted_key"), None);
        assert_eq!(store.commit_sequence(), 1);
    }

    #[test]
    fn test_recovery_validation_fails_closed_on_mismatch_or_corruption() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut store = RedbKVStore::open(tmp.path()).unwrap();

        // Missing metadata fails closed
        assert!(validate_recovery_state(&store, 1337, None).is_err());

        let genesis_hash = [7u8; 32];
        let meta = RecoveryMetadata {
            schema_version: STORAGE_SCHEMA_VERSION,
            chain_id: 1337,
            genesis_hash,
        };
        write_recovery_metadata(&mut store, &meta).unwrap();

        // Matching chain_id and genesis_hash succeeds
        assert!(validate_recovery_state(&store, 1337, Some(genesis_hash)).is_ok());

        // Wrong chain_id fails closed
        assert!(validate_recovery_state(&store, 9999, None).is_err());

        // Wrong genesis_hash fails closed
        assert!(validate_recovery_state(&store, 1337, Some([8u8; 32])).is_err());

        // Unsupported schema version fails closed
        let bad_schema = RecoveryMetadata {
            schema_version: 99,
            chain_id: 1337,
            genesis_hash,
        };
        write_recovery_metadata(&mut store, &bad_schema).unwrap();
        assert!(validate_recovery_state(&store, 1337, None).is_err());
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
