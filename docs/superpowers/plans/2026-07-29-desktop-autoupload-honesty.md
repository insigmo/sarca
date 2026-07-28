# Desktop Auto-upload Honesty + Hardened WalkDir

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** On Linux/Windows/macOS, after a Camera (or folder) sync tick the UI always explains Uploading=0 — uploading, already synced, no media, or `last_error` — and WalkDir discovers symlink-to-file media under the bound folder.

**Architecture:** Extend shared `SyncStatus` with `scanned` / `pending` / `already_synced` filled in `push_local`/`sync_binding`. Harden `collect_fs_candidates` with `follow_links(true)` and skip entries that cannot `strip_prefix` the root. SolidJS Settings Sync shows short EN copy from those counters (no new IPC).

**Tech Stack:** Rust (`sarca-sync`), Tauri client (unchanged commands), SolidJS + Vitest (`ui/`).

**Spec:** `docs/superpowers/specs/2026-07-29-desktop-autoupload-honesty-design.md`

## Global Constraints

- Platforms in scope: **Linux, Windows, macOS** (shared `FsMediaSource`); iOS Photos and Android MediaStore behavior unchanged beyond new status fields.
- No automatic index wipe / force-reupload-all button.
- No new Tauri commands — use existing `sync_statuses`.
- UI copy exact (EN): `No media files found in the local folder` and `{N} media file(s) found, all already uploaded`.
- Symlinks: follow links; if relative path cannot be formed via `strip_prefix(root)`, **skip** the entry.
- Soft walk policy unchanged: missing root hard-fail; zero files + walk errors hard-fail; otherwise Ok.
- `docs/` is gitignored — `git add -f` when committing under `docs/superpowers/`.
- Work on a dedicated feature branch off current `master`.

## File map

| File | Responsibility |
|------|----------------|
| `crates/sarca-sync/src/types.rs` | `SyncStatus` fields `scanned`, `pending`, `already_synced` |
| `crates/sarca-sync/src/engine.rs` | Fill counters in `push_local` / `sync_binding` / placeholders / error statuses |
| `crates/sarca-sync/src/candidate.rs` | `follow_links(true)` + skip outside-prefix entries |
| `ui/src/common/syncScanHint.js` | Pure helper: status → hint string or null |
| `ui/src/components/SettingsSyncPanel.jsx` | Render hint under Camera meta / near status |
| `ui/src/components/SettingsSyncPanel.test.jsx` | Vitest for hint copy |
| `ui/src/common/syncScanHint.test.js` | Unit tests for helper |
| `client/mobile/README.md` or sync docs | One-line note that desktop shows scan honesty (optional, Task 4) |

---

### Task 1: `SyncStatus` scan counters + engine fill

**Files:**
- Modify: `crates/sarca-sync/src/types.rs`
- Modify: `crates/sarca-sync/src/engine.rs`
- Test: `engine.rs` / `types` tests

**Interfaces:**
- Consumes: existing `SyncStatus`, `push_local`, `filter_pending_candidates`
- Produces: `SyncStatus { scanned, pending, already_synced, … }` with `#[serde(default)]` on new fields

- [ ] **Step 1: Write failing test for counters**

Add to `engine.rs` tests (or types):

```rust
#[test]
fn sync_status_scan_fields_default_to_zero() {
    let s = SyncStatus {
        binding_id: "b".into(),
        ..Default::default()
    };
    assert_eq!(s.scanned, 0);
    assert_eq!(s.pending, 0);
    assert_eq!(s.already_synced, 0);
}
```

And an integration-style test using a temp dir + `FsMediaSource` + index with one already-synced entry + one new file — call `push_local` with a stub API or only assert via a small helper. Prefer extracting:

```rust
pub fn scan_counters(scanned: usize, pending: usize) -> (usize, usize, usize) {
    (scanned, pending, scanned.saturating_sub(pending))
}
```

Test:

```rust
#[test]
fn scan_counters_compute_already_synced() {
    assert_eq!(scan_counters(5, 2), (5, 2, 3));
    assert_eq!(scan_counters(0, 0), (0, 0, 0));
}
```

Wire real fill in `push_local` returning counters somehow — cleanest: change `push_local` to return `(usize /*uploaded*/, ScanStats)` or set fields on a struct returned to `sync_binding`.

Recommended shape:

```rust
struct PushLocalResult {
    uploaded: usize,
    scanned: usize,
    pending: usize,
}

async fn push_local(...) -> Result<PushLocalResult> { ... }

// sync_binding:
let push = self.push_local(binding).await?;
Ok(SyncStatus {
    binding_id: ...,
    cursor,
    last_error: None,
    uploading: push.uploaded,
    downloading,
    conflicts: ...,
    scanned: push.scanned,
    pending: push.pending,
    already_synced: push.scanned.saturating_sub(push.pending),
})
```

