use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;

use anyhow::{bail, Result};

use crate::config;

/// Write PID file for the current process.
pub fn write_pid_file() -> Result<()> {
    let pid = std::process::id();
    let path = config::pid_file_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, pid.to_string())?;
    Ok(())
}

/// Remove PID file.
pub fn remove_pid_file() {
    let _ = fs::remove_file(config::pid_file_path());
}

/// Read PID from file.
fn read_pid() -> Option<u32> {
    let path = config::pid_file_path();
    let content = fs::read_to_string(&path).ok()?;
    content.trim().parse().ok()
}

/// Check if a process is alive.
#[cfg(unix)]
fn is_running(pid: u32) -> bool {
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

#[cfg(windows)]
fn is_running(pid: u32) -> bool {
    std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {}", pid), "/NH"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains(&pid.to_string()))
        .unwrap_or(false)
}

/// Stop the running daemon via HTTP RPC, with PID+SIGTERM fallback.
pub fn stop_daemon() -> Result<()> {
    let port = load_port();

    // Try HTTP shutdown first
    if let Some(_) = http_post_rpc(port, "shutdown") {
        std::thread::sleep(std::time::Duration::from_millis(500));
        remove_pid_file();
        println!("Stopped myagent");
        return Ok(());
    }

    // Fallback: PID file + SIGTERM
    let pid = read_pid().ok_or_else(|| anyhow::anyhow!("myagent is not running"))?;
    if !is_running(pid) {
        remove_pid_file();
        bail!("Process {pid} is not running (stale PID file removed)");
    }
    #[cfg(unix)]
    unsafe {
        libc::kill(pid as i32, libc::SIGTERM);
    }
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string()])
            .output();
    }
    remove_pid_file();
    println!("Stopped myagent (PID {pid})");
    Ok(())
}

/// Show daemon status via HTTP health check, with PID fallback.
pub fn show_status() -> Result<()> {
    let port = load_port();

    // Try HTTP health check
    if let Some(body) = http_get(port, "/health") {
        if let Ok(health) = serde_json::from_str::<serde_json::Value>(&body) {
            println!("myagent is running");
            println!("  Version: {}", health["version"].as_str().unwrap_or("?"));
            println!("  PID:     {}", health["pid"]);
            println!("  Uptime:  {}s", health["uptime"]);
            println!("  Port:    {}", health["port"]);
            return Ok(());
        }
    }

    // Fallback: PID file
    if let Some(pid) = read_pid() {
        if is_running(pid) {
            println!("myagent is running (PID {pid})");
        } else {
            remove_pid_file();
            println!("myagent is not running (stale PID file removed)");
        }
    } else {
        println!("myagent is not running");
    }
    Ok(())
}

/// Daemonize: re-launch self with `serve` subcommand, redirect stdio to log file.
pub fn daemonize() -> Result<()> {
    let exe = std::env::current_exe()?;

    // Collect global args (config path) if present
    let args: Vec<String> = std::env::args().collect();
    let mut new_args: Vec<String> = Vec::new();
    let mut i = 1; // skip exe name
    while i < args.len() {
        match args[i].as_str() {
            "start" | "restart" => {
                // skip the subcommand itself
                i += 1;
                continue;
            }
            "-c" | "--config" => {
                if i + 1 < args.len() {
                    new_args.push(args[i].clone());
                    new_args.push(args[i + 1].clone());
                    i += 2;
                    continue;
                }
            }
            _ => {}
        }
        i += 1;
    }
    new_args.push("serve".to_string());

    let log_dir = config::log_dir();
    fs::create_dir_all(&log_dir)?;
    let log_file = log_dir.join("myagent.log");

    // Rotate log if it exceeds 10MB
    rotate_log(&log_file, MAX_LOG_SIZE, MAX_LOG_FILES);

    let log_out = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file)?;
    let log_err = log_out.try_clone()?;

    let child = std::process::Command::new(exe)
        .args(&new_args)
        .stdout(log_out)
        .stderr(log_err)
        .stdin(std::process::Stdio::null())
        .spawn()?;

    println!("myagent started (PID {})", child.id());
    println!("Log: {}", log_file.display());
    Ok(())
}

/// Check if the daemon is currently running.
pub fn is_daemon_running() -> bool {
    if let Some(pid) = read_pid() {
        is_running(pid)
    } else {
        false
    }
}

/// Load port from config file, or use default.
fn load_port() -> u16 {
    let path = config::default_config_path();
    config::AppConfig::load(&path)
        .map(|c| c.port)
        .unwrap_or(config::DEFAULT_PORT)
}

/// Simple HTTP GET using raw TCP (no external deps needed for sync context).
fn http_get(port: u16, path: &str) -> Option<String> {
    let addr = format!("127.0.0.1:{}", port);
    let mut stream = TcpStream::connect(&addr).ok()?;
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(2)))
        .ok()?;
    let request = format!("GET {} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n", path);
    stream.write_all(request.as_bytes()).ok()?;
    let mut response = String::new();
    stream.read_to_string(&mut response).ok()?;
    // Extract body after \r\n\r\n
    response.split("\r\n\r\n").nth(1).map(|s| s.to_string())
}

/// Simple HTTP POST JSON-RPC using raw TCP.
fn http_post_rpc(port: u16, method: &str) -> Option<String> {
    let addr = format!("127.0.0.1:{}", port);
    let mut stream = TcpStream::connect(&addr).ok()?;
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(3)))
        .ok()?;
    let body = format!(
        r#"{{"jsonrpc":"2.0","method":"{}","id":1}}"#,
        method
    );
    let request = format!(
        "POST /rpc HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream.write_all(request.as_bytes()).ok()?;
    let mut response = String::new();
    stream.read_to_string(&mut response).ok()?;
    response.split("\r\n\r\n").nth(1).map(|s| s.to_string())
}

// ── Log rotation ──

const MAX_LOG_SIZE: u64 = 10 * 1024 * 1024; // 10 MB
const MAX_LOG_FILES: usize = 5;

/// Rotate log file if it exceeds max_size.
fn rotate_log(log_path: &std::path::Path, max_size: u64, max_files: usize) {
    let size = fs::metadata(log_path).map(|m| m.len()).unwrap_or(0);
    if size < max_size {
        return;
    }
    for i in (1..max_files).rev() {
        let from = log_path.with_extension(format!("log.{i}"));
        let to = log_path.with_extension(format!("log.{}", i + 1));
        let _ = fs::rename(&from, &to);
    }
    let _ = fs::rename(log_path, log_path.with_extension("log.1"));
    let _ = fs::remove_file(log_path.with_extension(format!("log.{}", max_files + 1)));
}

/// Clear all log files.
pub fn clear_logs() -> Result<()> {
    let log_dir = config::log_dir();
    if !log_dir.exists() {
        println!("No logs to clear.");
        return Ok(());
    }
    let mut count = 0;
    for entry in fs::read_dir(&log_dir)? {
        let entry = entry?;
        if entry.file_name().to_string_lossy().starts_with("myagent.log") {
            fs::remove_file(entry.path())?;
            count += 1;
        }
    }
    println!("Cleared {count} log file(s).");
    Ok(())
}
