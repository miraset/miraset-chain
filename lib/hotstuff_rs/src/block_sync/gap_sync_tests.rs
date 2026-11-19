//! MIRASET PATCH #22 regression tests (`MIRASET_PATCHES.md`): an advertised PC for a block we do
//! not hold, at least two views above our highest PC, starts a sync at once.

use super::*;

#[test]
fn test_a_missed_certified_block_triggers_sync() {
    // 8.5.3 trace: highest PC at view 46, block of view 47 never arrived, PC of view 48 advertised.
    assert!(misses_certified_blocks(48, false, 46));
    assert!(misses_certified_blocks(60, false, 46));
}

#[test]
fn test_a_pc_one_view_ahead_of_its_proposal_does_not_trigger_sync() {
    assert!(!misses_certified_blocks(47, false, 46));
}

#[test]
fn test_a_known_or_older_block_does_not_trigger_sync() {
    assert!(!misses_certified_blocks(48, true, 46));
    assert!(!misses_certified_blocks(40, false, 46));
    assert!(!misses_certified_blocks(46, false, 46));
}

#[test]
fn test_view_arithmetic_does_not_overflow() {
    assert!(!misses_certified_blocks(u64::MAX, false, u64::MAX));
    assert!(misses_certified_blocks(u64::MAX, false, u64::MAX - 2));
}

#[test]
fn test_gap_sync_is_spaced_from_the_end_of_the_last_one_and_never_repeats_a_block() {
    let ended = Instant::now();
    let a = CryptoHash::new([1; 32]);
    let b = CryptoHash::new([2; 32]);
    assert!(gap_sync_allowed(None, &a, ended));
    let last = (ended, a);
    // Another block, but too soon after the last session ended.
    assert!(!gap_sync_allowed(
        Some(&last),
        &b,
        ended + Duration::from_millis(500)
    ));
    assert!(gap_sync_allowed(
        Some(&last),
        &b,
        ended + MIN_GAP_SYNC_INTERVAL
    ));
    // The same block never again through this trigger.
    assert!(!gap_sync_allowed(
        Some(&last),
        &a,
        ended + Duration::from_secs(60)
    ));
}
