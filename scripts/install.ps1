Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$ProgressPreference = 'SilentlyContinue'

# Check for 32-bit Windows
if (-not [Environment]::Is64BitProcess) {
    Write-Error "myagent does not support 32-bit Windows."
    exit 1
}

$repo = "vecbase-labs/myagent"
$installDir = "$env:USERPROFILE\.myagent\bin"

Write-Host "Installing myagent..."

# 1. Get latest version
try {
    $release = Invoke-RestMethod "https://api.github.com/repos/$repo/releases/latest" -ErrorAction Stop
    $version = $release.tag_name
} catch {
    Write-Error "Failed to get latest version: $_"
    exit 1
}

Write-Host "Downloading myagent $version..."

# 2. Download
$filename = "myagent-windows-x86_64.zip"
$url = "https://github.com/$repo/releases/download/$version/$filename"
$tmp = "$env:TEMP\$filename"
try {
    Invoke-WebRequest $url -OutFile $tmp -ErrorAction Stop
} catch {
    Write-Error "Failed to download: $_"
    if (Test-Path $tmp) { Remove-Item -Force $tmp }
    exit 1
}

# 3. Install
New-Item -ItemType Directory -Force -Path $installDir | Out-Null
Expand-Archive -Path $tmp -DestinationPath $installDir -Force
Remove-Item $tmp

# 4. Add to PATH (user-level, permanent)
$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($userPath -notlike "*$installDir*") {
    [Environment]::SetEnvironmentVariable("Path", "$installDir;$userPath", "User")
    $env:Path = "$installDir;$env:Path"
}

Write-Host ""
Write-Host "myagent $version installed to $installDir\myagent.exe"
Write-Host ""
Write-Host "Open a new terminal and run: myagent init"
