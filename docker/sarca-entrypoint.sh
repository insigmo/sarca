#!/bin/sh
# Start as root so named volumes can be chown'd, then drop to nobody.
set -eu

WORK_DIR="${WORK_DIR:-/work}"
BOT_DATA="${TELEGRAM_BOT_API_DATA:-/var/lib/telegram-bot-api}"

mkdir -p "$WORK_DIR/uploads" "$WORK_DIR/chunk_cache" "$WORK_DIR/preview_cache"

fix_bot_api_perms() {
	# Local Bot API creates 0750 dirs / owner-only files. Sarca runs as nobody
	# in this container with the same volume mounted and must read+unlink documents.
	[ -d "$BOT_DATA" ] || return 0
	chmod -R a+rX "$BOT_DATA" 2>/dev/null || true
	find "$BOT_DATA" -type d -name documents -exec chmod -R a+rwX {} \; 2>/dev/null || true
}

if [ "$(id -u)" -eq 0 ]; then
	fix_bot_api_perms
	# Keep fixing perms for files created after startup (telegram umask/0750 race).
	# Must start before exec so the loop stays root while the app runs as nobody.
	(
		while true; do
			sleep 1
			fix_bot_api_perms
		done
	) &
	chown -R nobody:nogroup "$WORK_DIR" 2>/dev/null || chown -R 65534:65534 "$WORK_DIR"
	if command -v runuser >/dev/null 2>&1; then
		exec runuser -u nobody -g nogroup -- /sarca
	fi
	exec su -s /bin/sh nobody -c 'exec /sarca'
fi

# Already non-root: cannot chown volumes; run the app as-is.
exec /sarca
