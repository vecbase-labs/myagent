use std::io::Cursor;

use anyhow::Result;
use reqwest::Client;

use crate::update_check::{self, CURRENT_VERSION};

pub async fn run() -> Result<()> {
    println!("Checking for updates...");

    let (tag, assets) = update_check::fetch_release_info()
        .await
        .map_err(|_| anyhow::anyhow!("Update failed. Please check your network and try again."))?;
    let latest = tag.as_str();

    let current_ver = parse_ver(CURRENT_VERSION);
    let latest_ver = parse_ver(latest);

    match (current_ver, latest_ver) {
        (Some(c), Some(l)) if l <= c => {
            println!("Already up to date (v{CURRENT_VERSION}).");
            return Ok(());
        }
        _ => {}
    }

    println!("Updating {CURRENT_VERSION} → {latest}...");

    let target_asset = update_check::asset_name()?;
    let asset = assets
        .iter()
        .find(|a| a.name == target_asset)
        .ok_or_else(|| {
            anyhow::anyhow!("No release found for this platform.")
        })?;

    // Download
    let client = Client::new();
    let bytes = client
        .get(&asset.browser_download_url)
        .header("User-Agent", format!("myagent/{CURRENT_VERSION}"))
        .header("Accept", "application/octet-stream")
        .send()
        .await
        .map_err(|_| anyhow::anyhow!("Update failed. Please check your network and try again."))?
        .error_for_status()
        .map_err(|_| anyhow::anyhow!("Update failed. Please try again later."))?
        .bytes()
        .await
        .map_err(|_| anyhow::anyhow!("Download interrupted. Please try again."))?;

    // Extract
    let binary = extract_binary(&bytes, &asset.name)
        .map_err(|_| anyhow::anyhow!("Update failed. Please try again later."))?;

    // Write to temp and verify the new binary can actually run
    let tmp_dir = std::env::temp_dir().join("myagent-update");
    let cleanup = || { let _ = std::fs::remove_dir_all(&tmp_dir); };

    std::fs::create_dir_all(&tmp_dir)?;
    let bin_name = if cfg!(windows) { "myagent.exe" } else { "myagent" };
    let tmp_bin = tmp_dir.join(bin_name);
    std::fs::write(&tmp_bin, &binary)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp_bin, std::fs::Permissions::from_mode(0o755))?;
    }

    // Verify: run the new binary to confirm it's a valid executable.
    // If this fails, the current installation is completely untouched.
    let ok = std::process::Command::new(&tmp_bin)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if !ok {
        cleanup();
        anyhow::bail!("Update failed. Please try again later.");
    }

    // Verified — safe to replace. self_replace uses atomic rename,
    // so even if this fails the original binary remains intact.
    if self_replace::self_replace(&tmp_bin).is_err() {
        cleanup();
        anyhow::bail!("Update failed. Please try again later.");
    }

    cleanup();

    if crate::daemon::is_daemon_running() {
        println!("Updated to {latest}. Run `myagent restart` to apply to the daemon.");
    } else {
        println!("Updated to {latest}.");
    }

    Ok(())
}

fn extract_binary(data: &[u8], asset_name: &str) -> Result<Vec<u8>> {
    if asset_name.ends_with(".tar.gz") {
        extract_from_tar_gz(data)
    } else if asset_name.ends_with(".zip") {
        extract_from_zip(data)
    } else {
        anyhow::bail!("Unknown archive format")
    }
}

fn extract_from_tar_gz(data: &[u8]) -> Result<Vec<u8>> {
    let gz = flate2::read::GzDecoder::new(Cursor::new(data));
    let mut archive = tar::Archive::new(gz);
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?;
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        if name == "myagent" {
            let mut buf = Vec::new();
            std::io::Read::read_to_end(&mut entry, &mut buf)?;
            return Ok(buf);
        }
    }
    anyhow::bail!("Binary not found in archive")
}

fn extract_from_zip(data: &[u8]) -> Result<Vec<u8>> {
    let reader = Cursor::new(data);
    let mut archive = zip::ZipArchive::new(reader)?;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let name = file.name().to_string();
        if name == "myagent.exe" || name == "myagent" {
            let mut buf = Vec::new();
            std::io::Read::read_to_end(&mut file, &mut buf)?;
            return Ok(buf);
        }
    }
    anyhow::bail!("Binary not found in archive")
}

fn parse_ver(v: &str) -> Option<(u64, u64, u64)> {
    let mut iter = v.trim().split('.');
    let maj = iter.next()?.parse::<u64>().ok()?;
    let min = iter.next()?.parse::<u64>().ok()?;
    let pat = iter.next()?.parse::<u64>().ok()?;
    Some((maj, min, pat))
}