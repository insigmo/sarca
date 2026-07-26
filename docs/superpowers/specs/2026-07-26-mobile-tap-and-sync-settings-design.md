# Mobile tap + client sync settings

**Date:** 2026-07-26  
**Status:** Approved (user chose approach **C** for settings placement)

## Goals

1. On mobile viewport (≤840px) and mobile native clients (same web UI), **one tap opens** a file/folder; **tap on the checkbox hit-area only toggles selection**.
2. Native clients expose **two auto-upload / sync modes** in settings:
   - **Media auto-upload** — photos/videos from a gallery (or photos) folder → remote `Media/` folder (`auto_upload`).
   - **Folder sync** — pick local folder, then pick or **create** remote folder → two-way `sync` binding.
3. Settings UI placement (**C**):
   - **Brief** Sync section inside the website Settings modal, visible only when running inside the Tauri client.
   - **Full** management UI in the native client (opened from that section and/or tray).

## Non-goals (this iteration)

- iOS Photos / Android MediaStore observers (folder pick + walk is enough).
- Virtual drive.
- Changing desktop click semantics (click = select, double-click = open).

## Current behavior

- `FSListItem`: when `selectable` + `onSelectItem`, **single click always selects**; open is via double-click / Enter / context menu.
- Long-press already opens the context menu on touch.
- `sarca-sync` already has `BindingMode::AutoUpload` and `BindingMode::Sync`, plus Tauri commands `list_bindings` / `add_binding` / `remove_binding` / `sync_now`.
- Website `SettingsModal` has General / Access / Trash / Storage — no Sync tab.
- Client connect shell navigates to the server web UI after login; no Sync UI in the webview yet.

## Design

### 1. Mobile open vs select (`ui/src/components/FSListItem.jsx` + CSS)

Detection: `window.matchMedia('(max-width: 840px)')` (same breakpoint as existing mobile chrome). Also treat as mobile when `navigator.maxTouchPoints > 0` **and** width ≤840 so tablets in landscape stay desktop-like if wide.

**Mobile:**

| Gesture | Result |
| --- | --- |
| Tap main row/tile body | Open (file → `onOpen` / viewer; folder → navigate) |
| Tap checkbox hit-area | Toggle selection only (`onSelectItem` without treating as open) |
| Long-press | Context menu (unchanged) |

**Desktop:** unchanged — click selects (when selectable), double-click opens.

Implementation notes:

- Render an explicit checkbox control with `stopPropagation` on pointer events so the row open handler does not fire.
- Enlarge the checkbox hit target (~40×40 CSS px) for touch.
- Row `click` on mobile calls `handleNavigate()` instead of `onSelectItem`.

### 2. Website Settings — brief Sync tab (`SettingsModal`)

- New tab id: `sync` (only rendered when `isNativeClient()` is true).
- Detect native client: `Boolean(window.__TAURI_INTERNALS__ || window.__TAURI__)`.
- Brief UI:
  - Status lines: Media auto-upload on/off; count of folder sync bindings (via `invoke('list_bindings')` / `sync_statuses`).
  - Primary button **Manage sync…** → `invoke('open_sync_settings')` (new command) which shows the full native Sync UI / navigates the shell to a local sync page.
- If not native: tab hidden (browser users unaffected).

### 3. Native client — full Sync UI

Two surfaces (either is fine; prefer one HTML page in the client bundle for reliability when the webview is on the remote origin):

- **Preferred:** local Tauri page `sync.html` (or route in the connect shell) that the main webview can navigate to via `navigate_to_shell` + hash/query, **or** a second labeled window. Simplest MVP: **overlay page** served from the client frontend (`client/sync.html`) opened by navigating the webview back to the app origin (`tauri://localhost/sync.html`) while keeping session credentials for API folder create.
- Commands (extend existing):
  - `list_bindings`, `add_binding`, `remove_binding`, `sync_now`, `sync_statuses` (exist).
  - `open_sync_settings` — navigate main window to sync UI.
  - `pick_local_folder` — dialog plugin (exists pattern).
  - `ensure_remote_folder(storage_id, parent, name)` — create via Sarca API (wrap `create_folder`).
  - `list_storages` / reuse token + `GET /api/storages` from sync UI with stored session.

**3.1 Media auto-upload flow**

1. User enables Media auto-upload.
2. Pick local gallery/photos folder (system folder picker).
3. Ensure remote folder `Media` under storage root (create if missing).
4. Upsert binding: `mode=auto_upload`, `remote_root=Media`, `local_path=<picked>`.

**3.2 Folder sync flow**

1. Pick local folder.
2. Choose storage (default: first / current if known).
3. Pick existing remote folder **or** create new (name prompt + `create_folder`).
4. Upsert binding: `mode=sync`, `remote_root=<path>`, `local_path=<picked>`.

List UI: show each binding with mode, paths, remove, Sync now.

### 4. IPC / security

