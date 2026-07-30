#!/bin/sh
# Start as root so named volumes can be chown'd, then drop to nobody.
set -eu

WORK_DIR="${WORK_DIR:-/work}"
BOT_DATA="${TELEGRAM_BOT_API_DATA:-/var/lib/telegram-bot-api}"

mkdir -p "$WORK_DIR/uploads" "$WORK_DIR/chunk_cache" "$WORK_DIR/preview_cache"

if [ "$(id -u)" -eq 0 ]; then
	# Local Bot API creates 0750 dirs; Sarca (nobody) must read documents for preview/download.
	if [ -d "$BOT_DATA" ]; then
		chmod -R a+rX "$BOT_DATA" 2>/dev/null || true
		find "$BOT_DATA" -type d -name documents -exec chmod -R a+rwX {} \; 2>/dev/null || true
	fi
	chown -R nobody:nogroup "$WORK_DIR" 2>/dev/null || chown -R 65534:65534 "$WORK_DIR"
	if command -v runuser >/dev/null 2>&1; then
		exec runuser -u nobody -g nogroup -- /sarca
	fi
	exec su -s /bin/sh nobody -c 'exec /sarca'
fi

# Already non-root: cannot chown volumes; run the app as-is.
exec /sarca
