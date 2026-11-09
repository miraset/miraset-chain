//! End-to-end CLI start modes and offline database maintenance with the real binary
//! (TODO 8.4.5).

mod common;

use common::e2e::{STARTUP_TIMEOUT, Validator, latest_height, run_cli, sigterm, wait_for};
use std::ffi::OsStr;
use std::time::Duration;

const EXIT_TIMEOUT: Duration = Duration::from_secs(20);
const CLAP_USAGE_ERROR: i32 = 2;
const MIN_HEIGHT: u64 = 3;

fn os<'a>(args: &[&'a str]) -> Vec<&'a OsStr> {
    args.iter().map(|arg| OsStr::new(*arg)).collect()
}

#[test]
fn test_e2e_no_arguments_prints_help_and_implicit_start_is_rejected() {
    let (code, _, stderr) = run_cli(&[]);
    assert_eq!(code, Some(CLAP_USAGE_ERROR));
    assert!(
        stderr.contains("Usage:") && stderr.contains("start"),
        "{stderr}"
    );

    let (code, _, stderr) = run_cli(&os(&["--genesis", "genesis.json", "--key-file", "k"]));
    assert_eq!(code, Some(CLAP_USAGE_ERROR));
    assert!(stderr.contains("--genesis"), "{stderr}");
}

#[test]
fn test_e2e_db_commands_refuse_a_running_node_then_work_offline() {
    let mut v = Validator::init();
    let data_dir = v.data_dir();
    let data = data_dir.as_os_str();
    let mut run = v.start(&["--init-signing-guard", "--shutdown-drain-ms", "0"]);
    wait_for("committed blocks", STARTUP_TIMEOUT, &run, || {
        latest_height(v.rpc).is_some_and(|h| h >= MIN_HEIGHT)
    });

    let (code, _, stderr) = run_cli(&[
        OsStr::new("db"),
        OsStr::new("inspect"),
        OsStr::new("--data-dir"),
        data,
    ]);
    assert_eq!(code, Some(4), "in-use exit code: {stderr}");
    assert!(
        stderr.contains("error:") && stderr.contains("in use"),
        "{stderr}"
    );

    sigterm(run.child.id());
    let status = run.wait_exit(EXIT_TIMEOUT).expect("validator stops");
    assert!(status.success());

    let genesis = v.devnet_dir().join("genesis.json");
    let (code, stdout, stderr) = run_cli(&[
        OsStr::new("db"),
        OsStr::new("verify"),
        OsStr::new("--data-dir"),
        data,
        OsStr::new("--genesis"),
        genesis.as_os_str(),
    ]);
    assert_eq!(code, Some(0), "{stderr}");
    let report: serde_json::Value = serde_json::from_str(&stdout).expect("verify JSON");
    assert!(report["heights_checked"].as_u64().unwrap() > MIN_HEIGHT);
    assert_eq!(report["genesis_checked"], true);

    let backup = v.dir.path().join("chain-backup.redb");
    let backup_args = [
        OsStr::new("db"),
        OsStr::new("backup"),
        OsStr::new("--data-dir"),
        data,
        OsStr::new("--out"),
        backup.as_os_str(),
    ];
    let (code, _, stderr) = run_cli(&backup_args);
    assert_eq!(code, Some(0), "{stderr}");
    let (code, _, stderr) = run_cli(&backup_args);
    assert_eq!(code, Some(1));
    assert!(stderr.contains("never overwrite"), "{stderr}");

    let (code, stdout, stderr) = run_cli(&[
        OsStr::new("db"),
        OsStr::new("compact"),
        OsStr::new("--data-dir"),
        data,
    ]);
    assert_eq!(code, Some(0), "{stderr}");
    assert!(stdout.contains("bytes_after"), "{stdout}");

    // Restore: the backup replaces the database (same tip, so the kept guard has nothing newer).
    std::fs::rename(&backup, v.chain_file()).expect("restore backup");

    // The node still starts on the restored, maintained database and keeps finalizing.
    let height_before = {
        let (_, stdout, _) = run_cli(&[
            OsStr::new("db"),
            OsStr::new("inspect"),
            OsStr::new("--data-dir"),
            data,
        ]);
        let report: serde_json::Value = serde_json::from_str(&stdout).unwrap();
        report["committed_height"].as_u64().unwrap()
    };
    let mut run = v.start(&["--shutdown-drain-ms", "0"]);
    wait_for(
        "finalizing after maintenance",
        STARTUP_TIMEOUT,
        &run,
        || latest_height(v.rpc).is_some_and(|h| h > height_before),
    );
    sigterm(run.child.id());
    assert!(run.wait_exit(EXIT_TIMEOUT).expect("stops").success());
}
