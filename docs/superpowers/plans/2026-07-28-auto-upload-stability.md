# Auto-upload Stability + Settings Switches Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Linux-client auto-upload stable across enable/disable (soft-disable + preserved index), stop Camera ticks from starving folder uploads (per-binding fair scheduling), auto-enable `background_sync` when enabling uploads, replace client-settings checkboxes with explicit switches, and lock behavior in with Vitest + Rust tests.

**Architecture:** Bindings keep a durable SQLite row; UI/IPC flips `enabled` instead of delete/recreate. `SyncEngine` replaces the global multi-binding `tick_lock` with a per-binding run gate (skip if already in flight) plus a small concurrency semaphore so Camera and folder_upload can progress together. SolidJS Settings use a shared `SettingsSwitch` (`role="switch"`). Tests: `sarca-sync`/`sarca-client` unit tests + new Vitest suite in `ui/` with mocked `nativeInvoke`.

**Tech Stack:** Rust (`sarca-sync`, Tauri 2 `sarca-client`), SolidJS + Vitest + `@solidjs/testing-library` + jsdom, existing remote IPC/`capabilities/default.json` ACL.

**Spec:** `docs/superpowers/specs/2026-07-28-auto-upload-stability-design.md`  
**Acceptance:** `.cursor/acceptance/2026-07-28-auto-upload-stability.md`

## Global Constraints

- Soft-disable only: Camera/folder Off → `set_binding_enabled(false)`; never wipe index on toggle.
- Explicit Remove still calls `remove_binding` (deletes entries).
- Enabling Camera or adding a folder must set `background_sync: true` via `set_client_prefs`.
- Fairness: `auto_upload` and `folder_upload` must not serialize behind one global lock.
- Client settings only for switches (Sync + Security app lock) — not Files/share UIs.
- No Tauri WebDriver; no `notify` FS watchers; no server binding APIs.
- `docs/` is gitignored — force-add (`git add -f`) only when committing specs/plans under `docs/superpowers/`.
- Work on a dedicated feature branch off current `master` (do not mix with unrelated purge WIP).

## File map

| File | Responsibility |
|------|----------------|
| `crates/sarca-sync/src/index.rs` | `set_binding_enabled(id, enabled)` UPDATE; preserve entries |
| `crates/sarca-sync/src/scheduler.rs` | Per-binding in-flight gate + concurrency semaphore |
| `crates/sarca-sync/src/engine.rs` | Wire scheduler into `tick_filtered`; `set_binding_enabled`; optional single-binding tick |
| `crates/sarca-sync/src/lib.rs` | `mod scheduler` |
| `client/src-tauri/src/commands.rs` | `set_binding_enabled`, `update_binding_local_path`; optional `binding_id` on `sync_now` |
| `client/src-tauri/src/remote_ipc.rs` | Dispatch + allowlist for new commands |
| `client/src-tauri/build.rs` | AppManifest command list |
| `client/src-tauri/capabilities/default.json` | `allow-set-binding-enabled`, `allow-update-binding-local-path` |
| `client/src-tauri/src/lib.rs` | Register invoke handlers |
| `client/scripts/check-remote-acl.py` | Require new allows |
| `ui/src/components/SettingsSwitch.jsx` | Switch control |
| `ui/src/common/autoUploadActions.js` | Pure helpers for Camera toggle / prefs merge (testable) |
| `ui/src/components/SettingsSyncPanel.jsx` | Soft-disable UI, folder switches, prefs coupling, change-folder fix |
| `ui/src/components/SettingsModal.jsx` | App lock → SettingsSwitch |
| `ui/src/index.css` | `.settings-switch` styles |
| `ui/vite.config.js` + `ui/package.json` | Vitest harness |
| `ui/src/components/*.test.jsx` | UI regression suite |
| `client/src/sync.js` | Align legacy page with soft-disable if still callable |
| `.github/workflows/client.yml` | Add UI unit-test job (or step) for `ui/**` paths |

---

### Task 1: `LocalIndex::set_binding_enabled` (preserve entries)