On `push_local` error after list+filter succeeded, prefer attaching counters on the error path by updating status in `tick_filtered` Err arm — optional MVP: error statuses may leave counters 0; if list succeeded before fail, set counters on the Err `SyncStatus` in `tick_filtered` by catching a richer error — **YAGNI**: only fill counters on Ok path for Task 1; document that.

Update all `SyncStatus { ... }` literals in `engine.rs` (placeholders, error arms) to include the new fields (or rely on `..Default` / explicit 0).

- [ ] **Step 2: Run test — expect FAIL**

Run: `cargo test -p sarca-sync sync_status_scan_fields --lib`  
Expected: FAIL (fields missing)

- [ ] **Step 3: Implement types + engine fill**

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncStatus {
    pub binding_id: String,
    pub cursor: i64,
    pub last_error: Option<String>,
    pub uploading: usize,
    pub downloading: usize,
    pub conflicts: usize,
    #[serde(default)]
    pub scanned: usize,
    #[serde(default)]
    pub pending: usize,
    #[serde(default)]
    pub already_synced: usize,
}
```

In `push_local` after `filter_pending_candidates`:

```rust
let scanned = candidates.len();
let pending_n = pending_candidates.len();
// ... existing upload loop ...
Ok(PushLocalResult {
    uploaded,
    scanned,
    pending: pending_n,
})
```

- [ ] **Step 4: Run full crate tests**

Run: `cargo test -p sarca-sync --lib`  
Expected: PASS (fix all SyncStatus constructions)

- [ ] **Step 5: Commit**

```bash
git add crates/sarca-sync/src/types.rs crates/sarca-sync/src/engine.rs
git commit -m "$(cat <<'EOF'
feat(sync): expose scanned/pending/already_synced on SyncStatus

EOF
)"
```

---

### Task 2: WalkDir `follow_links` + skip bad relative paths

**Files:**
- Modify: `crates/sarca-sync/src/candidate.rs`
- Test: same file `#[cfg(unix)]`

**Interfaces:**
- Consumes: `WalkDir`, `LocalCandidate`
- Produces: same `collect_fs_candidates` signature; behavior change only

- [ ] **Step 1: Write failing symlink test**

```rust
#[test]
#[cfg(unix)]
fn collect_fs_candidates_follows_symlink_to_media_file() {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("real");
    std::fs::create_dir_all(&real).unwrap();
    std::fs::write(real.join("a.jpg"), b"x").unwrap();
    let link = dir.path().join("link.jpg");
    std::os::unix::fs::symlink(real.join("a.jpg"), &link).unwrap();

    let got = collect_fs_candidates(dir.path(), true).unwrap();
    assert!(
        got.iter().any(|c| c.relative_path == "link.jpg" || c.relative_path.ends_with("a.jpg")),
        "symlink-to-file must be collected: {got:?}"
    );
}
```

Prefer asserting `relative_path == "link.jpg"` when the symlink itself is under root and followed as a file.

- [ ] **Step 2: Run — expect FAIL** (symlink currently skipped as non-file)

Run: `cargo test -p sarca-sync collect_fs_candidates_follows_symlink --lib`

- [ ] **Step 3: Implement**

```rust
for entry in WalkDir::new(root).follow_links(true) {
    // ... same error handling ...
    if !entry.file_type().is_file() {
        continue;
    }
    let path = entry.path().to_path_buf();
    let Ok(rel_os) = path.strip_prefix(root) else {
        warn!(path = %path.display(), root = %root.display(), "skip entry outside binding root");
        continue;
    };
    let rel = rel_os.to_string_lossy().replace('\\', "/");
    // ...
}
```

- [ ] **Step 4: Run candidate + full sarca-sync tests**

Run: `cargo test -p sarca-sync --lib`  
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/sarca-sync/src/candidate.rs
git commit -m "$(cat <<'EOF'
fix(sync): follow symlink files when collecting upload candidates

EOF
)"
```

---

### Task 3: UI scan hint helper + SettingsSyncPanel

**Files:**
- Create: `ui/src/common/syncScanHint.js`
- Create: `ui/src/common/syncScanHint.test.js`
- Modify: `ui/src/components/SettingsSyncPanel.jsx`
- Modify: `ui/src/components/SettingsSyncPanel.test.jsx`

**Interfaces:**
- Consumes: status objects from `sync_statuses`; transfer snap unfinished counts
- Produces:

```js
/**
 * @param {{ last_error?: string|null, scanned?: number, pending?: number, already_synced?: number }|null|undefined} status
 * @param {{ unfinishedUploads?: number }} [opts]
 * @returns {string|null}
 */
