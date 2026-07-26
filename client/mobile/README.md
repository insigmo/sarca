# Mobile targets (Tauri 2)

This directory tracks mobile-specific notes and checked-in config helpers.
Generated native projects (`gen/android`, `gen/apple`) are produced by:

```bash
cd client
pnpm install
pnpm tauri android init
pnpm tauri ios init   # requires macOS + Xcode
```

## Android HTTP (cleartext)

Self-hosted servers often use `http://` on LAN. After `tauri android init`, run:

```bash
./scripts/patch-android-http.sh
```

CI runs this automatically before building the APK. It enables cleartext traffic so both
`http://` and `https://` work in the WebView.

## Android sideload signing

Release APKs **must** be signed (Pixel / modern Android reject unsigned packages).

CI signs with either:

1. Repo secrets `ANDROID_KEYSTORE_BASE64` / `ANDROID_KEYSTORE_PASSWORD` / `ANDROID_KEY_ALIAS` / `ANDROID_KEY_PASSWORD`, or
2. The committed sideload keystore [`sarca-sideload.p12`](./sarca-sideload.p12)
   - alias: `sarca`
   - password: `sarca-sideload`

Manual resign:

```bash
# after installing Android build-tools
./scripts/sign-android-apk.sh path/to/app-unsigned.apk path/to/sarca_client_android_arm64.apk
```

## Auto-upload (camera / gallery)

1. Open **Settings → Sync** (desktop: also **Sarca → Sync settings** or tray → Sync settings).
2. Enable Media auto-upload and pick a Photos / Camera / Downloads folder.
3. Sync loop uploads new/changed files only (no remote→local deletes).

**Browse / folder picker**

- **Desktop (Linux / Windows / macOS):** native OS folder dialog via `tauri-plugin-dialog`.
- **Android:** SAF `ACTION_OPEN_DOCUMENT_TREE`; primary/external volumes resolve to
  `/storage/emulated/0/...` (or `/storage/<uuid>/...`) so the sync walker can read files.
  Typed `window.prompt` is only used if the tree URI cannot be mapped to a filesystem path.
- **iOS:** typed path fallback (no walkable folder picker yet).

The APK patch script adds media read permissions and installs `FolderPickerPlugin.kt`.

Platform media-library observers (Android MediaStore, iOS Photos) can be layered later as
Tauri plugins; folder path + walk is the MVP that works on all targets.

## Background

- **Android:** consider WorkManager via a small plugin after MVP.
- **iOS:** background URLSession / BGProcessingTask — constrained; do not promise desktop parity.
