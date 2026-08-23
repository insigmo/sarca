# Build the Linux server binary into runtime/sarca from a Windows host.
#
# `task up` bind-mounts runtime/sarca into the Linux container, so the file has
# to be an ELF binary -- a native `cargo build` on Windows produces a PE that the
# container cannot exec. Docker is already required by `task up`, so the build
# runs inside a Linux Rust image instead of cross-compiling.
#
# Usage: powershell -File scripts/build-linux-binary.ps1 [-RootDir <repo>]

param(
    [string]$RootDir = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path,
    [string]$Image = $(if ($env:SARCA_LINUX_BUILDER_IMAGE) { $env:SARCA_LINUX_BUILDER_IMAGE } else { "rust:1-bookworm" }),
    # Named volumes, not the bind mount: Linux artifacts stay out of the host's
    # target/ (used by native builds and by `task lint`), and the build tree
    # avoids slow bind-mount I/O.
    [string]$CargoVolume = "sarca-linux-cargo",
    [string]$TargetVolume = "sarca-linux-target"
)

$ErrorActionPreference = "Stop"

$root = (Resolve-Path $RootDir).Path
$runtime = Join-Path $root "runtime"
if (-not (Test-Path $runtime)) {
    New-Item -ItemType Directory -Path $runtime | Out-Null
}

if (-not (Get-Command docker -ErrorAction SilentlyContinue)) {
    throw "docker not found on PATH -- it is required to build the Linux binary on Windows"
}

Write-Host "building linux sarca in $Image (cache volumes: $CargoVolume, $TargetVolume)"
Write-Host "first run downloads the image and the crate index; later runs reuse the volumes"

# Everything happens in the container: the copy into the bind mount runs as
# root there, so a runtime/sarca left root-owned by an earlier Docker copy is
# still overwritable.
$build = 'set -eu; cargo build --release -p sarca; cp /target/release/sarca /src/runtime/sarca; chmod 755 /src/runtime/sarca'

docker run --rm `
    -v "${root}:/src" `
    -v "${CargoVolume}:/usr/local/cargo/registry" `
    -v "${TargetVolume}:/target" `
    -w /src `
    -e CARGO_TARGET_DIR=/target `
    $Image sh -c $build

if ($LASTEXITCODE -ne 0) {
    throw "linux build failed (docker run exited $LASTEXITCODE)"
}

Write-Host "binary -> $(Join-Path $runtime 'sarca')"
