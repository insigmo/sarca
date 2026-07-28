# Android DCIM MediaStore Auto-upload + Transfer Waiting Queue

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** On Android, discover DCIM photos/videos via MediaStore (not silent WalkDir) and upload them; fill Sync transfer Waiting→Active→Done so Upload/Download lists and counts reflect the full pass.

**Architecture:** Keep `sarca-sync` free of JNI. Add `LocalCandidate` + optional `LocalMediaSource` on `SyncEngineConfig`. Default source walks the filesystem (with hard errors on unreadable trees). Android client registers a MediaStore-backed source that lists DCIM via a Kotlin plugin, materializes readable paths, then reuses the same push/hash/upload pipeline. Transfer queue gains real `enqueue_waiting` + `promote` before each upload.

**Tech Stack:** Rust (`sarca-sync`, Tauri 2 `sarca-client`), Kotlin Tauri plugin, SolidJS Settings Sync panel (Vitest), existing `patch-android-http.sh`.

**Spec:** `docs/superpowers/specs/2026-07-28-android-dcim-mediastore-autoupload-design.md`  
**Acceptance:** `.cursor/acceptance/2026-07-28-android-autoupload-works.md`

## Global Constraints

- Camera on Android uses **MediaStore filtered to DCIM** (including `DCIM/Camera/…`); binding `local_path` stays `/storage/emulated/0/DCIM`.
- Desktop / non-Android Camera and all `folder_upload` keep **WalkDir** (no SAF rewrite in this plan).
- No `MANAGE_EXTERNAL_STORAGE`; rely on existing `READ_MEDIA_*`.
- Discovery **errors** must set binding `last_error` — never silent `Ok(0)` when the walker/plugin fails.
- Successful empty MediaStore query → `Ok(0)`, no error.
- Transfer list order: **Active → Waiting → Done**; unfinished counts = active + waiting.
- Waiting cap **2000**; Done cap **100**.
- No new webview Tauri commands for MediaStore (native-side only).
- No WorkManager / FGS / iOS Photos / durable queue across restarts.
- `docs/` is gitignored — `git add -f` only when committing under `docs/superpowers/`.
- Work on a dedicated feature branch off current `master`.

## File map

| File | Responsibility |
|------|----------------|
| `crates/sarca-sync/src/transfer.rs` | `enqueue_waiting`, `promote`, `MAX_WAITING`; snapshot unchanged |
| `crates/sarca-sync/src/candidate.rs` | `LocalCandidate`, `strip_dcim_relative`, FS walk collector |
| `crates/sarca-sync/src/media_source.rs` | `LocalMediaSource` trait + default FS impl |
| `crates/sarca-sync/src/engine.rs` | Wire source into `push_local`; Waiting→Active; walk errors |
| `crates/sarca-sync/src/lib.rs` | Export new modules/types |
| `client/mobile/android/java/app/sarca/client/mediastore/MediaStorePlugin.kt` | `listDcimMedia`, `materializeForUpload` |
| `client/scripts/patch-android-http.sh` | Copy MediaStore plugin into `gen/android` |
| `client/src-tauri/src/mediastore.rs` | Tauri plugin register + Rust calls to Kotlin |
| `client/src-tauri/src/lib.rs` | `.plugin(mediastore::init())` |
| `client/src-tauri/src/state.rs` | Build `SyncEngine` with Android `LocalMediaSource` when applicable |
| `client/mobile/README.md` | Note MediaStore DCIM discovery |
| `ui/src/components/SettingsSyncPanel.test.jsx` | Waiting count / list order smoke |

---

### Task 1: Transfer queue Waiting API

**Files:**
- Modify: `crates/sarca-sync/src/transfer.rs`
- Test: same file `#[cfg(test)]`

