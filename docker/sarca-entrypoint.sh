#!/bin/sh
# Start as root so named volumes can be chown'd, then drop to nobody.
set -eu

WORK_DIR="${WORK_DIR:-/work}"
mkdir -p "$WORK_DIR/uploads"

if [ "$(id -u)" -eq 0 ]; then
	chown -R nobody:nogroup "$WORK_DIR" 2>/dev/null || chown -R 65534:65534 "$WORK_DIR"
	if command -v runuser >/dev/null 2>&1; then
		exec runuser -u nobody -g nogroup -- /sarca
	fi
	exec su -s /bin/sh nobody -c 'exec /sarca'
fi

# Already non-root: cannot chown volumes; run the app as-is.
exec /sarca
