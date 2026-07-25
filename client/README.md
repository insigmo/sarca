# Sarca native client (Tauri 2)

Cross-platform shell around [`sarca-sync`](../crates/sarca-sync) with a lightweight Sync UI.
The existing SolidJS web app in [`../ui`](../ui) remains the full browser UI; this client focuses on
bindings, tray background sync (desktop), and simplified mobile chrome.

## Platforms

| Target | Notes |
| --- | --- |
| Windows amd64 / arm64 | `cargo tauri build --target …` |
| Linux amd64 / arm64 | Needs WebKitGTK 4.1 + GTK 3 dev packages |
| macOS aarch64 | Apple Silicon only (per product plan) |
| Android / iOS | Tauri mobile; run `pnpm tauri android init` / `ios init` once tooling is installed |

## Dev (desktop)

```bash
# Linux deps (Debian/Ubuntu):
#   sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev \
#     libayatana-appindicator3-dev librsvg2-dev patchelf

cd client
pnpm install
pnpm tauri dev
```

## CI artifacts

GitHub Actions workflow [`.github/workflows/client.yml`](../.github/workflows/client.yml) builds in parallel:

| Job | Runner | Bundles |
| --- | --- | --- |
| linux-amd64 | ubuntu-22.04 | deb, AppImage |
| linux-arm64 | ubuntu-24.04-arm | deb, AppImage |
| windows-amd64 | windows-latest | NSIS, MSI |
| windows-arm64 | windows-11-arm | NSIS |
| macos-arm64 | macos-14 | app, dmg |
| android-arm64 | ubuntu + NDK | APK (best-effort) |
| ios-arm64 | macos-14 | simulator / unsigned (best-effort) |

Download from the Actions run → Artifacts (`sarca-client-*`).

## Mobile

Scaffolding lives under `client/mobile/`. After installing Android SDK / Xcode:

```bash
cd client
pnpm tauri android init   # generates gen/android if missing
pnpm tauri ios init       # macOS only
pnpm tauri android build
pnpm tauri ios build
```

Mobile sync is best-effort while foregrounded; OS background limits apply.

## Conflict UI

Engine prompts via `ConflictPrompt`. Desktop default is `KeepBothPrompt` until the webview
dialog is wired; set a custom prompt from Rust to ask the user (keep local / remote / both).
