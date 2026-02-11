use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use tokio::process::Command;
use tracing::{debug, info};

/// Maximum output size per stream (stdout/stderr) in bytes.
const MAX_OUTPUT_BYTES: usize = 512 * 1024; // 512 KiB

/// Supported shell types.
#[derive(Debug, Clone, Copy)]
pub enum ShellType {
    Bash,
    Zsh,
    Sh,
    PowerShell,
    Cmd,
}

impl ShellType {
    /// Human-readable name for tool descriptions.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Bash => "bash",
            Self::Zsh => "zsh",
            Self::Sh => "sh",
            Self::PowerShell => "powershell",
            Self::Cmd => "cmd",
        }
    }
}

/// Detected shell with its path and type.
#[derive(Debug, Clone)]
pub struct Shell {
    pub shell_type: ShellType,
    pub path: PathBuf,
}

impl Shell {
    /// Detect the best available shell for the current platform.
    pub fn detect() -> Self {
        #[cfg(unix)]
        {
            Self::detect_unix()
        }
        #[cfg(windows)]
        {
            Self::detect_windows()
        }
    }

    #[cfg(unix)]
    fn detect_unix() -> Self {
        // Try user's login shell from $SHELL
        if let Ok(shell_path) = std::env::var("SHELL") {
            let path = PathBuf::from(&shell_path);
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                match name {
                    "bash" => return Self { shell_type: ShellType::Bash, path },
                    "zsh" => return Self { shell_type: ShellType::Zsh, path },
                    _ => {}
                }
            }
        }
        // Fallback: prefer bash > zsh > sh
        for (name, st) in [("bash", ShellType::Bash), ("zsh", ShellType::Zsh)] {
            if let Ok(p) = which(name) {
                return Self { shell_type: st, path: p };
            }
        }
        Self { shell_type: ShellType::Sh, path: PathBuf::from("/bin/sh") }
    }

    #[cfg(windows)]
    fn detect_windows() -> Self {
        // Prefer pwsh (PowerShell 7+) > powershell (5.1) > cmd
        for name in ["pwsh.exe", "powershell.exe"] {
            if let Ok(p) = which(name) {
                return Self { shell_type: ShellType::PowerShell, path: p };
            }
        }
        Self { shell_type: ShellType::Cmd, path: PathBuf::from("cmd.exe") }
    }

    /// Build the command args for executing a string command.
    fn exec_args(&self, command: &str) -> Vec<String> {
        match self.shell_type {
            ShellType::Bash | ShellType::Zsh => vec![
                self.path.to_string_lossy().to_string(),
                "-lc".to_string(),
                command.to_string(),
            ],
            ShellType::Sh => vec![
                self.path.to_string_lossy().to_string(),
                "-c".to_string(),
                command.to_string(),
            ],
            ShellType::PowerShell => vec![
                self.path.to_string_lossy().to_string(),
                "-NoProfile".to_string(),
                "-Command".to_string(),
                command.to_string(),
            ],
            ShellType::Cmd => vec![
                self.path.to_string_lossy().to_string(),
                "/c".to_string(),
                command.to_string(),
            ],
        }
    }
}

/// Simple which: find executable in PATH.
fn which(name: &str) -> std::result::Result<PathBuf, ()> {
    let path_var = std::env::var("PATH").map_err(|_| ())?;
    #[cfg(unix)]
    let sep = ':';
    #[cfg(windows)]
    let sep = ';';
    for dir in path_var.split(sep) {
        let candidate = PathBuf::from(dir).join(name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(())
}

/// Execute a shell command with timeout and output capping.
pub async fn execute(
    shell: &Shell,
    command: &str,
    timeout_ms: u64,
    work_dir: &str,
) -> Result<String> {
    debug!("Executing {} in {work_dir}: {command}", shell.shell_type.name());
    info!("Shell: {}", truncate_str(command, 200));

    let args = shell.exec_args(command);
    let mut cmd = Command::new(&args[0]);
    cmd.args(&args[1..])
        .current_dir(work_dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);

    let child = cmd.spawn()?;
    let timeout = Duration::from_millis(timeout_ms);
    let result = tokio::time::timeout(timeout, child.wait_with_output()).await;

    match result {
        Ok(Ok(output)) => {
            let exit_code = output.status.code().unwrap_or(-1);
            let stdout = truncate_output(&output.stdout);
            let stderr = truncate_output(&output.stderr);

            let mut result = String::new();
            if !stdout.is_empty() {
                result.push_str(&stdout);
            }
            if !stderr.is_empty() {
                if !result.is_empty() {
                    result.push_str("\n--- stderr ---\n");
                }
                result.push_str(&stderr);
            }
            if result.is_empty() {
                result = "(no output)".to_string();
            }
            result.push_str(&format!("\n\nExit code: {exit_code}"));
            Ok(result)
        }
        Ok(Err(e)) => Err(anyhow::anyhow!("Failed to execute command: {e}")),
        Err(_) => {
            debug!("Shell command timed out after {timeout_ms}ms");
            Ok(format!(
                "Command timed out after {timeout_ms}ms.\n\nExit code: 124"
            ))
        }
    }
}

fn truncate_output(bytes: &[u8]) -> String {
    let s = String::from_utf8_lossy(bytes);
    if s.len() > MAX_OUTPUT_BYTES {
        let truncated = &s[..MAX_OUTPUT_BYTES];
        format!("{truncated}\n\n... (output truncated at {MAX_OUTPUT_BYTES} bytes)")
    } else {
        s.to_string()
    }
}

fn truncate_str(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        let mut end = max;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        &s[..end]
    }
}
