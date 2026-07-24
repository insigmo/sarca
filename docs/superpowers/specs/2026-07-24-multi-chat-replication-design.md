# Multi-chat storage replication (2026-07-24)

## Problem

A storage maps 1:1 to a single Telegram chat. If that chat is deleted, all file bytes become unreachable even though Postgres still has metadata (`files` / `file_chunks`). There is no redundancy and no UI signal that a channel is gone.

## Goal

Replicate every uploaded chunk across up to **3** Telegram chats for one storage. Reads prefer the current primary chat and fail over on errors. Dead chats are detected (on read failure and via a periodic health check), marked in the DB, surfaced in the UI, and remain editable so the operator can replace the chat id.

## Decisions (locked)

| Topic | Choice |
|-------|--------|
| Channel slots | Table `storage_channels`, max 3 per storage |
| Primary selection | Storage field `primary_position` (1–3); slots do **not** reorder |
| Failover | On primary death, `primary_position` advances to next **active** slot in circle `1→2→3→1` |
| Replication transport | Prefer Telegram `copyMessage`; fallback download + `sendDocument` |
| Upload success | File is readable after primary upload succeeds; secondaries catch up async |
| Dead channel UX | Keep slot, `status=dead`, editable chat id + name; warning icon on storage list |
| Roles in UI | Primary/Secondary are **backend-only**; UI shows channel list + status only |
| Channel title | Auto-fetch via Telegram `getChat` when id is set and name is empty |
| Channel management | After create: add / remove / edit slots (cannot remove last active) |
| Catch-up after replace | Background replicate all existing chunks into the repaired/new channel |
| Failed replica | Status in settings + **«Догрузить»** button |
| Health on file error | Check **all** channels of that storage, not only the failing one |
| Scheduler | Every **30 minutes**, health-check all storage channels |

## Data model

### `storage_channels`

| Column | Type | Notes |
|--------|------|-------|
| `id` | UUID PK | |
| `storage_id` | UUID FK → storages | |
| `position` | SMALLINT | Fixed slot `1`, `2`, or `3`; unique per `(storage_id, position)` |
| `chat_id` | BIGINT | Telegram chat id; unique globally among channels |
| `name` | TEXT | Display name (from user or `getChat`) |
| `status` | TEXT | `active` \| `dead` |

Constraints: 1–3 rows per storage; at least one `active` required for normal operation.

### `storages` changes

- Remove `chat_id` (migrate existing value into `storage_channels` with `position=1`, `status=active`).
- Add `primary_position SMALLINT NOT NULL DEFAULT 1` — which slot is currently used for new uploads and preferred reads.

API list payloads include `has_dead_channel: bool` (derived) for the warning icon. Do **not** expose primary/secondary labels in the UI.

### `file_chunks` / replicas

Keep logical chunk row (`file_chunks`: `id`, `file_id`, `position` within file).

Add `chunk_replicas`:

| Column | Type | Notes |
|--------|------|-------|
| `id` | UUID PK | |
| `chunk_id` | UUID FK → file_chunks | |
| `channel_id` | UUID FK → storage_channels | |
| `telegram_file_id` | TEXT NULL | Set when uploaded/copied |
| `telegram_message_id` | BIGINT NULL | Needed for `copyMessage` from this chat |
| `status` | TEXT | `pending` \| `uploaded` \| `failed` |

Unique `(chunk_id, channel_id)`.

Migration of existing `file_chunks.telegram_file_id`: move into one `chunk_replicas` row for the storage’s slot-1 channel with `status=uploaded` and `telegram_message_id=NULL` (copy from that chunk requires fallback re-upload until a new message id exists).

After migration, drop `file_chunks.telegram_file_id`.

## Write path

1. Resolve primary channel: `storage_channels` where `position = storages.primary_position` and `status=active`. If that slot is dead, advance `primary_position` (circle) until an active slot is found; if none, fail the upload.
2. Split file and `sendDocument` each part to the primary `chat_id`. Persist chunk + primary replica (`uploaded`, store `file_id` + `message_id` from Bot API).
3. For every other **active** channel, insert `chunk_replicas` with `status=pending`.
4. Mark `files.is_uploaded=true` after primary completes (same user-visible success as today).
5. Background replication worker drains `pending`/`failed` (or explicit «Догрузить») jobs:
   - Prefer `copyMessage` from any replica that has `telegram_message_id` + live channel.
   - Else download via `getFile` from any `uploaded` replica and `sendDocument` to the target.
   - Success → `uploaded` + ids; hard failure → `failed`.

Adding a new channel or replacing a dead channel’s `chat_id`: set `status=active`, resolve name via `getChat` if needed, enqueue `pending` replicas for all existing chunks, worker catch-up.

Removing a channel slot: delete its `chunk_replicas` rows and the channel row. Do not delete Telegram messages. Forbid removing the last `active` channel. If removed slot was `primary_position`, advance primary to next active.

## Read path

1. Prefer replicas on the channel at `primary_position` with `status=uploaded`.
2. On Telegram/download error for a file:
   - Run health-check on **all** channels of the storage (`getChat` or equivalent).
   - Mark unreachable chats `dead`.
   - If primary became dead, set `primary_position` to the next active in circle.
   - Retry read from the new primary (then other actives) using that channel’s replica `telegram_file_id`.
3. Transient errors (network, 429, 5xx): retry without marking dead.

## Dead-channel detection

A channel is marked `dead` when Telegram indicates the chat is gone or the bot can no longer access it (e.g. chat not found / kicked), confirmed via health-check.

Triggers:

1. Error while reading/uploading/replicating involving that chat.
2. Scheduler every **30 minutes** over all `storage_channels`.

On mark-dead:

- Persist `status=dead`.
- If it was primary → rotate `primary_position`.
- Storage list exposes `has_dead_channel=true` → warning icon in UI.
- Settings copy (localized as implemented in UI): channel `{name}` (`{chat_id}`) was deleted; please set a new chat id or add another channel.

Editing a dead channel: update `chat_id` / `name`, set `active`, enqueue full catch-up replication. Primary pointer is **not** moved back automatically.

## UI

### Create storage

- Storage name.
- One required channel: chat id; name optional (auto from Telegram).
- «Добавить ещё канал» up to 3 total.

### Storage list

- Warning icon when `has_dead_channel`.

### Storage settings

- Channel slots: chat id, name, status (`active` / deleted).
- No Primary/Secondary labels or primary highlighting.
- Add / edit / remove (guards as above).
- Replication summary: pending / failed counts or per-channel status + **«Догрузить»**.

### File browser / download

- Failover and health-check stay on the backend; no per-file primary UI.

## Workers / bots

Existing `storage_workers` (bot tokens bound to a storage) are unchanged: any worker of the storage may `sendDocument` / `copyMessage` / `getFile` against any of that storage’s chats. No per-channel tokens in this design.

## Out of scope

- Cross-storage replication
- More than 3 chats
- Deleting Telegram messages when removing a slot or storage
- Automatic demotion of primary back to a repaired former primary
- Encrypted or integrity-hashed replica comparison beyond relying on Telegram copy/upload success

## Verify

1. Create storage with 1 chat; upload a file; download works.
2. Add 2nd and 3rd chats; replicas become `uploaded` (via copy); download still works.
3. Simulate primary chat loss (or mark unreachable); next channel becomes primary; download works from secondary; list shows warning; settings show dead channel editable.
4. Replace dead chat id; catch-up fills replicas; warning clears when no dead remain.
5. Force a replica `failed`; «Догрузить» recovers it.
6. Scheduler marks a deleted chat within ~30 minutes without a user download.
