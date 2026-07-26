# Mobile tap + client sync settings Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** On mobile (≤840px) make one tap open a file/folder while checkbox-only toggles selection; expose a brief Sync tab in website Settings when running in the native client; and ship a full local Sync settings UI (Media auto-upload + folder sync with create-remote-folder) opened via tray and `sarca-sync://open` deep link.

**Architecture:** Keep desktop FSListItem click=select / double-click=open. On mobile viewports, row tap calls `handleNavigate()` and an explicit checkbox (~40×40) with `stopPropagation` owns selection. Native detection uses `localStorage.sarca_native=1` set during session inject. Because the webview is on a remote origin (no `invoke`), the Settings Sync tab CTA uses `location.assign('sarca-sync://open')`; Rust intercepts that scheme via a Tauri plugin `on_navigation` hook and navigates the main window to local `sync.html`. Full Sync UI lives in the client frontend bundle and reuses existing `list_bindings` / `add_binding` / `remove_binding` / `sync_now`, plus new `open_sync_settings`, `pick_local_folder`, `ensure_remote_folder`, and `list_storages`.

**Tech Stack:** SolidJS (website UI), vanilla JS + Vite multi-page (client), Tauri 2 (Rust commands, tray, navigation intercept), `sarca-sync` crate (`BindingMode`, `SarcaApi::create_folder`), `@tauri-apps/plugin-dialog`.

## Global Constraints

- Mobile breakpoint is **max-width: 840px**; tablets wider than 840px keep desktop click semantics.
- Desktop click semantics **unchanged**: click selects (when selectable), double-click opens.
- Long-press context menu on touch **unchanged**.
- Brief Sync tab only when `localStorage.sarca_native === '1'` (and/or equivalent native flag).
- Full Sync UI is **local** `client/sync.html` — not embedded in the remote website.
- Deep link scheme: **`sarca-sync://open`** intercepted in Rust; do not use `shell.open` for this.
- Reuse existing binding commands; do **not** delete iOS job / Photos observer scaffolding if present.
- Match existing code style; minimal diffs; no drive-by refactors.
- `docs/` is gitignored — force-add plan/spec under `docs/superpowers/` when committing docs.
- Commit messages in English, concise; push to `origin/master` after commits so CI runs.
- No new npm dependencies unless unavoidable; prefer existing Fluent icons (`cloud` / `cloudFilled`) for Sync tab.

## File map

| File | Responsibility |
|------|----------------|
| `ui/src/components/FSListItem.jsx` | Mobile open-on-tap; explicit checkbox select-only |
| `ui/src/index.css` | Checkbox hit target (~40×40); mobile select chrome |
| `ui/src/common/settings.js` | Add `'sync'` to `SettingsTab` union |
| `ui/src/components/SettingsModal.jsx` | Brief Sync tab + `sarca-sync://open` CTA |
| `client/src-tauri/src/state.rs` | Inject `sarca_native=1`; `navigate_to_sync_settings` |
| `client/src-tauri/src/commands.rs` | `open_sync_settings`, `pick_local_folder`, `ensure_remote_folder`, `list_storages` |
| `client/src-tauri/src/lib.rs` | Tray “Sync settings”; register commands; `sarca-sync://` navigation intercept |
| `crates/sarca-sync/src/api.rs` | `list_storages` HTTP helper (wrap `GET /api/storages`) |
| `crates/sarca-sync/src/lib.rs` | Re-export storage DTO if needed |
| `client/sync.html` | Full Sync settings page markup |
| `client/src/sync.js` | Media auto-upload + folder sync flows |
| `client/vite.config.js` | Multi-page build for `index.html` + `sync.html` |

---

### Task 1: Mobile open-on-tap + checkbox select in `FSListItem`

**Files:**
- Modify: `ui/src/components/FSListItem.jsx`
- Modify: `ui/src/index.css` (after `.fs-list-item` / `.fs-grid-item` blocks ~1401–1590, and inside `@media (max-width: 840px)` ~2325+)

**Interfaces:**
- Consumes: existing `props.selectable`, `props.selected`, `props.onSelectItem`, `handleNavigate`, long-press handlers
- Produces: `isMobileTapOpen()` helper; checkbox UI with `stopPropagation`; mobile row click → open

- [ ] **Step 1: Add mobile detection + click/checkbox handlers**

