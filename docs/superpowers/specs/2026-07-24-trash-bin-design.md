# Trash Bin Design

## Goal

Deleted files move to a per-storage trash instead of being removed immediately. Users can restore them, permanently delete them (including Telegram messages), or empty the trash. A global retention setting (default 30 days, max 30) auto-purges expired items.

## Decisions

| Topic | Choice |
|-------|--------|
| Soft-delete model | `files.deleted_at` (NULL = live) |
| Restore | Yes |
| Retention scope | Global app setting |
| Retention range | 1–30 days, default 30 |
| Trash UI | Button on Files page → in-page trash mode for that storage |
| Restore conflict | Dialog: Replace / Rename / Cancel |
| Folder delete | Folder is a container in trash; restore whole folder or individual files |
| Telegram cleanup | Only on permanent purge (single item, empty trash, or retention expiry) |

## Data model

### `files.deleted_at`

- `TIMESTAMPTZ NULL`
- Live listings (`list_dir`, `search`, download, upload path checks) use `deleted_at IS NULL`
- Soft-delete sets `deleted_at = now()` on the file/folder marker and all path-prefix descendants (same matching rules as today’s folder delete)
- Parent folder markers for the live tree are recreated when needed (same as current delete)

### Unique path constraint

Replace `UNIQUE (path, storage_id)` with a partial unique index:

```sql
CREATE UNIQUE INDEX files_path_storage_id_alive_uidx
  ON files (path, storage_id)
  WHERE deleted_at IS NULL;
```

This allows a live file at path `P` while a trashed copy of `P` still exists.

### `app_settings`

Key-value table for mutable global settings:

```sql
CREATE TABLE IF NOT EXISTS app_settings (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
```

- Key `trash_retention_days`, value `"30"` by default (seeded on init if missing)
- Validated to integer in `1..=30` on write

## Backend behavior

### Soft delete (`DELETE /storages/:id/files/*path`)

- Access: Write
- Sets `deleted_at` instead of hard `DELETE`
- Does **not** touch `file_chunks` / `chunk_replicas` or Telegram

### List trash (`GET /storages/:id/trash?path=`)

- Access: Read
- Same directory-listing shape as `list_dir`, but only rows with `deleted_at IS NOT NULL`
- Optional `path` prefix for browsing folder containers inside trash

### Restore (`POST /storages/:id/trash/restore`)

Body:

```json
{ "path": "folder/file.txt", "on_conflict": null | "replace" | "rename" }
```

- Access: Write
- Folder path: clear `deleted_at` on that folder marker and all currently-trashed descendants under the prefix
- File path: clear `deleted_at` on that file; ensure live parent folder markers exist
- If a **live** row already occupies the target path:
  - `on_conflict` omitted → `409` `TrashPathConflict`
  - `replace` → permanently purge the live conflicting row(s) (Telegram + hard DB delete), then restore
  - `rename` → pick next free name (`name (1).ext`, …) and restore under that path

### Permanent delete (`DELETE /storages/:id/trash/*path`)

- Access: Write
- Resolve trashed file ids under path/prefix
- For each replica with `telegram_message_id`: call Telegram `deleteMessage(chat_id, message_id)`
  - Treat “message not found / already deleted” as success
  - Log and continue on other failures (best-effort; still hard-delete DB so trash doesn’t stick forever)
- Hard-delete matching `files` rows (CASCADE chunks/replicas)

### Empty trash (`DELETE /storages/:id/trash`)

- Same purge path for all trashed rows in the storage

### Settings (`GET/PUT /settings/trash`)

```json
{ "retention_days": 30 }
```

- Any authenticated user can read; write allowed for authenticated users (single-tenant app pattern matches other settings surfaces)
- Clamp/reject outside `1..=30`

## Telegram + background worker

- Add `TelegramBotApi::delete_message(chat_id, message_id, storage_id)`
- `TrashPurgeService::spawn_loop` (pattern like `ChannelHealthService`), interval ~10 minutes:
  1. Read retention days from `app_settings`
  2. Select files with `deleted_at < now() - retention`
  3. Purge via shared helper used by permanent delete / empty trash

## UI

### Files page

- Toolbar next to **Create**: **Trash** button toggles trash mode
- Trash mode:
  - Lists trashed items (folders navigable as containers)
  - Item actions: **Restore**, **Delete forever**
  - Toolbar: **Empty trash** → small confirm modal (Yes/No). Yes runs empty; No closes modal
  - Back / exit trash returns to normal Files view
- Soft delete confirm copy updates from permanent delete to “move to trash”
- Restore conflict → modal: Replace / Rename / Cancel

### Settings

- New tab **Trash** (or General) in `SettingsModal`:
  - Number field “Days in trash” (1–30), default 30, save via API

## Out of scope

- Preview/download of trashed files without restore
- Telegram cleanup when deleting an entire storage (follow-up)
- Per-storage retention overrides
