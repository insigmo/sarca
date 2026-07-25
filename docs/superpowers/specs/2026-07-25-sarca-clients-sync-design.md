# Sarca cross-platform clients + file sync

**Date:** 2026-07-25  
**Status:** Approved  
**Scope:** Native clients (Win/Linux/macOS-arm, iOS, Android), shared sync engine, auto-upload, server changelog. Virtual drive is Stage 2.

## Goals

1. One codebase for UI (SolidJS) and sync logic (Rust) across desktop and mobile.
2. Bidirectional folder sync and one-way auto-upload (Seafile-like, file-level).
3. Lightweight binaries (Tauri 2, not Electron).
4. Stable sync over existing Telegram-backed storage (speed secondary).
5. Conflict resolution via user prompt (keep local / keep remote / keep both).

## Non-goals (MVP)

- Selective sync
- App Store / Play / Microsoft Store (own artifacts first)
- Guaranteed mobile background sync equal to desktop
- Virtual / network drive (Stage 2)

## Stack

| Layer | Choice |
| --- | --- |
| UI | Existing SolidJS app in `ui/` (desktop: full; mobile: simplified chrome) |
| Shell | Tauri 2 (`client/`) |
| Sync engine | Rust crate `crates/sarca-sync` |
| Local index | SQLite per device |
| Transport | Existing JWT + multipart upload / download HTTP APIs |
| Server delta | `GET /api/storages/:id/sync/changelog` + optional `content_hash` on files |

## Sync model

### Binding

```text
(storage_id, remote_root, local_path, mode)
mode = Sync | AutoUpload
```

Multiple bindings per device. Entire storage (or a remote root path) maps to one local folder; no selective-sync UI in MVP.

### Modes

- **Sync:** upload local creates/updates; download remote creates/updates; propagate deletes according to last-writer + conflict rules; prompt on conflict.
- **AutoUpload:** watch local paths (folders + camera/gallery on mobile); upload new/changed files only; never delete or overwrite local from remote.

### Conflict

When both sides changed since last synced revision (hash/mtime disagree):

1. Pause that path.
2. Ask user: keep local | keep remote | keep both (`name (conflict).<ext>`).
3. Resume.

### Stability mechanisms

- Local SQLite index: path, size, mtime, content_hash, remote_file_id, last_synced_revision.
- Content hash (SHA-256) computed on client; stored on server when provided at upload.
- Server change log with monotonic cursor (bigint) so clients avoid full tree scans.
- Idempotent upload: same path + hash → no re-upload.
- Desktop: system tray + OS autostart; sync runs with window closed.
- Mobile: foreground + best-effort OS background tasks.

## Server API additions

### `content_hash` on `files`

Nullable `TEXT`. Set from multipart field `content_hash` on upload when provided. Exposed on file info and changelog entries.

### `GET /api/storages/:storage_id/sync/changelog`

Query:

- `cursor` — last seen event id (omit or `0` for start)
- `limit` — default 500, max 2000

Response:

```json
{
  "events": [
    {
      "id": 42,
      "op": "upsert",
      "path": "docs/a.txt",
      "file_id": "...",
      "size": 123,
      "is_file": true,
      "content_hash": "sha256:...",
      "source_mtime": "2026-07-25T12:00:00Z",
      "updated_at": "2026-07-25T12:00:01Z"
    },
    {
      "id": 43,
      "op": "delete",
      "path": "docs/old.txt",
      "file_id": null,
      "size": null,
      "is_file": true,
      "content_hash": null,
      "source_mtime": null,
      "updated_at": "2026-07-25T12:01:00Z"
    }
  ],
  "next_cursor": 43,
  "has_more": false
}
```

Events are appended on create/update/rename/move/copy/soft-delete/hard-purge. Rename/move emit `delete` (old path) + `upsert` (new path) or a single `upsert` with path change recorded as delete+upsert for simpler clients.

### Auth

Same Bearer JWT as other storage routes; requires read access. Upload/delete still need write.

## Repository layout

```text
crates/sarca-sync/     # sync engine library
client/                # Tauri 2 app (desktop + mobile targets)
  src-tauri/
  # frontend: reuses ../ui via Vite root / alias
docs/superpowers/specs/2026-07-25-sarca-clients-sync-design.md
docs/superpowers/specs/2026-07-25-sarca-virtual-drive-design.md  # Stage 2
```

## Client UI

- **Desktop:** full existing web UI + Sync settings (bindings list, conflict modal, tray status).
- **Mobile:** simplified shell (reuse mobile chrome); Sync/AutoUpload settings; camera/folder pickers for auto-upload.

## Platform matrix

| Target | Arch |
| --- | --- |
| Windows | amd64, arm64 |
| Linux | amd64, arm64 |
| macOS | aarch64 only |
| Android | arm64 (primary) |
| iOS | arm64 |

## Stage 2 — virtual drive

Separate design: Win ProjFS/CSFME or network drive; Linux/macOS FUSE/FSKit. Depends on stable folder sync. See `2026-07-25-sarca-virtual-drive-design.md`.

## Success criteria

- Bind a storage to a local folder; create/edit/delete on either side converges without data loss.
- Auto-upload folder/camera uploads new files reliably with retry.
- Conflicts never silently overwrite; user chooses.
- Desktop sync continues in tray.
- Release artifacts build for listed desktop targets; mobile project scaffolds build where toolchain exists.