In `ui/src/components/FSListItem.jsx`, after `const showSelect = () => …`, add:

```js
	const isMobileTapOpen = () => {
		if (typeof window === 'undefined' || !window.matchMedia) return false
		return window.matchMedia('(max-width: 840px)').matches
	}

	const handleSelectOnly = (event) => {
		event.preventDefault()
		event.stopPropagation()
		if (suppressClickAfterLongPress) {
			suppressClickAfterLongPress = false
			return
		}
		if (!showSelect() || typeof props.onSelectItem !== 'function') return
		props.onSelectItem(props.fsElement, event)
	}
```

Replace `handleItemClick` with:

```js
	const handleItemClick = (event) => {
		if (suppressClickAfterLongPress) {
			suppressClickAfterLongPress = false
			event.preventDefault()
			event.stopPropagation()
			return
		}
		if (isMobileTapOpen()) {
			event.preventDefault()
			event.stopPropagation()
			handleNavigate()
			return
		}
		if (showSelect() && typeof props.onSelectItem === 'function') {
			event.preventDefault()
			event.stopPropagation()
			props.onSelectItem(props.fsElement, event)
			return
		}
		handleNavigate()
	}
```

- [ ] **Step 2: Render explicit checkbox on list + grid items**

Inside the **grid** fallback root (before `FileTypeIcon`), and inside the **list** row (before `FileTypeIcon`), add:

```jsx
						<Show when={showSelect()}>
							<label
								class="fs-item-check"
								onClick={handleSelectOnly}
								onPointerDown={(e) => e.stopPropagation()}
								onTouchStart={(e) => e.stopPropagation()}
							>
								<input
									type="checkbox"
									class="fs-item-check__input"
									checked={isSelected()}
									onChange={handleSelectOnly}
									onClick={handleSelectOnly}
									aria-label={
										isSelected()
											? `Deselect ${displayName()}`
											: `Select ${displayName()}`
									}
								/>
								<span class="fs-item-check__box" aria-hidden="true" />
							</label>
						</Show>
```

Update the JSDoc on `onSelectItem` to note: desktop = row click selects; mobile = checkbox only selects, row tap opens.

- [ ] **Step 3: Add CSS for ~40×40 checkbox hit target**

Append near other `.fs-list-item` rules in `ui/src/index.css`:

```css
.fs-item-check {
	position: relative;
	display: inline-flex;
	align-items: center;
	justify-content: center;
	width: 40px;
	height: 40px;
	flex: 0 0 40px;
	margin: 0;
	cursor: pointer;
	z-index: 2;
}

.fs-item-check__input {
	position: absolute;
	inset: 0;
	opacity: 0;
	margin: 0;
	cursor: pointer;
}

.fs-item-check__box {
	width: 18px;
	height: 18px;
	border-radius: 4px;
	border: 1.5px solid color-mix(in srgb, var(--sarca-ink, #1b1a19) 35%, transparent);
	background: var(--sarca-surface, #fff);
	box-shadow: inset 0 0 0 0 var(--sarca-teal, #0078d4);
	transition: background 0.12s ease, border-color 0.12s ease, box-shadow 0.12s ease;
}

.fs-item-check__input:checked + .fs-item-check__box {
	border-color: var(--sarca-teal, #0078d4);
	background: var(--sarca-teal, #0078d4);
	box-shadow: inset 0 0 0 2px color-mix(in srgb, #fff 85%, transparent);
}

.fs-item-check__input:focus-visible + .fs-item-check__box {
	outline: 2px solid color-mix(in srgb, var(--sarca-teal, #0078d4) 55%, transparent);
	outline-offset: 2px;
}

.fs-grid-item .fs-item-check {
	position: absolute;
	top: 4px;
	left: 4px;
}

.fs-list-item .fs-item-check {
	margin-right: 4px;
}

@media (max-width: 840px) {
	.fs-item-check {
		width: 40px;
		height: 40px;
	}
}
```

Ensure `.fs-grid-item` still has `position: relative` (it already does).

- [ ] **Step 4: Verify build**

```bash
cd /home/beta/git/sarca/ui && pnpm exec vite build
```

Expected: build succeeds with no errors referencing `FSListItem` / `fs-item-check`.

- [ ] **Step 5: Commit**