export function syncScanHint(status, opts = {}) { ... }
```

Rules (exact copy):

1. If `status.last_error` → `null` (error banner owns it)
2. If `(opts.unfinishedUploads || 0) > 0` or `Number(status.pending) > 0` and transfers active — if unfinished uploads > 0 → `null` (queue owns UX)
3. If `Number(status.scanned) === 0` → `'No media files found in the local folder'`
4. If `scanned > 0` && `pending === 0` && unfinishedUploads === 0 →  
   `` `${scanned} media file(s) found, all already uploaded` ``  
   (use `file` vs `files` correctly: 1 → `file`, else `files`)
5. Else `null`

Camera binding: pick `statuses().find(s => s.binding_id === autoBinding()?.id)`.

Render:

```jsx
<Show when={cameraScanHint()}>
  <p class="settings-bot-hint">{cameraScanHint()}</p>
</Show>
```

near Camera meta (after `settings-sync-panel__meta`).

- [ ] **Step 1: Write Vitest for helper (fail until implemented)**

```js
import { describe, it, expect } from 'vitest'
import { syncScanHint } from './syncScanHint'

describe('syncScanHint', () => {
  it('returns null when last_error set', () => {
    expect(syncScanHint({ last_error: 'x', scanned: 0 })).toBeNull()
  })
  it('reports empty folder', () => {
    expect(syncScanHint({ scanned: 0, pending: 0 })).toBe(
      'No media files found in the local folder',
    )
  })
  it('reports already uploaded', () => {
    expect(syncScanHint({ scanned: 5, pending: 0, already_synced: 5 })).toBe(
      '5 media files found, all already uploaded',
    )
    expect(syncScanHint({ scanned: 1, pending: 0 })).toBe(
      '1 media file found, all already uploaded',
    )
  })
  it('returns null while uploads unfinished', () => {
    expect(
      syncScanHint({ scanned: 5, pending: 2 }, { unfinishedUploads: 2 }),
    ).toBeNull()
  })
})
```

- [ ] **Step 2: Run — FAIL**

Run: `cd ui && pnpm exec vitest run src/common/syncScanHint.test.js`

- [ ] **Step 3: Implement helper + panel wiring + panel test**

Panel test: mock `sync_statuses` → `[{ binding_id: 'cam', scanned: 5, pending: 0, already_synced: 5, ... }]` with camera binding id `cam`; assert text `all already uploaded`.

- [ ] **Step 4: Run Vitest**

Run: `cd ui && pnpm exec vitest run src/common/syncScanHint.test.js src/components/SettingsSyncPanel.test.jsx`  
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add ui/src/common/syncScanHint.js ui/src/common/syncScanHint.test.js \
  ui/src/components/SettingsSyncPanel.jsx ui/src/components/SettingsSyncPanel.test.jsx
git commit -m "$(cat <<'EOF'
feat(ui): show desktop auto-upload scan honesty hints

EOF
)"
```

---

### Task 4: Docs + verification

**Files:**
- Modify: `client/mobile/README.md` (Auto-upload bullet: desktop shows scanned/already-uploaded hints)
- Optional: `.cursor/acceptance/2026-07-29-desktop-autoupload-honesty.md`
- Force-add spec/plan if committing docs

- [ ] **Step 1: README one-liner** under Auto-upload

- [ ] **Step 2: Run evidence**

```bash
cargo test -p sarca-sync --lib
cd client/src-tauri && cargo test --lib
cd ui && pnpm exec vitest run src/common/syncScanHint.test.js src/components/SettingsSyncPanel.test.jsx
```

Expected: all PASS

- [ ] **Step 3: Commit**

```bash
git add client/mobile/README.md
git add -f docs/superpowers/specs/2026-07-29-desktop-autoupload-honesty-design.md \
         docs/superpowers/plans/2026-07-29-desktop-autoupload-honesty.md
git commit -m "$(cat <<'EOF'
docs: desktop auto-upload honesty plan and README note

EOF
)"
```

---

## Spec coverage

| Spec item | Task |
|-----------|------|
| `scanned` / `pending` / `already_synced` on SyncStatus | 1 |
| Fill on successful push | 1 |
| UI copy empty / already uploaded | 3 |
| Prefer error banner / queue over hint | 3 |
| WalkDir follow_links + skip bad strip_prefix | 2 |
| No new IPC / no index wipe / no iOS | all |
| Tests Rust + Vitest | 1–3 |
| Manual Linux | 4 evidence notes |

## Placeholder self-review

- Exact UI strings locked in Global Constraints and Task 3.
- `PushLocalResult` naming consistent across Task 1 steps.
- Error-path counters explicitly YAGNI (Ok path only).