**Interfaces:**
- Consumes: existing `TransferQueue`, `TransferItem`, `TransferStatus`, `begin`, `complete`, `abandon`
- Produces:
  - `pub const MAX_WAITING: usize = 2000;`
  - `pub fn enqueue_waiting(&mut self, binding_id: &str, direction: TransferDirection, relative_path: &str, size: Option<i64>) -> Option<String>` — returns `Some(id)` if enqueued, `None` if over cap (upload may still proceed without a queue row)
  - `pub fn promote(&mut self, id: &str) -> bool` — move Waiting → Active; return false if id missing
  - Keep `begin` for callers that jump straight to Active (downloads may keep using it until Task 3 wires them)

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn enqueue_waiting_then_promote_then_complete() {
    let mut q = TransferQueue::default();
    let id = q
        .enqueue_waiting("b1", TransferDirection::Upload, "Camera/a.jpg", Some(10))
        .expect("enqueued");
    let snap = q.snapshot();
    assert_eq!(snap.uploading, 1);
    assert_eq!(snap.items[0].status, TransferStatus::Waiting);
    assert!(q.promote(&id));
    assert_eq!(q.snapshot().items[0].status, TransferStatus::Active);
    q.complete(&id);
    assert_eq!(q.snapshot().uploading, 0);
    assert_eq!(q.snapshot().items[0].status, TransferStatus::Done);
}

#[test]
fn waiting_cap_returns_none_beyond_limit() {
    let mut q = TransferQueue::default();
    for i in 0..MAX_WAITING {
        assert!(q
            .enqueue_waiting("b", TransferDirection::Upload, &format!("f{i}.jpg"), None)
            .is_some());
    }
    assert!(q
        .enqueue_waiting("b", TransferDirection::Upload, "overflow.jpg", None)
        .is_none());
    assert_eq!(q.snapshot().uploading, MAX_WAITING);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p sarca-sync enqueue_waiting_then_promote --lib`  
Expected: FAIL (methods missing)

- [ ] **Step 3: Implement**

In `transfer.rs`:

```rust
pub const MAX_WAITING: usize = 2000;

impl TransferQueue {
    pub fn enqueue_waiting(
        &mut self,
        binding_id: &str,
        direction: TransferDirection,
        relative_path: &str,
        size: Option<i64>,
    ) -> Option<String> {
        let (path, name) = split_path(relative_path);
        self.active.retain(|i| {
            !(i.binding_id == binding_id
                && i.direction == direction
                && i.path == path
                && i.name == name)
        });
        self.waiting.retain(|i| {
            !(i.binding_id == binding_id
                && i.direction == direction
                && i.path == path
                && i.name == name)
        });
        if self.waiting.len() >= MAX_WAITING {
            return None;
        }
        let id = Uuid::new_v4().to_string();
        self.waiting.push(TransferItem {
            id: id.clone(),
            binding_id: binding_id.to_owned(),
            direction,
            path,
            name,
            size,
            status: TransferStatus::Waiting,
            updated_at_ms: now_ms(),
        });
        Some(id)
    }

    pub fn promote(&mut self, id: &str) -> bool {
        let Some(pos) = self.waiting.iter().position(|i| i.id == id) else {
            return false;
        };
        let mut item = self.waiting.remove(pos);
        item.status = TransferStatus::Active;
        item.updated_at_ms = now_ms();
        self.active.push(item);
        true
    }
}
```

Deduplicate: if `begin` is called for a path already in waiting, remove waiting entry first (existing `begin` already retains waiting — keep that).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p sarca-sync --lib transfer::`  
Expected: PASS (including existing `begin_complete` / `done_list_is_capped`)

- [ ] **Step 5: Commit**

```bash
git add crates/sarca-sync/src/transfer.rs
git commit -m "$(cat <<'EOF'
feat(sync): enqueue Waiting transfers and promote to Active

EOF
)"
```

---

### Task 2: `LocalCandidate` + FS collector (fail on walk errors)

**Files:**
- Create: `crates/sarca-sync/src/candidate.rs`
- Modify: `crates/sarca-sync/src/lib.rs` (`mod candidate; pub use …`)
- Test: `candidate.rs` `#[cfg(test)]`

**Interfaces:**
- Consumes: `is_media_file` from `engine` (move `is_media_file` into `candidate.rs` **or** keep in engine and import — prefer **move** `is_media_file` to `candidate.rs` and re-export from `lib.rs` / `engine` for compatibility)
- Produces:
  - `pub struct LocalCandidate { pub relative_path: String, pub absolute_path: PathBuf, pub size: i64, pub mtime_ms: i64, pub ephemeral: bool }`
  - `pub fn strip_dcim_prefix(relative_path: &str) -> String` — `"DCIM/Camera/a.jpg"` → `"Camera/a.jpg"`; `"Camera/a.jpg"` unchanged; trim slashes
  - `pub fn collect_fs_candidates(root: &Path, media_only: bool) -> Result<Vec<LocalCandidate>>` — errors if root missing **or** any `WalkDir` `Err`; does **not** `create_dir_all`

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn strip_dcim_prefix_strips_only_dcim_root() {
    assert_eq!(strip_dcim_prefix("DCIM/Camera/a.jpg"), "Camera/a.jpg");
    assert_eq!(strip_dcim_prefix("DCIM/a.jpg"), "a.jpg");
    assert_eq!(strip_dcim_prefix("Camera/a.jpg"), "Camera/a.jpg");
    assert_eq!(strip_dcim_prefix("dcim/x.jpg"), "dcim/x.jpg"); // case-sensitive; MediaStore uses DCIM
}

#[test]
fn collect_fs_candidates_lists_media_and_fails_on_missing_root() {
    let dir = tempfile::tempdir().unwrap();
    let pics = dir.path().join("pics");
    std::fs::create_dir_all(&pics).unwrap();
    std::fs::write(pics.join("a.jpg"), b"x").unwrap();
    std::fs::write(pics.join("note.txt"), b"y").unwrap();
    let got = collect_fs_candidates(&pics, true).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].relative_path, "a.jpg");
    assert!(!got[0].ephemeral);

    let missing = dir.path().join("nope");
    assert!(collect_fs_candidates(&missing, true).is_err());
}
```

- [ ] **Step 2: Run tests — expect FAIL**

Run: `cargo test -p sarca-sync strip_dcim_prefix --lib`  
Expected: FAIL (module missing)

- [ ] **Step 3: Implement `candidate.rs`**

```rust
use std::path::{Path, PathBuf};
use anyhow::{bail, Context, Result};
use walkdir::WalkDir;
use crate::index::mtime_ms_from_system;

