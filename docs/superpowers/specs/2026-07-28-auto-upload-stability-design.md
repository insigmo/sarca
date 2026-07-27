# Auto-upload stability, fair ticks, settings switches, and tests

**Date:** 2026-07-28  
**Status:** Approved for planning  
**Context:** Linux Tauri client + SolidJS Settings (Sync / Security). Auto-upload today deletes bindings on toggle (wipes SQLite index), holds a global `tick_lock` across all bindings, and uses styled native checkboxes that read poorly as on/off controls. No UI test harness exists yet.

## Goals

1. Make Camera and folder auto-upload **stable across enable/disable** (no index wipe on toggle).
2. Stop a long Camera pass from **starving folder uploads** (fair / per-binding scheduling).
3. When any auto-upload is enabled, **turn on `background_sync`** automatically.
4. Replace client-settings checkboxes with an explicit **switch** component (Sync + Security).
5. Add a **large** regression suite: Vitest UI tests (mocked native bridge) + Rust engine/command tests.

## Scope

**In:**

- `crates/sarca-sync` — binding enable flag usage, tick scheduling, index preservation
- `client/src-tauri` — IPC `set_binding_enabled`, prefs auto-enable, ACL/allowlists, background loop
- `ui/` — `SettingsSyncPanel`, `SettingsModal` (app lock), new `SettingsSwitch`, Vitest harness
- CI hooks for `pnpm test` (UI) and existing cargo test jobs

**Out:**

- Real Tauri WebDriver / full desktop GUI automation
- Files/share/other non-settings checkboxes
- Reintroducing filesystem `notify` watchers (keep poll + WalkDir)
- Redesign of Sync tab layout beyond switches + folder enable toggles
- Changing server APIs (bindings stay client-local)

## Decisions (from brainstorming)

| Topic | Choice |
|-------|--------|
| Failure focus | Full lifecycle: bindings + background sync |
| Tests | Vitest + Testing Library **and** Rust integration/unit |
| Disable semantics | Soft-disable (`enabled=false`), keep index + path + id |
| Switches | All **client settings** checkboxes (Sync + Security), not whole app |
| Background sync | Auto-enable `background_sync` when enabling Camera or adding a folder |
| Scheduling | Soft-disable **plus** fair / per-binding ticks (not soft-disable alone) |

---

## 1. Binding lifecycle

### Soft-disable

- Camera toggle **Off** → `set_binding_enabled({ id, enabled: false })`, **not** `remove_binding`.
- Camera toggle **On** when a disabled (or enabled) `auto_upload` row already exists → `set_binding_enabled(true)` (same id). Do **not** remove+add.
- Camera **On** when no `auto_upload` row exists → `add_binding` as today (`enabled: true`), ensure remote `Camera/`, then kick sync.
- Explicit **Remove** (folder rows, or a future “remove Camera binding”) still calls `remove_binding` and clears index entries for that id.

### UI truth

- Camera switch is on iff there is an `auto_upload` binding with `enabled === true`.
- A disabled Camera binding still exists in SQLite; UI may keep local path from that row so re-enable needs no re-pick.
- Folder rows: each `folder_upload` (and legacy `sync` if listed) gets an enable **switch** + existing Remove.

### Change local folder

- If an `auto_upload` binding exists (enabled or not): **upsert** same id with new `local_path` (via existing `upsert_binding` / new thin command if needed), then kick sync.
- Must **not** take the early-return path that only refreshes UI when “already enabled”.

### Background sync coupling

- On successful Camera enable **or** successful add folder binding: write prefs with `background_sync: true` (merge with current prefs) via `set_client_prefs`.
- User can still turn Background backup off afterward; we do not force it on every tick—only on those enable/add actions.

### IPC / engine API

- Add `SyncEngine` / index helper: `set_binding_enabled(id, enabled)` (UPDATE `enabled` column; no entry deletes).
- Expose Tauri command + remote IPC allowlist entry `set_binding_enabled`.
- `list_bindings` already returns `enabled`; UI and status filters must respect it.
- `tick` / `tick_filtered` already skip `!enabled`; keep that invariant and cover it in tests.

---

## 2. Fair scheduling

### Problem

One global `tick_lock` wraps **all** bindings in a single tick. After recreate (and even on cold gallery), a long Camera walk holds the lock so folder bindings do not start until Camera finishes—looks like “folders don’t upload”.

### Design

