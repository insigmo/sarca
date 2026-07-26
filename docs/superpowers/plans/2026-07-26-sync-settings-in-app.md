# Sync settings in-app Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move all Sync UX into in-app Settings (3rd tab), remove sidebar/FAB/connect-shell Sync entry points, fix mobile file gestures and Linux folder-picker hang, route auto-upload to remote `Camera/`, and add General/Security Seafile-parity items (skip theme).

**Architecture:** Keep Tauri sync commands + `sarca-sync` engine. Enable remote-origin IPC so the website Settings Sync tab can `invoke` those commands (capabilities `remote.urls`). Inject a thin `__sarcaInvoke` bridge. Retarget `sarca-sync://` / tray / menu to open Settings → Sync (query flag) instead of navigating to `sync.html` as primary UI. Fix `pick_local_folder` to async non-blocking on all platforms. Mobile: hide tile stars; disable drag while long-press menu is active.

**Tech Stack:** SolidJS (`ui/`), Tauri 2 + `tauri-plugin-dialog` (`client/src-tauri/`), vanilla JS connect shell (`client/`), `sarca-sync` crate, optional `network-interface` for Wi‑Fi check.

## Global Constraints

- Spec: `docs/superpowers/specs/2026-07-26-sync-settings-in-app-design.md` (Approved).
- Sync is the **3rd** settings tab when shown; native-only for Sync content that needs client APIs.
- Storage settings: Sync tab **always** (native). System Settings: Sync tab **only when a storage is open**.
- Remote auto-upload destination is **`Camera/`** at storage root (create if missing). Not `Media/` / `Photo/`.
- Toggle labels: «Включить автозагрузку фото и видео»; mobile Wi‑Fi «Загружать только через WIFI» default **ON**.
- Skip theme / night-mode changes.
- Mobile breakpoint **max-width: 840px**.
- `docs/` is gitignored — force-add when committing docs.
- Commit messages English; frequent commits; `git pull --rebase` if needed; **no force push**; push to `origin/master`.
- If `/tmp` full: `CARGO_TARGET_DIR=/home/beta/git/sarca/target`.
- Probe versions (reference): `@tauri-apps/plugin-dialog` 2.7.2, `plugin-fs` 2.5.1, `@tauri-apps/api` 2.11.1, `@tauri-apps/cli` 2.11.4, `rfd` 0.17.2, `network-interface` 2.0.5.

## File map

| File | Responsibility |
|------|----------------|
| `client/src-tauri/capabilities/default.json` | Allow remote `http(s)://*` IPC for Sync commands |
| `client/src-tauri/src/commands.rs` | Async `pick_local_folder`; prefs; Wi‑Fi; gallery default; cache/about helpers; app-lock prefs |
| `client/src-tauri/src/state.rs` | Remove Sync FAB; inject `__sarcaInvoke` + open-settings bridge; prefs paths |
| `client/src-tauri/src/lib.rs` | Retarget sync deep-link → Settings Sync; register new commands; drop FAB inject |
| `client/src-tauri/Cargo.toml` | Add `network-interface` if used for Wi‑Fi |
| `client/index.html` + `client/src/main.js` | Remove connect-shell Sync button |
| `client/src/sync.js` + `client/sync.html` | Point auto-upload at `Camera/`; keep as non-primary fallback only |
| `ui/src/common/settings.js` | Add `'security'` tab; Sync visibility helpers |
| `ui/src/common/nativeBridge.js` | **Create** — invoke wrapper for remote webview |
| `ui/src/common/nativeClient.js` | Open Settings Sync (no `sync.html` primary); platform helpers |
| `ui/src/components/SettingsSyncPanel.jsx` | **Create** — full Sync tab UI (toggles, bindings, sync now) |
| `ui/src/components/SettingsModal.jsx` | Tab order General → Access → **Sync** → … → **Security**; General extras; Security panel |
| `ui/src/components/StorageSettingsModal.jsx` | Insert Sync as 3rd tab (native) |
| `ui/src/components/FilesSidebar.jsx` | Remove Sync sidebar entry |
| `ui/src/components/FSListItem.jsx` | Hide star on mobile; no drag after/during long-press |
| `ui/src/index.css` | Hide stars on mobile; Sync toggle styles |
| `ui/src/components/AppLockGate.jsx` | **Create** — PIN lock overlay when enabled |

