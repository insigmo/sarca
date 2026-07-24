# Multi-chat storage replication Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replicate file chunks across up to 3 Telegram chats per storage with circular primary failover, dead-channel detection, catch-up replication via `copyMessage`, and UI for channel management + warning icon.

**Architecture:** Replace `storages.chat_id` with `storage_channels` (slots 1–3) + `storages.primary_position`. Store per-channel Telegram ids in `chunk_replicas`. Upload to current primary, async-replicate to other active channels. Health-check on read errors and every 30 minutes.

**Tech Stack:** Rust (Axum, SQLx/Postgres), SolidJS UI, Telegram Bot API (`sendDocument`, `copyMessage`, `getChat`, `getFile`).

**Spec:** `docs/superpowers/specs/2026-07-24-multi-chat-replication-design.md`

## Global Constraints

- Max 3 channels per storage; at least one active required.
- Primary/Secondary roles are backend-only (never shown in UI).
- Prefer `copyMessage`; fallback download + `sendDocument`.
- File readable after primary upload; secondaries catch up async.
- Dead channels stay editable; warning icon when any channel is dead.
- Auto-fetch channel title via `getChat` when name empty.
- Scheduler interval: 30 minutes.
- Do not delete Telegram messages when removing a channel slot.

---

## File map

| Path | Role |
|------|------|
| `sarca/src/startup.rs` | Schema create/migrate |
| `sarca/src/models/storage_channels.rs` | Channel model |
| `sarca/src/models/chunk_replicas.rs` | Replica model |
| `sarca/src/models/storages.rs` | Drop chat_id; add primary_position; has_dead_channel |
| `sarca/src/models/file_chunks.rs` | Drop telegram_file_id from logical chunk |
| `sarca/src/repositories/storage_channels.rs` | CRUD + health helpers |
| `sarca/src/repositories/chunk_replicas.rs` | Replica CRUD / pending queue |
| `sarca/src/repositories/storages.rs` | Create with channels; list has_dead |
| `sarca/src/repositories/files.rs` | Chunks without file_id; join replicas for download |
| `sarca/src/schemas/storages.rs` | Multi-channel create/update APIs |
| `sarca/src/common/telegram_api/*` | message_id parse, getChat, copyMessage, chat-dead detection |
| `sarca/src/services/storages.rs` | Channel CRUD orchestration |
| `sarca/src/services/storage_manager.rs` | Primary upload + enqueue replicas |
| `sarca/src/services/replication.rs` | Worker: copy/fallback + «Догрузить» |
| `sarca/src/services/channel_health.rs` | Health-check + primary rotation |
| `sarca/src/routers/storages.rs` | Channel + retry endpoints |
| `sarca/src/routers/files.rs` | Download via primary replica + failover |
| `sarca/src/main.rs` | Spawn replication + health scheduler |
| `ui/src/pages/Storages/*`, `StorageSettingsModal.jsx`, `api/index.js` | Multi-channel UI + warning |

---

### Task 1: Schema migration + models

**Files:**
- Modify: `sarca/src/startup.rs`
- Create: `sarca/src/models/storage_channels.rs`, `sarca/src/models/chunk_replicas.rs`
- Modify: `sarca/src/models/storages.rs`, `sarca/src/models/file_chunks.rs`, `sarca/src/models/mod.rs`

- [ ] **Step 1: Add migration SQL in `init_db`**

After existing table creates, add idempotent steps:

1. `CREATE TABLE IF NOT EXISTS storage_channels (...)`
2. `ALTER TABLE storages ADD COLUMN IF NOT EXISTS primary_position SMALLINT NOT NULL DEFAULT 1`
3. Migrate: for each storage still having `chat_id` column with data and no channels, insert channel position=1
4. `CREATE TABLE IF NOT EXISTS chunk_replicas (...)`
5. Migrate: for each `file_chunks` row that still has `telegram_file_id`, insert replica for that storage's channel at position matching the file's storage primary slot (use channel at storages.primary_position or position=1)
6. Drop `file_chunks.telegram_file_id` if present (`ALTER ... DROP COLUMN IF EXISTS`)
7. Drop `storages.chat_id` if present

Use Postgres-friendly checks (`information_schema.columns`) so restarts are safe.

Channel status values: `'active' | 'dead'`. Replica status: `'pending' | 'uploaded' | 'failed'`.

- [ ] **Step 2: Models**

```rust
// storage_channels.rs
pub struct StorageChannel {
    pub id: Uuid,
    pub storage_id: Uuid,
    pub position: i16, // 1..=3
    pub chat_id: ChatId,
    pub name: String,
    pub status: String, // active|dead
}

// chunk_replicas.rs
pub struct ChunkReplica {
    pub id: Uuid,
    pub chunk_id: Uuid,
    pub channel_id: Uuid,
    pub telegram_file_id: Option<String>,
    pub telegram_message_id: Option<i64>,
    pub status: String,
}

// FileChunk: remove telegram_file_id
pub struct FileChunk {
    pub id: Uuid,
    pub file_id: Uuid,
    pub position: Position,
}

// Storage: remove chat_id, add primary_position
// StorageWithInfo: remove chat_id, add primary_position + has_dead_channel: bool
```

