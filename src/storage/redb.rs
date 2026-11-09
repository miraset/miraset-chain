use crate::consensus::SafetyHalt;
use crate::core::types::compute_state_commitment;
use hotstuff_rs::block_tree::pluggables::{KVGet, KVStore, WriteBatch};
use redb::{
    Database, ReadTransaction, ReadableDatabase, ReadableTable, TableDefinition, TableHandle,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

const BLOCKTREE_TABLE: TableDefinition<&[u8], &[u8]> = TableDefinition::new("blocktree");
const APP_STATE_TABLE: TableDefinition<&[u8], &[u8]> = TableDefinition::new("cf_state");
const RECEIPTS_INDEX_TABLE: TableDefinition<&[u8], &[u8]> = TableDefinition::new("cf_receipts");
const SNAPSHOTS_TABLE: TableDefinition<&[u8], &[u8]> = TableDefinition::new("cf_snapshots");

/// Every table of a chain database, for whole-database copies (`db backup`).
const ALL_TABLES: [TableDefinition<&[u8], &[u8]>; 4] = [
    BLOCKTREE_TABLE,
    APP_STATE_TABLE,
    RECEIPTS_INDEX_TABLE,
    SNAPSHOTS_TABLE,
];

const RECOVERY_META_KEY: &[u8] = b"meta:recovery_v1";
pub(crate) const COMMIT_SEQ_KEY: &[u8] = b"meta:commit_seq_v1";

/// Current durable storage schema version (PRD R05).
pub const STORAGE_SCHEMA_VERSION: u32 = 1;

/// Versioned deterministic snapshot of finalized application and receipt state (PRD R05/R10).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateSnapshot {
    pub schema_version: u32,
    pub chain_id: u64,
    pub commit_sequence: u64,
    pub state_root: [u8; 32],
    pub state_entries: Vec<(Vec<u8>, Vec<u8>)>,
    pub receipt_entries: Vec<(Vec<u8>, Vec<u8>)>,
    pub integrity_hash: [u8; 32],
}

impl StateSnapshot {
    pub fn compute_integrity_hash(
        schema_version: u32,
        chain_id: u64,
        commit_sequence: u64,
        state_root: &[u8; 32],
        state_entries: &[(Vec<u8>, Vec<u8>)],
        receipt_entries: &[(Vec<u8>, Vec<u8>)],
    ) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(b"MIRASET:SNAPSHOT:v1");
        hasher.update(schema_version.to_be_bytes());
        hasher.update(chain_id.to_be_bytes());
        hasher.update(commit_sequence.to_be_bytes());
        hasher.update(state_root);
        hasher.update((state_entries.len() as u32).to_be_bytes());
        for (k, v) in state_entries {
            hasher.update((k.len() as u32).to_be_bytes());
            hasher.update(k);
            hasher.update((v.len() as u32).to_be_bytes());
            hasher.update(v);
        }
        hasher.update((receipt_entries.len() as u32).to_be_bytes());
        for (k, v) in receipt_entries {
            hasher.update((k.len() as u32).to_be_bytes());
            hasher.update(k);
            hasher.update((v.len() as u32).to_be_bytes());
            hasher.update(v);
        }
        hasher.finalize().into()
    }

    /// Fail-closed verification of snapshot schema version, chain ID, state root, and integrity hash.
    pub fn verify(&self, expected_chain_id: u64) -> Result<(), String> {
        if self.schema_version != STORAGE_SCHEMA_VERSION {
            return Err(format!(
                "unsupported snapshot schema version: {}",
                self.schema_version
            ));
        }
        if self.chain_id != expected_chain_id {
            return Err(format!(
                "snapshot chain_id mismatch: found {}, expected {}",
                self.chain_id, expected_chain_id
            ));
        }

        let mut all_refs: Vec<(&[u8], &[u8])> = Vec::with_capacity(
            self.state_entries
                .len()
                .saturating_add(self.receipt_entries.len()),
        );
        for (k, v) in &self.state_entries {
            all_refs.push((k.as_slice(), v.as_slice()));
        }
        for (k, v) in &self.receipt_entries {
            all_refs.push((k.as_slice(), v.as_slice()));
        }
        let computed_root = compute_state_commitment(&all_refs);
        if computed_root != self.state_root {
            return Err("snapshot state_root mismatch".to_string());
        }

        let expected_integrity = Self::compute_integrity_hash(
            self.schema_version,
            self.chain_id,
            self.commit_sequence,
            &self.state_root,
            &self.state_entries,
            &self.receipt_entries,
        );
        if expected_integrity != self.integrity_hash {
            return Err("snapshot integrity hash mismatch".to_string());
        }
        Ok(())
    }
}

