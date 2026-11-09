//! Compatibility overrides for `hotstuff_rs` v0.4.0 `KVGet` provided methods.
//!
//! The upstream default `KVGet::block_data` checks every datum with `Iterator::find` and then
//! collects the *same, already-exhausted* iterator, so any block read back from the KV store has
//! empty `Data`. That silently breaks block sync (synced blocks carry no payload, so a lagging
//! replica can never catch up) and any commit-time reads of block payloads.
//!
//! Every `KVGet` implementation in this crate must override `block_data` with
//! [`read_block_data`] until a fixed upstream release is adopted.

use hotstuff_rs::block_tree::pluggables::{KVGet, KVGetError, Key};
use hotstuff_rs::block_tree::variables::{self, concat};
use hotstuff_rs::types::data_types::{CryptoHash, Data, Datum};

/// Read a block's data from `kv`, failing closed if any datum declared by its length is missing.
pub fn read_block_data<G: KVGet + ?Sized>(
    kv: &G,
    block: &CryptoHash,
) -> Result<Option<Data>, KVGetError> {
    let Some(len) = kv.block_data_len(block)? else {
        return Ok(None);
    };

    let prefix = concat(
        &variables::BLOCKS,
        &concat(&block.bytes(), &variables::BLOCK_DATA),
    );
    let data = (0..len.int())
        .map(|index| {
            let datum_key = concat(&prefix, &index.to_le_bytes());
            kv.get(&datum_key).map(Datum::new)
        })
        .collect::<Option<Vec<Datum>>>()
        .ok_or(KVGetError::ValueExpectedButNotFound {
            key: Key::BlockData { block: *block },
        })?;

    Ok(Some(Data::new(data)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::MemKVStore;
    use hotstuff_rs::block_tree::pluggables::KVStore;

    fn datum_prefix(block: &CryptoHash) -> Vec<u8> {
        concat(
            &variables::BLOCKS,
            &concat(&block.bytes(), &variables::BLOCK_DATA),
        )
    }

    /// Write `datums` using the hotstuff_rs key layout under a declared length of `declared_len`.
    fn store_block_data(block: &CryptoHash, datums: &[&[u8]], declared_len: u32) -> MemKVStore {
        let store = MemKVStore::new();
        let len_key = concat(
            &variables::BLOCKS,
            &concat(&block.bytes(), &variables::BLOCK_DATA_LEN),
        );
        store.insert_direct(&len_key, &declared_len.to_le_bytes());
        for (index, datum) in datums.iter().enumerate() {
            let index = u32::try_from(index).expect("test datum index fits u32");
            store.insert_direct(&concat(&datum_prefix(block), &index.to_le_bytes()), datum);
        }
        store
    }

    fn datum_bytes(data: Option<Data>) -> Option<Vec<Vec<u8>>> {
        data.map(|d| d.vec().iter().map(|x| x.bytes().clone()).collect())
    }

    #[test]
    fn test_block_data_round_trips_all_datums() {
        let block = CryptoHash::new([7u8; 32]);
        let store = store_block_data(&block, &[b"payload-0", &[1, 2]], 2);
        let expected = Some(vec![b"payload-0".to_vec(), vec![1, 2]]);

        assert_eq!(
            datum_bytes(read_block_data(&store, &block).unwrap()),
            expected
        );
        // The overridden trait methods used by hotstuff_rs internals must agree.
        assert_eq!(datum_bytes(store.block_data(&block).unwrap()), expected);
        assert_eq!(
            datum_bytes(store.snapshot().block_data(&block).unwrap()),
            expected
        );
    }

    #[test]
    fn test_block_data_missing_block_and_torn_write() {
        let block = CryptoHash::new([8u8; 32]);
        assert!(
            read_block_data(&MemKVStore::new(), &block)
                .unwrap()
                .is_none()
        );

        // Length declares two datums but only one was written.
        let store = store_block_data(&block, &[b"a"], 2);
        assert!(matches!(
            read_block_data(&store, &block),
            Err(KVGetError::ValueExpectedButNotFound { .. })
        ));
    }
}