- Sync manage commands only needed in the native shell; when the webview is on a remote origin, `invoke` is unavailable — **Manage sync…** must either:
  - switch webview to local `sync.html` (recommended), or
  - use a custom scheme / event bridge.
- Recommendation: **navigate to local sync page** for full UI; brief status in Settings can use `invoke` only while still on local pages, or skip live status and only show the Manage button that navigates locally.

Refined **C** for MVP:

- In remote web UI Settings: Sync tab with copy + **Open sync settings** that calls a deep link / `shell.open` is wrong; use Tauri `emit` won't work cross-origin.
- Practical approach: inject a small floating “Sync” entry only in native builds by having the connect flow set `localStorage.sarca_native=1`, and Sync tab button triggers `location` change is impossible for invoke…

**Best MVP for C with remote webview:**

1. After login, keep a **tray / overflow menu** item “Sync settings” (desktop) that navigates to local sync UI.
2. In website Settings, Sync tab visible when `localStorage.sarca_native === '1'` (set during session inject). Button text: “Open in app…” that uses `window.location = 'sarca://sync'` **or** document that users open Sync from the app menu — **better:** use Tauri’s `onCustomProtocol` / inject a JS bridge:

During session inject, also inject:

```js
window.__sarcaNative = {
  openSyncSettings: () => window.__TAURI__?.core.invoke('open_sync_settings')
};
```

But `__TAURI__` is **not** available on remote origins.

So the brief web Settings Sync tab cannot `invoke` while showing the remote site. Options:

1. Sync tab only says “Use **Sync settings** from the app menu / tray” (brief) + full UI in local client pages — still satisfies **C** (brief on site + full in client).
2. Or open a second Tauri window with local sync UI from tray only; Settings tab is informational.

**Decision (MVP):**  
- Website Sync tab (when `localStorage.sarca_native=1`): short explanation + list is **not** live; CTA “Open Sync settings” is disabled with helper text pointing to tray/menu **or** we add a visible in-app header chip injected…  

Simplest clean approach that still matches C:

- Set `sarca_native=1` on inject.
- Sync tab in Settings: status text + button that copies nothing; instead **desktop tray already has Sync**; for **mobile**, add a Settings row that cannot invoke — navigate user to disconnect? Bad.

**Mobile native Sync access:** add Sync entry to the **Files sidebar** footer when `sarca_native=1`, which does `window.open` won't work.

**Required:** a way to call native from remote webview. Tauri 2 supports [capabilities for remote domains](https://v2.tauri.app/security/capabilities/) via `remote` ACL — enable IPC for the user’s server origin dynamically is hard.

**Pragmatic MVP approved path:**

1. Full Sync UI as **local** `client/sync.html` (and `pnpm` page).
2. Desktop: tray menu **Sync settings** → navigate to local sync page.
3. Mobile: after connect, keep a **native** path — e.g. always show Sync as a **second bottom-nav / button in the local chrome** isn’t available when remote…

Actually re-read C: "кратко в сайте + полный UI в клиенте". Brief on site can be **documentation-only** in Settings when `sarca_native=1`, and full UI opened via:

- Desktop tray
- On Android: add Sync to the **connect shell** and also expose via **Android back** / app bar — when user opens Settings Sync tab, show button “Open Sync settings” that uses **custom URL scheme** handled by the Rust side: inject link `iframe.src = 'sarca-sync://open'` with `on_page_load` / navigation handler intercepting `sarca-sync://`.

**Implement:** register navigation handler; if URL starts with `sarca-sync://open`, prevent navigation and `open_sync_settings` (show local sync page). Settings Sync tab button = `<a href="sarca-sync://open">` or `location.assign('sarca-sync://open')`.

This works from remote WebView.

## File touch list

| Area | Files |
| --- | --- |
| Mobile tap | `ui/src/components/FSListItem.jsx`, `ui/src/index.css` |
| Settings brief | `ui/src/components/SettingsModal.jsx`, `ui/src/common/settings.js`, CSS |
| Inject flag + deep link | `client/src-tauri/src/state.rs` (inject script), `client/src-tauri/src/lib.rs` (navigation intercept) |
| Sync UI | `client/sync.html`, `client/src/sync.js`, styles |
| Commands | `client/src-tauri/src/commands.rs`, `crates/sarca-sync/src/api.rs` (`create_folder` already exists) |
| Tray | `client/src-tauri/src/lib.rs` menu item |

## Success criteria

- On Pixel / narrow viewport: tap file opens viewer; tap checkbox toggles selection only.
- Desktop behavior unchanged.
- In native client Settings: Sync tab visible with brief copy + deep link to full Sync UI.
- Full Sync UI can enable Media auto-upload and folder sync with create-remote-folder during setup.
- Bindings persist and `sync_now` / background loop use them.
- Changes committed and pushed to `master`.

## Out of scope follow-ups

- Per-storage Media folders beyond default storage.
- Conflict UI beyond existing KeepBoth.
- Background gallery observers.
