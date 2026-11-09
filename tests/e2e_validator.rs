//! End-to-end: the real `miraset-chain` validator binary as a separate process (TODO 8.3.2).
//!
//! Each test creates a one-validator devnet with `devnet-init`, runs the binary on free local
//! ports, observes it only through HTTP (`/health`, `/block/latest`) and its log output, and
//! crashes it with SIGKILL (no graceful shutdown).
//!
//! Not covered here: the exit-code-3 safety halt. It needs an injected guard or storage fault,
//! which the production binary deliberately does not expose. It is covered in-process by
//! `tests/vote_safety.rs` (T5) and `tests/consensus_panic_halt.rs`.

use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_miraset-chain");
const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);
/// A wiped validator walks its old views on 1s view timeouts before voting again.
const RECOVERY_TIMEOUT: Duration = Duration::from_secs(120);
const POLL: Duration = Duration::from_millis(100);
const HTTP_TIMEOUT: Duration = Duration::from_secs(2);
const MIN_HEIGHT_BEFORE_CRASH: u64 = 2;

struct Validator {
    dir: tempfile::TempDir,
    rpc: SocketAddr,
    p2p: SocketAddr,
    runs: usize,
}

struct Running {
    child: Child,
    log: PathBuf,
}

fn free_addr() -> SocketAddr {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind ephemeral port")
        .local_addr()
        .expect("local addr")
}

impl Validator {
    /// `devnet-init` with one validator, then point its genesis P2P address at a free port.
    fn init() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let devnet = dir.path().join("devnet");
        let status = Command::new(BIN)
            .args(["devnet-init", "--validators", "1", "--p2p-addr-template"])
            .arg("127.0.0.1:1{i}")
            .arg("--out")
            .arg(&devnet)
            .stdout(Stdio::null())
            .status()
            .expect("run devnet-init");
        assert!(status.success(), "devnet-init failed");

        let p2p = free_addr();
        let genesis_path = devnet.join("genesis.json");
        let mut genesis: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&genesis_path).unwrap()).unwrap();
        genesis["validators"][0]["p2p_addr"] = serde_json::Value::String(p2p.to_string());
        fs::write(
            &genesis_path,
            serde_json::to_string_pretty(&genesis).unwrap(),
        )
        .unwrap();

        Self {
            dir,
            rpc: free_addr(),
            p2p,
            runs: 0,
        }
    }

    fn data_dir(&self) -> PathBuf {
        self.dir.path().join("data")
    }

    fn guard_file(&self) -> PathBuf {
        self.data_dir().join("signing-guard.redb")
    }

    fn chain_file(&self) -> PathBuf {
        self.data_dir().join("chain.redb")
    }

    fn start(&mut self, extra: &[&str]) -> Running {
        self.runs += 1;
        let log = self.dir.path().join(format!("run{}.log", self.runs));
        let out = fs::File::create(&log).expect("create log");
        let devnet = self.dir.path().join("devnet");
        let child = Command::new(BIN)
            .arg("--genesis")
            .arg(devnet.join("genesis.json"))
            .arg("--key-file")
            .arg(devnet.join("node0.key"))
            .arg("--data-dir")
            .arg(self.data_dir())
            .arg("--p2p-addr")
            .arg(self.p2p.to_string())
            .arg("--rpc-addr")
            .arg(self.rpc.to_string())
            .args(extra)
            .stdout(out.try_clone().expect("clone log"))
            .stderr(out)
            .spawn()
            .expect("spawn validator");
        Running { child, log }
    }
}

impl Running {
    fn log_text(&self) -> String {
        fs::read_to_string(&self.log).unwrap_or_default()
    }

    /// SIGKILL: no graceful shutdown, no flushing beyond what was already committed.
    fn kill(mut self) -> String {
        self.child.kill().expect("kill validator");
        self.child.wait().expect("reap validator");
        self.log_text()
    }

    fn wait_exit(&mut self, timeout: Duration) -> Option<ExitStatus> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if let Some(status) = self.child.try_wait().expect("poll child") {
                return Some(status);
            }
            thread::sleep(POLL);
        }
        None
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Minimal HTTP/1.1 GET; returns (status code, body).
fn http_get(addr: SocketAddr, path: &str) -> Option<(u16, String)> {
    let mut stream = TcpStream::connect_timeout(&addr, HTTP_TIMEOUT).ok()?;
    stream.set_read_timeout(Some(HTTP_TIMEOUT)).ok()?;
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"
    )
    .ok()?;
    let mut response = String::new();
    stream.read_to_string(&mut response).ok()?;
    let status = response.split_whitespace().nth(1)?.parse().ok()?;
    let body = response.split_once("\r\n\r\n")?.1.to_string();
    Some((status, body))
}

fn health(addr: SocketAddr) -> Option<(u16, serde_json::Value)> {
    let (status, body) = http_get(addr, "/health")?;
    Some((status, serde_json::from_str(&body).ok()?))
}

fn latest_height(addr: SocketAddr) -> Option<u64> {
    let (_, body) = http_get(addr, "/block/latest")?;
    let json: serde_json::Value = serde_json::from_str(&body).ok()?;
    json.pointer("/data/height")
        .or_else(|| json.get("height"))
        .and_then(serde_json::Value::as_u64)
}

fn wait_for(what: &str, timeout: Duration, run: &Running, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while !condition() {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {what}; log:\n{}",
            run.log_text()
        );
        thread::sleep(POLL);
    }
}

fn wait_healthy_with_height(v: &Validator, run: &Running, min_height: u64, timeout: Duration) {
    wait_for("healthy node with committed height", timeout, run, || {
        health(v.rpc).is_some_and(|(code, json)| code == 200 && json["status"] == "ok")
            && latest_height(v.rpc).is_some_and(|h| h >= min_height)
    });
}

/// Parse "last signed view: N" from the startup banner.
fn last_signed_view(log: &str) -> Option<u64> {
    log.lines()
        .find_map(|line| line.split("last signed view: ").nth(1))
        .and_then(|rest| rest.trim_end_matches(')').parse().ok())
}

fn assert_exists(path: &Path) {
    assert!(path.exists(), "{} should exist", path.display());
}

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

fn latest_height_after_start(v: &Validator, run: &Running) -> u64 {
    let mut height = None;
    wait_for("RPC up", STARTUP_TIMEOUT, run, || {
        height = latest_height(v.rpc);
        height.is_some()
    });
    height.unwrap()
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