1. **Per-binding concurrency control**  
   - Do not hold one mutex across the entire multi-binding pass.  
   - Allow at most one in-flight tick **per binding id**; different bindings may run concurrently (bounded, e.g. small semaphore if needed to avoid hammering disk/network).  
   - `sync_now` may accept optional `binding_id` to kick only that binding without waiting on others.

2. **Background loop**  
   - Still gated by `prefs.background_sync`.  
   - Each wake: list enabled bindings, schedule/run them under per-binding rules (round-robin or parallel-with-cap), so folder_upload is not serialized behind a full Camera rescan.  
   - Preserve upload-only before legacy two-way when ordering still matters for shared resources; fairness between `auto_upload` and `folder_upload` is mandatory.

3. **Statuses**  
   - Continue publishing per-binding `SyncStatus` as each binding progresses so the UI is not blank during long runs.

### Non-goals for this change

- Exact realtime FS events  
- Guaranteeing Camera always finishes before folders (opposite: folders must not starve)

---

## 3. Settings switches

### Component

- New `ui/src/components/SettingsSwitch.jsx`:
  - Visual track + thumb (not native checkbox appearance)
  - `role="switch"`, `aria-checked`, keyboard activation
  - Props: `checked`, `disabled`, `onChange` / `onToggle`, optional `label` / children layout compatible with `.settings-toggle` row

### Call sites (client settings only)

- `SettingsSyncPanel.jsx` — Camera, Wi‑Fi only (mobile), Background backup; folder row enable switches
- `SettingsModal.jsx` — App lock

### CSS

- Styles under `.settings-switch` (track/thumb/checked/disabled/focus-visible)
- Retire or stop relying on `.settings-toggle input[type='checkbox']` for these screens once migrated

---

## 4. Testing

### UI (Vitest)

- Add Vitest + Solid testing library + jsdom to `ui/`
- Scripts: `test` / `test:watch` in `ui/package.json`
- Mock `nativeInvoke`, `pickLocalFolder`, platform helpers

**Required cases (minimum):**

1. Camera off→on with no binding → `add_binding` (+ ensure folder + prefs `background_sync: true`)
2. Camera on→off → `set_binding_enabled(false)`, **never** `remove_binding`
3. Camera off→on with existing disabled binding → `set_binding_enabled(true)`, same id, no remove/add
4. Second on while already enabled → no remove/add churn
5. Change folder with existing binding → path update / upsert, not silent early-return
6. Add folder → `add_binding` + `background_sync: true`
7. Folder row toggle → `set_binding_enabled`
8. `SettingsSwitch` a11y: role, aria-checked, click toggles
9. App lock switch path in SettingsModal (mocked prefs)

### Rust

- `sarca-sync`: disable does not delete entries; tick skips disabled; two bindings — Camera long work does not prevent folder binding from starting (fairness assertion with controlled sync hooks or short-circuit test doubles if needed)
- `sarca-client`: `set_binding_enabled` registered + ACL/IPC allowlist; prefs helper for auto-enable background; keep desktop `wifi_only` non-blocking regression

### CI

- Wire UI unit tests into the existing client/UI workflow
- Keep `cargo test -p sarca-sync` and `cargo test -p sarca-client --lib`

---

## 5. Rollout / compatibility

- Existing DBs already have `enabled INTEGER`; no migration beyond using the column from UI/IPC
- Users who previously toggled Camera off have **no** row (removed). First On after upgrade still `add_binding` (cold index). Subsequent toggles use soft-disable.
- Legacy `client/src/sync.js` page: align with soft-disable if still shipped; otherwise leave unused and document

## Success criteria

- Toggle Camera off/on repeatedly without losing folder upload progress or forcing full gallery rehash
- With Camera + folders enabled and `background_sync` on, folder uploads make progress even while Camera is scanning a large tree
- Enabling Camera or adding a folder turns Background backup on
- Client settings show switches (`role="switch"`), not bare checkboxes
- New Vitest + Rust tests fail if soft-disable or fairness regresses

## Open implementation notes (for the plan, not unresolved product questions)

- Prefer extending `upsert_binding` for path changes over a second SQLite API if one write path is enough
- Fairness may use `tokio::sync::Mutex` keyed by binding id + optional global concurrency cap (e.g. 2)
- Extract pure helpers from `SettingsSyncPanel` where it simplifies Vitest (optional, not required if component tests with mocks are sufficient)
