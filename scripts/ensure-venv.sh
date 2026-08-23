#!/bin/sh
# Source from Taskfile so the repo's Python venv resolves on every host.
#
# Usage: . /path/to/ensure-venv.sh [venv-dir]
# After sourcing, VENV_PY holds the venv interpreter (absolute when the dir is).
#
# Windows venvs put their executables in Scripts/ and name them *.exe, POSIX
# ones use bin/ — hardcoding either path is what makes `task e2e` fail on the
# other platform.

_sarca_venv_dir="${1:-${SARCA_VENV:-.venv}}"

_sarca_venv_py() {
  # -f, not -x: Go's stat (Task's shell interpreter) reports no executable bit
  # for Windows files, so -x never matches Scripts/python.exe.
  for _cand in "$1/bin/python" "$1/Scripts/python.exe"; do
    if [ -f "$_cand" ]; then
      echo "$_cand"
      return 0
    fi
  done
  return 1
}

_sarca_host_py() {
  # `python3` on Windows is often the Store stub, which prints an ad and exits
  # non-zero, so every candidate has to prove it can actually run something.
  for _cand in python3 python; do
    command -v "$_cand" >/dev/null 2>&1 || continue
    if "$_cand" -c "import sys" >/dev/null 2>&1; then
      echo "$_cand"
      return 0
    fi
  done
  return 1
}

if ! VENV_PY="$(_sarca_venv_py "$_sarca_venv_dir")"; then
  if ! _sarca_host_py_bin="$(_sarca_host_py)"; then
    echo "error: no working python found (looked for python3, python)" >&2
    return 1 2>/dev/null || exit 1
  fi
  "$_sarca_host_py_bin" -m venv "$_sarca_venv_dir" || {
    echo "error: python -m venv $_sarca_venv_dir failed" >&2
    return 1 2>/dev/null || exit 1
  }
  if ! VENV_PY="$(_sarca_venv_py "$_sarca_venv_dir")"; then
    echo "error: venv created but no interpreter under $_sarca_venv_dir" >&2
    return 1 2>/dev/null || exit 1
  fi
fi

export VENV_PY
unset _sarca_venv_dir _sarca_host_py_bin 2>/dev/null || true
unset -f _sarca_venv_py _sarca_host_py 2>/dev/null || true
