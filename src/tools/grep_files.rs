use std::path::Path;
use std::time::Duration;

use anyhow::Result;
use tokio::process::Command;

const DEFAULT_LIMIT: usize = 100;
const MAX_LIMIT: usize = 2000;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

/// Search files matching a regex pattern, returning file paths sorted by modification time.
/// Uses ripgrep (rg) only — matches Codex behavior.
pub async fn execute(
    pattern: &str,
    include: Option<&str>,
    search_path: Option<&str>,
    limit: usize,
    work_dir: &str,
) -> Result<String> {
    let pattern = pattern.trim();
    if pattern.is_empty() {
        return Err(anyhow::anyhow!("pattern must not be empty"));
    }

    let limit = if limit == 0 { DEFAULT_LIMIT } else { limit.min(MAX_LIMIT) };

    let dir = search_path.unwrap_or(work_dir);
    let path = if Path::new(dir).is_absolute() {
        dir.into()
    } else {
        Path::new(work_dir).join(dir)
    };

    if !path.exists() {
        return Err(anyhow::anyhow!("unable to access `{}`: path does not exist", path.display()));
    }

    let include = include
        .map(|s| s.trim())
        .and_then(|s| if s.is_empty() { None } else { Some(s) });

    let results = run_rg_search(pattern, include, &path, limit, work_dir).await?;

    if results.is_empty() {
        Ok("No matches found.".to_string())
    } else {
        Ok(results.join("\n"))
    }
}

async fn run_rg_search(
    pattern: &str,
    include: Option<&str>,
    search_path: &Path,
    limit: usize,
    cwd: &str,
) -> Result<Vec<String>> {
    let mut cmd = Command::new("rg");
    cmd.current_dir(cwd)
        .arg("--files-with-matches")
        .arg("--sortr=modified")
        .arg("--regexp")
        .arg(pattern)
        .arg("--no-messages");

    if let Some(glob) = include {
        cmd.arg("--glob").arg(glob);
    }

    cmd.arg("--").arg(search_path);

    let output = tokio::time::timeout(COMMAND_TIMEOUT, cmd.output())
        .await
        .map_err(|_| anyhow::anyhow!("rg timed out after 30 seconds"))?
        .map_err(|e| anyhow::anyhow!("failed to launch rg: {e}. Ensure ripgrep is installed and on PATH."))?;

    match output.status.code() {
        Some(0) => Ok(parse_results(&output.stdout, limit)),
        Some(1) => Ok(Vec::new()),
        _ => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(anyhow::anyhow!("rg failed: {stderr}"))
        }
    }
}

fn parse_results(stdout: &[u8], limit: usize) -> Vec<String> {
    let mut results = Vec::new();
    for line in stdout.split(|byte| *byte == b'\n') {
        if line.is_empty() {
            continue;
        }
        if let Ok(text) = std::str::from_utf8(line) {
            if text.is_empty() {
                continue;
            }
            results.push(text.to_string());
            if results.len() == limit {
                break;
            }
        }
    }
    results
}
