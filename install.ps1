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

function Get-EnvValue {
    param([string]$Path, [string]$Key)
    if (-not (Test-Path $Path)) { return "" }
    $pattern = "^\s*$([regex]::Escape($Key))=(.*)$"
    $match = Select-String -Path $Path -Pattern $pattern | Select-Object -First 1
    if (-not $match) { return "" }
    return [string]$match.Matches[0].Groups[1].Value.Trim()
}

function Set-EnvKey {
    param([string]$Path, [string]$Key, [string]$Value)
    $lines = @()
    if (Test-Path $Path) {
        $lines = Get-Content -Path $Path
    }
    $pattern = "^\s*$([regex]::Escape($Key))="
    $done = $false
    $out = foreach ($line in $lines) {
        if ($line -match $pattern) {
            if (-not $done) {
                "$Key=$Value"
                $done = $true
            }
        } else {
            $line
        }
    }
    if (-not $done) {
        $out = @($out) + @("$Key=$Value")
    }
    Set-Content -Path $Path -Value $out -Encoding UTF8
}

function New-SecretKey {
    $openssl = Get-Command openssl -ErrorAction SilentlyContinue
    if ($openssl) {
        return (& openssl rand -hex 512).Trim()
    }
    $bytes = New-Object byte[] 512
    $rng = [System.Security.Cryptography.RandomNumberGenerator]::Create()
    try {
        $rng.GetBytes($bytes)
    } finally {
        $rng.Dispose()
    }
    return ($bytes | ForEach-Object { $_.ToString("x2") }) -join ""
}

function Test-PlaceholderTelegram([string]$Value) {
    switch ($Value) {
        { $_ -in @("", "00000000", "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx", "XXX", "xxx") } { return $true }
        default { return $false }
    }
}

function Test-PlaceholderSecret([string]$Value) {
    switch ($Value) {
        { $_ -in @("", "XXX", "xxx", "change-me", "change-me-to-a-long-random-string") } { return $true }
        default { return $false }
    }
}

function Test-PlaceholderEmail([string]$Value) {
    switch ($Value) {
        { $_ -in @("", "admin@example.com", "sarca@sarca.sarca") } { return $true }
        default { return $false }
    }
}

function Test-PlaceholderPassword([string]$Value) {
    switch ($Value) {
        { $_ -in @("", "change-me", "sarca") } { return $true }
        default { return $false }
    }
}

function Read-SecretHost([string]$Prompt) {
    $secure = Read-Host $Prompt -AsSecureString
    $bstr = [System.Runtime.InteropServices.Marshal]::SecureStringToBSTR($secure)
    try {
        return [System.Runtime.InteropServices.Marshal]::PtrToStringBSTR($bstr)
    } finally {
        [System.Runtime.InteropServices.Marshal]::ZeroFreeBSTR($bstr)
    }
}

function Configure-Interactive {
    param([string]$EnvFile)

    $email = Get-EnvValue -Path $EnvFile -Key "SUPERUSER_EMAIL"
    $password = Get-EnvValue -Path $EnvFile -Key "SUPERUSER_PASS"

    if ((Test-PlaceholderEmail $email) -or (Test-PlaceholderPassword $password)) {
        Write-Host ""
        Write-Host "Bootstrap admin account (used to sign in to the web UI)"
        Write-Host ""
        while (Test-PlaceholderEmail $email) {
            $email = Read-Host "SUPERUSER_EMAIL"
            if (Test-PlaceholderEmail $email) {
                Write-Host "email is required"
            }
        }
        while (Test-PlaceholderPassword $password) {
            $password = Read-SecretHost "SUPERUSER_PASS"
            if (Test-PlaceholderPassword $password) {
                Write-Host "password is required"
            }
        }
        Set-EnvKey -Path $EnvFile -Key "SUPERUSER_EMAIL" -Value $email
        Set-EnvKey -Path $EnvFile -Key "SUPERUSER_PASS" -Value $password
    } else {
        Write-Host "Admin credentials already set — skipping prompt"
    }

    $apiId = Get-EnvValue -Path $EnvFile -Key "TELEGRAM_API_ID"
    $apiHash = Get-EnvValue -Path $EnvFile -Key "TELEGRAM_API_HASH"

    if ((Test-PlaceholderTelegram $apiId) -or (Test-PlaceholderTelegram $apiHash)) {
        Write-Host ""
        Write-Host "Telegram API credentials (needed for Local Bot API / large files)"
        Write-Host "  1. Open https://my.telegram.org and sign in"
        Write-Host "  2. Open API development tools"
        Write-Host "  3. Create an app if needed, then copy api_id and api_hash"
        Write-Host ""
        while (Test-PlaceholderTelegram $apiId) {
            $apiId = Read-Host "TELEGRAM_API_ID"
            if (Test-PlaceholderTelegram $apiId) {
                Write-Host "api_id is required"
            }
        }
        while (Test-PlaceholderTelegram $apiHash) {
            $apiHash = Read-Host "TELEGRAM_API_HASH"
            if (Test-PlaceholderTelegram $apiHash) {
                Write-Host "api_hash is required"
            }
        }
        Set-EnvKey -Path $EnvFile -Key "TELEGRAM_API_ID" -Value $apiId
        Set-EnvKey -Path $EnvFile -Key "TELEGRAM_API_HASH" -Value $apiHash
    } else {
        Write-Host "Telegram API credentials already set — skipping prompt"
    }

    $secret = Get-EnvValue -Path $EnvFile -Key "SECRET_KEY"
    if (Test-PlaceholderSecret $secret) {
        $secret = New-SecretKey
        Set-EnvKey -Path $EnvFile -Key "SECRET_KEY" -Value $secret
        Write-Host "Generated SECRET_KEY (openssl rand -hex 512)"
    }
}

function Merge-EnvDefaults {
    param(
        [string]$EnvFile,
        [System.Collections.Specialized.OrderedDictionary]$Defaults
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
    $secret = New-SecretKey
    $workUnix = ($WorkDir -replace '\\', '/')
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
        Write-Host "Wrote $envFile"
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
$envFile = Join-Path $Prefix "sarca.conf"
Configure-Interactive -EnvFile $envFile

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

$port = Get-EnvValue -Path $envFile -Key "PORT"
if ([string]::IsNullOrWhiteSpace($port)) { $port = "8000" }

Write-Host ""
Write-Host "Installed $Version."
Write-Host "  app:      $Prefix"
Write-Host "  version:  $(Join-Path $Prefix 'VERSION')"
Write-Host "  launcher: $launcherCmd"
Write-Host ""
Write-Host "Starting Sarca (Postgres must be reachable — DATABASE_* in sarca.conf)…"
Start-Process -FilePath $launcherCmd -WindowStyle Hidden
Write-Host "Started."
Write-Host "Open http://127.0.0.1:$port"
