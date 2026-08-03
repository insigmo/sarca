#!/usr/bin/env bash
# Sign an Android APK with the ANDROID_KEYSTORE_* secrets.
# Usage: sign-android-apk.sh <input.apk> <output.apk>
#
# The committed `mobile/sarca-sideload.p12` is a throwaway key: it lives in this
# repository together with its password, so anyone can sign an APK with it.
# Android identifies an app by its signer, so a forged APK signed with that key
# installs over a real Sarca as an "update" and inherits its data directory,
# granted permissions and MediaStore access. It is therefore usable only for
# local and CI smoke builds, and only when the caller opts in explicitly with
# SARCA_ALLOW_PUBLIC_KEYSTORE=1. Anything distributed to users must be signed
# with a private key.
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
elif [[ "${SARCA_ALLOW_PUBLIC_KEYSTORE:-0}" == "1" ]]; then
  KS="$DEFAULT_KS"
  ALIAS="$DEFAULT_ALIAS"
  STORE_PASS="$DEFAULT_PASS"
  KEY_PASS="$DEFAULT_PASS"
  echo "WARNING: signing with the PUBLIC committed sideload keystore: $KS" >&2
  echo "WARNING: the private key and password are in the repository. Never distribute this APK." >&2
else
  cat >&2 <<'EOF'
ANDROID_KEYSTORE_BASE64 / ANDROID_KEYSTORE_PASSWORD are not set.

Refusing to sign with the committed sideload keystore: its private key and
password are public, so the resulting APK can be forged by anyone and Android
would accept the forgery as an update of the installed app.

  * To ship: generate a private release key and export it to the environment.
      keytool -genkeypair -v -keystore release.p12 -storetype PKCS12 \
        -alias sarca -keyalg RSA -keysize 4096 -validity 10000
      export ANDROID_KEYSTORE_BASE64="$(base64 -w0 release.p12)"
      export ANDROID_KEYSTORE_PASSWORD=...
  * For a local throwaway build only: SARCA_ALLOW_PUBLIC_KEYSTORE=1
EOF
  exit 1
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
