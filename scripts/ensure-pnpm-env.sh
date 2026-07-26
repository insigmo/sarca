#!/bin/sh
# Source from Taskfile / install scripts so pnpm works with a minimal PATH
# (Task and IDEs often omit nvm/fnm/asdf shims from login-shell PATH).
#
# Usage: . /path/to/ensure-pnpm-env.sh
# After sourcing, PNPM is set (e.g. "pnpm" or "corepack pnpm") and PATH is updated.

# Avoid re-running when sourced multiple times in one shell.
if [ -n "${_SARCA_PNPM_ENV_LOADED:-}" ]; then
  return 0 2>/dev/null || exit 0
fi
_SARCA_PNPM_ENV_LOADED=1

_sarca_prepend_path() {
  dir="$1"
  [ -d "$dir" ] || return 0
  case ":${PATH}:" in
    *:"$dir":*) ;;
    *) PATH="$dir:$PATH" ;;
  esac
}

_sarca_prepend_path "${HOME}/.local/share/pnpm"
_sarca_prepend_path "${HOME}/.local/bin"
# Corepack / standalone pnpm locations
_sarca_prepend_path "${HOME}/.cache/node/corepack"
_sarca_prepend_path "${HOME}/.node/corepack/shims"

# nvm: newest version ends up first (prepend in ascending order)
if [ -d "${HOME}/.nvm/versions/node" ]; then
  for d in $(ls -1d "${HOME}/.nvm/versions/node"/v*/bin 2>/dev/null | sort -V); do
    _sarca_prepend_path "$d"
  done
fi

# fnm
if [ -d "${HOME}/.local/share/fnm/node-versions" ]; then
  for d in $(ls -1d "${HOME}/.local/share/fnm/node-versions"/*/installation/bin 2>/dev/null | sort -V); do
    _sarca_prepend_path "$d"
  done
fi

# asdf
if [ -d "${HOME}/.asdf/installs/nodejs" ]; then
  for d in $(ls -1d "${HOME}/.asdf/installs/nodejs"/*/bin 2>/dev/null | sort -V); do
    _sarca_prepend_path "$d"
  done
fi

# volta
_sarca_prepend_path "${HOME}/.volta/bin"

export PATH

if ! command -v pnpm >/dev/null 2>&1; then
  if command -v corepack >/dev/null 2>&1; then
    corepack enable >/dev/null 2>&1 || true
    corepack prepare pnpm@11.17.0 --activate >/dev/null 2>&1 || true
  fi
fi

if command -v pnpm >/dev/null 2>&1; then
  PNPM=pnpm
elif command -v corepack >/dev/null 2>&1; then
  PNPM="corepack pnpm"
else
  echo "error: pnpm not found (install Node.js + enable corepack, or install pnpm)" >&2
  echo "  looked in PATH after adding nvm/fnm/asdf/volta and ~/.local/share/pnpm" >&2
  return 1 2>/dev/null || exit 1
fi

export PNPM
unset -f _sarca_prepend_path 2>/dev/null || true