fn is_app_state_key(key: &[u8]) -> bool {
    key.starts_with(b"acc:")
        || key.starts_with(b"worker:")
        || key.starts_with(b"workers:")
        || key.starts_with(b"epoch:")
        || key.starts_with(b"state:")
        || key.starts_with(b"\x01") // hotstuff_rs committed app_state namespace prefix
}

fn is_receipt_index_key(key: &[u8]) -> bool {
    key.starts_with(b"task:") || key.ends_with(b"task:") || key.windows(5).any(|w| w == b"task:")
}

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

impl RedbSnapshot {
    /// Snapshot over an existing read transaction (also from a read-only database).
    pub(crate) fn from_read_txn(read_txn: ReadTransaction) -> Self {
        Self { read_txn }
    }
}

/// Read one chain-database key in `read_txn`, the same way `RedbKVStore` does.
pub(crate) fn get_in(read_txn: &ReadTransaction, key: &[u8]) -> Option<Vec<u8>> {
    let table = read_txn.open_table(BLOCKTREE_TABLE).ok()?;
    let val = table.get(key).ok()??;
    Some(val.value().to_vec())
}

/// Copy every table visible in `source` into `target` in one write transaction, returning the
/// number of entries copied. Tables absent from `source` are skipped.
pub(crate) fn copy_all_tables(source: &ReadTransaction, target: &Database) -> Result<u64, String> {
    let write_txn = target
        .begin_write()
        .map_err(|e| format!("cannot write backup: {e}"))?;
    let mut copied = 0u64;
    for definition in ALL_TABLES {
        let from = match source.open_table(definition) {
            Ok(table) => table,
            Err(redb::TableError::TableDoesNotExist(_)) => continue,
            Err(e) => return Err(format!("cannot read table {}: {e}", definition.name())),
        };
        let mut to = write_txn
            .open_table(definition)
            .map_err(|e| format!("cannot create table {}: {e}", definition.name()))?;
        let entries = from
            .iter()
            .map_err(|e| format!("cannot scan table {}: {e}", definition.name()))?;
        for entry in entries {
            let (key, value) =
                entry.map_err(|e| format!("cannot read table {}: {e}", definition.name()))?;
            to.insert(key.value(), value.value())
                .map_err(|e| format!("cannot copy into {}: {e}", definition.name()))?;
            copied += 1;
        }
    }
    write_txn
        .commit()
        .map_err(|e| format!("cannot commit backup: {e}"))?;
    Ok(copied)
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
    safety_halt: Option<SafetyHalt>,
}