---

### Task 1: Remove Sync entry points (sidebar, FAB, connect shell)

**Files:**
- Modify: `ui/src/components/FilesSidebar.jsx`
- Modify: `client/src-tauri/src/state.rs` (`native_chrome_js`)
- Modify: `client/index.html`
- Modify: `client/src/main.js`

**Interfaces:**
- Consumes: `nativeClientStore`, `OPEN_SYNC_JS` (keep bridge, drop FAB)
- Produces: No sidebar Sync; no `#sarca-native-sync-fab`; no `#openSync`

- [ ] **Step 1: Remove sidebar Sync**

In `FilesSidebar.jsx`, delete the `<Show when={props.showSync}>…Sync…</Show>` block (~139–149), remove `showSync` / `onOpenSync` from the exported props object (~249–252), and drop unused `openNativeSyncSettings` / `openSidebarSync` imports/helpers if unused.

- [ ] **Step 2: Remove FAB from `native_chrome_js`**

Replace `native_chrome_js()` body so it only marks native + injects open-settings / invoke bridge (Task 3), **without** creating `#sarca-native-sync-fab`.

```rust
pub fn native_chrome_js() -> String {
    format!(
        r#"(function(){{
  try {{
    localStorage.setItem('sarca_native', '1');
    window.__SARCA_NATIVE__ = 1;
    try {{ window.dispatchEvent(new Event('sarca-native')); }} catch (_) {{}}
    {open_sync}
  }} catch (e) {{}}
}})();"#,
        open_sync = OPEN_SYNC_JS
    )
}
```

- [ ] **Step 3: Remove connect-shell Sync button**

In `client/index.html`, remove the `.secondary-actions` / `#openSync` block and rewrite the hint to mention Settings → Sync (not a separate Sync button).

In `client/src/main.js`, delete `setSyncEnabled`, `#openSync` click handler, and all calls to `setSyncEnabled`.

- [ ] **Step 4: Commit**

```bash
git add ui/src/components/FilesSidebar.jsx client/src-tauri/src/state.rs client/index.html client/src/main.js
git commit -m "$(cat <<'EOF'
Remove Sync sidebar, FAB, and connect-shell entry points.

EOF
)"
```

---

### Task 2: Mobile file UX — hide star, long-press without drag

**Files:**
- Modify: `ui/src/components/FSListItem.jsx`
- Modify: `ui/src/index.css`

**Interfaces:**
- Consumes: `isMobileTapOpen`, `LONG_PRESS_MS`, `canFavorite`, `dragEnabled`
- Produces: `showTileStar()` false on mobile; `dragEnabled()` false while long-press armed / after menu open on touch

- [ ] **Step 1: Hide tile star on mobile**

```js
const showTileStar = () => canFavorite() && !isMobileTapOpen()
```

- [ ] **Step 2: Prevent drag from long-press**

Add a module-level (or component `let`) flag `suppressDragAfterLongPress = false`. In `openContextMenuAt` (touch path via long-press timer), set `suppressDragAfterLongPress = true`. Clear it on next `touchend`/`pointerup` after a short timeout.

Update:

```js
const dragEnabled = () =>
  Boolean(props.draggableItem) &&
  !isParentNav() &&
  !suppressDragAfterLongPress &&
  !isMobileTapOpen()
```

(Desktop keeps drag; mobile viewport disables HTML5 drag entirely — matches “long-press must not start drag”.)

- [ ] **Step 3: CSS hide stars on ≤840px**

Inside `@media (max-width: 840px)`:

```css
.fs-grid-item__star,
.fs-list-item__star {
	display: none !important;
}
```

- [ ] **Step 4: Commit**

```bash
git add ui/src/components/FSListItem.jsx ui/src/index.css
git commit -m "$(cat <<'EOF'
Hide mobile tile favorites and disable drag on touch long-press.

EOF
)"
```

---

### Task 3: Remote IPC bridge + async folder picker + Camera/

**Files:**
- Modify: `client/src-tauri/capabilities/default.json`
- Modify: `client/src-tauri/src/commands.rs`
- Modify: `client/src-tauri/src/state.rs` (`OPEN_SYNC_JS` → also `__sarcaInvoke`)
- Modify: `client/src/sync.js` (`Media` → `Camera`)
- Modify: `client/sync.html` (labels)
- Create: `ui/src/common/nativeBridge.js`

