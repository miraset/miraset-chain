//! MIRASET PATCH #21 regression tests (`MIRASET_PATCHES.md`): a sync server escapes blacklisting
//! only when the session made real progress (blocks inserted or committed), never by a client-side
//! continuation alone (8.5.3 review: a server answering with one known block and then an empty
//! response must still be blacklisted, as upstream did).

use super::*;

#[test]
fn test_an_incorrect_block_is_always_blacklisted() {
    assert!(blacklists(SessionEnd::IncorrectBlock, false));
    assert!(blacklists(SessionEnd::IncorrectBlock, true));
}

#[test]
fn test_an_unsafe_block_is_blacklisted_unless_the_session_made_progress() {
    // Upstream: always blacklisted. Patch #10: not after our own blocks lost a fork.
    assert!(blacklists(SessionEnd::UnsafeBlock, false));
    assert!(!blacklists(SessionEnd::UnsafeBlock, true));
}

#[test]
fn test_known_blocks_then_nothing_is_blacklisted_without_progress() {
    // Review scenario A: known blocks only (no insert, no commit), then an empty response.
    assert!(blacklists(
        SessionEnd::NoNewBlocks {
            below_commitment: true
        },
        false
    ));
}

#[test]
fn test_no_new_blocks_after_progress_or_within_the_commitment_is_not_blacklisted() {
    assert!(!blacklists(
        SessionEnd::NoNewBlocks {
            below_commitment: true
        },
        true
    ));
    assert!(!blacklists(
        SessionEnd::NoNewBlocks {
            below_commitment: false
        },
        false
    ));
}

#[test]
fn test_a_missing_response_below_the_commitment_is_blacklisted_as_upstream() {
    assert!(blacklists(
        SessionEnd::NoResponse {
            below_commitment: true
        },
        true
    ));
    assert!(!blacklists(
        SessionEnd::NoResponse {
            below_commitment: false
        },
        false
    ));
}