#[derive(Debug, Clone)]
pub struct LocalCandidate {
    pub relative_path: String,
    pub absolute_path: PathBuf,
    pub size: i64,
    pub mtime_ms: i64,
    pub ephemeral: bool,
}

pub fn strip_dcim_prefix(relative_path: &str) -> String {
    let r = relative_path.trim().trim_matches('/');
    if let Some(rest) = r.strip_prefix("DCIM/") {
        return rest.trim_matches('/').to_owned();
    }
    if r == "DCIM" {
        return String::new();
    }
    r.to_owned()
}

pub fn is_media_file(path: &Path) -> bool {
    // move existing match list from engine.rs unchanged
}

pub fn collect_fs_candidates(root: &Path, media_only: bool) -> Result<Vec<LocalCandidate>> {
    if !root.exists() {
        bail!("local folder missing or unreadable: {}", root.display());
    }
    let mut out = Vec::new();
    for entry in WalkDir::new(root) {
        let entry = entry.with_context(|| format!("walk {}", root.display()))?;
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path().to_path_buf();
        if media_only && !is_media_file(&path) {
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        if rel.is_empty() {
            continue;
        }
        let meta = entry.metadata().with_context(|| format!("meta {}", path.display()))?;
        let mtime = meta.modified().ok().map(mtime_ms_from_system).unwrap_or(0);
        out.push(LocalCandidate {
            relative_path: rel,
            absolute_path: path,
            size: meta.len() as i64,
            mtime_ms: mtime,
            ephemeral: false,
        });
    }
    Ok(out)
}
```

Re-export `is_media_file` from `lib.rs`; change `engine.rs` to `use crate::candidate::is_media_file` or delete duplicate.

- [ ] **Step 4: Run tests — expect PASS**

Run: `cargo test -p sarca-sync --lib candidate::`  
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/sarca-sync/src/candidate.rs crates/sarca-sync/src/lib.rs crates/sarca-sync/src/engine.rs
git commit -m "$(cat <<'EOF'
feat(sync): LocalCandidate FS collector that surfaces walk errors

EOF
)"
```

---

### Task 3: `LocalMediaSource` + wire `push_local` (Waiting + candidates)

**Files:**
- Create: `crates/sarca-sync/src/media_source.rs`
- Modify: `crates/sarca-sync/src/engine.rs` (`SyncEngineConfig`, `push_local`, transfer helpers)
- Modify: `crates/sarca-sync/src/lib.rs`
- Test: `engine.rs` / `media_source.rs` tests with tempfile + noop API if needed

**Interfaces:**
- Consumes: `LocalCandidate`, `collect_fs_candidates`, `TransferQueue::{enqueue_waiting,promote,complete,abandon}`
- Produces:
  - `#[async_trait] pub trait LocalMediaSource: Send + Sync { async fn list_candidates(&self, binding: &Binding) -> Result<Vec<LocalCandidate>>; }`
  - `pub struct FsMediaSource;` implementing trait via `collect_fs_candidates(Path::new(&binding.local_path), media_only)` where `media_only = matches!(binding.mode, AutoUpload)`
  - `SyncEngineConfig { …, pub media_source: Arc<dyn LocalMediaSource> }` — default `FsMediaSource` in all existing `SyncEngine::open` / test helpers
  - Engine helpers: `transfer_enqueue_waiting`, `transfer_promote`
  - `push_local` flow:
    1. `let candidates = self.config.media_source.list_candidates(binding).await?;` — **no** `create_dir_all` early return
    2. For each candidate: index size/mtime skip (same as today) → if needs work, `enqueue_waiting`
    3. For each needing work: `promote` (if had id) else `begin`; hash; upload from `absolute_path`; `complete`; if `ephemeral`, `tokio::fs::remove_file` best-effort
    4. Sync-mode local deletes unchanged (still FS-based)

- [ ] **Step 1: Write failing test for waiting enqueue during push**

Prefer a focused unit test on a helper, or integration-style with a stub API. Minimal approach — test queue wiring via a small `pub(crate)` helper used by `push_local`:

```rust
#[test]
fn plan_uploads_enqueues_waiting_for_changed_files() {
    // Build two LocalCandidates; mock index empty → both get waiting ids
    // Call a pure function `fn waiting_ids_for_candidates(...)` if extracted,
    // OR spin SyncEngine with FsMediaSource + temp files and assert transfer_queue
    // snapshot uploading >= 1 after starting push (may need stub SarcaApi — skip network
    // by testing only the pre-upload enqueue loop extracted as:
    //   collect_pending_uploads(index, binding_id, candidates) -> Vec<(LocalCandidate, Option<String>)>
}
```

Concrete extract:

```rust
/// Returns candidates that need hash/upload, each paired with a waiting queue id (if capped: None).
pub fn select_pending_uploads(
    index: &LocalIndex,
    binding_id: &str,
    candidates: &[LocalCandidate],
    queue: &mut TransferQueue,
) -> Result<Vec<(LocalCandidate, Option<String>)>> {
    let mut out = Vec::new();
    for c in candidates {
        let existing = index.get_entry(binding_id, &c.relative_path)?;
        let needs = existing
            .as_ref()
            .is_none_or(|e| e.size != c.size || e.mtime_ms != c.mtime_ms);
        if !needs {
            continue;
        }
        let tid = queue.enqueue_waiting(
            binding_id,
            TransferDirection::Upload,
            &c.relative_path,
            Some(c.size),
        );
        out.push((c.clone(), tid));
    }
    Ok(out)
}
```

Test:

```rust
#[test]
fn select_pending_uploads_marks_waiting() {
    let dir = tempfile::tempdir().unwrap();
    let idx = LocalIndex::open(&dir.path().join("i.sqlite")).unwrap();
    let id = "b1";
    idx.upsert_binding(&Binding { /* AutoUpload, local_path pics */ … }).unwrap();
    let c = LocalCandidate {
        relative_path: "a.jpg".into(),
        absolute_path: dir.path().join("a.jpg"),
        size: 3,
        mtime_ms: 1,
        ephemeral: false,
    };
    let mut q = TransferQueue::default();
    let pending = select_pending_uploads(&idx, id, &[c], &mut q).unwrap();
    assert_eq!(pending.len(), 1);
    assert!(pending[0].1.is_some());
    assert_eq!(q.snapshot().uploading, 1);
    assert_eq!(q.snapshot().items[0].status, TransferStatus::Waiting);
}
```

- [ ] **Step 2: Run test — expect FAIL**

Run: `cargo test -p sarca-sync select_pending_uploads_marks_waiting --lib`  
Expected: FAIL

- [ ] **Step 3: Implement trait + refactor `push_local`**

1. Add `media_source.rs` with trait + `FsMediaSource`.
2. Extend `SyncEngineConfig` with `media_source: Arc<dyn LocalMediaSource>` (update all constructors in tests/`state.rs` later).
3. Replace WalkDir body in `push_local` with `list_candidates` + `select_pending_uploads` + promote/upload loop.
4. On upload failure: `abandon` waiting/active id.
5. Remove `if !root.exists() { create_dir_all; return Ok(0) }` for upload path — FS source errors instead. (`bootstrap_snapshot` / pull may still create dirs.)

- [ ] **Step 4: Run full crate tests**

Run: `cargo test -p sarca-sync --lib`  
Expected: PASS (fix any test that constructed `SyncEngineConfig` without `media_source`)

- [ ] **Step 5: Commit**

```bash
git add crates/sarca-sync/src/
git commit -m "$(cat <<'EOF'
feat(sync): LocalMediaSource + Waiting enqueue in push_local

EOF
)"
```

---

### Task 4: Kotlin `MediaStorePlugin`

**Files:**
- Create: `client/mobile/android/java/app/sarca/client/mediastore/MediaStorePlugin.kt`
- Modify: `client/scripts/patch-android-http.sh` (copy plugin like Startup)

**Interfaces:**
- Consumes: Android `ContentResolver`, `MediaStore.Images` / `Video`, app cache dir
- Produces: Tauri plugin commands:
  - `listDcimMedia` → `JSArray` of objects `{ uri, displayName, relativePath, size, dateModifiedMs }`
  - `materializeForUpload({ uri })` → `{ path, ephemeral: boolean }`

- [ ] **Step 1: Implement plugin (no JVM unit test in CI — verified by Rust bridge + device)**

`MediaStorePlugin.kt` outline:

```kotlin
@TauriPlugin
class MediaStorePlugin(private val activity: Activity) : Plugin(activity) {
  @Command
  fun listDcimMedia(invoke: Invoke) {
    try {
      val items = JSArray()
      items.putAll(queryCollection(MediaStore.Images.Media.EXTERNAL_CONTENT_URI))
      items.putAll(queryCollection(MediaStore.Video.Media.EXTERNAL_CONTENT_URI))
      val ret = JSObject()
      ret.put("items", items)
      invoke.resolve(ret)
    } catch (ex: SecurityException) {
      invoke.reject(ex.message ?: "MediaStore permission denied")
    } catch (ex: Exception) {
      invoke.reject(ex.message ?: "MediaStore query failed")
    }
  }

  @Command
  fun materializeForUpload(invoke: Invoke) {
    try {
      val uri = Uri.parse(invoke.getArgs().getString("uri"))
      // Optional: if DATA column readable, return path + ephemeral=false
      // Else copy InputStream to cacheDir/sarca-upload/<uuid>_<name>
      val ret = JSObject()
      ret.put("path", absPath)
      ret.put("ephemeral", true)
      invoke.resolve(ret)
    } catch (ex: Exception) {
      invoke.reject(ex.message ?: "materialize failed")
    }
  }

  private fun isDcimRelative(rel: String?): Boolean {
    if (rel == null) return false
    val n = rel.trimStart('/')
    return n == "DCIM" || n == "DCIM/" || n.startsWith("DCIM/")
  }
}
```

Query columns: `_ID`, `DISPLAY_NAME`, `RELATIVE_PATH`, `SIZE`, `DATE_MODIFIED` (seconds → ms `* 1000`), build `content://` via `ContentUris.withAppendedId`.

Relative path field from MediaStore often includes trailing slash (`DCIM/Camera/`); keep as-returned for Rust `strip_dcim_prefix` after joining with displayName:

Rust side builds: `strip_dcim_prefix(format!("{}{}", relativePath, displayName))` with slash normalization.

- [ ] **Step 2: Patch script**

Add next to Startup copy:

```bash
MEDIASTORE_SRC="$ROOT/mobile/android/java/app/sarca/client/mediastore/MediaStorePlugin.kt"
MEDIASTORE_DST="$APP_SRC/java/app/sarca/client/mediastore/MediaStorePlugin.kt"
if [[ -f "$MEDIASTORE_SRC" ]]; then
  mkdir -p "$(dirname "$MEDIASTORE_DST")"
  cp -a "$MEDIASTORE_SRC" "$MEDIASTORE_DST"
  echo "Installed MediaStorePlugin.kt → $MEDIASTORE_DST"
fi
```

- [ ] **Step 3: Sanity — script copies when gen exists**

Run (if `gen/android` present): `./client/scripts/patch-android-http.sh`  
Expected: log line `Installed MediaStorePlugin.kt`

- [ ] **Step 4: Commit**

```bash
git add client/mobile/android/java/app/sarca/client/mediastore/MediaStorePlugin.kt client/scripts/patch-android-http.sh
git commit -m "$(cat <<'EOF'
feat(android): MediaStore plugin for DCIM listing and materialize

EOF
)"
```

---

### Task 5: Rust MediaStore bridge + Android `LocalMediaSource`

**Files:**
- Create: `client/src-tauri/src/mediastore.rs`
- Modify: `client/src-tauri/src/lib.rs` (mod + plugin)
- Modify: `client/src-tauri/src/state.rs` (pass Android source into `SyncEngineConfig`)
- Test: pure helpers in `mediastore.rs` for relative path join (cfg-independent)

**Interfaces:**
- Consumes: Kotlin plugin via `run_mobile_plugin` (same pattern as `folder_picker.rs` / `startup.rs`)
- Produces:
  - `pub fn init<R: Runtime>() -> TauriPlugin<R>`
  - `pub async fn list_dcim_media<R: Runtime>(app: &AppHandle<R>) -> Result<Vec<MediaStoreItem>, String>`
  - `pub async fn materialize_for_upload<R: Runtime>(app: &AppHandle<R>, uri: &str) -> Result<(PathBuf, bool), String>`
  - `pub struct AndroidDcimMediaSource { app: AppHandle }` implementing `sarca_sync::LocalMediaSource`:
    - `list_candidates`: call `list_dcim_media`, for each item materialize, build `LocalCandidate { relative_path: strip_dcim_prefix(join(rel, name)), …, ephemeral }`
  - On non-Android: `state.rs` keeps `FsMediaSource`
  - On Android: `media_source: Arc::new(AndroidDcimMediaSource::new(app))` **only for engine used for sync** — note: `FsMediaSource` still required for `folder_upload`. So the trait impl must branch **inside** `list_candidates` by `binding.mode`:

```rust
async fn list_candidates(&self, binding: &Binding) -> Result<Vec<LocalCandidate>> {
    match binding.mode {
        BindingMode::AutoUpload => self.list_dcim_via_mediastore().await,
        _ => collect_fs_candidates(Path::new(&binding.local_path), false /* folder: all files */).map_err(|e| e.into()),
    }
}
```

For Android AutoUpload ignore `binding.local_path` filesystem (keep for UI). For FolderUpload/Sync use FS walk (may still be empty on Android — out of scope).

Desktop: always `FsMediaSource` (AutoUpload uses media_only=true).

- [ ] **Step 1: Failing test for path join helper**

```rust
#[test]
fn media_relative_under_dcim() {
    assert_eq!(
        media_item_relative_path("DCIM/Camera/", "IMG_1.jpg"),
        "Camera/IMG_1.jpg"
    );
    assert_eq!(
        media_item_relative_path("DCIM/", "x.mp4"),
        "x.mp4"
    );
}

fn media_item_relative_path(relative_path: &str, display_name: &str) -> String {
    let joined = format!(
        "{}/{}",
        relative_path.trim_matches('/'),
        display_name.trim_matches('/')
    );
    sarca_sync::strip_dcim_prefix(&joined)
}
```

Put helper in `mediastore.rs` and export for the source.

- [ ] **Step 2: Implement bridge + wire `state.rs` / `lib.rs`**

Register:

```rust
.plugin(mediastore::init())
```

Construct engine with Android source when `cfg(target_os = "android")`.

- [ ] **Step 3: Compile client lib tests on host**

Run: `cd client/src-tauri && cargo test --lib`  
Expected: PASS (Android cfg code not linked on Linux; helpers tested)

- [ ] **Step 4: Commit**

```bash
git add client/src-tauri/src/mediastore.rs client/src-tauri/src/lib.rs client/src-tauri/src/state.rs
git commit -m "$(cat <<'EOF'
feat(android): wire MediaStore LocalMediaSource into SyncEngine

EOF
)"
```

---

### Task 6: UI regression — Waiting counts visible

**Files:**
- Modify: `ui/src/components/SettingsSyncPanel.test.jsx` (extend existing transfer queue mock test)

**Interfaces:**
- Consumes: `sync_transfer_queue` snapshot shape `{ uploading, downloading, items }`
- Produces: assertion that Uploading row shows count when items include `waiting`

- [ ] **Step 1: Extend / add Vitest**

```js
it('shows unfinished upload count including waiting', async () => {
  // mock sync_transfer_queue → { uploading: 2, downloading: 0, items: [
  //   { direction: 'upload', status: 'active', name: 'a.jpg', path: '', size: 1 },
  //   { direction: 'upload', status: 'waiting', name: 'b.jpg', path: '', size: 1 },
  // ]}
  // render panel, expect text '2' near Uploading (existing test pattern)
})
```

Follow patterns already in `SettingsSyncPanel.test.jsx` around line ~405.

- [ ] **Step 2: Run**

Run: `cd ui && pnpm exec vitest run src/components/SettingsSyncPanel.test.jsx src/common/syncTransferQueue.test.js`  
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add ui/src/components/SettingsSyncPanel.test.jsx
git commit -m "$(cat <<'EOF'
test(ui): Sync Uploading count includes waiting transfers

EOF
)"
```

---

### Task 7: Docs + acceptance verify commands

**Files:**
- Modify: `client/mobile/README.md` (Auto-upload section: MediaStore for Camera/DCIM on Android)
- Modify: `.cursor/acceptance/2026-07-28-android-autoupload-works.md` (status → verifying when running evidence)

- [ ] **Step 1: README blurb**

Replace “folder walk” implication for Camera on Android with: Camera auto-upload lists DCIM via MediaStore; folder auto-upload still walks the picked path.

- [ ] **Step 2: Run automated evidence**

```bash
cargo test -p sarca-sync --lib
cd client/src-tauri && cargo test --lib
cd ui && pnpm exec vitest run src/common/syncTransferQueue.test.js src/components/SettingsSyncPanel.test.jsx
```

Expected: all PASS

- [ ] **Step 3: Commit docs/README**

```bash
git add client/mobile/README.md
git add -f docs/superpowers/specs/2026-07-28-android-dcim-mediastore-autoupload-design.md \
         docs/superpowers/plans/2026-07-28-android-dcim-mediastore-autoupload.md