**Interfaces:**
- Consumes: `DialogExt::pick_folder` (async callback)
- Produces:
  - `pick_local_folder(app) -> Result<Option<String>, String>` (async)
  - `default_gallery_path() -> Result<String, String>`
  - `window.__sarcaInvoke(cmd, args) -> Promise`
  - `nativeInvoke(cmd, args)` in UI

- [ ] **Step 1: Enable remote capabilities**

```json
{
  "$schema": "https://schema.tauri.app/config/2/capability",
  "identifier": "default",
  "description": "Default Sarca client capabilities",
  "windows": ["main"],
  "remote": {
    "urls": ["http://*", "https://*"]
  },
  "permissions": [
    "core:default",
    "dialog:default",
    "notification:default",
    "shell:allow-open"
  ]
}
```

- [ ] **Step 2: Async non-blocking `pick_local_folder` (all platforms)**

Replace the command (remove `#[cfg(desktop)]` early-`None` on mobile). Use oneshot + `pick_folder` callback so the async runtime is not blocked:

```rust
#[tauri::command]
pub async fn pick_local_folder(app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    use tokio::sync::oneshot;

    let (tx, rx) = oneshot::channel();
    app.dialog()
        .file()
        .set_title("Choose folder")
        .pick_folder(move |folder| {
            let _ = tx.send(folder);
        });
    let folder = rx.await.map_err(|e| e.to_string())?;
    Ok(folder
        .and_then(|p| p.into_path().ok())
        .map(|p| p.to_string_lossy().into_owned()))
}
```

Add:

```rust
#[tauri::command]
pub fn default_gallery_path() -> String {
    #[cfg(target_os = "android")]
    { return "/storage/emulated/0/DCIM".into(); }
    #[cfg(target_os = "ios")]
    { return "".into(); }
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        std::env::var("HOME")
            .map(|h| format!("{h}/Pictures"))
            .unwrap_or_else(|_| "Pictures".into())
    }
}
```

Register `default_gallery_path` in `lib.rs` invoke handler.

- [ ] **Step 3: Inject `__sarcaInvoke` + retarget open-sync to Settings**

Update `OPEN_SYNC_JS` in `state.rs`:

```rust
pub const OPEN_SYNC_JS: &str = r#"
function __sarcaInvoke(cmd, args){
  return new Promise(function(resolve, reject){
    try {
      if (window.__TAURI_INTERNALS__ && typeof window.__TAURI_INTERNALS__.invoke === 'function') {
        window.__TAURI_INTERNALS__.invoke(cmd, args || {}).then(resolve, reject);
        return;
      }
    } catch (e) { reject(e); return; }
    reject(new Error('Native bridge unavailable'));
  });
}
window.__sarcaInvoke = __sarcaInvoke;
function __sarcaOpenSyncSettings(){
  try {
    var u = new URL(location.href);
    u.searchParams.set('__sarca_open_settings', 'sync');
    history.replaceState(null, '', u.pathname + u.search + u.hash);
    window.dispatchEvent(new CustomEvent('sarca-open-settings', { detail: { tab: 'sync' } }));
    return;
  } catch (_) {}
}
window.__sarcaOpenSyncSettings = __sarcaOpenSyncSettings;
"#;
```

- [ ] **Step 4: UI `nativeBridge.js`**

```js
export async function nativeInvoke(cmd, args = {}) {
  try {
    if (typeof window.__sarcaInvoke === 'function') {
      return await window.__sarcaInvoke(cmd, args)
    }
  } catch (e) {
    // fall through
  }
  try {
    const inv = window.__TAURI_INTERNALS__?.invoke
    if (typeof inv === 'function') return await inv(cmd, args)
  } catch (e) {
    // fall through
  }
  throw new Error('Native bridge unavailable')
}

export function isMobileNativePlatform(label) {
  const p = String(label || '').toLowerCase()
  return p === 'android' || p === 'ios'
}
```

- [ ] **Step 5: Change `Media` → `Camera` in `sync.js` / `sync.html`**

In enable-auto-upload handler, `name: "Camera"` and display strings `Camera` instead of `Media`.