- [ ] **Step 3: Compile models module; fix obvious import breaks later in Task 2+**

- [ ] **Step 4: Commit**

```bash
git add sarca/src/startup.rs sarca/src/models/
git commit -m "feat: multi-chat schema and models"
```

---

### Task 2: Repositories for channels and replicas

**Files:**
- Create: `sarca/src/repositories/storage_channels.rs`, `sarca/src/repositories/chunk_replicas.rs`
- Modify: `sarca/src/repositories/mod.rs`, `storages.rs`, `files.rs`

- [ ] **Step 1: `StorageChannelsRepository`**

Methods: `list_by_storage`, `get_by_id`, `insert`, `update` (chat_id/name/status), `delete`, `count_active`, `mark_dead`, `get_at_position`.

- [ ] **Step 2: `ChunkReplicasRepository`**

Methods: `insert_batch`, `list_pending` (limit), `mark_uploaded`, `mark_failed`, `list_by_chunk`, `get_uploaded_for_channel`, `enqueue_for_channel` (all chunks of storage → pending for one channel), `delete_by_channel`, `replication_stats(storage_id)`.

- [ ] **Step 3: Update `StoragesRepository`**

- `create(name, primary_position=1)` without chat_id; caller inserts channels.
- `list_by_user_id`: join/exists for `has_dead_channel`; select `primary_position`.
- `get_by_file_id` / `get_by_id` return new Storage shape.
- `set_primary_position(storage_id, pos)`.

- [ ] **Step 4: Update `FilesRepository` chunk APIs**

- `create_chunks_batch` inserts into `file_chunks` without telegram_file_id.
- Add helpers to load chunks with preferred replica for a channel id.
- Download path will resolve `telegram_file_id` from replica.

- [ ] **Step 5: `cargo check` in `sarca/`; fix compile errors in touched repos**

- [ ] **Step 6: Commit**

```bash
git commit -m "feat: channel and replica repositories"
```

---

### Task 3: Telegram API — getChat, copyMessage, message_id, dead detection

**Files:**
- Modify: `sarca/src/common/telegram_api/schemas.rs`, `bot_api.rs`

- [ ] **Step 1: Parse `message_id` from sendDocument**

```rust
pub struct UploadResultSchema {
    pub message_id: i64,
    pub document: UploadSchema,
}

pub struct UploadOutcome {
    pub file_id: String,
    pub message_id: i64,
}
```

Change `upload` / `upload_file_part` to return `UploadOutcome`.

- [ ] **Step 2: Add `get_chat(chat_id, storage_id) -> SarcaResult<ChatInfo { id, title }>`**

- [ ] **Step 3: Add `copy_message(from_chat_id, message_id, to_chat_id, storage_id) -> UploadOutcome`**

Parse copied message document file_id + message_id.

- [ ] **Step 4: Helper `fn is_chat_dead_error(err: &SarcaError) -> bool`**

Match Telegram body substrings: `chat not found`, `bot was kicked`, `chat_id is empty`, `PEER_ID_INVALID`, etc.

- [ ] **Step 5: Commit**

```bash
git commit -m "feat: telegram getChat, copyMessage, message_id"
```

---

### Task 4: Channel health + primary rotation service

**Files:**
- Create: `sarca/src/services/channel_health.rs`
- Modify: `sarca/src/services/mod.rs`

- [ ] **Step 1: Implement**

```rust
pub async fn health_check_storage(db, telegram_base, rate_limit, storage_id) -> SarcaResult<HealthReport>;
pub async fn rotate_primary_if_needed(db, storage_id) -> SarcaResult<()>;
pub fn next_active_position(primary: i16, channels: &[StorageChannel]) -> Option<i16>;
```

Circle: from current primary, try `pos%3+1` until an active channel found (skip dead). If current primary dead, update `storages.primary_position`.

`health_check_storage`: for each channel call `getChat`; on dead error → `mark_dead` + rotate if needed.

- [ ] **Step 2: Unit-test `next_active_position` in the same file (`#[cfg(test)]`)**

Cases: 1 dead → 2; 2 dead with primary 2 → 3; all dead → None; wrap 3→1.

- [ ] **Step 3: Commit**

```bash
git commit -m "feat: channel health check and primary rotation"
```

---

### Task 5: Upload path + replication worker

**Files:**
- Modify: `sarca/src/services/storage_manager.rs`
- Create: `sarca/src/services/replication.rs`
- Modify: `sarca/src/main.rs`, `storage_manager.rs` (optional enqueue hook)

- [ ] **Step 1: Primary upload**

Resolve primary channel via `primary_position` (rotate if dead). Upload chunks with `sendDocument`. Insert `file_chunks` + primary `chunk_replicas` (`uploaded` + ids). For each other active channel insert `pending` replicas. Thumb still uploads to primary chat.

- [ ] **Step 2: Replication worker loop**

Spawn in `main.rs` alongside StorageManager:

