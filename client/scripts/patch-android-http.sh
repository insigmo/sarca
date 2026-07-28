#!/usr/bin/env bash
# Post-init patches for the generated Tauri Android project:
# - Allow http:// (cleartext) for self-hosted Sarca on LAN (https:// still works)
# - Install SAF folder-picker + startup permission plugins
# - Replace Tauri default launcher icons with Sarca icons from src-tauri/icons/android
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
GEN="${1:-$ROOT/src-tauri/gen/android}"
APP_SRC="$GEN/app/src/main"
MANIFEST="$APP_SRC/AndroidManifest.xml"
RES_DIR="$APP_SRC/res"
XML_DIR="$RES_DIR/xml"
SRC_CFG="$ROOT/mobile/android/res/xml/network_security_config.xml"
ICONS_SRC="$ROOT/src-tauri/icons/android"
FOLDER_PICKER_SRC="$ROOT/mobile/android/java/app/sarca/client/folderpicker/FolderPickerPlugin.kt"
FOLDER_PICKER_DST="$APP_SRC/java/app/sarca/client/folderpicker/FolderPickerPlugin.kt"
STARTUP_SRC="$ROOT/mobile/android/java/app/sarca/client/startup/StartupPlugin.kt"
STARTUP_DST="$APP_SRC/java/app/sarca/client/startup/StartupPlugin.kt"

if [[ ! -f "$MANIFEST" ]]; then
  echo "AndroidManifest not found at $MANIFEST (run tauri android init first)" >&2
  exit 1
fi

mkdir -p "$XML_DIR"
cp -a "$SRC_CFG" "$XML_DIR/network_security_config.xml"

# `tauri android init` seeds Tauri's default logo. `tauri icon` only writes into
# gen/android when that tree already exists; CI inits after icons are generated
# into icons/android/, so we must copy Sarca mipmaps in explicitly.
if [[ ! -d "$ICONS_SRC" ]]; then
  echo "Sarca Android icons not found at $ICONS_SRC (run: pnpm exec tauri icon ../logo.svg)" >&2
  exit 1
fi
if [[ ! -d "$RES_DIR" ]]; then
  echo "Android res/ not found at $RES_DIR" >&2
  exit 1
fi
# Drop template vector layers that would otherwise compete with Sarca mipmaps.
rm -f \
  "$RES_DIR/drawable/ic_launcher_background.xml" \
  "$RES_DIR/drawable-v24/ic_launcher_foreground.xml"
cp -a "$ICONS_SRC"/. "$RES_DIR"/
echo "Installed Sarca launcher icons → $RES_DIR"

if [[ -f "$FOLDER_PICKER_SRC" ]]; then
  mkdir -p "$(dirname "$FOLDER_PICKER_DST")"
  cp -a "$FOLDER_PICKER_SRC" "$FOLDER_PICKER_DST"
  echo "Installed FolderPickerPlugin.kt → $FOLDER_PICKER_DST"
fi

if [[ -f "$STARTUP_SRC" ]]; then
  mkdir -p "$(dirname "$STARTUP_DST")"
  cp -a "$STARTUP_SRC" "$STARTUP_DST"
  echo "Installed StartupPlugin.kt → $STARTUP_DST"
fi

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
# Battery exemption so background sync is less likely to be killed.
for perm in (
    "android.permission.READ_MEDIA_IMAGES",
    "android.permission.READ_MEDIA_VIDEO",
    "android.permission.READ_EXTERNAL_STORAGE",
    "android.permission.REQUEST_IGNORE_BATTERY_OPTIMIZATIONS",
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
print(f"Patched cleartext HTTP + media/battery permissions into {path}")
PY
