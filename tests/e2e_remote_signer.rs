//! End-to-end: `miraset-chain signer` and a validator started with `--remote-signer`, both as
//! real processes (TODO 8.3.4, key custody D3).

mod common;

use common::e2e::*;
use std::thread;
use std::time::{Duration, Instant};

const HALT_TIMEOUT: Duration = Duration::from_secs(30);
const SAFETY_HALT_EXIT_CODE: i32 = 3;

/// Wait for the signer's banner, printed after it binds. The socket path alone is not enough:
/// a SIGKILLed signer leaves its stale socket file behind.
fn wait_for_socket(v: &Validator, signer: &Running) {
    wait_for("signer listening", STARTUP_TIMEOUT, signer, || {
        v.signer_socket().exists() && signer.log_text().contains("Socket:")
    });
}

#[test]
fn test_e2e_validator_signs_through_remote_signer_and_holds_no_key() {
    let mut v = Validator::init();
    let signer = v.start_signer(&["--init-signing-guard"]);
    wait_for_socket(&v, &signer);
    let socket = v.signer_socket();
    let node = v.start_remote(&socket, &[]);
    wait_healthy_with_height(&v, &node, 2, STARTUP_TIMEOUT);

    // The validator was never given a key path, and the signer recorded the votes.
    let node_log = node.log_text();
    assert!(!node_log.contains("node0.key"), "{node_log}");
    node.kill();
    signer.kill();
    let restarted = v.start_signer(&[]);
    wait_for_socket(&v, &restarted);
    let view = last_signed_view(&restarted.log_text()).expect("signer banner");
    assert!(
        view > 0,
        "the signer's guard recorded the validator's votes"
    );
}

#[test]
fn test_e2e_validator_halts_when_its_signer_dies_and_recovers_after_restart() {
    let mut v = Validator::init();
    let signer = v.start_signer(&["--init-signing-guard"]);
    wait_for_socket(&v, &signer);
    let socket = v.signer_socket();
    let mut node = v.start_remote(&socket, &[]);
    wait_healthy_with_height(&v, &node, 2, STARTUP_TIMEOUT);
    let height_before = latest_height(v.rpc).unwrap();

    signer.kill();
    let mut saw_503 = false;
    let deadline = Instant::now() + HALT_TIMEOUT;
    let status = loop {
        if let Some(status) = node.child.try_wait().expect("poll validator") {
            break status;
        }
        saw_503 |=
            health(v.rpc).is_some_and(|(code, json)| code == 503 && json["status"] == "halted");
        assert!(
            Instant::now() < deadline,
            "validator kept running without its signer"
        );
        thread::sleep(Duration::from_millis(100));
    };
    assert_eq!(
        status.code(),
        Some(SAFETY_HALT_EXIT_CODE),
        "{}",
        node.log_text()
    );
    assert!(saw_503, "/health never reported the halt");

    let signer = v.start_signer(&[]);
    wait_for_socket(&v, &signer);
    let node = v.start_remote(&socket, &[]);
    wait_healthy_with_height(&v, &node, height_before + 1, STARTUP_TIMEOUT);
}

#[test]
fn test_e2e_signer_refuses_to_create_a_guard_implicitly() {
    let mut v = Validator::init();
    let mut signer = v.start_signer(&[]);
    let status = signer
        .wait_exit(STARTUP_TIMEOUT)
        .expect("signer must exit without a guard");
    assert_eq!(status.code(), Some(1));
    let log = signer.log_text();
    assert!(log.contains("signing guard not found"), "{log}");
    assert!(!v.signer_guard_file().exists());
}

#[test]
fn test_e2e_validator_rejects_a_signer_for_another_key() {
    let mut v = Validator::init();
    let signer = v.start_signer(&["--init-signing-guard"]);
    wait_for_socket(&v, &signer);
    let socket = v.signer_socket();
    let wrong = "11".repeat(32);
    let mut node = v.start_remote(&socket, &["--validator-pubkey", &wrong]);
    let status = node
        .wait_exit(STARTUP_TIMEOUT)
        .expect("validator must refuse a signer serving another key");
    assert_eq!(status.code(), Some(1));
    assert!(node.log_text().contains("expected"), "{}", node.log_text());
}