```bash
git add ui/src/components/FSListItem.jsx ui/src/index.css
git commit -m "$(cat <<'EOF'
feat(ui): open on tap and checkbox-only select on mobile

EOF
)"
git push origin master
```

---

### Task 2: Settings store + brief Sync tab (native-only)

**Files:**
- Modify: `ui/src/common/settings.js`
- Modify: `ui/src/components/SettingsModal.jsx`

**Interfaces:**
- Consumes: `localStorage.sarca_native === '1'`
- Produces: `SettingsTab` includes `'sync'`; Sync tab CTA assigns `sarca-sync://open`

- [ ] **Step 1: Extend tab union**

In `ui/src/common/settings.js`, change the typedef to:

```js
/**
 * Shared open state for the Settings modal / bottom sheet.
 * @typedef {'general' | 'access' | 'trash' | 'storage' | 'sync'} SettingsTab
 */
```

Leave default tab `'general'`.

- [ ] **Step 2: Add native detector + Sync nav item + panel**

Near the top of `SettingsModal` component body (after hooks), add:

```js
	const isNativeClient = () => {
		try {
			return localStorage.getItem('sarca_native') === '1'
		} catch {
			return false
		}
	}

	const openNativeSyncSettings = (event) => {
		event?.preventDefault?.()
		// Cross-origin webview cannot invoke; Rust intercepts this scheme.
		window.location.assign('sarca-sync://open')
	}
```

In the settings nav, after the Storage button, add:

```jsx
								<Show when={isNativeClient()}>
									<button
										type="button"
										class="settings-nav__item"
										classList={{ 'settings-nav__item--active': tab() === 'sync' }}
										onClick={() => setTab('sync')}
									>
										<span class="settings-nav__icon" aria-hidden="true">
											<FluentIcon
												name={tab() === 'sync' ? 'cloudFilled' : 'cloud'}
												size={20}
											/>
										</span>
										<span class="settings-nav__text">
											<span class="settings-nav__title">Sync</span>
											<span class="settings-nav__desc">Auto-upload &amp; folders</span>
										</span>
									</button>
								</Show>
```

In `settings-modal__body`, add a Sync panel (alongside other `Show when={tab() === …}` blocks):

```jsx
								<Show when={tab() === 'sync'}>
									<div class="settings-sync">
										<p class="settings-bot-hint">
											Configure Media auto-upload and folder sync in the Sarca
											app. Bindings run in the background while you are connected.
										</p>
										<ul class="settings-sync__status">
											<li>Media auto-upload and folder sync are managed in the app.</li>
											<li>
												On desktop, you can also open Sync from the tray menu
												(Sync settings).
											</li>
										</ul>
										<Button
											variant="contained"
											color="secondary"
											onClick={openNativeSyncSettings}
										>
											Open Sync settings
										</Button>
									</div>
								</Show>
```

Optionally update the subtitle under the Settings title to mention Sync when native:

```jsx
								<p class="settings-modal__sub">
									{isNativeClient()
										? 'General, access, trash, storage, and sync'
										: 'General, access, trash, and storage'}
								</p>
```

- [ ] **Step 3: Minimal Sync panel CSS**

In `ui/src/index.css` near other settings styles:

```css
.settings-sync {
	display: flex;
	flex-direction: column;
	gap: 16px;
	max-width: 420px;
}

.settings-sync__status {
	margin: 0;
	padding-left: 1.2em;
	color: var(--sarca-ink-soft, #605e5c);
	font-size: 0.92rem;
	line-height: 1.45;
}
```

- [ ] **Step 4: Verify**

```bash
cd /home/beta/git/sarca/ui && pnpm exec vite build
```

Expected: success. Manually: in DevTools set `localStorage.sarca_native='1'`, open Settings → Sync tab visible; clear key → tab hidden.

- [ ] **Step 5: Commit + push**

```bash
git add ui/src/common/settings.js ui/src/components/SettingsModal.jsx ui/src/index.css
git commit -m "$(cat <<'EOF'
feat(ui): add brief Sync tab for native client settings

EOF
)"
git push origin master
```

---

### Task 3: `sarca-sync` — `list_storages` API helper

**Files:**
- Modify: `crates/sarca-sync/src/api.rs`
- Modify: `crates/sarca-sync/src/lib.rs` (export `StorageSummary` if not already public via `api`)

