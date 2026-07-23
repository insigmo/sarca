#!/bin/sh
# Load TELEGRAM_API_ID / TELEGRAM_API_HASH from mounted sarca.conf, then
# hand off to the image's docker-entrypoint.sh.
set -e

CONF="${SARCA_CONF:-/sarca.conf}"

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

exec /docker-entrypoint.sh "$@"