- [ ] **Step 6: Commit**

```bash
git add client/src-tauri/capabilities/default.json client/src-tauri/src/commands.rs client/src-tauri/src/state.rs client/src-tauri/src/lib.rs client/src/sync.js client/sync.html ui/src/common/nativeBridge.js
git commit -m "$(cat <<'EOF'
Enable remote Sync IPC, async folder picker, and Camera/ destination.

EOF
)"
```

---

### Task 4: Client prefs — Wi‑Fi only, background backup, app lock

**Files:**
- Modify: `client/src-tauri/Cargo.toml` (add `network-interface = "2"`)
- Modify: `client/src-tauri/src/commands.rs`
- Modify: `client/src-tauri/src/lib.rs`
- Modify: `client/src-tauri/src/state.rs` (optional prefs helpers on `AppSyncState`)
- Modify: `crates/sarca-sync/src/engine.rs` (skip auto-upload tick when Wi‑Fi-only and not on Wi‑Fi)

**Interfaces:**
- Produces JSON prefs at `{app_data_dir}/client_prefs.json`:
  ```json
  {
    "wifi_only": true,
    "background_sync": true,
    "app_lock_enabled": false,
    "app_lock_pin": null
  }
  ```
- Commands:
  - `get_client_prefs() -> ClientPrefs`
  - `set_client_prefs(prefs: ClientPrefs) -> ()`
  - `is_on_wifi() -> bool`
  - `get_about() -> { version, platform }`
  - `clear_local_cache() -> { cleared: bool }` (best-effort delete sync temp/cache dir under data_dir)
  - `get_cache_size() -> u64`

- [ ] **Step 1: Add prefs struct + load/save**

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientPrefs {
    #[serde(default = "default_true")]
    pub wifi_only: bool,
    #[serde(default = "default_true")]
    pub background_sync: bool,
    #[serde(default)]
    pub app_lock_enabled: bool,
    #[serde(default)]
    pub app_lock_pin: Option<String>,
}
fn default_true() -> bool { true }
```

- [ ] **Step 2: Wi‑Fi detection**

```rust
#[tauri::command]
pub fn is_on_wifi() -> bool {
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    {
        use network_interface::{NetworkInterface, NetworkInterfaceConfig};
        NetworkInterface::show()
            .ok()
            .map(|list| {
                list.iter().any(|iface| {
                    let n = iface.name.to_lowercase();
                    (n.starts_with("wl") || n.starts_with("en") || n.contains("wi-fi") || n.contains("wlan"))
                        && iface.addr.iter().any(|a| matches!(a, network_interface::Addr::V4(_)))
                })
            })
            .unwrap_or(true)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        // Mobile: best-effort — treat as Wi‑Fi until OS plugin exists; engine still respects prefs when false.
        true
    }
}
```

(If overly noisy on Ethernet-named interfaces, prefer: return `true` when any non-loopback iface is up; document as best-effort. Prefer matching `wlan`/`wl`/`wifi` names for “Wi‑Fi only”.)

- [ ] **Step 3: Gate auto-upload in engine tick**

In `SyncEngine::tick` / `sync_binding`, before AutoUpload push: if prefs.wifi_only && !is_on_wifi(), skip that binding (log + continue). Pass prefs via `SyncEngineConfig` or a shared `Arc<Mutex<ClientPrefs>>` updated from commands.

- [ ] **Step 4: Commit**

```bash
git add client/src-tauri/Cargo.toml client/src-tauri/src/commands.rs client/src-tauri/src/lib.rs client/src-tauri/src/state.rs crates/sarca-sync/src/engine.rs
git commit -m "$(cat <<'EOF'
Add client prefs for Wi-Fi-only sync, background toggle, and app lock.

