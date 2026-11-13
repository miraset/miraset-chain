//! MIRASET PATCH #15 regression test (`MIRASET_PATCHES.md`): a Block Sync Response stops before
//! the block that would take it past the byte limit, but always carries at least one block.

use crate::block_tree::accessors::public::ResponseBytes;
use crate::hotstuff::types::PhaseCertificate;
use crate::types::block::Block;
use crate::types::data_types::{BlockHeight, CryptoHash, Data, Datum};

fn block(height: u64, data_len: usize) -> Block {
    Block::new(
        BlockHeight::new(height),
        PhaseCertificate::genesis_pc(),
        CryptoHash::new([height as u8; 32]),
        Data::new(vec![Datum::new(vec![0; data_len])]),
    )
}

/// How many of five 100-byte blocks a response with `limit` admits.
fn admitted(limit: u64) -> usize {
    let mut bytes = ResponseBytes::new(limit);
    (0..5).map(|h| block(h, 100)).take_while(|b| bytes.admit(b)).count()
}

#[test]
fn test_response_blocks_stop_at_the_byte_limit() {
    assert_eq!(admitted(u64::MAX), 5, "no cap by default");
    assert_eq!(admitted(300), 3, "exactly at the limit fits");
    assert_eq!(admitted(299), 2);
    assert_eq!(admitted(10), 1, "one block even over the limit");
}
