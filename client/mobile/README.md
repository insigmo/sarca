# Mobile targets (Tauri 2)

This directory tracks mobile-specific notes and checked-in config helpers.
Generated native projects (`gen/android`, `gen/apple`) are produced by:

```bash
cd client
pnpm install
pnpm tauri android init
pnpm tauri ios init   # requires macOS + Xcode
```

## Auto-upload (camera / gallery)

1. Add a binding with mode `auto_upload`.
2. Point `local_path` at a user-picked Photos / Camera / Downloads folder via the system picker.
3. Sync loop uploads new/changed files only (no remote→local deletes).

Platform media-library observers (Android MediaStore, iOS Photos) can be layered later as
Tauri plugins; folder pick + walk is the MVP that works on all targets.

## Background

- **Android:** consider WorkManager via a small plugin after MVP.
- **iOS:** background URLSession / BGProcessingTask — constrained; do not promise desktop parity.
