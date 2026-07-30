#!/bin/sh
# Load TELEGRAM_API_ID / TELEGRAM_API_HASH from mounted sarca.conf, then
# hand off to the image's docker-entrypoint.sh.
set -eu

CONF="${SARCA_CONF:-/sarca.conf}"
DATA_DIR="${TELEGRAM_WORK_DIR:-/var/lib/telegram-bot-api}"

conf_get() {
	key="$1"
	line=$(grep -E "^[[:space:]]*${key}=" "$CONF" 2>/dev/null | head -1) || true
	[ -n "$line" ] || return 0
	val=${line#*=}
	# trim whitespace / CR
	val=$(printf '%s' "$val" | tr -d '\r' | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//')
	# strip matching quotes
	first=$(printf '%s' "$val" | cut -c1)
	last=$(printf '%s' "$val" | awk '{print substr($0,length($0),1)}')
	if [ "$first" = '"' ] && [ "$last" = '"' ]; then
		val=$(printf '%s' "$val" | sed -e 's/^"//' -e 's/"$//')
	elif [ "$first" = "'" ] && [ "$last" = "'" ]; then
		val=$(printf '%s' "$val" | sed -e "s/^'//" -e "s/'$//")
	fi
	printf '%s' "$val"
}

if [ ! -f "$CONF" ]; then
	echo "error: $CONF not found (mount sarca.conf into the container)" >&2
	exit 1
fi

TELEGRAM_API_ID=$(conf_get TELEGRAM_API_ID)
TELEGRAM_API_HASH=$(conf_get TELEGRAM_API_HASH)
export TELEGRAM_API_ID TELEGRAM_API_HASH

if [ -z "$TELEGRAM_API_ID" ] || [ -z "$TELEGRAM_API_HASH" ]; then
	echo "error: set TELEGRAM_API_ID and TELEGRAM_API_HASH in sarca.conf" >&2
	exit 1
fi

# Local Bot API creates per-bot dirs as 0750 (owner telegram-bot-api). Sarca runs as
# `nobody` in another container with the same volume mounted, so it cannot traverse
# those dirs unless they are world-executable/readable. Keep permissions open enough
# for cross-container reads (files stay on the private Docker volume).
#
# Also set a permissive umask so newly created files are more likely world-readable
# before the chmod loop catches them (Sarca retries PermissionDenied briefly).
#
# Sarca deletes Local Bot API `documents/` copies after upload/download; those files
# must be world-writable (or Sarca cannot unlink). Periodic prune covers orphans from
# crashes / older RO mounts (tdlib/telegram-bot-api#303).
umask 022

fix_bot_api_perms() {
	chmod -R a+rX "$DATA_DIR" 2>/dev/null || true
	# Allow Sarca (nobody) to unlink documents after it finishes with them.
	find "$DATA_DIR" -type d -name documents -exec chmod -R a+rwX {} \; 2>/dev/null || true
}

prune_stale_local_copies() {
	# Downloaded/uploaded document copies are safe to remove after Bot API's ~1h window.
	find "$DATA_DIR" -path '*/documents/*' -type f -mmin +90 -delete 2>/dev/null || true
	# Orphaned temp upload staging only (never touch very fresh temps Bot API may use).
	find "$DATA_DIR" -path '*/temp/*' -type f -mmin +1440 -delete 2>/dev/null || true
}

fix_bot_api_perms
prune_stale_local_copies
(
	while true; do
		sleep 1
		fix_bot_api_perms
	done
) &
(
	while true; do
		sleep 300
		prune_stale_local_copies
	done
) &

exec /docker-entrypoint.sh "$@"
