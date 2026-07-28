# Android DCIM auto-upload via MediaStore + Sync transfer queue Waiting

**Date:** 2026-07-28  
**Status:** Approved for planning (chat)  
**Context:** Tauri 2 Android client (`targetSdk` 36). Camera auto-upload binds to `/storage/emulated/0/DCIM` and discovers files with Rust `WalkDir`. On device: toggle works, Upload/Download counts stay **0**, no `last_error` (silent empty). Transfer queue only marks a file `Active` at upload start — `Waiting` is never filled, so the list does not show the rest of the pass.

## Goals

1. On Android, with media permission granted and Camera auto-upload enabled, **photos/videos under DCIM** (including `DCIM/Camera/…`) are discovered and uploaded to remote `Camera/<device>/…`.
2. Stop silent “0 uploaded / empty queue / no error” when discovery fails or returns nothing despite an expected DCIM tree.
3. During a sync pass, Sync Settings transfer lists show **Active → Waiting → Done** (currently transferring, then queued, then recently finished); unfinished counts = active + waiting.

## Non-goals

- WorkManager / foreground service / uploads after the process is killed
- iOS Photos library
- Durable transfer queue across app restarts
- Reworking `folder_upload` to SAF/`DocumentFile` (follow-up; desktop WalkDir stays)
- `MANAGE_EXTERNAL_STORAGE` / “All files access”
- Changing remote layout or soft-disable / `background_sync` coupling

## Decisions

| Topic | Choice |
|-------|--------|
| Android Camera discovery | **MediaStore** filtered to DCIM (approach 1) |
| Desktop / non-Android Camera | Keep existing `WalkDir` on `local_path` |
| Folder auto-upload | Unchanged WalkDir (out of scope for SAF rewrite) |
| Reading bytes for upload | Prefer filesystem path when MediaStore exposes one; else copy URI → app cache, upload, delete temp |
| Empty / failed discovery | Set binding `last_error` (no silent success with 0) |
| Queue | Pre-enqueue candidates as `Waiting`, promote to `Active`, then `Done` |
| Permissions | Existing `READ_MEDIA_IMAGES` / `READ_MEDIA_VIDEO` (+ legacy storage); prompt “Allow all” remains required for full DCIM |

---

## 1. Android MediaStore plugin

### Location

- Kotlin: `client/mobile/android/java/app/sarca/client/mediastore/` (e.g. `MediaStorePlugin.kt`)
- Installed into `gen/android` by `client/scripts/patch-android-http.sh` (same pattern as `StartupPlugin` / `FolderPickerPlugin`)
- Rust bridge: `client/src-tauri/src/mediastore.rs` (or extend `startup` module only if it stays small) + register plugin in `lib.rs`

### Commands

1. **`listDcimMedia`**  
   Query `MediaStore.Images.Media` + `MediaStore.Video.Media` (external volume).  
   Keep rows whose `RELATIVE_PATH` is `DCIM` or starts with `DCIM/` (case-sensitive as MediaStore stores it; normalize with trim/`startsWith("DCIM")` carefully so `DCIM_backup` does not match).  
   Return JSON array of:

   ```json
   {
     "uri": "content://…",
     "displayName": "IMG_001.jpg",
     "relativePath": "DCIM/Camera/",
     "size": 12345,
     "dateModifiedMs": 1710000000000
   }
   ```

   - `relativePath` + `displayName` build the sync relative key (e.g. `Camera/IMG_001.jpg` under DCIM root — strip the `DCIM/` prefix so remote layout under `Camera/<device>/` matches today’s WalkDir-under-DCIM behavior).
   - Skip non-files / zero-size if needed; do not filter by extension in Kotlin if MIME is image/video (MediaStore collections already scoped).

2. **`materializeForUpload`**  
   Input: `{ "uri": "content://…" }`.  
   Output: `{ "path": "/data/…/cache/sarca-upload/…" }` absolute path Rust can `File::open`.  
   - If a reliable absolute path is available and readable, may return it without copy (optional optimization; not required for correctness).  
   - Otherwise open `ContentResolver.openInputStream(uri)`, copy to app cache under a dedicated dir, return that path.  
   - Caller deletes the cache file after upload (Rust) when it was a materialization (flag `ephemeral: true` in response).

### Permissions / empty results

- Plugin does not re-request permissions (StartupPlugin already does).  
- If query throws `SecurityException` (or equivalent) → reject with a clear message; engine sets binding `last_error`.  
- Empty list with a **successful** query → treat as “nothing to upload” (`Ok(0)`, no error). Silent failure is only forbidden when discovery **errors** or WalkDir is unreadable.  
- Limited “Selected photos” access: upload whatever MediaStore returns; do not invent a separate partial-access error for MVP.

---

## 2. Engine: Android AutoUpload push path

### Seam

Introduce a small discovery abstraction used by `push_local` for media-only bindings:

- **Default (desktop / iOS / folder modes):** walk `local_path` with `WalkDir`, but **do not** `filter_map(Result::ok)` away errors — if the root is unreadable or walk yields `Err`, fail the binding with that error (or accumulate and fail if zero files visited and at least one walk error).
- **Android + `BindingMode::AutoUpload`:** call MediaStore `listDcimMedia` via the Tauri plugin (injected callback / platform fn on `SyncEngineConfig` or client wrapper that passes listed entries into the engine). Prefer keeping `sarca-sync` free of Android JNI: client (`commands` / thin adapter) lists media, then engine accepts either paths from walk or a precomputed `Vec<LocalCandidate { relative_path, absolute_path_or_materialize_key, size, mtime }>`.

