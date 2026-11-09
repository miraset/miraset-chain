//! End-to-end: the real `miraset-chain` validator binary as a separate process (TODO 8.3.2).
//!
//! Each test creates a one-validator devnet with `devnet-init`, runs the binary on free local
//! ports, observes it only through HTTP (`/health`, `/block/latest`) and its log output, and
//! crashes it with SIGKILL (no graceful shutdown).
//!
//! Not covered here: the exit-code-3 safety halt. It needs an injected guard or storage fault,
//! which the production binary deliberately does not expose. It is covered in-process by
//! `tests/vote_safety.rs` (T5) and `tests/consensus_panic_halt.rs`.

mod common;

use common::e2e::*;
use std::fs;

#[test]
fn test_e2e_guard_survives_sigkill_and_chain_continues() {
    let mut v = Validator::init();
    let first = v.start(&[]);
    wait_healthy_with_height(&v, &first, MIN_HEIGHT_BEFORE_CRASH, STARTUP_TIMEOUT);
    assert_exists(&v.guard_file());
    let height_before = latest_height(v.rpc).unwrap();
    let first_log = first.kill();
    assert_eq!(
        last_signed_view(&first_log),
        None,
        "fresh guard has no record"
    );

    let second = v.start(&[]);
    wait_healthy_with_height(&v, &second, height_before + 1, STARTUP_TIMEOUT);
    let view = last_signed_view(&second.log_text()).expect("banner shows last signed view");
    assert!(view > 0, "guard kept the votes signed before SIGKILL");
    assert!(!second.log_text().contains("SAFETY HALT"));
}

#[test]
fn test_e2e_missing_guard_with_chain_data_refuses_start_until_init() {
    let mut v = Validator::init();
    let first = v.start(&[]);
    wait_healthy_with_height(&v, &first, MIN_HEIGHT_BEFORE_CRASH, STARTUP_TIMEOUT);
    first.kill();
    fs::remove_file(v.guard_file()).expect("delete guard");

    let mut refused = v.start(&[]);
    let status = refused
        .wait_exit(STARTUP_TIMEOUT)
        .expect("validator must exit instead of running without its guard");
    assert_eq!(status.code(), Some(1));
    let log = refused.log_text();
    assert!(log.contains("signing guard not found"), "{log}");
    assert!(log.contains("--init-signing-guard"), "{log}");
    assert!(
        !v.guard_file().exists(),
        "refused start must not create a guard"
    );

    let initialized = v.start(&["--init-signing-guard"]);
    let height = latest_height_after_start(&v, &initialized);
    wait_healthy_with_height(&v, &initialized, height + 1, STARTUP_TIMEOUT);
    assert_exists(&v.guard_file());
}

/// Key custody D4: the running validator has a zero core limit and is not dumpable, so other
/// processes of the same user cannot read its memory or environment through `/proc`.
#[cfg(target_os = "linux")]
#[test]
fn test_e2e_validator_process_is_hardened() {
    let mut v = Validator::init();
    let run = v.start(&[]);
    wait_healthy_with_height(&v, &run, 1, STARTUP_TIMEOUT);
    let pid = run.child.id();

    let limits = fs::read_to_string(format!("/proc/{pid}/limits")).expect("limits readable");
    let core = limits
        .lines()
        .find(|line| line.starts_with("Max core file size"))
        .expect("core limit line");
    assert_eq!(
        core.split_whitespace().skip(4).take(2).collect::<Vec<_>>(),
        ["0", "0"],
        "{core}"
    );

    // A non-dumpable process's /proc entries belong to root, so even its own user cannot read
    // its environment or memory. Root bypasses this, so only assert as a normal user.
    if miraset_chain::keys::effective_uid() != 0 {
        let environ = fs::read(format!("/proc/{pid}/environ"));
        assert!(
            environ.is_err(),
            "same-user process could read the validator's environment"
        );
    }
}

#[test]
fn test_e2e_wiped_chain_data_with_guard_kept_recovers_without_resigning() {
    let mut v = Validator::init();
    let first = v.start(&[]);
    wait_healthy_with_height(&v, &first, MIN_HEIGHT_BEFORE_CRASH, STARTUP_TIMEOUT);
    first.kill();
    fs::remove_file(v.chain_file()).expect("wipe chain data");

    let second = v.start(&[]);
    let guard_view = last_signed_view(&{
        let mut text = String::new();
        wait_for("startup banner", STARTUP_TIMEOUT, &second, || {
            text = second.log_text();
            text.contains("last signed view")
        });
        text
    })
    .expect("guard record kept across the wipe");
    assert!(guard_view > 0);

    // The fresh chain restarts from genesis: old views are refused until views pass the guard
    // record, then the validator votes and finalizes again.
    wait_healthy_with_height(&v, &second, 1, RECOVERY_TIMEOUT);
    let log = second.log_text();
    assert!(
        log.contains("stale phase vote"),
        "guard should refuse votes for old views; log:\n{log}"
    );
    assert!(!log.contains("SAFETY HALT"), "{log}");
    assert_eq!(
        health(v.rpc).map(|(code, _)| code),
        Some(200),
        "stale refusals must not halt"
    );
}
