use hotstuff_rs::block_tree::pluggables::{KVGet, KVStore, WriteBatch};
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

/// In-memory implementation of WriteBatch.
#[derive(Debug, Default, Clone)]
pub struct MemWriteBatch {
    operations: Vec<(Vec<u8>, Option<Vec<u8>>)>,
}

impl WriteBatch for MemWriteBatch {
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

/// Point-in-time snapshot of the in-memory KV store.
#[derive(Clone, Debug)]
pub struct MemSnapshot {
    data: Arc<BTreeMap<Vec<u8>, Vec<u8>>>,
}

impl KVGet for MemSnapshot {
    fn get(&self, key: &[u8]) -> Option<Vec<u8>> {
        self.data.get(key).cloned()
    }
}

/// Thread-safe in-memory KVStore for HotStuff-rs consensus and state machine.
#[derive(Clone, Debug, Default)]
pub struct MemKVStore {
    state: Arc<RwLock<BTreeMap<Vec<u8>, Vec<u8>>>>,
}

impl MemKVStore {
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(BTreeMap::new())),
        }
    }

    pub fn insert_direct(&self, key: &[u8], value: &[u8]) {
        let mut lock = self.state.write().expect("lock poisoned");
        lock.insert(key.to_vec(), value.to_vec());
    }

    pub fn len(&self) -> usize {
        self.state.read().expect("lock poisoned").len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl KVGet for MemKVStore {
    fn get(&self, key: &[u8]) -> Option<Vec<u8>> {
        let lock = self.state.read().expect("lock poisoned");
        lock.get(key).cloned()
    }
}

impl KVStore for MemKVStore {
    type WriteBatch = MemWriteBatch;
    type Snapshot<'a>
        = MemSnapshot
    where
        Self: 'a;

    fn clear(&mut self) {
        let mut lock = self.state.write().expect("lock poisoned");
        lock.clear();
    }

    fn snapshot<'a>(&'a self) -> Self::Snapshot<'a> {
        let lock = self.state.read().expect("lock poisoned");
        MemSnapshot {
            data: Arc::new(lock.clone()),
        }
    }

    fn write(&mut self, write_batch: Self::WriteBatch) {
        let mut lock = self.state.write().expect("lock poisoned");
        for (key, val_opt) in write_batch.operations {
            match val_opt {
                Some(val) => {
                    lock.insert(key, val);
                }
                None => {
                    lock.remove(&key);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mem_store_basic_operations() {
        let mut store = MemKVStore::new();
        assert!(store.is_empty());

        let mut batch = MemWriteBatch::new();
        batch.set(b"key1", b"val1");
        batch.set(b"key2", b"val2");
        store.write(batch);

        assert_eq!(store.get(b"key1"), Some(b"val1".to_vec()));
        assert_eq!(store.get(b"key2"), Some(b"val2".to_vec()));
        assert_eq!(store.get(b"nonexistent"), None);
        assert_eq!(store.len(), 2);

        let mut batch_del = MemWriteBatch::new();
        batch_del.delete(b"key1");
        store.write(batch_del);

        assert_eq!(store.get(b"key1"), None);
        assert_eq!(store.get(b"key2"), Some(b"val2".to_vec()));
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn test_mem_snapshot_isolation() {
        let mut store = MemKVStore::new();
        let mut batch = MemWriteBatch::new();
        batch.set(b"foo", b"bar");
        store.write(batch);

        let snap = store.snapshot();
        assert_eq!(snap.get(b"foo"), Some(b"bar".to_vec()));

        let mut batch_update = MemWriteBatch::new();
        batch_update.set(b"foo", b"baz");
        batch_update.set(b"new_key", b"123");
        store.write(batch_update);

        // Snapshot retains original value
        assert_eq!(snap.get(b"foo"), Some(b"bar".to_vec()));
        assert_eq!(snap.get(b"new_key"), None);

        // Store reflects updated value
        assert_eq!(store.get(b"foo"), Some(b"baz".to_vec()));
        assert_eq!(store.get(b"new_key"), Some(b"123".to_vec()));
    }

    #[test]
    fn test_mem_store_clear() {
        let mut store = MemKVStore::new();
        let mut batch = MemWriteBatch::new();
        batch.set(b"a", b"1");
        batch.set(b"b", b"2");
        store.write(batch);
        assert_eq!(store.len(), 2);

        store.clear();
        assert_eq!(store.len(), 0);
        assert_eq!(store.get(b"a"), None);
    }
}
