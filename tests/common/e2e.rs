//! Shared helpers for end-to-end tests that run the real `miraset-chain` binary.
#![allow(dead_code)]

use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub const BIN: &str = env!("CARGO_BIN_EXE_miraset-chain");
pub const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);
/// A wiped validator walks its old views on 1s view timeouts before voting again.
pub const RECOVERY_TIMEOUT: Duration = Duration::from_secs(120);
pub const POLL: Duration = Duration::from_millis(100);
pub const HTTP_TIMEOUT: Duration = Duration::from_secs(2);
pub const MIN_HEIGHT_BEFORE_CRASH: u64 = 2;

pub struct Validator {
    pub dir: tempfile::TempDir,
    pub rpc: SocketAddr,
    pub p2p: SocketAddr,
    pub runs: usize,
    /// Extra environment for spawned processes (for example `RUST_LOG`).
    pub env: Vec<(String, String)>,
}

pub struct Running {
    pub child: Child,
    pub log: PathBuf,
}

pub fn free_addr() -> SocketAddr {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind ephemeral port")
        .local_addr()
        .expect("local addr")
}

impl Validator {
    /// `devnet-init` with one validator, then point its genesis P2P address at a free port.
    pub fn init() -> Self {
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
            env: Vec::new(),
        }
    }

    pub fn data_dir(&self) -> PathBuf {
        self.dir.path().join("data")
    }

    pub fn guard_file(&self) -> PathBuf {
        self.data_dir().join("signing-guard.redb")
    }

    pub fn chain_file(&self) -> PathBuf {
        self.data_dir().join("chain.redb")
    }

    pub fn devnet_dir(&self) -> PathBuf {
        self.dir.path().join("devnet")
    }

    pub fn key_file(&self) -> PathBuf {
        self.devnet_dir().join("node0.key")
    }

    /// Validator with its key file in-process.
    pub fn start(&mut self, extra: &[&str]) -> Running {
        let key = self.key_file();
        self.start_with_key_args(&["--key-file".as_ref(), key.as_os_str()], extra)
    }

    /// Validator signing through a `miraset-chain signer` at `socket`; it is given no key.
    pub fn start_remote(&mut self, socket: &Path, extra: &[&str]) -> Running {
        self.start_with_key_args(&["--remote-signer".as_ref(), socket.as_os_str()], extra)
    }

    pub fn signer_dir(&self) -> PathBuf {
        self.dir.path().join("signer")
    }

    pub fn signer_socket(&self) -> PathBuf {
        self.signer_dir().join("signer.sock")
    }

    pub fn signer_guard_file(&self) -> PathBuf {
        self.signer_dir().join("signing-guard.redb")
    }

    /// `miraset-chain signer` for this validator's key.
    pub fn start_signer(&mut self, extra: &[&str]) -> Running {
        fs::create_dir_all(self.signer_dir()).expect("signer dir");
        let mut args: Vec<std::ffi::OsString> = vec![
            "signer".into(),
            "--genesis".into(),
            self.devnet_dir().join("genesis.json").into(),
            "--key-file".into(),
            self.key_file().into(),
            "--signing-guard".into(),
            self.signer_guard_file().into(),
            "--socket".into(),
            self.signer_socket().into(),
        ];
        args.extend(extra.iter().map(|a| a.into()));
        self.spawn("signer", &args)
    }

    fn spawn(&mut self, label: &str, args: &[std::ffi::OsString]) -> Running {
        self.runs += 1;
        let log = self.dir.path().join(format!("{label}{}.log", self.runs));
        let out = fs::File::create(&log).expect("create log");
        let child = Command::new(BIN)
            .args(args)
            .envs(self.env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .stdout(out.try_clone().expect("clone log"))
            .stderr(out)
            .spawn()
            .expect("spawn process");
        Running { child, log }
    }

    fn start_with_key_args(&mut self, key_args: &[&std::ffi::OsStr], extra: &[&str]) -> Running {
        let devnet = self.devnet_dir();
        let mut args: Vec<std::ffi::OsString> = vec![
            "start".into(),
            "--genesis".into(),
            devnet.join("genesis.json").into(),
        ];
        args.extend(key_args.iter().map(|a| a.to_os_string()));
        args.extend(
            [
                "--data-dir".into(),
                self.data_dir().into(),
                "--p2p-addr".into(),
                self.p2p.to_string().into(),
                "--rpc-addr".into(),
                self.rpc.to_string().into(),
            ]
            .into_iter()
            .chain(extra.iter().map(|a| std::ffi::OsString::from(*a))),
        );
        self.spawn("validator", &args)
    }
}

impl Running {
    pub fn log_text(&self) -> String {
        fs::read_to_string(&self.log).unwrap_or_default()
    }

    /// SIGKILL: no graceful shutdown, no flushing beyond what was already committed.
    pub fn kill(mut self) -> String {
        self.child.kill().expect("kill validator");
        self.child.wait().expect("reap validator");
        self.log_text()
    }

    pub fn wait_exit(&mut self, timeout: Duration) -> Option<ExitStatus> {
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
pub fn http_get(addr: SocketAddr, path: &str) -> Option<(u16, String)> {
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

pub fn health(addr: SocketAddr) -> Option<(u16, serde_json::Value)> {
    let (status, body) = http_get(addr, "/v1/health")?;
    Some((status, serde_json::from_str(&body).ok()?))
}

pub fn latest_height(addr: SocketAddr) -> Option<u64> {
    let (_, body) = http_get(addr, "/v1/blocks/latest")?;
    let json: serde_json::Value = serde_json::from_str(&body).ok()?;
    json.pointer("/data/height")
        .or_else(|| json.get("height"))
        .and_then(serde_json::Value::as_u64)
}

pub fn wait_for(what: &str, timeout: Duration, run: &Running, mut condition: impl FnMut() -> bool) {
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

pub fn wait_healthy_with_height(v: &Validator, run: &Running, min_height: u64, timeout: Duration) {
    wait_for("healthy node with committed height", timeout, run, || {
        health(v.rpc).is_some_and(|(code, json)| code == 200 && json["status"] == "ok")
            && latest_height(v.rpc).is_some_and(|h| h >= min_height)
    });
}

/// Parse "last signed view: N" from the startup banner.
pub fn last_signed_view(log: &str) -> Option<u64> {
    log.lines()
        .find_map(|line| line.split("last signed view: ").nth(1))
        .and_then(|rest| rest.trim_end_matches(')').parse().ok())
}

pub fn assert_exists(path: &Path) {
    assert!(path.exists(), "{} should exist", path.display());
}

pub fn latest_height_after_start(v: &Validator, run: &Running) -> u64 {
    let mut height = None;
    wait_for("RPC up", STARTUP_TIMEOUT, run, || {
        height = latest_height(v.rpc);
        height.is_some()
    });
    height.unwrap()
}

/// SIGTERM a spawned process (graceful shutdown).
pub fn sigterm(pid: u32) {
    let pid = libc::pid_t::try_from(pid).expect("pid fits pid_t");
    // SAFETY: plain kill(2) on a child process the test spawned and has not reaped.
    let rc = unsafe { libc::kill(pid, libc::SIGTERM) };
    assert_eq!(rc, 0, "SIGTERM failed");
}

/// Run the binary to completion; returns (exit code, stdout, stderr).
pub fn run_cli(args: &[&std::ffi::OsStr]) -> (Option<i32>, String, String) {
    let output = Command::new(BIN).args(args).output().expect("run binary");
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}