**Interfaces:**
- Consumes: authenticated `SarcaApi` (`base_url`, bearer token)
- Produces: `SarcaApi::list_storages() -> Result<Vec<StorageSummary>>` where `StorageSummary { id: Uuid, name: String }`

- [ ] **Step 1: Add DTO + method**

In `crates/sarca-sync/src/api.rs`, after `LoginResponse`:

```rust
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StorageSummary {
    pub id: Uuid,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
struct StoragesResponse {
    pub storages: Vec<StorageSummary>,
}
```

Add `use serde::Serialize` if only `Deserialize` is imported (api.rs currently has `use serde::Deserialize` — extend to `Serialize, Deserialize`).

Implement on `SarcaApi`:

```rust
    pub async fn list_storages(&self) -> Result<Vec<StorageSummary>> {
        let url = format!("{}/api/storages", self.base_url);
        let resp = self
            .auth(self.client.get(url))
            .send()
            .await?
            .error_for_status()?;
        let body: StoragesResponse = resp.json().await.context("invalid storages response")?;
        Ok(body.storages)
    }
```

`create_folder` already exists and treats HTTP 409 as success — keep that for idempotent Media folder create.

- [ ] **Step 2: Re-export**

In `crates/sarca-sync/src/lib.rs`, ensure `StorageSummary` is public:

```rust
pub use api::{LoginResponse, SarcaApi, StorageSummary, normalize_server_url};
```

(Adjust to match existing `pub use` style — only add `StorageSummary` to the existing export line.)

- [ ] **Step 3: Verify compile**

```bash
cd /home/beta/git/sarca && cargo check -p sarca-sync
```

Expected: finished successfully.

- [ ] **Step 4: Commit + push**

```bash
git add crates/sarca-sync/src/api.rs crates/sarca-sync/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(sarca-sync): add list_storages API helper

EOF
)"
git push origin master
```

---

### Task 4: Tauri commands — open sync, pick folder, ensure remote, list storages

**Files:**
- Modify: `client/src-tauri/src/state.rs`
- Modify: `client/src-tauri/src/commands.rs`
- Modify: `client/src-tauri/src/lib.rs` (register handlers only in this task; tray/deep-link in Task 5)

**Interfaces:**
- Consumes: `AppSyncState`, `SarcaApi::list_storages`, `SarcaApi::create_folder`, dialog plugin
- Produces:
  - `navigate_to_sync_settings(app) -> Result<(), String>`
  - `open_sync_settings(app) -> Result<(), String>`
  - `pick_local_folder(app) -> Result<Option<String>, String>`
  - `ensure_remote_folder(state, storage_id, parent, name) -> Result<String, String>` (returns remote path)
  - `list_storages(state) -> Result<Vec<StorageDto>, String>`

- [ ] **Step 1: Inject `sarca_native=1` + sync navigation helper**

In `SessionInject::eval_script` in `state.rs`, set the native flag alongside tokens:

```rust
    localStorage.setItem('access_token', {access});
    localStorage.setItem('refresh_token', {refresh});
    localStorage.setItem('user', {user});
    localStorage.setItem('sarca_native', '1');
    if (sessionStorage.getItem('__sarca_native_session') !== '1') {{
      sessionStorage.setItem('__sarca_native_session', '1');
      location.replace('/');
    }}
```

Add after `navigate_to_shell`:

```rust
pub fn navigate_to_sync_settings(app: &AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "main window missing".to_string())?;
    let state = app.state::<AppSyncState>();
    let base = state
        .shell_url()
        .unwrap_or_else(|| Url::parse("tauri://localhost").expect("valid shell url"));
    let sync_url = base
        .join("sync.html")
        .map_err(|e| e.to_string())?;
    // Remember shell base if we somehow lost it
    state.remember_shell_url(base);
    window.navigate(sync_url).map_err(|e| e.to_string())
}
```

- [ ] **Step 2: Implement commands**

Append to `commands.rs` (update imports as needed):