impl RedbKVStore {
    /// Open or create a Redb database at the specified path with separated tables
    /// (`blocktree`, `cf_state`, `cf_receipts`, `cf_snapshots`).
    pub fn open(path: impl AsRef<Path>) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let db = Database::create(path)?;
        let write_txn = db.begin_write()?;
        {
            let _ = write_txn.open_table(BLOCKTREE_TABLE)?;
            let _ = write_txn.open_table(APP_STATE_TABLE)?;
            let _ = write_txn.open_table(RECEIPTS_INDEX_TABLE)?;
            let _ = write_txn.open_table(SNAPSHOTS_TABLE)?;
        }
        write_txn.commit()?;
        Ok(Self {
            db: Arc::new(db),
            fail_next_write: Arc::new(AtomicBool::new(false)),
            safety_halt: None,
        })
    }

    /// Report write failures through `halt` (TODO 8.3.2, gap G2) before the replica thread stops.
    pub fn with_safety_halt(self, halt: SafetyHalt) -> Self {
        Self {
            safety_halt: Some(halt),
            ..self
        }
    }

    /// Access the underlying Redb Database instance.
    pub fn db(&self) -> &Database {
        &self.db
    }

    /// Inject a simulated write/flush failure on the next `try_write` / `write` call (for AT05 fault testing).
    pub fn set_fail_next_write(&self, fail: bool) {
        self.fail_next_write.store(fail, Ordering::SeqCst);
    }

    /// Atomically apply a `RedbWriteBatch` across separated tables and increment the durable commit sequence number.
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
            let mut state_table = write_txn
                .open_table(APP_STATE_TABLE)
                .map_err(|e| format!("failed to open cf_state table: {e}"))?;
            let mut receipts_table = write_txn
                .open_table(RECEIPTS_INDEX_TABLE)
                .map_err(|e| format!("failed to open cf_receipts table: {e}"))?;

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
                        if is_receipt_index_key(&key) {
                            receipts_table
                                .insert(key.as_slice(), val.as_slice())
                                .map_err(|e| format!("cf_receipts insert failed: {e}"))?;
                        } else if is_app_state_key(&key) {
                            state_table
                                .insert(key.as_slice(), val.as_slice())
                                .map_err(|e| format!("cf_state insert failed: {e}"))?;
                        }
                    }
                    None => {
                        let _ = table
                            .remove(key.as_slice())
                            .map_err(|e| format!("remove failed: {e}"))?;
                        if is_receipt_index_key(&key) {
                            let _ = receipts_table
                                .remove(key.as_slice())
                                .map_err(|e| format!("cf_receipts remove failed: {e}"))?;
                        } else if is_app_state_key(&key) {
                            let _ = state_table
                                .remove(key.as_slice())
                                .map_err(|e| format!("cf_state remove failed: {e}"))?;
                        }
                    }
                }
            }

            if self.fail_next_write.swap(false, Ordering::SeqCst) {
                // Abort transaction before commit to simulate crash/flush failure at commit boundary
                drop(receipts_table);
                drop(state_table);
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

    /// Create a verifiable deterministic snapshot from `cf_state` and `cf_receipts` and persist it in `cf_snapshots`.
    pub fn create_snapshot(&mut self, chain_id: u64) -> Result<StateSnapshot, String> {
        let read_txn = self
            .db
            .begin_read()
            .map_err(|e| format!("failed to begin read txn: {e}"))?;
        let state_table = read_txn
            .open_table(APP_STATE_TABLE)
            .map_err(|e| format!("failed to open cf_state: {e}"))?;
        let receipts_table = read_txn
            .open_table(RECEIPTS_INDEX_TABLE)
            .map_err(|e| format!("failed to open cf_receipts: {e}"))?;

        let mut state_entries = Vec::new();
        for entry in state_table.iter().map_err(|e| format!("{e}"))? {
            let (k, v) = entry.map_err(|e| format!("{e}"))?;
            state_entries.push((k.value().to_vec(), v.value().to_vec()));
        }
        state_entries.sort_by(|a, b| a.0.cmp(&b.0));

        let mut receipt_entries = Vec::new();
        for entry in receipts_table.iter().map_err(|e| format!("{e}"))? {
            let (k, v) = entry.map_err(|e| format!("{e}"))?;
            receipt_entries.push((k.value().to_vec(), v.value().to_vec()));
        }
        receipt_entries.sort_by(|a, b| a.0.cmp(&b.0));
        drop(receipts_table);
        drop(state_table);
        drop(read_txn);

        let mut all_refs: Vec<(&[u8], &[u8])> =
            Vec::with_capacity(state_entries.len().saturating_add(receipt_entries.len()));
        for (k, v) in &state_entries {
            all_refs.push((k.as_slice(), v.as_slice()));
        }
        for (k, v) in &receipt_entries {
            all_refs.push((k.as_slice(), v.as_slice()));
        }
        let state_root = compute_state_commitment(&all_refs);
        let commit_sequence = self.commit_sequence();
        let integrity_hash = StateSnapshot::compute_integrity_hash(
            STORAGE_SCHEMA_VERSION,
            chain_id,
            commit_sequence,
            &state_root,
            &state_entries,
            &receipt_entries,
        );

        let snapshot = StateSnapshot {
            schema_version: STORAGE_SCHEMA_VERSION,
            chain_id,
            commit_sequence,
            state_root,
            state_entries,
            receipt_entries,
            integrity_hash,
        };

        let encoded = serde_json::to_vec(&snapshot)
            .map_err(|e| format!("failed to serialize snapshot: {e}"))?;
        let write_txn = self
            .db
            .begin_write()
            .map_err(|e| format!("failed to begin write txn for snapshot: {e}"))?;
        {
            let mut snap_table = write_txn
                .open_table(SNAPSHOTS_TABLE)
                .map_err(|e| format!("failed to open cf_snapshots: {e}"))?;
            snap_table
                .insert(commit_sequence.to_be_bytes().as_slice(), encoded.as_slice())
                .map_err(|e| format!("failed to insert snapshot: {e}"))?;
        }
        write_txn
            .commit()
            .map_err(|e| format!("failed to commit snapshot: {e}"))?;

        Ok(snapshot)
    }

    /// Restore a verified `StateSnapshot` fail-closed into the database.
    pub fn restore_snapshot(
        &mut self,
        snapshot: &StateSnapshot,
        expected_chain_id: u64,
    ) -> Result<(), String> {
        snapshot.verify(expected_chain_id)?;
        let mut batch = RedbWriteBatch::new();
        for (k, v) in &snapshot.state_entries {
            batch.set(k, v);
        }
        for (k, v) in &snapshot.receipt_entries {
            batch.set(k, v);
        }
        self.try_write(batch)?;
        Ok(())
    }

    /// Prune non-state/non-metadata keys matching `prefix` while preserving finalized state,
    /// receipt indexes, and recovery metadata.
    pub fn prune_keys_with_prefix(&mut self, prefix: &[u8]) -> Result<usize, String> {
        if is_app_state_key(prefix) || is_receipt_index_key(prefix) || prefix.starts_with(b"meta:")
        {
            return Err(
                "refusing to prune protected state, receipt, or metadata prefix".to_string(),
            );
        }

        let read_txn = self
            .db
            .begin_read()
            .map_err(|e| format!("failed to begin read txn: {e}"))?;
        let table = read_txn
            .open_table(BLOCKTREE_TABLE)
            .map_err(|e| format!("failed to open blocktree table: {e}"))?;
        let mut to_remove = Vec::new();
        for entry in table.iter().map_err(|e| format!("{e}"))? {
            let (k, _) = entry.map_err(|e| format!("{e}"))?;
            let key_bytes = k.value();
            if key_bytes.starts_with(prefix)
                && !is_app_state_key(key_bytes)
                && !is_receipt_index_key(key_bytes)
                && !key_bytes.starts_with(b"meta:")
            {
                to_remove.push(key_bytes.to_vec());
            }
        }
        drop(table);
        drop(read_txn);

        let count = to_remove.len();
        if count > 0 {
            let mut batch = RedbWriteBatch::new();
            for k in to_remove {
                batch.delete(&k);
            }
            self.try_write(batch)?;
        }
        Ok(count)
    }

    /// Explicit, fail-closed schema migration check. Never performs destructive automatic repair.
    pub fn migrate_schema(&mut self, from_version: u32, to_version: u32) -> Result<(), String> {
        let meta = read_recovery_metadata(self)?.ok_or_else(|| {
            "cannot migrate uninitialized store (missing recovery metadata)".to_string()
        })?;
        if meta.schema_version != from_version {
            return Err(format!(
                "migration source version mismatch: store is at v{}, requested from v{}",
                meta.schema_version, from_version
            ));
        }
        if to_version != STORAGE_SCHEMA_VERSION {
            return Err(format!(
                "unsupported target schema version v{} (supported: v{})",
                to_version, STORAGE_SCHEMA_VERSION
            ));
        }
        Ok(())
    }
}

