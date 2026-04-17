use std::io;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

fn main() -> Result<(), String> {
    let base_dir = current_exe_dir()?;

    let node_path = base_dir.join("miraset.exe");
    let worker_path = base_dir.join("miraset-worker.exe");
    let wallet_path = base_dir.join("wallet-miraset.exe");

    for path in [&node_path, &worker_path, &wallet_path] {
        if !path.exists() {
            return Err(format!("Missing executable: {}", path.display()));
        }
    }

    println!("Starting MIRASET node...");
    let node = spawn(&node_path, &["node", "start"])?;

    thread::sleep(Duration::from_secs(2));

    println!("Starting MIRASET worker...");
    let worker = spawn(&worker_path, &[])?;

    thread::sleep(Duration::from_secs(1));

    println!("Starting MIRASET wallet GUI...");
    let wallet = spawn(&wallet_path, &[])?;

    let stop_flag = Arc::new(AtomicBool::new(false));
    let stop_flag_ctrlc = Arc::clone(&stop_flag);
    ctrlc::set_handler(move || {
        stop_flag_ctrlc.store(true, Ordering::SeqCst);
    })
    .map_err(|err| format!("Failed to set Ctrl+C handler: {err}"))?;

    let children = Arc::new(Mutex::new(vec![node, worker, wallet]));
    let children_wait = Arc::clone(&children);
    let stop_flag_wait = Arc::clone(&stop_flag);

    thread::spawn(move || {
        let _ = wait_for_enter();
        stop_flag_wait.store(true, Ordering::SeqCst);
        let _ = terminate_all(&children_wait);
    });

    while !stop_flag.load(Ordering::SeqCst) {
        if let Ok(mut guards) = children.lock() {
            guards.retain_mut(|child| match child.try_wait() {
                Ok(Some(status)) => {
                    println!("Process exited with status: {status}");
                    false
                }
                Ok(None) => true,
                Err(err) => {
                    println!("Failed to poll child: {err}");
                    true
                }
            });
            if guards.is_empty() {
                break;
            }
        }
        thread::sleep(Duration::from_millis(500));
    }

    terminate_all(&children)?;

    Ok(())
}

fn current_exe_dir() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|err| err.to_string())?;
    exe.parent()
        .map(|dir| dir.to_path_buf())
        .ok_or_else(|| "Cannot determine executable directory".to_string())
}

fn spawn(path: &PathBuf, args: &[&str]) -> Result<Child, String> {
    Command::new(path)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|err| format!("Failed to start {}: {err}", path.display()))
}

fn wait_for_enter() -> io::Result<()> {
    println!("Press Enter to stop all services...");
    let mut buffer = String::new();
    io::stdin().read_line(&mut buffer)?;
    Ok(())
}

fn terminate_all(children: &Arc<Mutex<Vec<Child>>>) -> Result<(), String> {
    if let Ok(mut guards) = children.lock() {
        for child in guards.iter_mut() {
            let _ = child.kill();
        }
        for child in guards.iter_mut() {
            let _ = child.wait();
        }
        guards.clear();
    }
    Ok(())
}
