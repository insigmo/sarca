# Build the native Windows server binary and install it into the native prefix.
#
# The Windows half of `task deploy-native`. The Linux/macOS half installs the
# same binary `task binary` produced; on Windows `task binary` deliberately
# builds a Linux ELF for Docker, so the native .exe is compiled here instead.
#
# Usage: powershell -File scripts/install-native.ps1 [-Prefix <dir>]

param(
    [string]$RootDir = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path,
    # Same location install.ps1 uses, so a dev build lands where a released
    # install would.
    [string]$Prefix = $(if ($env:SARCA_HOME) { $env:SARCA_HOME } else { Join-Path $env:LOCALAPPDATA "Sarca" })
)

$ErrorActionPreference = "Stop"

$root = (Resolve-Path $RootDir).Path

Push-Location $root
try {
    cargo build --release -p sarca
    if ($LASTEXITCODE -ne 0) {
        throw "cargo build failed (exit $LASTEXITCODE)"
    }

    # Honours CARGO_TARGET_DIR / .cargo/config.toml instead of assuming ./target.
    $metadata = cargo metadata --format-version 1 --no-deps | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0) {
        throw "cargo metadata failed (exit $LASTEXITCODE)"
    }
    $targetDir = $metadata.target_directory
}
finally {
    Pop-Location
}

$src = Join-Path $targetDir "release\sarca.exe"
if (-not (Test-Path $src)) {
    throw "built binary not found at $src"
}

if (-not (Test-Path $Prefix)) {
    New-Item -ItemType Directory -Path $Prefix | Out-Null
}

$dest = Join-Path $Prefix "sarca.exe"
Copy-Item -Path $src -Destination $dest -Force

Write-Host "native binary -> $dest"
