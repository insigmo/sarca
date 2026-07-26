#!/usr/bin/env bash
# Build the Sarca Linux .deb (Tauri) and install it on this machine.
# Needs sudo for apt-get (will prompt for a password in a TTY).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
REPO_ROOT="$(cd "$ROOT/.." && pwd)"
# shellcheck source=../../scripts/ensure-pnpm-env.sh
. "$REPO_ROOT/scripts/ensure-pnpm-env.sh"
cd "$ROOT"

echo "Building Linux .deb (pnpm tauri build --bundles deb)…"
# shellcheck disable=SC2086 # PNPM may be "corepack pnpm"
$PNPM tauri build --bundles deb

search_dirs=()
[[ -d "$ROOT/src-tauri/target" ]] && search_dirs+=("$ROOT/src-tauri/target")
[[ -d "$ROOT/../target" ]] && search_dirs+=("$ROOT/../target")

if [[ ${#search_dirs[@]} -eq 0 ]]; then
  echo "error: no Cargo target directory found under client/src-tauri/target or repo target/" >&2
  exit 1
fi

DEB="$(
  find "${search_dirs[@]}" -type f -path '*/release/bundle/deb/*.deb' -printf '%T@\t%p\n' 2>/dev/null \
    | sort -nr \
    | head -1 \
    | cut -f2-
)"

if [[ -z "${DEB}" || ! -f "${DEB}" ]]; then
  echo "error: no .deb produced under */release/bundle/deb/" >&2
  exit 1
fi

echo "Installing ${DEB}…"
sudo apt-get install -y "${DEB}"
echo "Installed ${DEB}"
