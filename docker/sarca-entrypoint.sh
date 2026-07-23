#!/bin/sh
# Ensure WORK_DIR is writable by the app user (named volumes mount as root).
set -e

WORK_DIR="${WORK_DIR:-/work}"
mkdir -p "$WORK_DIR/uploads"
# UID/GID of `nobody` on Ubuntu/Debian
chown -R 65534:65534 "$WORK_DIR" 2>/dev/null || chown -R nobody:nogroup "$WORK_DIR"

if command -v runuser >/dev/null 2>&1; then
	exec runuser -u nobody -g nogroup -- /sarca
fi
# Fallback if runuser is unavailable
exec /sarca