```rust
use sarca_sync::StorageSummary;
use tauri_plugin_dialog::{DialogExt, FilePath};

#[derive(Serialize)]
pub struct StorageDto {
    pub id: String,
    pub name: String,
}

#[tauri::command]
pub fn open_sync_settings(app: AppHandle) -> Result<(), String> {
    crate::state::navigate_to_sync_settings(&app)
}

#[tauri::command]
pub async fn pick_local_folder(app: AppHandle) -> Result<Option<String>, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Choose folder")
        .pick_folder(move |path| {
            let _ = tx.send(path);
        });
    let picked = rx.await.map_err(|_| "folder picker cancelled".to_string())?;
    Ok(picked.map(|p: FilePath| p.to_string()))
}

#[tauri::command]
pub async fn list_storages(
    state: State<'_, AppSyncState>,
) -> Result<Vec<StorageDto>, String> {
    let cfg = state.server.lock().await.clone();
    if !cfg.is_connected() {
        return Err("Not connected".into());
    }
    let api = SarcaApi::new(&cfg.base_url, &cfg.access_token);
    let list = api.list_storages().await.map_err(|e| e.to_string())?;
    Ok(list
        .into_iter()
        .map(|s: StorageSummary| StorageDto {
            id: s.id.to_string(),
            name: s.name,
        })
        .collect())
}

#[tauri::command]
pub async fn ensure_remote_folder(
    state: State<'_, AppSyncState>,
    storage_id: String,
    parent: String,
    name: String,
) -> Result<String, String> {
    let cfg = state.server.lock().await.clone();
    if !cfg.is_connected() {
        return Err("Not connected".into());
    }
    let name = name.trim().to_owned();
    if name.is_empty() {
        return Err("Folder name is required".into());
    }
    let sid = uuid::Uuid::parse_str(&storage_id).map_err(|e| e.to_string())?;
    let parent = parent.trim().trim_matches('/').to_owned();
    let api = SarcaApi::new(&cfg.base_url, &cfg.access_token);
    api.create_folder(&sid, &parent, &name)
        .await
        .map_err(|e| e.to_string())?;
    let remote = if parent.is_empty() {
        format!("{name}/")
    } else {
        format!("{parent}/{name}/")
    };
    Ok(remote)
}
```

If `FilePath::to_string` / dialog API differs slightly on this Tauri version, match the pattern already used by any existing dialog usage, or fall back to:

```rust
Ok(picked.and_then(|p| p.into_path().ok()).map(|p| p.to_string_lossy().into_owned()))
```

Register in `lib.rs` `invoke_handler`:

```rust
            commands::open_sync_settings,
            commands::pick_local_folder,
            commands::list_storages,
            commands::ensure_remote_folder,
```

- [ ] **Step 3: Verify**

```bash
cd /home/beta/git/sarca/client/src-tauri && cargo check
```

Expected: success (fix dialog API mismatches if any).

- [ ] **Step 4: Commit + push**

```bash
git add client/src-tauri/src/state.rs client/src-tauri/src/commands.rs client/src-tauri/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(client): add sync settings commands and native flag inject

EOF
)"
git push origin master
```

---

### Task 5: Tray menu + `sarca-sync://open` navigation intercept

**Files:**
- Modify: `client/src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: `navigate_to_sync_settings`, tray `MenuItem`
- Produces: tray id `"sync_settings"`; plugin `on_navigation` denies `sarca-sync://*` and opens sync UI

- [ ] **Step 1: Tray item**

In the desktop tray setup, add a menu item after `sync_now`:

```rust
                let sync_settings = MenuItem::with_id(
                    app,
                    "sync_settings",
                    "Sync settings",
                    true,
                    None::<&str>,
                )?;
                let menu = Menu::with_items(
                    app,
                    &[&show, &sync_now, &sync_settings, &disconnect, &quit],
                )?;
```

In `on_menu_event`, handle:

```rust
                        "sync_settings" => {
                            let _ = state::navigate_to_sync_settings(app);
                        }
```

- [ ] **Step 2: Deep-link navigation intercept**

Before `.setup(...)`, chain a tiny plugin (Tauri 2 `plugin::Builder`):

```rust
    use tauri::plugin::{Builder as PluginBuilder, TauriPlugin};
    use tauri::Runtime;

    fn sarca_nav_plugin<R: Runtime>() -> TauriPlugin<R> {
        PluginBuilder::new("sarca-nav")
            .on_navigation(|webview, url| {
                if url.scheme() == "sarca-sync" {
                    let app = webview.app_handle().clone();
                    let _ = state::navigate_to_sync_settings(&app);
                    return false;
                }
                true
            })
            .build()
    }

    // on the builder:
    builder = builder.plugin(sarca_nav_plugin());
```

