//! End-to-end observability and lifecycle with the real binary (TODO 8.4.4, PRD R12 / AT12):
//! JSON logs, `/livez`, `/readyz`, `/metrics`, SIGTERM drain and no secret in the output.

mod common;

use common::e2e::{STARTUP_TIMEOUT, Validator, http_get, sigterm, wait_for};
use std::fs;
use std::time::Duration;

/// Long enough that the drain is observed well inside it, even on a loaded machine.
const DRAIN_MS: u64 = 8_000;
const DRAIN_OBSERVE_TIMEOUT: Duration = Duration::from_secs(5);
/// Longer than any test wait: only a second signal can end this drain in time.
const LONG_DRAIN_MS: &str = "60000";
const EARLY_EXIT_TIMEOUT: Duration = Duration::from_secs(10);
const EXIT_TIMEOUT: Duration = Duration::from_secs(20);

fn status_of(addr: std::net::SocketAddr, path: &str) -> Option<(u16, serde_json::Value)> {
    let (code, body) = http_get(addr, path)?;
    Some((code, serde_json::from_str(&body).ok()?))
}

#[test]
fn test_e2e_probes_metrics_json_logs_and_graceful_drain() {
    let mut v = Validator::init();
    // Trace level: dependencies (hyper, tonic, the hotstuff log bridge) log the most here.
    v.env.push(("RUST_LOG".into(), "trace".into()));
    let drain = DRAIN_MS.to_string();
    let mut run = v.start(&[
        "--init-signing-guard",
        "--log-format",
        "json",
        "--shutdown-drain-ms",
        &drain,
    ]);

    wait_for("ready validator", STARTUP_TIMEOUT, &run, || {
        status_of(v.rpc, "/readyz")
            .is_some_and(|(code, json)| code == 200 && json["status"] == "ready")
    });
    let (code, live_body) = http_get(v.rpc, "/livez").expect("livez");
    let live: serde_json::Value = serde_json::from_str(&live_body).unwrap();
    assert_eq!((code, live["status"].as_str()), (200, Some("alive")));
    let (_, ready_body) = http_get(v.rpc, "/readyz").expect("readyz");

    let (code, metrics) = http_get(v.rpc, "/metrics").expect("metrics");
    assert_eq!(code, 200);
    for series in [
        "miraset_commits_total ",
        "miraset_ready 1",
        "miraset_safety_halted 0",
        "miraset_p2p_dropped_messages_total ",
        "miraset_syncing 0",
        "miraset_blocks_synced_total ",
        "miraset_vote_refused_total{layer=\"signer\",reason=\"stale\"} 0",
        "miraset_vote_refused_total{layer=\"transport\",reason=\"conflicting\"} 0",
        // Measurement campaign (TODO 8.5.3): block shape, view health, storage, process, P2P.
        "miraset_p2p_rejected_total{reason=\"unknown_validator\"} 0",
        "miraset_block_txs_count ",
        "miraset_empty_blocks_total ",
        "miraset_commit_interval_seconds_bucket{le=\"+Inf\"} ",
        "miraset_view ",
        "miraset_view_timeouts_total ",
        "miraset_storage_bytes{file=\"chain_db\"} ",
        "miraset_storage_bytes{file=\"signing_guard\"} ",
        "miraset_process_resident_bytes ",
        "miraset_process_open_fds ",
        "miraset_p2p_bytes_total{direction=\"sent\"} ",
    ] {
        assert!(metrics.contains(series), "{series} missing:\n{metrics}");
    }

    sigterm(run.child.id());
    wait_for("draining readiness", DRAIN_OBSERVE_TIMEOUT, &run, || {
        status_of(v.rpc, "/readyz")
            .is_some_and(|(code, json)| code == 503 && json["status"] == "draining")
    });
    let status = run
        .wait_exit(EXIT_TIMEOUT)
        .expect("validator exits after SIGTERM");
    assert!(status.success(), "graceful exit expected, got {status:?}");

    let log = run.log_text();
    // Every diagnostic line is one JSON object with a level and a message.
    let mut lines = 0;
    for line in log.lines().filter(|l| !l.trim().is_empty()) {
        let json: serde_json::Value =
            serde_json::from_str(line).unwrap_or_else(|_| panic!("not JSON: {line}"));
        assert!(
            json["level"].is_string() && json["message"].is_string(),
            "{line}"
        );
        lines += 1;
    }
    assert!(lines >= 3, "{log}");
    for expected in [
        "starting MIRASET experimental/devnet validator",
        "last signed view",
        "shutdown: admission stopped",
        "shutdown complete",
    ] {
        assert!(log.contains(expected), "{expected} missing:\n{log}");
    }

    // The validator secret never appears in logs, probes or metrics.
    let secret = fs::read_to_string(v.key_file()).expect("key file");
    let secret = secret.trim();
    assert_eq!(secret.len(), 64, "hex-encoded 32-byte key");
    for (name, text) in [
        ("log", &log),
        ("metrics", &metrics),
        ("livez", &live_body),
        ("readyz", &ready_body),
    ] {
        assert!(
            !text.to_lowercase().contains(&secret.to_lowercase()),
            "secret in {name}"
        );
    }
}

#[test]
fn test_e2e_text_logs_are_default_and_invalid_drain_is_refused() {
    let mut v = Validator::init();
    let mut run = v.start(&["--init-signing-guard", "--shutdown-drain-ms", "60001"]);
    let status = run.wait_exit(EXIT_TIMEOUT).expect("exits on invalid flag");
    assert!(!status.success());
    assert!(
        run.log_text().contains("shutdown-drain-ms"),
        "{}",
        run.log_text()
    );

    // An invalid RUST_LOG is reported and falls back to `info` instead of silencing logs.
    v.env
        .push(("RUST_LOG".into(), "miraset_chain=notalevel".into()));
    let mut run = v.start(&["--init-signing-guard", "--shutdown-drain-ms", LONG_DRAIN_MS]);
    const STARTUP: &str = "starting MIRASET experimental/devnet validator";
    wait_for("startup line", STARTUP_TIMEOUT, &run, || {
        run.log_text().contains(STARTUP)
    });
    let log = run.log_text();
    let startup = log.lines().find(|l| l.contains(STARTUP)).unwrap();
    assert!(
        serde_json::from_str::<serde_json::Value>(startup).is_err(),
        "{startup}"
    );
    assert!(startup.contains("INFO"), "{startup}");
    assert!(log.contains("RUST_LOG is invalid"), "{log}");

    // A second signal ends a long drain early.
    sigterm(run.child.id());
    wait_for("drain started", STARTUP_TIMEOUT, &run, || {
        run.log_text().contains("shutdown: admission stopped")
    });
    sigterm(run.child.id());
    let status = run
        .wait_exit(EARLY_EXIT_TIMEOUT)
        .expect("second signal ends the drain");
    assert!(status.success());
    assert!(
        run.log_text().contains("second signal"),
        "{}",
        run.log_text()
    );
}
