//! Regression tests for the vendored `hotstuff_rs` `KVGet::block_data` fix
//! (`lib/hotstuff_rs/MIRASET_PATCHES.md`): upstream v0.4.0 returned empty block data, which
//! broke block sync and commit-time payload reads.

use crate::storage::MemKVStore;
use hotstuff_rs::block_tree::pluggables::{KVGet, KVGetError, KVStore};
use hotstuff_rs::block_tree::variables::{self, concat};
use hotstuff_rs::types::data_types::{CryptoHash, Data};

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

    assert_eq!(datum_bytes(store.block_data(&block).unwrap()), expected);
    // Snapshots (used by block sync and commit handlers) must agree.
    assert_eq!(
        datum_bytes(store.snapshot().block_data(&block).unwrap()),
        expected
    );
}

#[test]
fn test_block_data_missing_block_and_torn_write() {
    let block = CryptoHash::new([8u8; 32]);
    assert!(MemKVStore::new().block_data(&block).unwrap().is_none());

    // Length declares two datums but only one was written.
    let store = store_block_data(&block, &[b"a"], 2);
    assert!(matches!(
        store.block_data(&block),
        Err(KVGetError::ValueExpectedButNotFound { .. })
    ));
}