/// Write recovery metadata into any `KVStore`.
pub fn write_recovery_metadata<K: KVStore>(
    store: &mut K,
    meta: &RecoveryMetadata,
) -> Result<(), String> {
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
    let meta = read_recovery_metadata(store)?.ok_or_else(|| {
        "missing recovery metadata in store (uninitialized or incomplete genesis)".to_string()
    })?;

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
        get_in(&self.db.begin_read().ok()?, key)
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
        // `KVStore::write` cannot return an error, so a failure must stop the replica thread.
        // Halt first so the fault is visible (`/health` 503, process exit) instead of silent.
        if let Err(err) = self.try_write(write_batch) {
            if let Some(halt) = &self.safety_halt {
                halt.trigger(format!("chain store write failed: {err}"));
            }
            panic!("failed to commit durable write batch: {err}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_write_failure_triggers_safety_halt_before_panicking() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let halt = SafetyHalt::new();
        let mut store = RedbKVStore::open(tmp.path())
            .unwrap()
            .with_safety_halt(halt.clone());
        let mut ok_batch = RedbWriteBatch::new();
        ok_batch.set(b"k", b"v");
        store.write(ok_batch);
        assert!(!halt.is_halted(), "successful writes do not halt");

        store.set_fail_next_write(true);
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            store.write(RedbWriteBatch::new());
        }));

        assert!(outcome.is_err(), "write failure must still stop the caller");
        assert!(
            halt.reason()
                .is_some_and(|r| r.contains("chain store write failed"))
        );
    }

    #[test]
    fn test_write_failure_without_halt_handle_still_panics() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut store = RedbKVStore::open(tmp.path()).unwrap();
        store.set_fail_next_write(true);
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            store.write(RedbWriteBatch::new());
        }));
        assert!(outcome.is_err());
    }

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

    #[test]
    fn test_redb_separated_tables_snapshot_restore_and_corruption_rejection() {
        let tmp1 = tempfile::NamedTempFile::new().unwrap();
        let mut store1 = RedbKVStore::open(tmp1.path()).unwrap();

        let mut batch = RedbWriteBatch::new();
        batch.set(b"acc:alice", b"{\"balance\":5000,\"nonce\":1}");
        batch.set(b"worker:w1", b"{\"worker_id\":\"w1\"}");
        batch.set(b"task:t1", b"settled");
        batch.set(b"temp:fork:1", b"stale_branch");
        store1.write(batch);

        let snapshot = store1.create_snapshot(1337).unwrap();
        assert!(snapshot.verify(1337).is_ok());
        assert_eq!(snapshot.state_entries.len(), 2);
        assert_eq!(snapshot.receipt_entries.len(), 1);

        // Wrong chain_id fails snapshot verification
        assert!(snapshot.verify(9999).is_err());

        // Tampered state_entries fails state_root and integrity check
        let mut tampered = snapshot.clone();
        tampered.state_entries[0].1 = b"{\"balance\":999999,\"nonce\":1}".to_vec();
        assert!(tampered.verify(1337).is_err());

        // Restore valid snapshot into a fresh store
        let tmp2 = tempfile::NamedTempFile::new().unwrap();
        let mut store2 = RedbKVStore::open(tmp2.path()).unwrap();
        assert!(store2.restore_snapshot(&tampered, 1337).is_err());
        store2.restore_snapshot(&snapshot, 1337).unwrap();

        assert_eq!(
            store2.get(b"acc:alice"),
            Some(b"{\"balance\":5000,\"nonce\":1}".to_vec())
        );
        assert_eq!(store2.get(b"task:t1"), Some(b"settled".to_vec()));
    }

    #[test]
    fn test_redb_pruning_and_schema_migration_fail_closed() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut store = RedbKVStore::open(tmp.path()).unwrap();

        let meta = RecoveryMetadata {
            schema_version: STORAGE_SCHEMA_VERSION,
            chain_id: 1337,
            genesis_hash: [1u8; 32],
        };
        write_recovery_metadata(&mut store, &meta).unwrap();

        let mut batch = RedbWriteBatch::new();
        batch.set(b"acc:alice", b"1000");
        batch.set(b"task:t1", b"settled");
        batch.set(b"stale_branch:1", b"old_block_1");
        batch.set(b"stale_branch:2", b"old_block_2");
        store.write(batch);

        // Protected prefixes cannot be pruned
        assert!(store.prune_keys_with_prefix(b"acc:").is_err());
        assert!(store.prune_keys_with_prefix(b"task:").is_err());
        assert!(store.prune_keys_with_prefix(b"meta:").is_err());

        // Pruning ephemeral branch prefix removes only non-state keys
        let pruned = store.prune_keys_with_prefix(b"stale_branch:").unwrap();
        assert_eq!(pruned, 2);
        assert_eq!(store.get(b"stale_branch:1"), None);
        assert_eq!(store.get(b"acc:alice"), Some(b"1000".to_vec()));
        assert_eq!(store.get(b"task:t1"), Some(b"settled".to_vec()));

        // Schema migration checks
        assert!(
            store
                .migrate_schema(STORAGE_SCHEMA_VERSION, STORAGE_SCHEMA_VERSION)
                .is_ok()
        );
        assert!(store.migrate_schema(2, STORAGE_SCHEMA_VERSION).is_err());
        assert!(store.migrate_schema(STORAGE_SCHEMA_VERSION, 99).is_err());
    }
}