Place the helper function above `run()`, and call `.plugin(sarca_nav_plugin())` on the main builder (works for desktop + mobile).

- [ ] **Step 3: Verify**

```bash
cd /home/beta/git/sarca/client/src-tauri && cargo check
```

Expected: success.

- [ ] **Step 4: Commit + push**

```bash
git add client/src-tauri/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(client): tray Sync settings and sarca-sync deep link

EOF
)"
git push origin master
```

---

### Task 6: Full Sync UI page (`sync.html` + `sync.js`) + Vite MPA

**Files:**
- Create: `client/sync.html`
- Create: `client/src/sync.js`
- Modify: `client/vite.config.js`

**Interfaces:**
- Consumes: `invoke('list_bindings'|'add_binding'|'remove_binding'|'sync_now'|'sync_statuses'|'list_storages'|'pick_local_folder'|'ensure_remote_folder'|'open_app'|'get_session')`
- Produces: Media auto-upload flow (`mode=auto_upload`, `remote_root=Media`); folder sync flow (`mode=sync` + create remote folder)

- [ ] **Step 1: Multi-page Vite config**

Update `client/vite.config.js` `build` section:

```js
  build: {
    target: process.env.TAURI_ENV_PLATFORM === "windows" ? "chrome105" : "safari13",
    minify: !process.env.TAURI_ENV_DEBUG ? "esbuild" : false,
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
    outDir: "dist",
    emptyOutDir: true,
    rollupOptions: {
      input: {
        main: path.resolve(__dirname, "index.html"),
        sync: path.resolve(__dirname, "sync.html"),
      },
    },
  },
```

- [ ] **Step 2: Create `client/sync.html`**

Reuse connect-shell CSS variables (copy the `:root` / dark media block from `index.html`). Structure:

```html
<!doctype html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover" />
    <title>Sarca Sync</title>
    <link rel="icon" href="/logo.svg" type="image/svg+xml" />
    <style>/* reuse shell tokens; page layout: max-width 640px card */</style>
  </head>
  <body>
    <main class="sync-page">
      <header class="sync-header">
        <h1>Sync settings</h1>
        <p class="muted">Media auto-upload and folder sync</p>
        <div class="sync-header__actions">
          <button type="button" id="backToApp" class="btn secondary">Back to app</button>
          <button type="button" id="syncNow" class="btn">Sync now</button>
        </div>
      </header>

      <section class="sync-card">
        <h2>Media auto-upload</h2>
        <p class="muted">Photos/videos from a local gallery folder → remote <code>Media/</code>.</p>
        <p id="mediaStatus" class="status">Off</p>
        <button type="button" id="enableMedia" class="btn">Enable / change folder…</button>
        <button type="button" id="disableMedia" class="btn secondary">Disable</button>
      </section>

      <section class="sync-card">
        <h2>Folder sync</h2>
        <p class="muted">Two-way sync between a local folder and a remote folder.</p>
        <label for="storageSelect">Storage</label>
        <select id="storageSelect"></select>
        <label for="localPath">Local folder</label>
        <div class="row">
          <input id="localPath" type="text" readonly placeholder="Choose a folder…" />
          <button type="button" id="pickLocal" class="btn secondary">Browse…</button>
        </div>
        <label for="remoteRoot">Remote folder path</label>
        <input id="remoteRoot" type="text" placeholder="e.g. Projects/Notes or leave empty for root child" />
        <label for="newFolderName">Or create remote folder</label>
        <div class="row">
          <input id="newFolderName" type="text" placeholder="New folder name" />
          <button type="button" id="createRemote" class="btn secondary">Create</button>
        </div>
        <button type="button" id="addSync" class="btn">Add folder sync</button>
      </section>

      <section class="sync-card">
        <h2>Bindings</h2>
        <div id="bindings"></div>
        <pre id="status" class="status-pre"></pre>
      </section>
    </main>
    <script type="module" src="/src/sync.js"></script>
  </body>
</html>
```

Style `.btn`, `.sync-card`, `.row` consistently with the connect shell (accent buttons, soft borders). Keep CSS in the HTML `<style>` block to avoid extra files.

- [ ] **Step 3: Create `client/src/sync.js`**

