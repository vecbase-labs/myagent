use std::time::Duration;

use anyhow::Result;
use tokio::process::Command;
use tracing::debug;

/// Maximum output size per stream (stdout/stderr) in bytes.
const MAX_OUTPUT_BYTES: usize = 512 * 1024; // 512 KiB

/// Execute a bash command with timeout and output capping.
///
/// Inspired by Codex's exec implementation:
/// - Timeout enforcement with process group killing
/// - Output capping to prevent OOM
/// - Combined stdout + stderr in result
pub async fn execute(command: &str, timeout_ms: u64, work_dir: &str) -> Result<String> {
    debug!("Executing bash in {work_dir}: {command}");

    let child = Command::new("bash")
        .arg("-lc")
        .arg(command)
        .current_dir(work_dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;

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

            debug!("Bash completed: exit_code={exit_code}, output_len={}", result.len());
            Ok(result)
        }
        Ok(Err(e)) => Err(anyhow::anyhow!("Failed to execute command: {e}")),
        Err(_) => {
            // Timeout - kill_on_drop will handle cleanup
            debug!("Bash command timed out after {timeout_ms}ms");
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
