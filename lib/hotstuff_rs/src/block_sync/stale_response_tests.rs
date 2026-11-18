//! MIRASET PATCH #21 regression tests (`MIRASET_PATCHES.md`): a sync response answers a request
//! unless it starts above the requested height. A late answer to an earlier request from a lower
//! height still carries the requested blocks (its known prefix is skipped); one from above our
//! blocks cannot be inserted and is skipped.

use super::*;
use crate::hotstuff::types::PhaseCertificate;
use crate::types::data_types::{CryptoHash, Data, Datum};

fn response(heights: &[u64]) -> BlockSyncResponse {
    BlockSyncResponse {
        blocks: heights
            .iter()
            .map(|&h| {
                Block::new(
                    BlockHeight::new(h),
                    PhaseCertificate::genesis_pc(),
                    CryptoHash::new([h as u8; 32]),
                    Data::new(vec![Datum::new(vec![])]),
                )
            })
            .collect(),
        highest_pc: PhaseCertificate::genesis_pc(),
    }
}

#[test]
fn test_a_response_starting_at_the_requested_height_answers_it() {
    assert!(answers_request(
        &response(&[10, 11, 12]),
        BlockHeight::new(10)
    ));
}

#[test]
fn test_a_late_response_from_a_lower_height_answers_it() {
    // Measured (8.5.3): one ~4 MB block took longer than the timeout; its late response arrived
    // while the next session asked from the committed height again.
    assert!(answers_request(
        &response(&[7, 8, 9, 10, 11]),
        BlockHeight::new(10)
    ));
    assert!(answers_request(&response(&[7, 8, 9]), BlockHeight::new(10)));
}

#[test]
fn test_a_response_starting_above_the_requested_height_does_not_answer_it() {
    assert!(!answers_request(&response(&[12]), BlockHeight::new(10)));
}

#[test]
fn test_an_empty_response_answers_any_request() {
    assert!(answers_request(&response(&[]), BlockHeight::new(10)));
}
