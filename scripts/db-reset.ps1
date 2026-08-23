# Wipe SQLite metadata (delete database file), then restart Sarca
# so init_db + create_superuser recreate an empty schema.
#
# Windows counterpart of db-reset.sh -- same steps, no sed/grep/curl needed.
# Usage: powershell -File scripts/db-reset.ps1

param(
    [string]$RootDir = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
)

$ErrorActionPreference = "Stop"

$root = (Resolve-Path $RootDir).Path
$conf = Join-Path $root "sarca.conf"
if (-not (Test-Path $conf)) {
    Write-Error "error: $conf not found"
    exit 1
}

function Get-ConfValue([string]$Key) {
    foreach ($line in Get-Content -LiteralPath $conf) {
        if ($line -match "^\s*$([regex]::Escape($Key))=(.*)$") {
            return $Matches[1].Trim()
        }
    }
    return ""
}

$workDir = Get-ConfValue "WORK_DIR"
if ([string]::IsNullOrWhiteSpace($workDir)) { $workDir = Join-Path $root "work" }
$sqlitePath = Get-ConfValue "SQLITE_PATH"
if ([string]::IsNullOrWhiteSpace($sqlitePath)) { $sqlitePath = Join-Path $workDir "sarca.sqlite" }

$dbFiles = @($sqlitePath, "$sqlitePath-wal", "$sqlitePath-shm")
if (-not ($dbFiles | Where-Object { Test-Path -LiteralPath $_ })) {
    Write-Host "No SQLite database at $sqlitePath -- nothing to wipe."
    exit 0
}

$compose = @("compose", "-f", "compose.yml", "-f", "compose.dev.yml", "--env-file", "sarca.conf")

Push-Location $root
try {
    $running = @()
    try {
        $running = @(docker @compose ps --status running --services 2>$null)
    }
    catch {
        $running = @()
    }

    $usingDocker = [bool]($running | Where-Object { $_.Trim() -eq "sarca" })
    if ($usingDocker) {
        Write-Host "Stopping Sarca container..."
        docker @compose stop sarca | Out-Null
    }

    Write-Host "WARNING: deleting SQLite database at $sqlitePath..."
    foreach ($f in $dbFiles) {
        if (Test-Path -LiteralPath $f) {
            Remove-Item -LiteralPath $f -Force
        }
    }

    if (-not $usingDocker) {
        Write-Host "DB file removed. Restart Sarca to recreate schema + superuser."
        Write-Host "DB reset complete (empty schema + superuser from sarca.conf)."
        exit 0
    }

    Write-Host "Starting Sarca to recreate schema + superuser..."
    docker @compose start sarca | Out-Null

    $port = Get-ConfValue "PORT"
    if ([string]::IsNullOrWhiteSpace($port)) { $port = "8000" }

    $ok = $false
    foreach ($attempt in 1..60) {
        try {
            Invoke-WebRequest -Uri "http://127.0.0.1:$port/" -UseBasicParsing -TimeoutSec 2 | Out-Null
            $ok = $true
            break
        }
        catch {
            Start-Sleep -Milliseconds 500
        }
    }

    if (-not $ok) {
        Write-Error "error: Sarca did not become ready on :$port -- check: docker logs sarca"
        exit 1
    }
}
finally {
    Pop-Location
}

Write-Host "DB reset complete (empty schema + superuser from sarca.conf)."
