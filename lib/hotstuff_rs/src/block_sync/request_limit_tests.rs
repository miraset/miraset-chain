//! MIRASET PATCH #19 regression tests (`MIRASET_PATCHES.md`): the request limit halves on every
//! response timeout, down to one block, and doubles back to the configured limit on responses
//! with new blocks; at one block, timeouts lengthen the response wait.

use super::*;

#[test]
fn test_request_limit_starts_at_the_configured_limit() {
    assert_eq!(AdaptiveRequestLimit::new(50).get(), 50);
}

#[test]
fn test_timeouts_halve_the_request_limit_down_to_one_block() {
    let mut limit = AdaptiveRequestLimit::new(50);
    limit.on_timeout();
    assert_eq!(limit.get(), 25);
    limit.on_timeout();
    assert_eq!(limit.get(), 12);
    for _ in 0..10 {
        limit.on_timeout();
    }
    assert_eq!(limit.get(), 1);
}

#[test]
fn test_responses_double_the_request_limit_back_to_the_configured_limit() {
    let mut limit = AdaptiveRequestLimit::new(50);
    for _ in 0..10 {
        limit.on_timeout();
    }
    limit.on_response();
    assert_eq!(limit.get(), 2);
    for _ in 0..10 {
        limit.on_response();
    }
    assert_eq!(limit.get(), 50);
}

#[test]
fn test_a_zero_configured_limit_still_requests_one_block() {
    let mut limit = AdaptiveRequestLimit::new(0);
    assert_eq!(limit.get(), 1);
    limit.on_response();
    assert_eq!(limit.get(), 1);
}

#[test]
fn test_the_response_timeout_doubles_only_once_the_limit_is_one_block() {
    let base = Duration::from_millis(1500);
    let mut limit = AdaptiveRequestLimit::new(4);
    assert_eq!(limit.response_timeout(base), base);
    limit.on_timeout(); // 2 blocks
    limit.on_timeout(); // 1 block
    assert_eq!(limit.response_timeout(base), base);
    limit.on_timeout(); // one block did not arrive in time: wait longer
    assert_eq!(limit.response_timeout(base), base * 2);
    for _ in 0..10 {
        limit.on_timeout();
    }
    assert_eq!(
        limit.response_timeout(base),
        base * MAX_RESPONSE_TIMEOUT_FACTOR
    );
    assert_eq!(limit.get(), 1);
}

#[test]
fn test_a_response_restores_the_configured_timeout() {
    let base = Duration::from_millis(1500);
    let mut limit = AdaptiveRequestLimit::new(1);
    limit.on_timeout();
    limit.on_timeout();
    assert_eq!(limit.response_timeout(base), base * 4);
    limit.on_response();
    assert_eq!(limit.response_timeout(base), base);
}

#[test]
fn test_a_session_is_capped_at_the_longest_response_wait() {
    let configured = Duration::from_millis(1500);
    let mut limit = AdaptiveRequestLimit::new(1);
    for _ in 0..10 {
        limit.on_timeout();
    }
    assert_eq!(limit.response_timeout(configured), Duration::from_secs(12));
    assert_eq!(session_time_cap(configured), Duration::from_secs(12));
}
