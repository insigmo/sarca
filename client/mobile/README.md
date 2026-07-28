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
`http://` and `https://` work in the WebView, installs mobile plugins, and replaces
Tauri’s default launcher icons with Sarca icons from `src-tauri/icons/android/`.

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

**Discovery**

- **Camera on Android:** lists DCIM photos/videos via **MediaStore** (not a filesystem walk).
  Discovery errors surface as `last_error` instead of silent zero uploads.
- **Desktop Camera:** path walk (follows symlink files). After each tick, Sync Settings
  shows an honesty hint when Uploading is 0 — e.g. no media in the folder, or all
  already uploaded — instead of a silent zero.
- **Folder auto-upload (all platforms):** walks the picked local path with `WalkDir` as before.

**Browse / folder picker**

- **Desktop (Linux / Windows / macOS):** native OS folder dialog via `tauri-plugin-dialog`.
- **Android:** SAF `ACTION_OPEN_DOCUMENT_TREE`; primary/external volumes resolve to
  `/storage/emulated/0/...` (or `/storage/<uuid>/...`) so folder auto-upload can walk files.
  Typed `window.prompt` is only used if the tree URI cannot be mapped to a filesystem path.
- **iOS:** typed path fallback (no walkable folder picker yet).

The APK patch script adds media read + battery-optimization permissions and installs
`FolderPickerPlugin.kt` plus `StartupPlugin.kt` (runtime permission prompts + device model).

On each Android app start the client asks for photo/video access and battery-optimization
exemption so background auto-upload is less likely to be killed.

## Background

- **Android:** consider WorkManager via a small plugin after MVP.
- **iOS:** background URLSession / BGProcessingTask — constrained; do not promise desktop parity.