```rust
tokio::spawn(replication::run_loop(db, config));
```

Every few seconds (e.g. 5s): take batch of pending/failed (optional: only pending automatically; failed waits for button — auto-retry pending with limited attempts, mark failed after N). For each:

1. Find source replica with `uploaded` + `telegram_message_id` on an active channel → `copyMessage`
2. Else source with `telegram_file_id` → download bytes → `sendDocument` to target
3. On chat-dead → health_check_storage; skip
4. Else mark failed

- [ ] **Step 3: `retry_failed(storage_id)` sets failed→pending for that storage**

- [ ] **Step 4: Commit**

```bash
git commit -m "feat: primary upload and async chunk replication"
```

---

### Task 6: Download failover

**Files:**
- Modify: `sarca/src/routers/files.rs` (and helpers that use `chunk.telegram_file_id`)

- [ ] **Step 1: Resolve telegram file ids via replicas**

When loading chunks for download, join preferred replica for current primary. If missing/failed download:

1. `health_check_storage`
2. Re-resolve primary
3. Try next active channels' uploaded replicas

- [ ] **Step 2: Chunk cache keys remain telegram_file_id strings (unchanged)**

- [ ] **Step 3: Commit**

```bash
git commit -m "feat: download failover across channel replicas"
```

---

### Task 7: Storage API — multi-channel CRUD + retry

**Files:**
- Modify: `schemas/storages.rs`, `services/storages.rs`, `routers/storages.rs`, `startup.rs` bootstrap

Schemas:

```rust
pub struct ChannelInput { pub chat_id: ChatId, pub name: Option<String> }
pub struct InStorageSchema { pub name: String, pub channels: Vec<ChannelInput> } // 1..=3

pub struct StorageChannelSchema { /* id, position, chat_id, name, status */ }
pub struct StorageDetail { /* storage fields + channels + replication stats */ }
```

Endpoints:

- `POST /storages` — create with 1–3 channels; fetch titles via getChat when name empty
- `GET /storages/:id` — include channels + `has_dead_channel` + replication `{pending, failed, uploaded}`
- `POST /storages/:id/channels` — add channel (max 3); enqueue catch-up
- `PUT /storages/:id/channels/:channel_id` — edit chat_id/name; if was dead → active + enqueue catch-up
- `DELETE /storages/:id/channels/:channel_id` — forbid if last active; rotate primary if needed
- `POST /storages/:id/replication/retry` — failed → pending

Update env bootstrap to insert `storage_channels` instead of `chat_id` on storages.

- [ ] **Step 1: Implement schemas + service + router**
- [ ] **Step 2: `cargo check`**
- [ ] **Step 3: Commit**

```bash
git commit -m "feat: multi-channel storage API"
```

---

### Task 8: Health scheduler (30 min)

**Files:**
- Modify: `sarca/src/main.rs` (or small `services/channel_health_scheduler.rs`)

- [ ] **Step 1: Spawn**

```rust
tokio::spawn(async move {
    let mut ticker = tokio::time::interval(Duration::from_secs(30 * 60));
    loop {
        ticker.tick().await;
        // list all storage ids; health_check_storage each
    }
});
```

- [ ] **Step 2: Commit**

```bash
git commit -m "feat: periodic channel health scheduler"
```

---

### Task 9: UI

**Files:**
- Modify: `ui/src/api/index.js`
- Modify: `ui/src/pages/Storages/StorageCreateForm.jsx`
- Modify: `ui/src/pages/Storages/index.jsx`
- Modify: `ui/src/components/StorageSettingsModal.jsx`
- Modify: `ui/src/index.css` (warning icon styles if needed)

- [ ] **Step 1: API client** — create with `channels[]`; channel CRUD; retry replication; types with `has_dead_channel`

- [ ] **Step 2: Create form** — 1 required channel + «Добавить ещё» up to 3; optional name fields

- [ ] **Step 3: List** — warning icon when `has_dead_channel`

- [ ] **Step 4: Settings modal** — list channels (id, name, status); add/edit/remove; dead copy; replication counts + «Догрузить». No Primary/Secondary labels.

- [ ] **Step 5: Commit**

```bash
git commit -m "feat: UI for multi-chat storage channels"
```

---

### Task 10: End-to-end verification

- [ ] **Step 1: `cargo check` / `cargo test` in `sarca/`**
- [ ] **Step 2: Fix remaining compile errors**
- [ ] **Step 3: Manual checklist against spec Verify section (if env available)**
- [ ] **Step 4: Final commit if fixes needed**

---

## Spec coverage check

| Spec item | Task |
|-----------|------|
| storage_channels + primary_position | 1–2 |
| chunk_replicas + migrate file_id | 1–2 |
| copyMessage + fallback | 3, 5 |
| Circular primary | 4 |
| Upload primary then async | 5 |
| Read failover + all-channel health on error | 4, 6 |
| Scheduler 30 min | 8 |
| UI create/settings/warning/retry | 9 |
| Auto getChat title | 7 |
| Catch-up on new/repaired channel | 5, 7 |
| No primary labels in UI | 9 |
