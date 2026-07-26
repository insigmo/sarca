#!/usr/bin/env bash
# Allow http:// (cleartext) in the generated Tauri Android project.
# Safe for self-hosted Sarca on LAN; https:// continues to work.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
GEN="${1:-$ROOT/src-tauri/gen/android}"
APP_SRC="$GEN/app/src/main"
MANIFEST="$APP_SRC/AndroidManifest.xml"
XML_DIR="$APP_SRC/res/xml"
SRC_CFG="$ROOT/mobile/android/res/xml/network_security_config.xml"

if [[ ! -f "$MANIFEST" ]]; then
  echo "AndroidManifest not found at $MANIFEST (run tauri android init first)" >&2
  exit 1
fi

mkdir -p "$XML_DIR"
cp -a "$SRC_CFG" "$XML_DIR/network_security_config.xml"

python3 - "$MANIFEST" <<'PY'
import re
import sys
from pathlib import Path

path = Path(sys.argv[1])
text = path.read_text()

if "android.permission.INTERNET" not in text:
    text = re.sub(
        r"(<manifest\b[^>]*>)",
        r'\1\n    <uses-permission android:name="android.permission.INTERNET" />',
        text,
        count=1,
    )

# Storage access for Media auto-upload / folder sync path walking.
for perm in (
    "android.permission.READ_MEDIA_IMAGES",
    "android.permission.READ_MEDIA_VIDEO",
    "android.permission.READ_EXTERNAL_STORAGE",
):
    if perm not in text:
        text = re.sub(
            r"(<manifest\b[^>]*>)",
            rf'\1\n    <uses-permission android:name="{perm}" />',
            text,
            count=1,
        )


def ensure_attr(application_tag: str, name: str, value: str) -> str:
    if re.search(rf"\b{re.escape(name)}\s*=", application_tag):
        return re.sub(
            rf'\b{re.escape(name)}\s*=\s*"[^"]*"',
            f'{name}="{value}"',
            application_tag,
            count=1,
        )
    return re.sub(r">\s*$", f'\n        {name}="{value}">', application_tag, count=1)


match = re.search(r"<application\b[^>]*>", text)
if not match:
    raise SystemExit("no <application> tag in AndroidManifest.xml")
app = match.group(0)
app = ensure_attr(app, "android:usesCleartextTraffic", "true")
app = ensure_attr(app, "android:networkSecurityConfig", "@xml/network_security_config")
text = text[: match.start()] + app + text[match.end() :]
path.write_text(text)
print(f"Patched cleartext HTTP + media storage permissions into {path}")
PY