git commit -m "$(cat <<'EOF'
docs: Android DCIM MediaStore auto-upload plan and README

EOF
)"
```

Manual device E2E remains in acceptance checklist (not blocking merge of code if CI green).

---

## Spec coverage checklist

| Spec requirement | Task |
|------------------|------|
| MediaStore DCIM list + materialize | 4, 5 |
| Engine Android AutoUpload uses MediaStore; desktop WalkDir | 3, 5 |
| No silent Ok(0) on discovery error | 2, 3 |
| Empty successful query → Ok(0) | 5 (plugin success + empty vec) |
| Waiting → Active → Done; counts; cap 2000 | 1, 3, 6 |
| Preserve relative path under DCIM | 2 (`strip_dcim_prefix`), 5 |
| No new webview commands | 5 (native only) |
| folder_upload unchanged / out of SAF rewrite | 5 (FS branch) |
| Vitest + cargo tests | 1–3, 6–7 |
| patch-android-http installs plugin | 4 |

## Placeholder / consistency self-review

- No TBD steps; method names aligned (`enqueue_waiting`, `promote`, `LocalCandidate`, `LocalMediaSource`, `strip_dcim_prefix`).
- `SyncEngineConfig` gains `media_source` — all call sites updated in Task 3/5.
- Kotlin returns `{ items: [...] }` wrapped object for easier serde (not a bare array).
