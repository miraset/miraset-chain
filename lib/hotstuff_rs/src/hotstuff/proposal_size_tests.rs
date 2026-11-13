//! MIRASET PATCH #16 regression test (`MIRASET_PATCHES.md`): a buffered proposal is counted
//! with its block data, so the progress-message buffer bounds memory in bytes.

use super::*;
use crate::types::block::Block;
use crate::types::data_types::{BlockHeight, ChainID, CryptoHash, Data, Datum, ViewNumber};

fn proposal(data_len: usize) -> HotStuffMessage {
    HotStuffMessage::Proposal(Proposal {
        chain_id: ChainID::new(1),
        view: ViewNumber::new(1),
        block: Block::new(
            BlockHeight::new(1),
            PhaseCertificate::genesis_pc(),
            CryptoHash::new([1; 32]),
            Data::new(vec![Datum::new(vec![0; data_len])]),
        ),
    })
}

#[test]
fn test_proposal_size_counts_block_data() {
    let empty = proposal(0).size();
    // Upstream returned the same struct size for both.
    assert_eq!(proposal(8 << 20).size(), empty + (8 << 20));
}
