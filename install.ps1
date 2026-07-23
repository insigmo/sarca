# Install Sarca from the latest GitHub Release (Windows amd64).
# Usage:
#   irm https://raw.githubusercontent.com/insigmo/sarca/refs/heads/master/install.ps1 | iex
#   or: .\install.ps1 [-Version v0.0.8] [-Prefix "$env:LOCALAPPDATA\Sarca"]

param(
    [string]$Repo = "insigmo/sarca",
    [string]$Version = $env:SARCA_VERSION,
    [string]$Prefix = $(if ($env:SARCA_HOME) { $env:SARCA_HOME } else { Join-Path $env:LOCALAPPDATA "Sarca" })
)

$ErrorActionPreference = "Stop"

$asset = "sarca_windows_amd64.zip"
if ([string]::IsNullOrWhiteSpace($Version)) {
    $url = "https://github.com/$Repo/releases/latest/download/$asset"
    $label = "latest"
} else {
    $url = "https://github.com/$Repo/releases/download/$Version/$asset"
    $label = $Version
}
$tmp = Join-Path $env:TEMP ("sarca-install-" + [guid]::NewGuid().ToString())
New-Item -ItemType Directory -Path $tmp | Out-Null

Write-Host "Installing Sarca $label ($asset) -> $Prefix"
$zip = Join-Path $tmp $asset
try {
    Invoke-WebRequest -Uri $url -OutFile $zip -UseBasicParsing
} catch {
    throw "Failed to download $url — publish a GitHub Release (tag v*) so /releases/latest has assets. $_"
}
Expand-Archive -Path $zip -DestinationPath $tmp -Force

$extracted = Get-ChildItem -Path $tmp -Directory | Select-Object -First 1
if (-not $extracted -or -not (Test-Path (Join-Path $extracted.FullName "sarca.exe"))) {
    throw "Release archive layout unexpected"
}

New-Item -ItemType Directory -Path $Prefix -Force | Out-Null
$work = Join-Path $Prefix "work"
New-Item -ItemType Directory -Path $work -Force | Out-Null

Copy-Item (Join-Path $extracted.FullName "sarca.exe") (Join-Path $Prefix "sarca.exe") -Force
if (Test-Path (Join-Path $Prefix "ui")) {
    Remove-Item (Join-Path $Prefix "ui") -Recurse -Force
}
Copy-Item (Join-Path $extracted.FullName "ui") (Join-Path $Prefix "ui") -Recurse -Force

$envFile = Join-Path $Prefix ".env"
if (-not (Test-Path $envFile)) {
    $secret = -join ((1..64) | ForEach-Object { "{0:x}" -f (Get-Random -Max 16) })
    @"
PORT=8000
WORKERS=4
CHANNEL_CAPACITY=32
SUPERUSER_EMAIL=admin@example.com
SUPERUSER_PASS=change-me
ACCESS_TOKEN_EXPIRE_IN_SECS=1800
REFRESH_TOKEN_EXPIRE_IN_DAYS=14
SECRET_KEY=$secret

TELEGRAM_LOCAL_API=false
TELEGRAM_API_BASE_URL=https://api.telegram.org
TELEGRAM_RATE_LIMIT=18
TELEGRAM_CHUNK_SIZE_MB=20
WORK_DIR=$($work -replace '\\','/')

DATABASE_USER=sarca
DATABASE_PASSWORD=sarca
DATABASE_NAME=sarca
DATABASE_HOST=127.0.0.1
DATABASE_PORT=5432
"@ | Set-Content -Path $envFile -Encoding UTF8
    Write-Host "Wrote $envFile — edit SUPERUSER_* / SECRET_KEY / DATABASE_* before first run"
}

$launcherPs1 = Join-Path $Prefix "sarca.ps1"
@"
`$ErrorActionPreference = 'Stop'
Set-Location '$Prefix'
if (Test-Path .env) {
  Get-Content .env | ForEach-Object {
    if (`$_ -match '^\s*#' -or `$_ -match '^\s*$') { return }
    `$name, `$value = `$_.Split('=', 2)
    if (`$name -and `$value -ne `$null) { Set-Item -Path ("Env:" + `$name) -Value `$value }
  }
}
& '$Prefix\sarca.exe' @args
"@ | Set-Content -Path $launcherPs1 -Encoding UTF8

$launcherCmd = Join-Path $Prefix "sarca.cmd"
@"
@echo off
powershell -NoProfile -ExecutionPolicy Bypass -File "$launcherPs1" %*
"@ | Set-Content -Path $launcherCmd -Encoding ASCII

Remove-Item $tmp -Recurse -Force

Write-Host ""
Write-Host "Installed."
Write-Host "  app:      $Prefix"
Write-Host "  launcher: $launcherCmd"
Write-Host ""
Write-Host "Next:"
Write-Host "  1. Edit $envFile"
Write-Host "  2. Ensure Postgres is reachable"
Write-Host "  3. Run:  $launcherCmd"
Write-Host "  4. Open http://127.0.0.1:8000"
