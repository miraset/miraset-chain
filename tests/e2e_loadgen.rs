//! End-to-end load generator (TODO 8.5.3): the real `loadgen` subcommand against a real
//! one-validator node funds its senders, offers a transfer/receipt mix and reports every
//! admitted item committed with a measured latency.

mod common;

use common::e2e::{BIN, STARTUP_TIMEOUT, Validator, wait_healthy_with_height};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::Duration;

/// Into the load run (funding takes a few blocks) before the observer node is restarted.
const RESTART_AFTER: Duration = Duration::from_secs(8);

fn loadgen_command(v: &Validator, extra: &[&str]) -> Command {
    let rpc = format!("http://{}", v.rpc);
    let mut command = Command::new(BIN);
    command
        .args(["loadgen", "--rpc", &rpc, "--faucet-key"])
        .arg(v.devnet_dir().join("faucet.key"))
        .arg("--coordinator-key")
        .arg(v.devnet_dir().join("coordinator.key"))
        .args(extra);
    command
}

fn loadgen(v: &Validator, extra: &[&str]) -> Output {
    loadgen_command(v, extra).output().expect("run loadgen")
}

#[test]
fn test_e2e_loadgen_reports_every_admitted_item_committed() {
    let mut v = Validator::init();
    let run = v.start(&["--init-signing-guard"]);
    wait_healthy_with_height(&v, &run, 1, STARTUP_TIMEOUT);

    let output = loadgen(
        &v,
        &[
            "--senders",
            "4",
            "--rate",
            "20",
            "--receipt-share",
            "0.25",
            "--duration-secs",
            "8",
            "--warmup-secs",
            "2",
            "--drain-timeout-secs",
            "60",
        ],
    );
    assert!(
        output.status.success(),
        "loadgen failed: {}\nnode log:\n{}",
        String::from_utf8_lossy(&output.stderr),
        run.log_text()
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON report");

    for kind in ["transactions", "receipts"] {
        let k = &report[kind];
        let admitted = k["admitted"].as_u64().unwrap();
        assert!(admitted > 0, "{kind}: nothing admitted: {report:#}");
        assert_eq!(
            k["committed"].as_u64(),
            Some(admitted),
            "{kind}: {report:#}"
        );
        assert_eq!(k["uncommitted"].as_u64(), Some(0), "{kind}: {report:#}");
        assert!(
            k["rejected"].as_object().unwrap().is_empty(),
            "{kind}: {report:#}"
        );
        assert!(
            k["latency"]["p50_ms"].as_f64().unwrap() > 0.0,
            "{kind}: {report:#}"
        );
    }
    assert_eq!(report["pending_at_end"].as_u64(), Some(0));
    assert!(report["blocks"]["blocks"].as_u64().unwrap() > 0);
    assert!(report["committed_per_sec"].as_f64().unwrap() > 0.0);
}

#[test]
fn test_e2e_loadgen_refuses_invalid_configuration_and_unfundable_runs() {
    let mut v = Validator::init();
    let run = v.start(&["--init-signing-guard"]);
    wait_healthy_with_height(&v, &run, 1, STARTUP_TIMEOUT);

    let invalid = loadgen(&v, &["--rate", "0"]);
    assert!(!invalid.status.success());
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("rate must be a positive number"));

    // The devnet faucet holds 1e9; 32 senders at u64::MAX / 64 each cannot be funded.
    let huge = (u64::MAX / 64).to_string();
    let unfundable = loadgen(
        &v,
        &[
            "--fund-amount",
            &huge,
            "--duration-secs",
            "2",
            "--warmup-secs",
            "1",
        ],
    );
    assert!(!unfundable.status.success());
    assert!(String::from_utf8_lossy(&unfundable.stderr).contains("cannot fund"));
    assert!(
        unfundable.stdout.is_empty(),
        "no report for a run that never started"
    );
}

/// The observer node is killed and restarted mid-run: the commit stream reconnects and items
/// keep being observed committed instead of the run silently reporting nothing after the drop.
#[test]
fn test_e2e_loadgen_survives_an_observer_node_restart() {
    let mut v = Validator::init();
    let first = v.start(&["--init-signing-guard"]);
    wait_healthy_with_height(&v, &first, 1, STARTUP_TIMEOUT);

    let child = loadgen_command(
        &v,
        &[
            "--senders",
            "2",
            "--rate",
            "10",
            "--receipt-share",
            "0.5",
            "--duration-secs",
            "20",
            "--warmup-secs",
            "1",
            "--drain-timeout-secs",
            "15",
        ],
    )
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()
    .expect("spawn loadgen");

    thread::sleep(RESTART_AFTER);
    let _ = first.kill();
    let second = v.start(&[]);
    wait_healthy_with_height(&v, &second, 1, STARTUP_TIMEOUT);

    let output = child.wait_with_output().expect("loadgen output");
    assert!(
        output.status.success(),
        "loadgen failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON report");
    assert!(
        report["observer_reconnects"].as_u64().unwrap() >= 1,
        "{report:#}"
    );
    // Items committed after the restart are still observed. (A one-validator chain commits
    // within milliseconds, so the killed mempool rarely holds anything to lose.)
    // Without reconnection nothing after the restart (at 8 s of 20 s) would be observed.
    let receipts = &report["receipts"];
    let admitted = receipts["admitted"].as_u64().unwrap();
    let committed = receipts["committed"].as_u64().unwrap();
    assert!(admitted > 0, "{report:#}");
    assert!(committed * 10 >= admitted * 8, "{report:#}");
    assert!(
        report["transactions"]["committed"].as_u64().unwrap() > 0,
        "{report:#}"
    );
}
