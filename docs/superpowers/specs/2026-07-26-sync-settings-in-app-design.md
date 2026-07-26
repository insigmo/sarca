# Sync / settings redesign (in-app Settings)

**Date:** 2026-07-26  
**Status:** Approved  
**Supersedes (placement & destinations):** parts of `2026-07-26-mobile-tap-and-sync-settings-design.md` that put Sync in a separate `sync.html` window and used remote `Media/` as the auto-upload destination.

## Goals

1. Put **all Sync UX inside Settings** (storage settings and, when a storage is open, system Settings). No separate Sync window as the primary UI.
2. Match **Seafile sync-related** features (camera/photo+video auto-upload, Wi‑Fi only, folder backup/sync, background backup/sync, sync now) on mobile and desktop native clients.
3. Route **non-sync Seafile settings** into General / Security (skip theme — already exists).
4. Fix mobile file gestures (tap / long-press) and folder-picker reliability (all platforms, including Linux hang).

## Non-goals

- Changing existing theme / night-mode UI (already configured elsewhere).
- Implementing a dedicated Sync page/`sync.html` as the main entry (keep Tauri commands + `sarca-sync` engine; wire them from Settings).
- Selective sync, virtual drive, or store distribution (unchanged from earlier client specs).
- Product code in this commit — this document is design-only.

## Approved placement (C + approach 2)

| Decision | Detail |
| --- | --- |
| Primary UI | **Everything in Settings** — no separate `sync.html` window as the main Sync UI |
| Tab order | **Sync is the 3rd settings tab** when the Sync tab is allowed |
| Storage settings | Sync tab **always** available (when native client) |
| System Settings | Sync tab **only when a storage is currently open** |
| Native-only | Sync tab content that needs client capabilities (picker, Wi‑Fi, background, bindings) is **native client only** |
| Remove entry points | Sync from **sidebar**, **FAB bottom-right**, and **connect-shell Sync button** |

Keep existing Tauri commands and the sync engine; Settings invokes them. `sync.html` (if still present in the bundle) is not the primary user-facing entry.

### Tab order when Sync is shown

Example for system Settings with an open storage (native):

1. General  
2. Access  
3. **Sync**  
4. … (existing tabs: Trash, Storage, etc.)  
5. **Security** (new — see below)

Storage settings use the same rule: Sync is third among its tabs when present.

## Auto-upload paths

| Side | Path / meaning |
| --- | --- |
| **Local source** | Smartphone **gallery / photos**; on desktop, **Pictures** folder (user can change via folder picker) |
| **Remote destination** | **`Camera/`** at the **storage root** — create if missing |
| **Not used** | Remote folders named `Media/` or `Photo/` for this auto-upload flow |

“Media” in earlier conversation means **photos and videos from the device gallery / Pictures**, not a remote folder named `Media`.

## Sync tab features (Seafile parity — sync-related only)

All of the following live on the **Sync** settings tab (same tab for mobile and desktop clients; platform-specific controls appear only where relevant).

1. **Toggle — enable photo + video auto-upload**  
   - Label (as requested): «Включить автозагрузку фото и видео»  
   - If the rest of Settings uses English (or another locale), either keep this Russian string as specified or provide an equivalent localized string; the approved meaning is “turn on photo and video auto-upload.”
2. **Mobile only, when auto-upload is ON:** second toggle «Загружать только через WIFI», **default ON**.
3. **Change local source** via a working folder picker (Android + Linux + other platforms). Linux must use an **async, non-blocking** dialog so the app does not freeze.
4. **Destination** fixed for auto-upload: remote **`Camera/`** (ensure/create under storage root).
5. **Folder backup / folder sync** (two-way bindings and Seafile-like folder backup controls wired to `sarca-sync`).
6. **Background backup / sync** (desktop required; mobile best-effort per OS, as in the client sync design).
7. **Sync now** on the same Sync tab (not a separate window).

