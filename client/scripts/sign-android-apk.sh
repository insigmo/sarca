#!/usr/bin/env bash
# Sign an Android APK with either ANDROID_KEYSTORE_* secrets or the committed sideload keystore.
# Usage: sign-android-apk.sh <input.apk> <output.apk>
set -euo pipefail

IN="${1:?input apk}"
OUT="${2:?output apk}"

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEFAULT_KS="${ROOT}/mobile/sarca-sideload.p12"
DEFAULT_ALIAS="sarca"
DEFAULT_PASS="sarca-sideload"

if [[ -n "${ANDROID_KEYSTORE_BASE64:-}" ]]; then
  KS="$(mktemp /tmp/sarca-ks.XXXXXX.p12)"
  trap 'rm -f "$KS"' EXIT
  echo "${ANDROID_KEYSTORE_BASE64}" | base64 -d >"$KS"
  ALIAS="${ANDROID_KEY_ALIAS:-sarca}"
  STORE_PASS="${ANDROID_KEYSTORE_PASSWORD:?ANDROID_KEYSTORE_PASSWORD required with ANDROID_KEYSTORE_BASE64}"
  KEY_PASS="${ANDROID_KEY_PASSWORD:-$STORE_PASS}"
else
  KS="$DEFAULT_KS"
  ALIAS="$DEFAULT_ALIAS"
  STORE_PASS="$DEFAULT_PASS"
  KEY_PASS="$DEFAULT_PASS"
  echo "Using committed sideload keystore: $KS"
fi

test -f "$IN"
test -f "$KS"

APKSIGNER="$(command -v apksigner || true)"
if [[ -z "$APKSIGNER" || ! -x "$APKSIGNER" ]]; then
  # GitHub-hosted / local Android SDK layouts
  shopt -s nullglob
  candidates=(
    "${ANDROID_HOME:-}/build-tools/34.0.0/apksigner"
    "${ANDROID_SDK_ROOT:-}/build-tools/34.0.0/apksigner"
    "${ANDROID_HOME:-}/build-tools/"*/apksigner
    "${ANDROID_SDK_ROOT:-}/build-tools/"*/apksigner
    /tmp/bt-mini/android-14/apksigner
  )
  for c in "${candidates[@]}"; do
    if [[ -x "$c" ]]; then
      APKSIGNER="$c"
      break
    fi
  done
  shopt -u nullglob
fi
if [[ -z "${APKSIGNER:-}" || ! -x "$APKSIGNER" ]]; then
  echo "apksigner not found (install Android build-tools)" >&2
  exit 1
fi
echo "Using apksigner: $APKSIGNER"

"$APKSIGNER" sign \
  --ks "$KS" \
  --ks-key-alias "$ALIAS" \
  --ks-pass "pass:${STORE_PASS}" \
  --key-pass "pass:${KEY_PASS}" \
  --v1-signing-enabled true \
  --v2-signing-enabled true \
  --v3-signing-enabled true \
  --v4-signing-enabled false \
  --out "$OUT" \
  "$IN"

rm -f "${OUT}.idsig"

"$APKSIGNER" verify --verbose "$OUT"
python3 - "$OUT" <<'PY'
import sys
path = sys.argv[1]
data = open(path, "rb").read()
if b"APK Sig Block 42" not in data:
    raise SystemExit(f"APK lacks v2/v3 signature block: {path}")
print(f"signed ok: {path}")
PY