```js
import { invoke } from "@tauri-apps/api/core";

const $ = (id) => document.getElementById(id);

function setMsg(text) {
  $("status").textContent = text || "";
}

async function refreshStorages() {
  const storages = await invoke("list_storages");
  const sel = $("storageSelect");
  sel.innerHTML = "";
  for (const s of storages) {
    const opt = document.createElement("option");
    opt.value = s.id;
    opt.textContent = s.name;
    sel.appendChild(opt);
  }
  if (!storages.length) {
    setMsg("No storages available. Connect and open the app first.");
  }
}

async function refreshBindings() {
  const bindings = await invoke("list_bindings");
  const host = $("bindings");
  host.innerHTML = "";

  const media = bindings.find((b) => b.mode === "auto_upload");
  $("mediaStatus").textContent = media
    ? `On → ${media.local_path} (remote ${media.remote_root || "Media"})`
    : "Off";

  const syncBindings = bindings.filter((b) => b.mode === "sync");
  if (!bindings.length) {
    host.innerHTML = `<p class="muted">No bindings yet.</p>`;
  } else {
    for (const b of bindings) {
      const row = document.createElement("div");
      row.className = "binding";
      row.innerHTML = `
        <div>
          <strong>${b.mode}</strong>
          <div class="muted">${b.local_path}</div>
          <div class="muted">${b.storage_id} / ${b.remote_root || "(root)"}</div>
        </div>
        <button type="button" data-id="${b.id}" class="btn secondary danger">Remove</button>
      `;
      row.querySelector("button").onclick = async () => {
        await invoke("remove_binding", { id: b.id });
        await refreshBindings();
      };
      host.appendChild(row);
    }
  }

  try {
    const statuses = await invoke("sync_statuses");
    setMsg(JSON.stringify(statuses, null, 2));
  } catch (e) {
    setMsg(String(e));
  }

  return { media, syncBindings };
}

window.addEventListener("DOMContentLoaded", async () => {
  try {
    await refreshStorages();
    await refreshBindings();
  } catch (e) {
    setMsg(String(e));
  }

  $("backToApp").onclick = async () => {
    try {
      await invoke("open_app");
    } catch (e) {
      setMsg(String(e));
    }
  };

  $("syncNow").onclick = async () => {
    try {
      await invoke("sync_now");
      await refreshBindings();
    } catch (e) {
      setMsg(String(e));
    }
  };

  $("pickLocal").onclick = async () => {
    const path = await invoke("pick_local_folder");
    if (path) $("localPath").value = path;
  };

  $("enableMedia").onclick = async () => {
    try {
      const path = await invoke("pick_local_folder");
      if (!path) return;
      const storageId = $("storageSelect").value;
      if (!storageId) throw new Error("Select a storage first");
      // Ensure remote Media/ under storage root
      const remote = await invoke("ensure_remote_folder", {
        storageId,
        parent: "",
        name: "Media",
      });
      // Replace existing auto_upload binding if any
      const bindings = await invoke("list_bindings");
      for (const b of bindings.filter((x) => x.mode === "auto_upload")) {
        await invoke("remove_binding", { id: b.id });
      }
      await invoke("add_binding", {
        storageId,
        remoteRoot: remote.replace(/\/?$/, "/") === "Media/" ? "Media" : remote.replace(/\/$/, ""),
        localPath: path,
        mode: "auto_upload",
      });
      await refreshBindings();
    } catch (e) {
      setMsg(String(e));
    }
  };

  $("disableMedia").onclick = async () => {
    const bindings = await invoke("list_bindings");
    for (const b of bindings.filter((x) => x.mode === "auto_upload")) {
      await invoke("remove_binding", { id: b.id });
    }
    await refreshBindings();
  };

  $("createRemote").onclick = async () => {
    try {
      const storageId = $("storageSelect").value;
      const name = $("newFolderName").value.trim();
      const parent = $("remoteRoot").value.trim().replace(/\/$/, "");
      if (!storageId || !name) throw new Error("Storage and folder name required");
      const remote = await invoke("ensure_remote_folder", {
        storageId,
        parent,
        name,
      });
      $("remoteRoot").value = remote.replace(/\/$/, "");
      $("newFolderName").value = "";
      setMsg(`Created ${remote}`);
    } catch (e) {
      setMsg(String(e));
    }
  };

  $("addSync").onclick = async () => {
    try {
      const storageId = $("storageSelect").value;
      const localPath = $("localPath").value.trim();
      let remoteRoot = $("remoteRoot").value.trim().replace(/\/$/, "");
      if (!storageId) throw new Error("Select a storage");
      if (!localPath) throw new Error("Pick a local folder");
      if (!remoteRoot) throw new Error("Set a remote folder path or create one");
      await invoke("add_binding", {
        storageId,
        remoteRoot,
        localPath,
        mode: "sync",
      });
      $("localPath").value = "";
      await refreshBindings();
    } catch (e) {
      setMsg(String(e));
    }
  };
});
```

