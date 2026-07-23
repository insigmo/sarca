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

function Migrate-LegacyEnv([string]$Prefix) {
    $conf = Join-Path $Prefix "sarca.conf"
    $legacy = Join-Path $Prefix ".env"
    if (-not (Test-Path $conf) -and (Test-Path $legacy)) {
        Move-Item $legacy $conf
        Write-Host "Migrated $legacy -> $conf"
    }
}

function Resolve-SarcaVersion {
    param([string]$Repo, [string]$Version)
    if (-not [string]::IsNullOrWhiteSpace($Version)) {
        return $Version.Trim()
    }
    $headers = @{
        Accept = "application/vnd.github+json"
        "Cache-Control" = "no-cache"
    }
    $release = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest" -Headers $headers
    if (-not $release.tag_name) {
        throw "Could not resolve latest release tag for $Repo"
    }
    return [string]$release.tag_name
}

function Test-EnvHasKey {
    param([string]$Path, [string]$Key)
    if (-not (Test-Path $Path)) { return $false }
    $pattern = "^\s*$([regex]::Escape($Key))="
    return [bool](Select-String -Path $Path -Pattern $pattern -Quiet)
}

function Merge-EnvDefaults {
    param(
        [string]$EnvFile,
        [hashtable]$Defaults
    )
    $added = $false
    foreach ($key in $Defaults.Keys) {
        if (Test-EnvHasKey -Path $EnvFile -Key $key) { continue }
        if (-not $added) {
            Add-Content -Path $EnvFile -Value ""
            $stamp = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mmZ")
            Add-Content -Path $EnvFile -Value "# Added by Sarca installer ($stamp)"
            $added = $true
        }
        Add-Content -Path $EnvFile -Value "$key=$($Defaults[$key])"
        Write-Host "  + $key"
    }
    if ($added) {
        Write-Host "Merged new keys into $EnvFile (existing values kept)"
    } else {
        Write-Host "Env already has all known keys — left $EnvFile unchanged"
    }
}

function Write-OrMergeEnv {
    param([string]$Prefix, [string]$WorkDir)
    $envFile = Join-Path $Prefix "sarca.conf"
    $secret = -join ((1..64) | ForEach-Object { "{0:x}" -f (Get-Random -Max 16) })
    $workUnix = ($WorkDir -replace '\\', '/')
    # Ordered list for fresh installs; hashtable merge order is not critical.
    $defaultsOrdered = [ordered]@{
        PORT = "8000"
        WORKERS = "4"
        CHANNEL_CAPACITY = "32"
        SUPERUSER_EMAIL = "admin@example.com"
        SUPERUSER_PASS = "change-me"
        ACCESS_TOKEN_EXPIRE_IN_SECS = "1800"
        REFRESH_TOKEN_EXPIRE_IN_DAYS = "14"
        SECRET_KEY = $secret
        TELEGRAM_LOCAL_API = "false"
        TELEGRAM_API_BASE_URL = "https://api.telegram.org"
        TELEGRAM_RATE_LIMIT = "18"
        TELEGRAM_CHUNK_SIZE_MB = "20"
        WORK_DIR = $workUnix
        TELEGRAM_BOT_TOKEN = ""
        TELEGRAM_CHANNEL_ID = ""
        STORAGE_NAME = ""
        TELEGRAM_API_ID = ""
        TELEGRAM_API_HASH = ""
        DATABASE_USER = "sarca"
        DATABASE_PASSWORD = "sarca"
        DATABASE_NAME = "sarca"
        DATABASE_HOST = "127.0.0.1"
        DATABASE_PORT = "5432"
    }

    if (-not (Test-Path $envFile)) {
        $lines = foreach ($key in $defaultsOrdered.Keys) {
            "$key=$($defaultsOrdered[$key])"
        }
        Set-Content -Path $envFile -Value $lines -Encoding UTF8
        Write-Host "Wrote $envFile — edit SUPERUSER_* / SECRET_KEY / DATABASE_* before first run"
        return
    }

    Write-Host "Updating $envFile (keeping existing values)…"
    Merge-EnvDefaults -EnvFile $envFile -Defaults $defaultsOrdered
}

$Version = Resolve-SarcaVersion -Repo $Repo -Version $Version
$asset = "sarca_windows_amd64.zip"
$url = "https://github.com/$Repo/releases/download/$Version/$asset"
$tmp = Join-Path $env:TEMP ("sarca-install-" + [guid]::NewGuid().ToString())
New-Item -ItemType Directory -Path $tmp | Out-Null

$prevFile = Join-Path $Prefix "VERSION"
$prev = ""
if (Test-Path $prevFile) {
    $prev = (Get-Content $prevFile -Raw).Trim()
}
if ($prev -and $prev -eq $Version) {
    Write-Host "Reinstalling Sarca $Version ($asset) -> $Prefix"
} elseif ($prev) {
    Write-Host "Upgrading Sarca $prev -> $Version ($asset) -> $Prefix"
} else {
    Write-Host "Installing Sarca $Version ($asset) -> $Prefix"
}

$zip = Join-Path $tmp $asset
try {
    Invoke-WebRequest -Uri $url -OutFile $zip -UseBasicParsing -Headers @{ "Cache-Control" = "no-cache" }
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
Set-Content -Path (Join-Path $Prefix "VERSION") -Value $Version -Encoding ASCII

Migrate-LegacyEnv -Prefix $Prefix
Write-OrMergeEnv -Prefix $Prefix -WorkDir $work

$launcherPs1 = Join-Path $Prefix "sarca.ps1"
@"
`$ErrorActionPreference = 'Stop'
Set-Location '$Prefix'
if (Test-Path sarca.conf) {
  Get-Content sarca.conf | ForEach-Object {
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

$envFile = Join-Path $Prefix "sarca.conf"
Write-Host ""
Write-Host "Installed $Version."
Write-Host "  app:      $Prefix"
Write-Host "  version:  $(Join-Path $Prefix 'VERSION')"
Write-Host "  launcher: $launcherCmd"
Write-Host ""
Write-Host "Next:"
Write-Host "  1. Edit $envFile"
Write-Host "  2. Ensure Postgres is reachable"
Write-Host "  3. Run:  $launcherCmd"
Write-Host "  4. Open http://127.0.0.1:8000"