**Files:**
- Modify: `crates/sarca-sync/src/index.rs`
- Test: same file `#[cfg(test)]` module (add at bottom of `index.rs` if missing)

**Interfaces:**
- Consumes: existing `upsert_binding`, `upsert_entry`, `list_bindings`, `remove_binding`, `get_entry`
- Produces: `pub fn set_binding_enabled(&self, id: &str, enabled: bool) -> Result<()>`

- [ ] **Step 1: Write the failing test**

Add to `index.rs` tests (create `#[cfg(test)] mod tests` if absent):

```rust
#[test]
fn set_binding_enabled_preserves_entries() {
    let dir = tempfile::tempdir().unwrap();
    let idx = LocalIndex::open(&dir.path().join("sync-index.sqlite")).unwrap();
    let id = "b1".to_string();
    let sid = uuid::Uuid::new_v4();
    idx.upsert_binding(&crate::types::Binding {
        id: id.clone(),
        storage_id: sid,
        remote_root: "Camera".into(),
        local_path: "/tmp/pics".into(),
        mode: crate::types::BindingMode::AutoUpload,
        enabled: true,
    })
    .unwrap();
    idx.upsert_entry(
        &id,
        &IndexEntry {
            relative_path: "a.jpg".into(),
            size: 10,
            mtime_ms: 1,
            content_hash: Some("abc".into()),
            remote_file_id: None,
            last_cursor: 0,
        },
    )
    .unwrap();

    idx.set_binding_enabled(&id, false).unwrap();
    let b = idx.list_bindings().unwrap().into_iter().find(|x| x.id == id).unwrap();
    assert!(!b.enabled);
    assert!(idx.get_entry(&id, "a.jpg").unwrap().is_some());

    idx.set_binding_enabled(&id, true).unwrap();
    let b = idx.list_bindings().unwrap().into_iter().find(|x| x.id == id).unwrap();
    assert!(b.enabled);
    assert!(idx.get_entry(&id, "a.jpg").unwrap().is_some());
}
```

