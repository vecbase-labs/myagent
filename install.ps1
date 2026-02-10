$ErrorActionPreference = "Stop"

$repo = "vecbase-labs/myagent"
$installDir = "$env:USERPROFILE\.myagent\bin"

Write-Host "Installing myagent..."

# 1. Get latest version
$release = Invoke-RestMethod "https://api.github.com/repos/$repo/releases/latest"
$version = $release.tag_name
Write-Host "Downloading myagent $version..."

# 2. Download
$filename = "myagent-windows-x86_64.zip"
$url = "https://github.com/$repo/releases/download/$version/$filename"
$tmp = "$env:TEMP\$filename"
Invoke-WebRequest $url -OutFile $tmp

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
