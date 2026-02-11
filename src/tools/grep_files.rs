use std::path::Path;
use std::time::Duration;

use anyhow::Result;
use tokio::process::Command;

const DEFAULT_LIMIT: usize = 100;
const TIMEOUT_SECS: u64 = 30;

/// Search files matching a regex pattern, returning file paths sorted by modification time.
/// Uses ripgrep (rg) if available, falls back to grep.
pub async fn execute(
    pattern: &str,
    include: Option<&str>,
    search_path: Option<&str>,
    limit: usize,
    work_dir: &str,
) -> Result<String> {
    let limit = if limit == 0 { DEFAULT_LIMIT } else { limit.min(2000) };

    let dir = search_path.unwrap_or(work_dir);
    let path = if Path::new(dir).is_absolute() {
        dir.to_string()
    } else {
        format!("{work_dir}/{dir}")
    };

    if !Path::new(&path).exists() {
        return Err(anyhow::anyhow!("Path does not exist: {path}"));
    }

    // Try ripgrep first, then grep
    let result = match try_ripgrep(pattern, include, &path, limit).await {
        Ok(files) => Ok(files),
        Err(_) => try_grep(pattern, include, &path, limit).await,
    };

    match result {
        Ok(files) if files.is_empty() => Ok("No matches found.".to_string()),
        Ok(files) => Ok(files.join("\n")),
        Err(e) => Err(e),
    }
}

async fn try_ripgrep(
    pattern: &str,
    include: Option<&str>,
    path: &str,
    limit: usize,
) -> Result<Vec<String>> {
    let mut cmd = Command::new("rg");
    cmd.arg("--files-with-matches")
        .arg("--sortr=modified")
        .arg("--max-count=1");

    if let Some(glob) = include {
        cmd.arg("--glob").arg(glob);
    }

    cmd.arg(pattern).arg(path);

    let output = tokio::time::timeout(
        Duration::from_secs(TIMEOUT_SECS),
        cmd.output(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("Search timed out after {TIMEOUT_SECS}s"))?
    .map_err(|e| anyhow::anyhow!("Failed to run rg: {e}"))?;

    match output.status.code() {
        Some(0) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let files: Vec<String> = stdout
                .lines()
                .take(limit)
                .map(|s| s.to_string())
                .collect();
            Ok(files)
        }
        Some(1) => Ok(Vec::new()), // No matches
        _ => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(anyhow::anyhow!("rg failed: {stderr}"))
        }
    }
}

async fn try_grep(
    pattern: &str,
    include: Option<&str>,
    path: &str,
    limit: usize,
) -> Result<Vec<String>> {
    let mut cmd = Command::new("grep");
    cmd.arg("-rl");

    if let Some(glob) = include {
        cmd.arg("--include").arg(glob);
    }

    cmd.arg(pattern).arg(path);

    let output = tokio::time::timeout(
        Duration::from_secs(TIMEOUT_SECS),
        cmd.output(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("Search timed out after {TIMEOUT_SECS}s"))?
    .map_err(|e| anyhow::anyhow!("Failed to run grep: {e}"))?;

    match output.status.code() {
        Some(0) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let files: Vec<String> = stdout
                .lines()
                .take(limit)
                .map(|s| s.to_string())
                .collect();
            Ok(files)
        }
        Some(1) => Ok(Vec::new()),
        _ => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(anyhow::anyhow!("grep failed: {stderr}"))
        }
    }
}