## Non-sync Seafile items → other tabs

| Seafile-style item | Sarca destination |
| --- | --- |
| Night mode / theme | **Skip** — already configured elsewhere |
| Account / server / space | **General** — space shows **occupied GB only** (storage is unlimited; do not show a quota cap) |
| Cache size / clear cache | **General** |
| App lock / security | New **Security** tab |
| About / version / logs | **General** |
| Camera upload, Wi‑Fi only, library=`Camera/`, folder backup, background backup | **Sync** |

## Mobile file UX

Applies to mobile viewport (≤840px) and mobile native clients using the same web UI.

| Gesture / UI | Behavior |
| --- | --- |
| Single tap on file | Open **preview** |
| Single tap on folder | Navigate into folder |
| Long-press | Open **context menu**; must **not** start a drag |
| Favorite star on tiles | **Hidden** on mobile |
| Favorite action | Only via **context menu** |
| Checkbox vs star | Fix overlap (star removed from tiles removes the conflict; checkbox hit-area remains usable) |

Desktop click semantics stay as today (select / double-click open) unless already changed by prior mobile-tap work.

## Folder picker

- Must work on **every** native client platform (Android, Linux, Windows, macOS; iOS when present).
- Error “Folder picker is unavailable” is a defect — fix platform bindings / dialog APIs.
- **Linux:** choosing a folder for auto-upload must open the system folder dialog **without freezing** the app (async / non-blocking; do not block the UI thread with a synchronous pick).

## Removals vs keep

**Remove as primary UX:**

- Dedicated Sync page / `sync.html` as the main entry  
- Sync in Files sidebar  
- Floating Sync FAB (bottom-right)  
- Connect-shell “Sync settings” button after connect  

**Keep:**

- Tauri sync commands and `sarca-sync` engine  
- Wiring those commands from the in-app Settings Sync tab  

## Requirements checklist (user bug list 1–12)

Use this as the acceptance checklist for implementation.

| # | Requirement | Status in this design |
| --- | --- | --- |
| 1 | Remove Sync from sidebar and FAB (bottom-right). Sync is the **3rd** Settings tab when allowed. | Placement C + approach 2 |
| 2 | Checkbox and star overlap on mobile: **hide star** on tiles; favorite only in context menu; long-press opens menu **without dragging**. | Mobile file UX |
| 3 | Mobile: one tap opens preview; long-press opens context menu. | Mobile file UX |
| 4 | “Media” = gallery photos/videos / desktop Pictures as **local source**, not a remote folder named `Media`. Remote auto-upload target is **`Camera/`**. | Auto-upload paths |
| 5 | Folder picker must work everywhere (no “unavailable” on device). | Folder picker |
| 6 | “Back to app” as a **back arrow** in the normal Settings chrome (no separate Sync window exit chrome). | Placement approach 2 |
| 7 | Remove Sync settings button from connect/login shell after connect. | Removals |
| 8 | Sync settings only in **storage settings**, or in **system Settings when a storage is open**. | Placement C |
| 9 | Replace Enable/Disable buttons with label + toggle: «Включить автозагрузку фото и видео». | Sync tab features |
| 10 | Mobile client only: when auto-upload ON, show «Загружать только через WIFI», default ON. | Sync tab features |
| 11 | Support Seafile sync-related features from screenshots on mobile **and** desktop; non-sync Seafile items go to General / Security as mapped. | Sync tab + non-sync mapping |
| 12 | Linux folder picker must not hang; open explorer/dialog asynchronously. | Folder picker |

## Self-review

- No open TBDs: destinations, placement, tab mapping, and gestures are fixed.
- Remote auto-upload folder is **`Camera/`** only — not `Media/`, not `Photo/`.
- This spec deliberately supersedes the earlier brief Sync tab + `sync.html` + `Media/` approach for primary UX and destination naming.
- Theme is explicitly out of scope for this redesign.
- Space in General shows occupied size only (unlimited storage).