EOF
)"
```

---

### Task 5: Settings Sync panel component + tab order

**Files:**
- Create: `ui/src/components/SettingsSyncPanel.jsx`
- Modify: `ui/src/common/settings.js`
- Modify: `ui/src/components/SettingsModal.jsx`
- Modify: `ui/src/components/StorageSettingsModal.jsx`
- Modify: `ui/src/common/nativeClient.js`
- Modify: `ui/src/index.css` (toggle row styles)

**Interfaces:**
- Consumes: `nativeInvoke`, existing binding commands, `ensure_remote_folder` with `name: 'Camera'`
- Produces: reusable `<SettingsSyncPanel storageId={...} />`

- [ ] **Step 1: Extend tab types**

`settings.js`:

```js
/** @typedef {'general' | 'access' | 'sync' | 'trash' | 'storage' | 'security'} SettingsTab */
```

- [ ] **Step 2: Implement `SettingsSyncPanel.jsx`**

Must include:
1. Toggle «Включить автозагрузку фото и видео» — enables/disables `auto_upload` binding to remote `Camera/` (create via `ensure_remote_folder`). Local path = `default_gallery_path` or picker.
2. When auto-upload ON **and** mobile platform: toggle «Загружать только через WIFI» bound to `wifi_only` pref (default ON).
3. Button to change local source folder (`pick_local_folder`).
4. Folder backup / sync section: list non-auto bindings; add sync binding (local pick + remote path / create folder); remove.
5. Background backup toggle → `background_sync` pref (desktop required UI; show on mobile as best-effort).
6. **Sync now** button → `sync_now`.
7. Status text from `sync_statuses`.

Use existing Fluent / MUI Button patterns from SettingsModal. Prefer Switch-like checkbox:

```jsx
<label class="settings-toggle">
  <span>Включить автозагрузку фото и видео</span>
  <input type="checkbox" checked={autoOn()} onChange={onToggleAuto} />
</label>
```

- [ ] **Step 3: System SettingsModal tab order**

Nav order when Sync shown:
1. General  
2. Access  
3. **Sync** (only if `isNative() && chrome.storageId()`)  
4. Trash  
5. Storage  
6. **Security** (always; Task 6 can stub then fill)

Replace brief Sync CTA with `<SettingsSyncPanel />`.

Listen for `sarca-open-settings` / URL `?__sarca_open_settings=sync` on mount → `openSettings('sync')` and strip the query param.

- [ ] **Step 4: StorageSettingsModal**

Tab union: `'general' | 'sync' | 'telegram'`. Order: General, Sync (native), Channels. Render `<SettingsSyncPanel storageId={props.storage.id} />`.

- [ ] **Step 5: Commit**

```bash
git add ui/src/components/SettingsSyncPanel.jsx ui/src/common/settings.js ui/src/components/SettingsModal.jsx ui/src/components/StorageSettingsModal.jsx ui/src/common/nativeClient.js ui/src/index.css
git commit -m "$(cat <<'EOF'
Add in-app Sync settings tab as the third Settings section.

EOF
)"
```

---

### Task 6: General extras + Security tab (app lock)

**Files:**
- Create: `ui/src/components/AppLockGate.jsx`
- Modify: `ui/src/components/SettingsModal.jsx`
- Modify: `ui/src/App.jsx` (or root layout) to mount `AppLockGate`
- Modify: `client/src-tauri/src/commands.rs` (about/cache already in Task 4)

**Interfaces:**
- Consumes: `store.user`, storage `size` from list/open storage, `nativeInvoke('get_about'|'get_cache_size'|'clear_local_cache'|'get_client_prefs'|'set_client_prefs')`
- Produces: General shows account/server, occupied GB only, cache clear, about; Security enables PIN lock

- [ ] **Step 1: General tab additions (do not change Theme UI)**

Below existing Theme + Session rows, add (native where needed):
- Account: `store.user?.email`; Server: `window.location.origin` (or session `base_url` via `get_session` when native).
- Occupied space: if `chrome.storageId()`, load storage `size` and show `X.XX GB used` only (no quota).
- Cache: show `get_cache_size` humanized + Clear button → `clear_local_cache`.
- About: app version from `get_about` / `import.meta.env` fallback; link/button to copy recent console hint “Logs: use system log for sarca-client”.

- [ ] **Step 2: Security tab**

Nav item Security (shield icon). Body:
- Toggle “App lock”
- When enabling: prompt for 4–8 digit PIN (confirm twice); save via `set_client_prefs`
- When disabling: require current PIN
- Change PIN button

- [ ] **Step 3: `AppLockGate`**

On app start (native): if `app_lock_enabled`, show full-screen PIN unlock before children. Unlock sets session flag `sessionStorage.sarca_unlocked=1` so lock applies once per process/session.

- [ ] **Step 4: Commit**

```bash
git add ui/src/components/AppLockGate.jsx ui/src/components/SettingsModal.jsx ui/src/App.jsx
git commit -m "$(cat <<'EOF'
Add General account/space/cache/about and Security app lock.

