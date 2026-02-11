use std::path::PathBuf;

#[allow(unused_imports)]
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::config;

pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const GITHUB_REPO: &str = "vecbase-labs/myagent";
#[allow(dead_code)]
const CHECK_INTERVAL_HOURS: i64 = 24;
#[allow(dead_code)]
const VERSION_FILENAME: &str = "version.json";

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct VersionInfo {
    pub latest_version: String,
    pub last_checked_at: DateTime<Utc>,
    #[serde(default)]
    pub dismissed_version: Option<String>,
}

#[allow(dead_code)]
fn version_file_path() -> PathBuf {
    config::config_dir().join(VERSION_FILENAME)
}

#[allow(dead_code)]
fn read_version_info() -> Option<VersionInfo> {
    let path = version_file_path();
    let content = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&content).ok()
}

#[allow(dead_code)]
fn write_version_info(info: &VersionInfo) -> anyhow::Result<()> {
    let path = version_file_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string(info)?;
    std::fs::write(&path, format!("{json}\n"))?;
    Ok(())
}

#[allow(dead_code)]
fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let mut iter = v.trim().split('.');
    let maj = iter.next()?.parse::<u64>().ok()?;
    let min = iter.next()?.parse::<u64>().ok()?;
    let pat = iter.next()?.parse::<u64>().ok()?;
    Some((maj, min, pat))
}

#[allow(dead_code)]
fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Some(l), Some(c)) => l > c,
        _ => false,
    }
}

/// Check GitHub API for latest release version.
#[allow(dead_code)]
async fn fetch_latest_version() -> anyhow::Result<String> {
    let url = format!("https://api.github.com/repos/{GITHUB_REPO}/releases/latest");
    let client = reqwest::Client::new();
    let resp: serde_json::Value = client
        .get(&url)
        .header("User-Agent", format!("myagent/{CURRENT_VERSION}"))
        .header("Accept", "application/vnd.github.v3+json")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let tag = resp["tag_name"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("No tag_name in release"))?;
    Ok(tag.to_string())
}

/// Background check: fetch latest version and update cache file.
#[allow(dead_code)]
async fn do_check() {
    match fetch_latest_version().await {
        Ok(latest) => {
            let prev = read_version_info();
            let info = VersionInfo {
                latest_version: latest,
                last_checked_at: Utc::now(),
                dismissed_version: prev.and_then(|p| p.dismissed_version),
            };
            if let Err(e) = write_version_info(&info) {
                tracing::debug!("Failed to write version cache: {e}");
            }
        }
        Err(e) => {
            tracing::debug!("Failed to check for updates: {e}");
        }
    }
}

/// Called on startup. Spawns background check if needed, returns update hint.
/// Only active in release builds.
pub fn check_on_startup() -> Option<String> {
    #[cfg(debug_assertions)]
    {
        return None;
    }

    #[cfg(not(debug_assertions))]
    {
        use chrono::Duration;

        let info = read_version_info();

        // Spawn background refresh if stale or missing
        let needs_check = match &info {
            None => true,
            Some(i) => i.last_checked_at < Utc::now() - Duration::hours(CHECK_INTERVAL_HOURS),
        };
        if needs_check {
            tokio::spawn(do_check());
        }

        // Return update hint from cached info
        info.and_then(|i| {
            if is_newer(&i.latest_version, CURRENT_VERSION) {
                // Respect dismissed version
                if i.dismissed_version.as_deref() == Some(i.latest_version.as_str()) {
                    return None;
                }
                Some(i.latest_version)
            } else {
                None
            }
        })
    }
}

/// Dismiss a specific version so the user won't be prompted again.
#[allow(dead_code)]
pub fn dismiss_version(version: &str) -> anyhow::Result<()> {
    if let Some(mut info) = read_version_info() {
        info.dismissed_version = Some(version.to_string());
        write_version_info(&info)?;
    }
    Ok(())
}

/// Get the asset name for the current platform.
pub fn asset_name() -> anyhow::Result<String> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;

    let (os_part, arch_part) = match (os, arch) {
        ("macos", "aarch64") => ("darwin", "aarch64"),
        ("macos", "x86_64") => ("darwin", "x86_64"),
        ("linux", "x86_64") => ("linux", "x86_64"),
        ("linux", "aarch64") => ("linux", "aarch64"),
        ("windows", "x86_64") => ("windows", "x86_64"),
        _ => anyhow::bail!("Unsupported platform: {os}/{arch}"),
    };

    if os == "windows" {
        Ok(format!("myagent-{os_part}-{arch_part}.zip"))
    } else {
        Ok(format!("myagent-{os_part}-{arch_part}.tar.gz"))
    }
}

/// Fetch latest release info from GitHub.
pub async fn fetch_release_info() -> anyhow::Result<(String, Vec<ReleaseAsset>)> {
    let url = format!("https://api.github.com/repos/{GITHUB_REPO}/releases/latest");
    let client = reqwest::Client::new();
    let resp: GithubRelease = client
        .get(&url)
        .header("User-Agent", format!("myagent/{CURRENT_VERSION}"))
        .header("Accept", "application/vnd.github.v3+json")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    Ok((resp.tag_name, resp.assets))
}

#[derive(Deserialize, Debug)]
pub struct GithubRelease {
    pub tag_name: String,
    pub assets: Vec<ReleaseAsset>,
}

#[derive(Deserialize, Debug)]
pub struct ReleaseAsset {
    pub name: String,
    pub browser_download_url: String,
}
