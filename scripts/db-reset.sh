#!/usr/bin/env bash
# Wipe SQLite metadata (delete database file), then restart Sarca
# so init_db + create_superuser recreate an empty schema.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

CONF="${ROOT}/sarca.conf"
if [ ! -f "${CONF}" ]; then
  echo "error: ${CONF} not found" >&2
  exit 1
fi

env_get() {
  sed -n "s/^[[:space:]]*$1=//p" "${CONF}" | head -1 | tr -d '\r'
}

WORK_DIR="$(env_get WORK_DIR)"
WORK_DIR="${WORK_DIR:-${ROOT}/work}"
SQLITE_PATH="$(env_get SQLITE_PATH)"
SQLITE_PATH="${SQLITE_PATH:-${WORK_DIR}/sarca.sqlite}"

if [ ! -f "${SQLITE_PATH}" ] && [ ! -f "${SQLITE_PATH}-wal" ] && [ ! -f "${SQLITE_PATH}-shm" ]; then
  echo "No SQLite database at ${SQLITE_PATH} — nothing to wipe."
  exit 0
fi

COMPOSE=(docker compose -f compose.yml -f compose.dev.yml --env-file sarca.conf)
using_docker=0
if "${COMPOSE[@]}" ps --status running --services 2>/dev/null | grep -qx sarca; then
  using_docker=1
  echo "Stopping Sarca container…"
  "${COMPOSE[@]}" stop sarca >/dev/null
fi

echo "WARNING: deleting SQLite database at ${SQLITE_PATH}…"
rm -f "${SQLITE_PATH}" "${SQLITE_PATH}-wal" "${SQLITE_PATH}-shm"

if [ "${using_docker}" -eq 1 ]; then
  echo "Starting Sarca to recreate schema + superuser…"
  "${COMPOSE[@]}" start sarca >/dev/null

  PORT="$(env_get PORT)"
  PORT="${PORT:-8000}"
  ok=0
  for _ in $(seq 1 60); do
    if curl -sf -o /dev/null "http://127.0.0.1:${PORT}/"; then
      ok=1
      break
    fi
    sleep 0.5
  done

  if [[ "$ok" -ne 1 ]]; then
    echo "error: Sarca did not become ready on :${PORT} — check: docker logs sarca" >&2
    exit 1
  fi
else
  echo "DB file removed. Restart Sarca to recreate schema + superuser."
fi

echo "DB reset complete (empty schema + superuser from sarca.conf)."
