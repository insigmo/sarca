#!/usr/bin/env bash
# Wipe all application data in Docker Postgres (sarca DB), then restart Sarca
# so init_db + create_superuser recreate an empty schema.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

COMPOSE=(docker compose --env-file sarca.conf)

if ! "${COMPOSE[@]}" ps --status running --services 2>/dev/null | grep -qx db; then
  echo "error: db service is not running — start the stack first (task up)" >&2
  exit 1
fi

echo "WARNING: dropping ALL tables in Postgres (sarca)…"
"${COMPOSE[@]}" stop sarca >/dev/null

"${COMPOSE[@]}" exec -T db psql -U sarca -d sarca -v ON_ERROR_STOP=1 <<'SQL'
SELECT pg_terminate_backend(pid)
FROM pg_stat_activity
WHERE datname = current_database()
  AND pid <> pg_backend_pid();

DROP SCHEMA public CASCADE;
CREATE SCHEMA public;
GRANT ALL ON SCHEMA public TO sarca;
GRANT ALL ON SCHEMA public TO public;
SQL

echo "Schema wiped. Starting Sarca to recreate tables + superuser…"
"${COMPOSE[@]}" start sarca >/dev/null

# Wait until Sarca is accepting HTTP again (schema init finishes at listen).
PORT="$(grep -E '^[[:space:]]*PORT=' sarca.conf 2>/dev/null | head -1 | cut -d= -f2- | tr -d '[:space:]')"
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

echo "DB reset complete (empty schema + superuser from sarca.conf)."