EOF
)"
```

---

### Task 7: Retarget deep links / tray / menu away from primary sync.html

**Files:**
- Modify: `client/src-tauri/src/lib.rs`
- Modify: `client/src-tauri/src/state.rs` (`navigate_to_sync_settings`)
- Modify: `ui/src/common/nativeClient.js`

**Interfaces:**
- Consumes: connected `ServerConfig::app_url`
- Produces: opening Sync navigates to server UI with `?__sarca_open_settings=sync` (or dispatches event if already there)

- [ ] **Step 1: Change `navigate_to_sync_settings`**

```rust
pub fn navigate_to_sync_settings(app: &AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "main window missing".to_string())?;
    let state = app.state::<AppSyncState>();
    let cfg = tauri::async_runtime::block_on(state.server.lock()).clone();
    if cfg.is_connected() {
        state.queue_inject(SessionInject::from(&cfg));
        let mut url = cfg.app_url().map_err(|e| e.to_string())?;
        url.query_pairs_mut().append_pair("__sarca_open_settings", "sync");
        window.navigate(url).map_err(|e| e.to_string())
    } else {
        // Not connected: stay on shell; Sync lives in Settings after connect.
        navigate_to_shell(app)
    }
}
```

Keep `sync.html` in the Vite MPA bundle for debugging but do not use as primary entry.

- [ ] **Step 2: `on_navigation` / page-load**

When `__sarca_open_settings=sync` is present on a **remote** URL, do **not** redirect to `sync.html`; allow navigation and let UI open Settings (strip param after handling). Remove the “wants_sync → navigate_to_sync_settings” page-load branch that forced `sync.html`, or gate it so it only runs for legacy `sarca-sync://` while connected → Settings query instead.

For scheme `sarca-sync://`, call the new `navigate_to_sync_settings` (Settings query).

- [ ] **Step 3: Update `openNativeSyncSettings`**

Prefer `settingsStore.openSettings('sync')` when already in app; else set query / dispatch event. Stop preferring navigate-to-`sync.html`.

- [ ] **Step 4: Commit**

```bash
git add client/src-tauri/src/lib.rs client/src-tauri/src/state.rs ui/src/common/nativeClient.js
git commit -m "$(cat <<'EOF'
Open Settings Sync tab from tray, menu, and deep links.

EOF
)"
```

---

### Task 8: Verify builds and push

**Files:** none new

- [ ] **Step 1: UI build**

```bash
cd /home/beta/git/sarca/ui && pnpm exec vite build
```

Expected: success.

- [ ] **Step 2: Client frontend + Rust check**

```bash
cd /home/beta/git/sarca/client && pnpm exec vite build
CARGO_TARGET_DIR=/home/beta/git/sarca/target cargo check --manifest-path /home/beta/git/sarca/client/src-tauri/Cargo.toml
```

Expected: success (or fix compile errors).

- [ ] **Step 3: Spec checklist self-review**

Confirm requirements 1–12 from the design spec each map to shipped code. Note any intentional best-effort gaps (mobile Wi‑Fi OS API, iOS gallery path).

- [ ] **Step 4: Push**

```bash
git pull --rebase origin master
git push origin master
```

---

## Self-review (plan vs spec)

| Spec item | Task |
|-----------|------|
| Remove sidebar / FAB / connect Sync | Task 1 |
| Sync 3rd tab; storage always / system when storage open | Task 5 |
| Hide star; long-press menu no drag; tap opens | Task 2 |
| Local gallery/Pictures → remote `Camera/` | Tasks 3, 5 |
| Folder picker everywhere; Linux async | Task 3 |
| Russian toggles + Wi‑Fi default ON | Tasks 4–5 |
| Folder backup + background + Sync now | Task 5 |
| General account/space/cache/about | Task 6 |
| Security app lock | Tasks 4, 6 |
| Skip theme | Task 6 (explicit) |
| Deep link / tray → Settings not sync.html primary | Task 7 |

No TBDs. `sync.html` retained only as non-primary fallback/debug.
