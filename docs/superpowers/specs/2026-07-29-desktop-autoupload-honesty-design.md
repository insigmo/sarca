# Desktop auto-upload honesty + hardened WalkDir

**Date:** 2026-07-29  
**Status:** Approved for planning (chat)  
**Context:** Android Camera auto-upload now works via MediaStore. On desktop (Linux / Windows / macOS) the user can enable Camera auto-upload against a folder that **contains** media, yet Uploading stays **0** with no clear explanation. Shared engine already has Waiting→Active→Done and soft walk errors; discovery is still `FsMediaSource` + WalkDir. iOS is out of scope.

## Goals

1. After a sync tick on desktop, the Sync UI never looks like a silent “success with nothing to do” when the truth is knowable: either files are uploading, already synced, missing from the folder, or failed with `last_error`.
2. Harden filesystem discovery so symlink-to-file media under the bound folder is collected (common desktop layout).
3. Keep one code path for Linux / Windows / macOS (`FsMediaSource`); do not add OS photo-library APIs.

## Non-goals

- iOS PhotoKit / Photos library
- Changing Android MediaStore behavior
- WorkManager / true background daemons
- Automatic full re-upload / wipe of the local sync index
- New “Force reupload all” button (follow-up if needed)
- Real cellular `wifi_only` detection on mobile

## Decisions

| Topic | Choice |
|-------|--------|
| Platforms | Linux, Windows, macOS only |
| Honesty | Extend `SyncStatus` with scan counters; UI copy for empty / already-synced |
| Discovery | Keep WalkDir; enable symlink **file** following |
| Index | Do not auto-clear; “already synced” is a valid outcome |
| Android / iOS | Unchanged |

---

## 1. Scan counters on `SyncStatus`

### Fields

Add to `SyncStatus` (serde `snake_case`, default 0 for backward-compatible JSON):

```rust
pub scanned: usize,          // candidates returned by LocalMediaSource this tick
pub pending: usize,          // after size/mtime filter (queued or about to upload)
pub already_synced: usize,   // scanned - pending (index says unchanged)
```

Semantics for upload-only bindings after `push_local` / end of `sync_binding`:

- `scanned` = `candidates.len()`
- `pending` = count that entered the Waiting enqueue set (or would have, before cap)
- `already_synced` = `scanned.saturating_sub(pending)` when no hard error  
  (If hash-skip later completes without upload, that file still counted as pending at enqueue time — acceptable; do not try to split hash-skip into a fourth counter for MVP.)

On discovery / push error: set `last_error` as today; counters may reflect partial progress (best-effort) or zeros — prefer filling `scanned` if list succeeded before failure.

Placeholder status at tick start may zero these fields; final status overwrites them.

### IPC / UI

- Existing `sync_statuses` already returns `SyncStatus`; no new command.
- Settings Sync panel (desktop + native): under the Camera binding meta line (or near Uploading row), show a short status hint when useful:

| Condition | Copy (EN) |
|-----------|-----------|
| `last_error` set | Keep existing error banner only |
| `scanned == 0` and no error | `No media files found in the local folder` |
| `scanned > 0` and `pending == 0` and `uploading == 0` (queue unfinished also 0) | `{scanned} media file(s) found, all already uploaded` |
| `pending > 0` or transfer queue uploading > 0 | Prefer queue counts; optional `{pending} waiting` is redundant if Uploading row works |

Use the Camera binding’s status when present; folder bindings can show the same pattern on their rows later if cheap — MVP: Camera + any status with these fields in the error/hint area is enough; folder rows may reuse the same helper.

---

## 2. Hardened WalkDir

### Symlinks

In `collect_fs_candidates`:

- Use `WalkDir::new(root).follow_links(true)` **or** equivalent that treats symlink-to-file as a file for `is_file()` / metadata.
- Avoid infinite loops: WalkDir’s `follow_links(true)` already has loop detection; keep max depth unlimited as today.
- Relative paths: still `strip_prefix(root)`; if follow escapes outside root, prefer skipping entries whose resolved path is outside `root` (canonicalize when practical) to avoid uploading unrelated trees via symlink dirs. **MVP rule:** follow links; if `strip_prefix` fails, skip the entry (do not use absolute path as relative).

### Empty vs error

Unchanged from soft-walk policy:

- Missing root → hard error  
- Zero files + walk errors → hard error  
- Zero files + no errors → `Ok([])` → UI “No media files…”  
- Some files + walk errors → Ok with found files  

### Session / kick

No new IPC. Confirm enable path and Upload now still call `sync_now` fire-and-forget and that `last_error` from failed ticks remains visible after refresh (already true if statuses retained). If a desktop-only race clears status too early, fix in the same change set — only if reproduced.

---

## 3. Testing

### Rust

- `SyncStatus` serialization includes new fields (default 0).
- After a fake tick / unit helper: given N candidates and M pending, status counters match.
- Symlink-to-`.jpg` under temp dir is collected when `follow_links` enabled (`#[cfg(unix)]`).

### Vitest

- Mock `sync_statuses` with `scanned: 5, pending: 0, already_synced: 5` → panel shows “already uploaded” copy.
- Mock `scanned: 0` → “No media files…” copy.

### Manual (Linux)

1. Point Camera auto-upload at a folder with new jpgs never uploaded → Waiting/Uploading non-zero, files appear under `Camera/<device>/`.  
2. Re-run Upload now → hint “all already uploaded”, Uploading 0.  
3. Empty folder → “No media files…”.

---

## 4. Rollout

- Backward compatible: old UIs ignore unknown JSON fields; new fields default to 0.  
- Ship with next client build; no DB migration.  
- Android continues MediaStore; counters still apply when Android ticks (nice consistency).

## Success criteria

- Desktop folder with media either uploads or shows an explicit already-synced / empty / error message — never unexplained Uploading 0 after a completed tick.  
- Symlink files under the bound tree are discovered.  
- `cargo test -p sarca-sync` and Sync panel Vitest pass.  
- iOS / Android MediaStore scope untouched beyond shared `SyncStatus` fields.

## Open implementation notes

- Prefer computing counters inside `push_local` / `sync_binding` return path rather than a parallel side channel.  
- `already_synced` naming in UI can say “already uploaded” for upload-only modes.  
- Cap Waiting at 2000 unchanged; `pending` may be > visible Waiting rows — UI copy may use `pending` from status, not queue length.