Normalize `remoteRoot` for Media to `"Media"` (engine expects path without requiring trailing slash — match existing binding conventions in `sarca-sync`).

- [ ] **Step 4: Build client frontend**

```bash
cd /home/beta/git/sarca/client && pnpm build
```

Expected: `dist/sync.html` and hashed `dist/assets/sync-*.js` exist; `dist/index.html` still present.

- [ ] **Step 5: Commit + push**

```bash
git add client/sync.html client/src/sync.js client/vite.config.js
git commit -m "$(cat <<'EOF'
feat(client): full Sync settings UI for media and folders

EOF
)"
git push origin master
```

---

### Task 7: End-to-end verification + plan checkboxes

**Files:**
- Modify: `docs/superpowers/plans/2026-07-26-mobile-tap-and-sync-settings.md` (mark tasks done as you go; final sweep)

**Interfaces:** none new

- [ ] **Step 1: Compile Rust client + UI**

```bash
cd /home/beta/git/sarca/ui && pnpm exec vite build
cd /home/beta/git/sarca/client && pnpm build
cd /home/beta/git/sarca/client/src-tauri && cargo check
```

Expected: all three succeed.

- [ ] **Step 2: Manual checklist (document results in commit message body if anything fails)**

1. Narrow viewport ≤840px: tap file opens viewer; tap checkbox toggles selection only; long-press opens menu.
2. Desktop ≥841px: click selects; double-click opens (unchanged).
3. With `sarca_native=1`: Settings shows Sync tab; CTA navigates to `sarca-sync://open` (in browser alone this may fail — expected; in Tauri it opens `sync.html`).
4. Tray → Sync settings opens `sync.html`.
5. Media enable: picks folder, creates `Media`, upserts `auto_upload` binding.
6. Folder sync: pick local, create remote folder, add `sync` binding; Remove + Sync now work.
7. Session inject sets `localStorage.sarca_native=1`.

- [ ] **Step 3: Final commit if any polish leftovers; ensure push**

```bash
git status
git push origin master
```

Expected: clean working tree (or only unrelated local files); `master` synced with `origin/master`.

---

## Self-review (spec coverage)

| Spec requirement | Task |
|------------------|------|
| Mobile ≤840px: tap row opens | Task 1 `handleItemClick` → `handleNavigate` |
| Tap checkbox selects only | Task 1 `fs-item-check` + `stopPropagation` |
| Long-press menu unchanged | Task 1 keeps existing touch timers |
| Desktop click/dblclick unchanged | Task 1 branches on `isMobileTapOpen()` |
| Brief Sync tab when native (`sarca_native=1`) | Task 2 |
| CTA via `sarca-sync://open` deep link | Task 2 + Task 5 intercept |
| Full Sync UI local `sync.html` | Task 6 |
| Media auto-upload → remote `Media/` | Task 6 `enableMedia` + Task 4 `ensure_remote_folder` |
| Folder sync + create remote folder | Task 6 |
| Tray Sync settings | Task 5 |
| Inject `sarca_native=1` | Task 4 |
| `open_sync_settings` / `pick_local_folder` / `ensure_remote_folder` / `list_storages` | Task 4–5 |
| Reuse list/add/remove/sync_now | Task 6 |
| Do not delete iOS job code | No task touches iOS Photos observers |

**Placeholder scan:** no TBD / TODO / “implement later” left in tasks.

**Type consistency:**
- Binding modes: `"auto_upload"` | `"sync"` (matches `parse_mode` / serde `snake_case`).
- Deep link: `sarca-sync://open` only.
- Storage DTO: `{ id: string, name: string }` from `list_storages`.
- `ensure_remote_folder` args: `{ storageId, parent, name }` → remote path string.
- Native flag key: `localStorage.sarca_native = '1'`.
