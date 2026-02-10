use std::fs;

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
fn read_pid() -> Result<u32> {
    let path = config::pid_file_path();
    let content = fs::read_to_string(&path)
        .map_err(|_| anyhow::anyhow!("No PID file found at {}. Is myagent running?", path.display()))?;
    let pid: u32 = content
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid PID file"))?;
    Ok(pid)
}

/// Check if a process is alive.
fn is_running(pid: u32) -> bool {
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

/// Stop the running daemon.
pub fn stop_daemon() -> Result<()> {
    let pid = read_pid()?;
    if !is_running(pid) {
        remove_pid_file();
        bail!("Process {pid} is not running (stale PID file removed)");
    }
    unsafe {
        libc::kill(pid as i32, libc::SIGTERM);
    }
    remove_pid_file();
    println!("Stopped myagent (PID {pid})");
    Ok(())
}

/// Show daemon status.
pub fn show_status() -> Result<()> {
    let pid = match read_pid() {
        Ok(pid) => pid,
        Err(_) => {
            println!("myagent is not running");
            return Ok(());
        }
    };
    if is_running(pid) {
        println!("myagent is running (PID {pid})");
    } else {
        remove_pid_file();
        println!("myagent is not running (stale PID file removed)");
    }
    Ok(())
}

/// Daemonize: re-launch self without -d flag, redirect stdio to log file.
pub fn daemonize() -> Result<()> {
    let exe = std::env::current_exe()?;
    let args: Vec<String> = std::env::args().collect();

    // Rebuild args without -d/--daemon
    let new_args: Vec<&str> = args
        .iter()
        .skip(1) // skip exe name
        .filter(|a| *a != "-d" && *a != "--daemon")
        .map(|s| s.as_str())
        .collect();

    let log_dir = config::log_dir();
    fs::create_dir_all(&log_dir)?;
    let log_file = log_dir.join("myagent.log");

    let log_out = fs::File::create(&log_file)?;
    let log_err = log_out.try_clone()?;

    let child = std::process::Command::new(exe)
        .args(&new_args)
        .stdout(log_out)
        .stderr(log_err)
        .stdin(std::process::Stdio::null())
        .spawn()?;

    println!("myagent started in background (PID {})", child.id());
    println!("Log: {}", log_file.display());
    Ok(())
}
