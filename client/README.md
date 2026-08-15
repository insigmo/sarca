# Sarca native client (Tauri 2)

Thin native shell around your Sarca server: enter the **server URL**, then the webview loads the
**same web UI** hosted by that server (sign in on the website). All buttons and features match
the browser. Phone screens use the site’s existing mobile layout (≤840px). Desktop keeps tray
sync in the background via [`sarca-sync`](../crates/sarca-sync).

## Platforms

| Target | Notes |
| --- | --- |
| Windows amd64 / arm64 | `cargo tauri build --target …` |
| Linux amd64 / arm64 | Needs WebKitGTK 4.1 + GTK 3 dev packages |
| macOS aarch64 / amd64 | Apple Silicon and Intel |
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

## Install local .deb (Linux)

Build the Tauri `.deb` and install it on this machine (`sudo` prompts for a password):

```bash
task install
# or from client/: pnpm run install:deb
# or Cursor/VS Code: Tasks: Run Task → install
```

On first launch enter your Sarca server URL and Connect. Sign in on the server’s web login page.
After that, the app shows the server’s web UI. Tray → **Disconnect** returns to the connect screen.

## Icons

App icons are generated from the site logo (`logo.svg`):

```bash
cd client
pnpm exec tauri icon ../logo.svg
# or: pnpm exec tauri icon public/logo.svg
```

Desktop icons land in `src-tauri/icons/`. Android mipmaps are also written under
`src-tauri/icons/android/` (and into `gen/android/.../res/` only if that tree
already exists). After `tauri android init`, `scripts/patch-android-http.sh`
copies the Sarca Android icons into the generated project so the APK does not
ship with Tauri’s default logo.

## CI artifacts

GitHub Actions workflow [`.github/workflows/client.yml`](../.github/workflows/client.yml) builds in parallel.
GitHub **Releases** (`.github/workflows/release.yml`) attach desktop installers plus:

| Asset | Notes |
| --- | --- |
| `sarca_client_android_arm64.apk` | Android arm64 sideload APK |
| `sarca_client_ios_arm64.ipa` | Device IPA when Apple signing secrets are set |
| `sarca_client_ios_arm64-simulator.zip` | Fallback without Apple certs |

Optional secrets for signed mobile builds: `ANDROID_KEYSTORE_BASE64`, `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS`, `ANDROID_KEY_PASSWORD`, `APPLE_CERTIFICATE_BASE64`, `APPLE_CERTIFICATE_PASSWORD`.

## Mobile

Scaffolding lives under `client/mobile/`. After installing Android SDK / Xcode:

```bash
cd client
pnpm tauri android init
pnpm tauri ios init
pnpm tauri android build
pnpm tauri ios build
```

The webview uses the mobile site layout automatically on narrow viewports.
