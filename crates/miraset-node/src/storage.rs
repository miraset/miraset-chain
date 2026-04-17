use anyhow::Result;
use miraset_core::{Address, Block, Event};
use sled::Db;
use std::path::Path;
use std::sync::Arc;

/// Persistent storage using Sled (pure Rust embedded database)
#[derive(Clone)]
pub struct Storage {
    db: Arc<Db>,
}

impl Storage {
    /// Open or create a new storage at the given path
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let db = sled::open(path)?;
        Ok(Self { db: Arc::new(db) })
    }

    /// Save a block
    pub fn save_block(&self, block: &Block) -> Result<()> {
        let key = format!("block:{}", block.height);
        let value = bincode::serialize(block)?;
        self.db.insert(key.as_bytes(), value)?;

        // Also save latest block pointer
        self.db.insert(b"latest_block", &block.height.to_le_bytes())?;

        Ok(())
    }

    /// Get a block by height
    pub fn get_block(&self, height: u64) -> Result<Option<Block>> {
        let key = format!("block:{}", height);
        match self.db.get(key.as_bytes())? {
            Some(bytes) => {
                let block = bincode::deserialize(&bytes)?;
                Ok(Some(block))
            }
            None => Ok(None),
        }
    }

    /// Get the latest block
    pub fn get_latest_block(&self) -> Result<Option<Block>> {
        match self.db.get(b"latest_block")? {
            Some(bytes) => {
                let height = u64::from_le_bytes(bytes.as_ref().try_into().unwrap());
                self.get_block(height)
            }
            None => Ok(None),
        }
    }

    /// Save account balance
    pub fn save_balance(&self, address: &Address, balance: u64) -> Result<()> {
        let key = format!("balance:{}", address.to_hex());
        self.db.insert(key.as_bytes(), &balance.to_le_bytes())?;
        Ok(())
    }

    /// Get account balance
    pub fn get_balance(&self, address: &Address) -> Result<u64> {
        let key = format!("balance:{}", address.to_hex());
        match self.db.get(key.as_bytes())? {
            Some(bytes) => Ok(u64::from_le_bytes(bytes.as_ref().try_into().unwrap())),
            None => Ok(0),
        }
    }

    /// Save account nonce
    pub fn save_nonce(&self, address: &Address, nonce: u64) -> Result<()> {
        let key = format!("nonce:{}", address.to_hex());
        self.db.insert(key.as_bytes(), &nonce.to_le_bytes())?;
        Ok(())
    }

    /// Get account nonce
    pub fn get_nonce(&self, address: &Address) -> Result<u64> {
        let key = format!("nonce:{}", address.to_hex());
        match self.db.get(key.as_bytes())? {
            Some(bytes) => Ok(u64::from_le_bytes(bytes.as_ref().try_into().unwrap())),
            None => Ok(0),
        }
    }

    /// Save an event
    pub fn save_event(&self, index: u64, event: &Event) -> Result<()> {
        let key = format!("event:{}", index);
        let value = serde_json::to_vec(event)?; // Use JSON for tagged enums
        self.db.insert(key.as_bytes(), value)?;

        // Update event counter
        self.db.insert(b"event_count", &index.to_le_bytes())?;

        Ok(())
    }

    /// Get events in range
    pub fn get_events(&self, from: u64, limit: usize) -> Result<Vec<Event>> {
        let mut events = Vec::new();

        for i in from.. {
            if events.len() >= limit {
                break;
            }

            let key = format!("event:{}", i);
            match self.db.get(key.as_bytes())? {
                Some(bytes) => {
                    let event: Event = serde_json::from_slice(&bytes)?; // Use JSON
                    events.push(event);
                }
                None => break,
            }
        }

        Ok(events)
    }

    /// Get total number of events
    pub fn get_event_count(&self) -> Result<u64> {
        match self.db.get(b"event_count")? {
            Some(bytes) => Ok(u64::from_le_bytes(bytes.as_ref().try_into().unwrap())),
            None => Ok(0),
        }
    }

    /// Flush all pending writes
    pub fn flush(&self) -> Result<()> {
        self.db.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use miraset_core::KeyPair;
    use tempfile::TempDir;

    #[test]
    fn test_storage_open() {
        let temp_dir = TempDir::new().unwrap();
        let storage = Storage::open(temp_dir.path()).unwrap();
        drop(storage);
    }

    #[test]
    fn test_save_and_load_block() {
        let temp_dir = TempDir::new().unwrap();
        let storage = Storage::open(temp_dir.path()).unwrap();

        let block = Block::genesis();
        storage.save_block(&block).unwrap();

        let loaded = storage.get_block(0).unwrap().unwrap();
        assert_eq!(loaded.height, 0);
    }

    #[test]
    fn test_balance_persistence() {
        let temp_dir = TempDir::new().unwrap();
        let storage = Storage::open(temp_dir.path()).unwrap();

        let kp = KeyPair::generate();
        let addr = kp.address();

        storage.save_balance(&addr, 1000).unwrap();
        let balance = storage.get_balance(&addr).unwrap();

        assert_eq!(balance, 1000);
    }

    #[test]
    fn test_nonce_persistence() {
        let temp_dir = TempDir::new().unwrap();
        let storage = Storage::open(temp_dir.path()).unwrap();

        let kp = KeyPair::generate();
        let addr = kp.address();

        storage.save_nonce(&addr, 5).unwrap();
        let nonce = storage.get_nonce(&addr).unwrap();

        assert_eq!(nonce, 5);
    }

    #[test]
    fn test_event_persistence() {
        let temp_dir = TempDir::new().unwrap();
        let storage = Storage::open(temp_dir.path()).unwrap();

        let kp = KeyPair::generate();
        let event = Event::Transferred {
            from: kp.address(),
            to: kp.address(),
            amount: 100,
            tx_hash: [1; 32],
            block_height: 1,
        };

        storage.save_event(0, &event).unwrap();

        let events = storage.get_events(0, 10).unwrap();
        assert_eq!(events.len(), 1);
    }
}