If `tempfile` is not already a dev-dependency of `sarca-sync`, add it in `crates/sarca-sync/Cargo.toml` under `[dev-dependencies]`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p sarca-sync set_binding_enabled_preserves_entries -- --nocapture`  
Expected: FAIL (method missing or compile error)

- [ ] **Step 3: Implement**

```rust
pub fn set_binding_enabled(&self, id: &str, enabled: bool) -> Result<()> {
    let n = self.lock()?.execute(
        "UPDATE bindings SET enabled = ?2 WHERE id = ?1",
        params![id, i64::from(enabled)],
    )?;
    if n == 0 {
        anyhow::bail!("binding not found: {id}");
    }
    Ok(())
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p sarca-sync set_binding_enabled_preserves_entries -- --nocapture`  
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/sarca-sync/src/index.rs crates/sarca-sync/Cargo.toml Cargo.lock
git commit -m "feat(sync): soft-disable bindings without wiping index entries"
```

---

### Task 2: `SyncEngine::set_binding_enabled` + disabled bindings skipped by tick

**Files:**
- Modify: `crates/sarca-sync/src/engine.rs`
- Test: `crates/sarca-sync/src/engine.rs` `#[cfg(test)]`

**Interfaces:**
- Consumes: `LocalIndex::set_binding_enabled`
- Produces: `pub fn set_binding_enabled(&self, id: &str, enabled: bool) -> Result<()>`

- [ ] **Step 1: Write failing test that disable is visible via `list_bindings`**

```rust
#[test]
fn engine_set_binding_enabled_updates_flag() {
    let dir = tempfile::tempdir().unwrap();
    let engine = SyncEngine::open(
        SyncEngineConfig {
            poll_interval: Duration::from_secs(30),
            api: Arc::new(tokio::sync::RwLock::new(SarcaApi::new(
                "http://127.0.0.1".into(),
                String::new(),
            ))),
            data_dir: dir.path().to_path_buf(),
        },
        Arc::new(KeepBothPrompt),
    )
    .unwrap();
    let id = "cam".to_string();
    engine
        .upsert_binding(&Binding {
            id: id.clone(),
            storage_id: uuid::Uuid::new_v4(),
            remote_root: "Camera".into(),
            local_path: dir.path().join("pics").to_string_lossy().into(),
            mode: BindingMode::AutoUpload,
            enabled: true,
        })
        .unwrap();
    engine.set_binding_enabled(&id, false).unwrap();
    let b = engine.list_bindings().unwrap().into_iter().find(|b| b.id == id).unwrap();
    assert!(!b.enabled);
}
```

- [ ] **Step 2: Run test — expect FAIL**

Run: `cargo test -p sarca-sync engine_set_binding_enabled_updates_flag -- --nocapture`

- [ ] **Step 3: Implement wrapper**

```rust
pub fn set_binding_enabled(&self, id: &str, enabled: bool) -> Result<()> {
    self.index.set_binding_enabled(id, enabled)
}
```

Confirm `tick_filtered` already has `.filter(|b| b.enabled && allow(b))` — do not remove that filter.

- [ ] **Step 4: Pass + commit**

```bash
cargo test -p sarca-sync engine_set_binding_enabled_updates_flag
git add crates/sarca-sync/src/engine.rs
git commit -m "feat(sync): expose set_binding_enabled on SyncEngine"
```

---

### Task 3: Per-binding fair scheduler

**Files:**
- Create: `crates/sarca-sync/src/scheduler.rs`
- Modify: `crates/sarca-sync/src/lib.rs` (add `mod scheduler; pub use scheduler::BindingScheduler;`)
- Modify: `crates/sarca-sync/src/engine.rs` (replace global `tick_lock` usage for multi-binding pass)

**Interfaces:**
- Consumes: `tokio::sync::{Mutex, Semaphore}`, `std::collections::HashMap`, `futures::future::join_all`
- Produces:
  - `pub struct BindingScheduler { /* private */ }`
  - `impl BindingScheduler { pub fn new(max_concurrent: usize) -> Self; pub async fn run<F, Fut, T>(&self, binding_id: &str, f: F) -> Option<T> where F: FnOnce() -> Fut, Fut: Future<Output = T>; }`
  - Semantics: if this `binding_id` is already in flight → return `None` immediately (skip). Else acquire a global semaphore permit (default max 2), run `f().await`, release.

- [ ] **Step 1: Write failing scheduler concurrency test** in `scheduler.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::time::{sleep, Duration};

    #[tokio::test]
    async fn two_bindings_overlap_in_time() {
        let sched = BindingScheduler::new(2);
        let live = Arc::new(AtomicUsize::new(0));
        let max_live = Arc::new(AtomicUsize::new(0));

        let mk = |id: &'static str| {
            let sched = &sched;
            let live = live.clone();
            let max_live = max_live.clone();
            async move {
                sched
                    .run(id, || {
                        let live = live.clone();
                        let max_live = max_live.clone();
                        async move {
                            let n = live.fetch_add(1, Ordering::SeqCst) + 1;
                            max_live.fetch_max(n, Ordering::SeqCst);
                            sleep(Duration::from_millis(80)).await;
                            live.fetch_sub(1, Ordering::SeqCst);
                        }
                    })
                    .await
            }
        };

        let (a, b) = tokio::join!(mk("camera"), mk("folder"));
        assert!(a.is_some() && b.is_some());
        assert!(
            max_live.load(Ordering::SeqCst) >= 2,
            "folder must start while camera still running"
        );
    }

    #[tokio::test]
    async fn same_binding_skips_when_busy() {
        let sched = BindingScheduler::new(2);
        let (first, second) = tokio::join!(
            sched.run("cam", || async {
                sleep(Duration::from_millis(100)).await;
                1
            }),
            async {
                sleep(Duration::from_millis(10)).await;
                sched.run("cam", || async { 2 }).await
            }
        );
        assert_eq!(first, Some(1));
        assert_eq!(second, None);
    }
}
```

- [ ] **Step 2: Run — expect FAIL (module missing)**

Run: `cargo test -p sarca-sync two_bindings_overlap_in_time -- --nocapture`

- [ ] **Step 3: Implement `BindingScheduler`**

```rust
use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};

pub struct BindingScheduler {
    in_flight: Mutex<HashMap<String, ()>>,
    slots: Arc<Semaphore>,
}

impl BindingScheduler {
    pub fn new(max_concurrent: usize) -> Self {
        Self {
            in_flight: Mutex::new(HashMap::new()),
            slots: Arc::new(Semaphore::new(max_concurrent.max(1))),
        }
    }

    pub async fn run<F, Fut, T>(&self, binding_id: &str, f: F) -> Option<T>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = T>,
    {
        {
            let mut guard = self.in_flight.lock().await;
            if guard.contains_key(binding_id) {
                return None;
            }
            guard.insert(binding_id.to_string(), ());
        }
        let permit: OwnedSemaphorePermit = self.slots.clone().acquire_owned().await.ok()?;
        let result = f().await;
        drop(permit);
        self.in_flight.lock().await.remove(binding_id);
        Some(result)
    }
}
```

- [ ] **Step 4: Wire into `SyncEngine`**

Replace field `tick_lock: tokio::sync::Mutex<()>` with `scheduler: BindingScheduler` (`BindingScheduler::new(2)` in `open`).

Rewrite `tick_filtered` approximately:

```rust
pub async fn tick_filtered<F>(&self, allow: F) -> Result<()>
where
    F: Fn(&Binding) -> bool,
{
    let mut bindings: Vec<Binding> = self
        .index
        .list_bindings()?
        .into_iter()
        .filter(|b| b.enabled && allow(b))
        .collect();
    bindings.sort_by_key(|b| match b.mode {
        BindingMode::AutoUpload | BindingMode::FolderUpload => 0,
        BindingMode::Sync => 1,
    });

    let mut statuses = self.statuses.read().await.clone();
    // Keep statuses for bindings not in this pass; update ones we run.
    let futs = bindings.into_iter().map(|binding| {
        let scheduler = &self.scheduler;
        async move {
            let ran = scheduler
                .run(&binding.id, || async {
                    // publish in-progress (same as today)
                    let status = match self.sync_binding(&binding).await {
                        Ok(s) => s,
                        Err(e) => { /* warn + SyncStatus with last_error */ }
                    };
                    status
                })
                .await;
            (binding.id, ran)
        }
    });
    let results = futures::future::join_all(futs).await;
    // merge Some(status) into statuses map by binding_id; write back
    Ok(())
}
```

Also add:

```rust
pub async fn tick_binding<F>(&self, binding_id: &str, allow: F) -> Result<()>
where
    F: Fn(&Binding) -> bool,
{
    self.tick_filtered(|b| b.id == binding_id && allow(b)).await
}
```

Fix borrow/`self` capture carefully (clone `Arc` pieces as needed — `SyncEngine` methods already use `&self`; scheduler field is fine; avoid holding `statuses` write lock across `sync_binding`).

- [ ] **Step 5: Run scheduler + existing engine tests**

Run: `cargo test -p sarca-sync`  
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add crates/sarca-sync/src/scheduler.rs crates/sarca-sync/src/lib.rs crates/sarca-sync/src/engine.rs
git commit -m "feat(sync): run enabled bindings concurrently with per-id skip"
```

---

### Task 4: Tauri IPC — `set_binding_enabled` + `update_binding_local_path` + optional `sync_now` binding filter

**Files:**
- Modify: `client/src-tauri/src/commands.rs`
- Modify: `client/src-tauri/src/remote_ipc.rs`
- Modify: `client/src-tauri/build.rs`
- Modify: `client/src-tauri/capabilities/default.json`
- Modify: `client/src-tauri/src/lib.rs` (`.invoke_handler` list)
- Modify: `client/scripts/check-remote-acl.py`

**Interfaces:**
- Consumes: `SyncEngine::set_binding_enabled`, `upsert_binding`, `list_bindings`, `tick_binding`
- Produces:
  - `set_binding_enabled(state, id: String, enabled: bool) -> Result<(), String>`
  - `update_binding_local_path(state, id: String, local_path: String) -> Result<Binding, String>`
  - `sync_now(..., binding_id: Option<String>)` — when `Some`, only that id

- [ ] **Step 1: Extend ACL regression lists (fail CI until wired)**

In `check-remote-acl.py` `REQUIRED`, append:
- `"allow-set-binding-enabled"`
- `"allow-update-binding-local-path"`

In `remote_ipc.rs` test `sync_security_commands_are_dispatched`, append `"set_binding_enabled"` and `"update_binding_local_path"`.

- [ ] **Step 2: Run ACL script — expect FAIL**

Run: `python3 client/scripts/check-remote-acl.py`  
Expected: FAIL missing allows

- [ ] **Step 3: Implement commands**

```rust
#[tauri::command]
pub fn set_binding_enabled(
    state: State<'_, AppSyncState>,
    id: String,
    enabled: bool,
) -> Result<(), String> {
    state
        .engine
        .set_binding_enabled(&id, enabled)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn update_binding_local_path(
    state: State<'_, AppSyncState>,
    id: String,
    local_path: String,
) -> Result<Binding, String> {
    let mut binding = state
        .engine
        .list_bindings()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|b| b.id == id)
        .ok_or_else(|| format!("binding not found: {id}"))?;
    binding.local_path = local_path;
    state
        .engine
        .upsert_binding(&binding)
        .map_err(|e| e.to_string())?;
    Ok(binding)
}
```

Update `sync_now` signature to accept `binding_id: Option<String>` and call `tick_binding` when present (still apply `allow_auto_upload` filter).

Register in `lib.rs`, `build.rs` `COMMANDS`, `REMOTE_SETTINGS_COMMANDS`, `dispatch` match arms (snake + camel args: `id`, `enabled`, `local_path`/`localPath`, `binding_id`/`bindingId`), and capabilities permissions.

- [ ] **Step 4: Unit test allowlist**

```rust
#[test]
fn soft_disable_commands_are_dispatched() {
    assert!(is_dispatched_command("set_binding_enabled"));
    assert!(is_dispatched_command("update_binding_local_path"));
}
```

- [ ] **Step 5: Verify**

Run:
```bash
python3 client/scripts/check-remote-acl.py
cargo test -p sarca-client --lib
```
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add client/src-tauri client/scripts/check-remote-acl.py
git commit -m "feat(client): IPC for soft-disable and binding path update"
```

---

### Task 5: Vitest harness in `ui/`

**Files:**
- Modify: `ui/package.json`
- Modify: `ui/vite.config.js`
- Create: `ui/src/test/setup.js`
- Create: `ui/src/components/SettingsSwitch.test.jsx` (first green smoke)

**Interfaces:**
- Produces: `pnpm test` script running Vitest

- [ ] **Step 1: Add deps + scripts**

```bash
cd ui && pnpm add -D vitest jsdom @solidjs/testing-library @testing-library/jest-dom
```

`package.json` scripts:
```json
"test": "vitest run",
"test:watch": "vitest"
```

`vite.config.js` add:
```js
test: {
  environment: 'jsdom',
  globals: true,
  setupFiles: './src/test/setup.js',
  include: ['src/**/*.test.{js,jsx}'],
},
```

`src/test/setup.js`:
```js
import '@testing-library/jest-dom/vitest'
```

- [ ] **Step 2: Write failing SettingsSwitch test first** (component not yet created — Task 6 may land with it; if preferred, create stub that fails a11y assert)

Minimal failing test file expecting `SettingsSwitch` export:

```jsx
import { render, fireEvent } from '@solidjs/testing-library'
import { describe, it, expect, vi } from 'vitest'
import SettingsSwitch from './SettingsSwitch'

describe('SettingsSwitch', () => {
  it('exposes role=switch and toggles', async () => {
    const onChange = vi.fn()
    const { getByRole } = render(() => (
      <SettingsSwitch checked={false} onChange={onChange} />
    ))
    const sw = getByRole('switch')
    expect(sw).toHaveAttribute('aria-checked', 'false')
    fireEvent.click(sw)
    expect(onChange).toHaveBeenCalledWith(true)
  })
})
```

- [ ] **Step 3: Run — expect FAIL**

Run: `cd ui && pnpm test`  
Expected: FAIL cannot resolve `./SettingsSwitch` or role missing

- [ ] **Step 4: Commit harness even if switch lands in next task** (or combine with Task 6 in one commit if executing back-to-back)

```bash
git add ui/package.json ui/pnpm-lock.yaml ui/vite.config.js ui/src/test
git commit -m "test(ui): add Vitest + Testing Library harness"
```

---

### Task 6: `SettingsSwitch` component + CSS

**Files:**
- Create: `ui/src/components/SettingsSwitch.jsx`
- Modify: `ui/src/index.css`
- Modify: `ui/src/components/SettingsSwitch.test.jsx`

**Interfaces:**
- Produces: `export default function SettingsSwitch(props)` with `checked`, `disabled`, `onChange(next: boolean)`, optional `id`

- [ ] **Step 1: Implement component**

```jsx
export default function SettingsSwitch(props) {
  const checked = () => Boolean(props.checked)
  const disabled = () => Boolean(props.disabled)
  const toggle = () => {
    if (disabled()) return
    props.onChange?.(!checked())
  }
  const onKeyDown = (e) => {
    if (e.key === ' ' || e.key === 'Enter') {
      e.preventDefault()
      toggle()
    }
  }
  return (
    <button
      type="button"
      id={props.id}
      role="switch"
      class="settings-switch"
      aria-checked={checked() ? 'true' : 'false'}
      disabled={disabled()}
      onClick={toggle}
      onKeyDown={onKeyDown}
    >
      <span class="settings-switch__thumb" aria-hidden="true" />
    </button>
  )
}
```

CSS (replace reliance on checkbox thumb for settings rows): track 44×24, thumb circle, checked uses `--sarca-accent`, disabled opacity 0.45, `:focus-visible` outline. Keep `.settings-toggle` as the row layout (label + switch).

- [ ] **Step 2: `pnpm test` — SettingsSwitch PASS**

- [ ] **Step 3: Commit**

```bash
git add ui/src/components/SettingsSwitch.jsx ui/src/index.css ui/src/components/SettingsSwitch.test.jsx
git commit -m "feat(ui): add SettingsSwitch for client settings toggles"
```

---

### Task 7: Pure auto-upload action helpers + Sync panel soft-disable

**Files:**
- Create: `ui/src/common/autoUploadActions.js`
- Create: `ui/src/common/autoUploadActions.test.js`
- Modify: `ui/src/components/SettingsSyncPanel.jsx`
- Create: `ui/src/components/SettingsSyncPanel.test.jsx`

**Interfaces:**
- Produces:
  - `export function cameraBinding(bindings)` → first `auto_upload` or null
  - `export function resolveCameraToggle(bindings, enable)` →  
    `{ action: 'noop' }` | `{ action: 'add' }` | `{ action: 'set_enabled', id, enabled }`  
    Rules: enable+no row→`add`; enable+row→`set_enabled` true (even if already true→still `set_enabled` or `noop` if already enabled — prefer `noop` when `enabled===true`); disable+row→`set_enabled` false; disable+no row→`noop`
  - `export function withBackgroundSyncOn(prefs)` → `{ ...prefs, background_sync: true }`

- [ ] **Step 1: Write helper unit tests (failing)**

```js
import { describe, it, expect } from 'vitest'
import {
  resolveCameraToggle,
  withBackgroundSyncOn,
} from './autoUploadActions'

describe('resolveCameraToggle', () => {
  it('adds when enabling with no binding', () => {
    expect(resolveCameraToggle([], true)).toEqual({ action: 'add' })
  })
  it('soft-disables existing enabled binding', () => {
    expect(
      resolveCameraToggle([{ id: '1', mode: 'auto_upload', enabled: true }], false),
    ).toEqual({ action: 'set_enabled', id: '1', enabled: false })
  })
  it('re-enables disabled binding without add', () => {
    expect(
      resolveCameraToggle([{ id: '1', mode: 'auto_upload', enabled: false }], true),
    ).toEqual({ action: 'set_enabled', id: '1', enabled: true })
  })
  it('noops when already enabled', () => {
    expect(
      resolveCameraToggle([{ id: '1', mode: 'auto_upload', enabled: true }], true),
    ).toEqual({ action: 'noop' })
  })
})

describe('withBackgroundSyncOn', () => {
  it('forces background_sync true', () => {
    expect(withBackgroundSyncOn({ wifi_only: true, background_sync: false }))
      .toEqual({ wifi_only: true, background_sync: true })
  })
})
```

- [ ] **Step 2: Implement helpers — tests PASS**

- [ ] **Step 3: Rewrite `SettingsSyncPanel` behavior**

Key changes:
1. `autoBinding()` = any `auto_upload` row (enabled or not); `cameraOn()` = `autoBinding()?.enabled === true`.
2. Camera `SettingsSwitch` `checked={cameraOn()}` → `setAutoUpload`.
3. `setAutoUpload(enable)`:
   - list live bindings
   - `resolveCameraToggle`
   - `add` branch: existing add_binding + ensure Camera + `savePrefs(withBackgroundSyncOn(prefs()))` + kick
   - `set_enabled` branch: `nativeInvoke('set_binding_enabled', { id, enabled })` + if enabling, `savePrefs(withBackgroundSyncOn(...))` + kick; **never** `remove_binding`
   - `noop` while already on: if caller passed a new `localPath` intent, that is **not** this function — see change-folder
4. Change local folder button:
   ```js
   const path = await pickFolder(localPath())
   if (!path) return
   setLocalPath(path)
   const existing = cameraBinding(await nativeInvoke('list_bindings'))
   if (existing) {
     await nativeInvoke('update_binding_local_path', { id: existing.id, localPath: path })
     if (!existing.enabled) {
       await nativeInvoke('set_binding_enabled', { id: existing.id, enabled: true })
     }
     await savePrefs(withBackgroundSyncOn(prefs()))
     kickSyncNow()
     await refresh()
   } else {
     await setAutoUpload(true)
   }
   ```
5. `addFolderUpload`: after successful `add_binding`, `savePrefs(withBackgroundSyncOn(prefs()))`.
6. Folder list: show **all** folder bindings (including disabled); each row has `SettingsSwitch` bound to `b.enabled` calling `set_binding_enabled`; Remove unchanged.
7. Replace all three checkboxes with `SettingsSwitch` inside `.settings-toggle` rows.
8. Show Change folder when `autoBinding()` exists (including disabled).

- [ ] **Step 4: Component tests with mocked bridge**

Mock module:
```js
vi.mock('../common/nativeBridge', () => ({
  nativeInvoke: vi.fn(),
  pickLocalFolder: vi.fn(),
  isMobileNativePlatform: () => false,
  formatBytes: (n) => String(n),
}))
```

Also stub `filesChromeStore` / `alertStore` as needed (vi.mock those modules returning `{ storageId: () => 'sid', storageName: () => 'S', addAlert: vi.fn() }`).

Cases (assert `nativeInvoke` call order / absence of `remove_binding`):
1. enable empty → `add_binding` + `set_client_prefs` with `background_sync: true`
2. disable enabled → `set_binding_enabled` false, no `remove_binding`
3. enable disabled → `set_binding_enabled` true, no `add_binding`
4. change folder with existing → `update_binding_local_path`
5. add folder → prefs background true
6. folder row switch → `set_binding_enabled`

- [ ] **Step 5: Run**

Run: `cd ui && pnpm test`  
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add ui/src/common/autoUploadActions.js ui/src/common/autoUploadActions.test.js \
  ui/src/components/SettingsSyncPanel.jsx ui/src/components/SettingsSyncPanel.test.jsx
git commit -m "fix(ui): soft-disable auto-upload and couple background sync"
```

---

### Task 8: SettingsModal app lock → SettingsSwitch

**Files:**
- Modify: `ui/src/components/SettingsModal.jsx`
- Create: `ui/src/components/SettingsModal.appLock.test.jsx` (narrow test) **or** extend an existing pattern — keep test focused on Security switch rendering when `isNative` mocked true

**Interfaces:**
- Consumes: `SettingsSwitch`
- Produces: App lock row uses switch; behavior of `saveAppLock` unchanged

- [ ] **Step 1: Replace checkbox with SettingsSwitch** wired to existing `lockEnabled` / `saveAppLock` logic (on→draft PIN message; off→`saveAppLock(false)`).

- [ ] **Step 2: Test** `getByRole('switch', { name: ... })` or unlabeled switch in Security section after opening modal tab — if modal mount is heavy, extract a tiny `AppLockToggle` presentational wrapper and test that instead.

- [ ] **Step 3: `pnpm test` PASS + commit**

```bash
git commit -m "feat(ui): use SettingsSwitch for app lock"
```

---

### Task 9: Legacy `client/src/sync.js` soft-disable align

**Files:**
- Modify: `client/src/sync.js`

**Interfaces:**
- Consumes: `set_binding_enabled` invoke

- [ ] **Step 1: Mirror Camera toggle logic** — disable via `set_binding_enabled`; enable existing via set_enabled; only `add_binding` when absent; on enable call `set_client_prefs` with `background_sync: true`.

- [ ] **Step 2: Manual smoke not required if page unused; grep confirms no remaining remove-on-toggle for auto_upload:**

Run: `rg "remove_binding" client/src/sync.js -n`  
Expected: remove only for explicit cleanup paths, not the main toggle-off path.

- [ ] **Step 3: Commit**

```bash
git commit -m "fix(client): soft-disable auto-upload on legacy sync page"
```

---

### Task 10: CI — run UI unit tests

**Files:**
- Modify: `.github/workflows/client.yml` **and/or** add `ui` path job in an existing UI workflow (prefer extending `client.yml` with a `ui-unit` job that triggers on `ui/**` as well, or add a step under a small workflow)

Recommended job:

```yaml
  ui-unit:
    name: ui unit tests
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: pnpm/action-setup@v4
        with:
          version: 11
      - uses: actions/setup-node@v4
        with:
          node-version: 22
          cache: pnpm
          cache-dependency-path: ui/pnpm-lock.yaml
      - run: pnpm install --frozen-lockfile
        working-directory: ui
      - run: pnpm test
        working-directory: ui
```

Also extend `on.pull_request.paths` / `push.paths` to include `ui/**` for this workflow **or** put the job in a UI-focused workflow that already watches `ui/**`.

- [ ] **Step 1: Add job**
- [ ] **Step 2: Locally run the same commands**
- [ ] **Step 3: Commit**

```bash
git commit -m "ci: run ui Vitest suite on client workflow"
```

---

### Task 11: Acceptance verification

**Files:** none (evidence only); update `.cursor/acceptance/2026-07-28-auto-upload-stability.md` status

- [ ] **Step 1: Run full evidence plan**

```bash
cargo test -p sarca-sync
cargo test -p sarca-client --lib
python3 client/scripts/check-remote-acl.py
cd ui && pnpm test
rg -n "set_binding_enabled|SettingsSwitch|role=.switch" client ui crates/sarca-sync
```

- [ ] **Step 2: Fill acceptance report** (PASS/FAIL per checkbox with command evidence)
- [ ] **Step 3: Only if PASS — mark acceptance `done` and stop (do not claim done otherwise)

---

## Spec coverage self-check

| Spec requirement | Task |
|------------------|------|
| Soft-disable Camera / preserve index | 1, 2, 7, 9 |
| Folder enable toggles + Remove keeps delete | 7 |
| Change local folder upsert path | 4, 7 |
| Auto `background_sync` on enable/add | 7, 9 |
| Fair / per-binding ticks | 3 |
| Optional targeted sync | 4 (`binding_id`) |
| SettingsSwitch Sync + Security | 6, 7, 8 |
| Vitest cases §4 | 5–8 |
| Rust disable + fairness tests | 1–3 |
| ACL / IPC allowlist | 4 |
| CI UI tests | 10 |
| No WebDriver / no notify / no server bindings | Global constraints |

## Placeholder / consistency notes

- Command names locked: `set_binding_enabled`, `update_binding_local_path`.
- Scheduler skip-when-busy (not queue) is intentional.
- Camera UI on-state = `enabled === true`, not merely row presence.