Practical shape (recommended):

1. `sarca-sync` gains `push_local` logic that:
   - Collects candidates (from walk or from injected list)
   - Enqueues transfer Waiting (see §3)
   - Hashes / uploads / updates index as today
2. Android client, before or inside a platform hook: MediaStore list → materialize as needed → pass candidates into engine API, **or** engine calls a `MediaLister` trait object supplied only on Android builds of the client.

Avoid duplicating upload/hash code paths.

### Path / remote mapping

- Binding `local_path` stays `/storage/emulated/0/DCIM` for UI.  
- Relative path for index/remote: path under DCIM (e.g. `Camera/IMG_001.jpg`), same as WalkDir `strip_prefix(DCIM)`.  
- Remote: still `join(remote_root, relative)` → `Camera/<device>/Camera/IMG_001.jpg` if device folder is `Camera/<device>` and file lives in `DCIM/Camera/` — **preserve current WalkDir semantics** (do not invent a new layout).

### `create_dir_all` on missing root

- On Android AutoUpload, **do not** `create_dir_all(/storage/emulated/0/DCIM)` and return 0 — that masks missing access. If FS root is missing, still rely on MediaStore; only fail if MediaStore call fails.

---

## 3. Transfer queue: Waiting → Active → Done

### Problem today

- `TransferQueue::begin` always inserts **Active**.  
- `waiting` exists but nothing enqueues into it.  
- Counts stay 0 until a file starts; UI never shows the backlog.

### Behavior

1. After candidate set is known (and after cheap skip of unchanged size/mtime if done before enqueue — see below), for each file that still needs hash/upload work: **`enqueue_waiting`**.
2. When upload (or download) of that item starts: **`promote_to_active(id)`** (or `begin` that moves waiting → active).
3. On success: **`complete(id)`** → Done (cap unchanged, e.g. 100).
4. On failure: **`abandon(id)`** + binding `last_error`.
5. Snapshot / UI sort unchanged: Active → Waiting → Done, then name (`syncTransferQueue.js`).
6. Counts: unfinished = Active + Waiting per direction (already defined).

### When to enqueue

- Prefer: after walk/MediaStore list, for each media file, if index says skip (same size+mtime and hash present) → do not enqueue.  
- If needs hash or upload → Waiting, then Active for hash+upload (one Active per file is fine; hashing can stay under Active for simplicity).  
- Optional later: separate “hashing” status — **out of scope**.

### Pre-scan vs memory

- Cap in-memory **Waiting** entries at **2000** (document constant in `transfer.rs`). Uploads beyond the cap still run; they simply may not all appear in the Waiting list. Done cap stays **100**. No “+K more” UI for MVP.

---

## 4. UI

- No new chrome required if engine fills Waiting.  
- Settings Sync already polls `sync_transfer_queue` and sorts Active → Waiting → Done.  
- Verify counts on Uploading/Downloading rows update when Waiting is non-empty.  
- Ensure empty state copy remains honest when discovery returned 0 candidates with no error (“Nothing to upload”).

---

## 5. Testing

### Rust (`sarca-sync`)

- Transfer queue: enqueue waiting → promote → complete; snapshot order and counts.  
- `push_local` (or helper) with injected candidates: Waiting appears before upload mock completes.  
- Walk error path: unreadable directory does not return Ok(0) without `last_error` (unit test with temp chmod if feasible, or inject failing walker).

### Client

- MediaStore adapter pure helpers (relative path strip `DCIM/`) unit-tested if logic lives in Rust.  
- ACL / IPC allowlist only if new Tauri commands are exposed to the webview (prefer keeping MediaStore calls native-side only so **no new webview commands**).

### UI (Vitest)

- Existing sort tests remain.  
- Panel test: mock snapshot with waiting items → Uploading count > 0 and list order Active, Waiting, Done.

### Manual (Android device)

1. Grant **Allow all** photos/videos.  
2. Open a storage → enable Camera auto-upload.  
3. Upload now (app in foreground).  
4. Upload list shows waiting/active entries for DCIM media; files appear under `Camera/<device>/…`.  
5. Deny permission / limited access: expect visible error or only partial set — not silent zero with “success”.

---

## 6. Rollout

- Patch script must install the new Kotlin sources every Android build (CI + local).  
- Existing bindings with `local_path=/storage/emulated/0/DCIM` keep working without migration.  
- Soft-disable / fair ticks / prefs coupling unchanged.

## Success criteria

- Android DCIM Camera auto-upload uploads real gallery files (or shows a clear error).  
- No silent Ok(0) when MediaStore/walk cannot run.  
- Upload list during a pass: currently uploading, then waiting, then done; counts non-zero while waiting exists.  
- Desktop auto-upload regression tests still pass.

## Open implementation notes

- Prefer `MediaLister` / candidate injection over `cfg(target_os = "android")` inside `sarca-sync` if it keeps the crate testable on Linux CI.  
- Materialize-to-cache for every video is costly; path passthrough when readable is a worthwhile optimization but not required for MVP.  
- `folder_upload` on Android may still be broken for the same scoped-storage reason — track as follow-up, not this spec.
