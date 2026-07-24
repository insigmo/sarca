# Setup Wizard Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans (or subagent-driven-development) to implement task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Two-phase first-run / new-storage wizard: Local Bot API credentials once, then bot token + `getUpdates` channel detection → atomic storage + worker — without Telegram user login.

**Architecture:** New authenticated `/api/setup/*` router; token-scoped Telegram client (no storage worker yet); `app_settings` flags + optional `sarca.conf` upsert for `TELEGRAM_API_ID`/`HASH`; SolidJS `SetupWizard` page replacing default new-storage flow.

**Tech Stack:** Rust/Axum/SQLx, SolidJS/SUID, Telegram Bot API.

## Global Constraints

- No MTProto / Telegram user login
- Channel discovery via `getUpdates` only (user posts in channel)
- Max 3 channels; atomic create storage + channels + worker
- Do not log full bot tokens
- Keep existing StorageCreateForm as advanced fallback at `/storages/register/manual` (or keep form reachable)
- `TELEGRAM_LOCAL_API` / base URL stay in conf; wizard does not flip Compose
- Follow patterns in `SettingsRouter`, `StoragesService::create`, `StorageWorkersService::create`

## File map

| File | Role |
|------|------|
| `sarca/src/common/telegram_api/token_client.rs` | Token-based getMe / deleteWebhook / getUpdates / getChat |
| `sarca/src/common/telegram_api/schemas.rs` | Update / getMe deserialize types |
| `sarca/src/conf.rs` | `find_loaded_conf_path`, `upsert_conf_keys` |
| `sarca/src/repositories/app_settings.rs` | Local-api keys helpers |
| `sarca/src/schemas/setup.rs` | Request/response DTOs |
| `sarca/src/services/setup.rs` | Setup business logic |
| `sarca/src/routers/setup.rs` | HTTP routes |
| `ui/src/components/SetupWizard.jsx` | Wizard UI |
| `ui/src/pages/Storages/index.jsx` + `App.jsx` | Entry points |
| `ui/src/api/index.js` | Setup API client |

---

### Task 1: Token Telegram client + update parsing

**Files:**
- Create: `sarca/src/common/telegram_api/token_client.rs`
- Modify: `sarca/src/common/telegram_api/mod.rs`, `schemas.rs`

**Interfaces:**
- `TelegramTokenClient::new(base_url, token)`
- `get_me() -> BotMe { id, username }`
- `delete_webhook() -> ()`
- `get_updates() -> Vec<DetectedChat>` where `DetectedChat { chat_id, title }`
- `get_chat(chat_id) -> ChatInfo`
- Pure fn `chats_from_updates_json(value) -> Vec<DetectedChat>` for unit test

- [ ] **Step 1:** Add schemas for getMe / getUpdates (channel_post.chat, my_chat_member.chat).
- [ ] **Step 2:** Implement client; mask token in logs like `bot_api.rs`.
- [ ] **Step 3:** Unit test: JSON with channel_post → negative chat_id + title.
- [ ] **Step 4:** `cargo test -p sarca chats_from_updates` passes.

---

### Task 2: app_settings + conf upsert for Local API

**Files:**
- Modify: `sarca/src/repositories/app_settings.rs`, `sarca/src/conf.rs`

**Keys:** `telegram_api_id`, `telegram_api_hash`, `local_api_skipped` (`"true"` / absent)

- [ ] **Step 1:** Repo helpers get/set id, hash, skipped flag.
- [ ] **Step 2:** `conf::upsert_conf_keys(path, &[("TELEGRAM_API_ID", ..), ("TELEGRAM_API_HASH", ..)])` rewrite/replace lines; best-effort if file missing/unwritable.
- [ ] **Step 3:** `conf::resolve_conf_path() -> Option<PathBuf>` (same candidates as load).

---

### Task 3: Setup service + router

**Files:**
- Create: `schemas/setup.rs`, `services/setup.rs`, `routers/setup.rs`
- Modify: mods, `server.rs`, `errors.rs` if needed

**Endpoints:** as in design spec.

**Status logic:**
- `uses_local_api` = base URL does not contain `api.telegram.org`
- `local_api_ready` = verify ping OK (GET base URL or `/` — accept any HTTP response that connects)
- `needs_local_api` = `uses_local_api && !local_api_ready && !skipped` OR optionally encourage Local API when on official API and not skipped — **v1:** only when `uses_local_api` and not ready/skipped; when on official API, `needs_local_api=false` but status exposes `uses_local_api=false` so UI can still offer optional Phase A with skip
- Spec C wants Phase A in wizard: show Phase A when `!local_api_ready && !local_api_skipped` **or** when `!uses_local_api && !local_api_skipped` (offer upgrade). Simpler v1: show Phase A when `!local_api_skipped && (!uses_local_api || !local_api_ready)` — on official API, Phase A explains enabling Local API in conf + credentials; skip continues.

**Practical v1 status fields:**
```json
{
  "has_storages": bool,
  "uses_local_api": bool,
  "local_api_ready": bool,
  "local_api_skipped": bool,
  "needs_local_api_phase": bool
}
```
`needs_local_api_phase = !local_api_skipped && !(uses_local_api && local_api_ready)`

**create storage:** reuse `StoragesService::create` + `StorageWorkersService::create` with worker name = bot username or storage name; chat names from getChat with token client.

- [ ] **Step 1:** Implement service methods.
- [ ] **Step 2:** Mount `/setup` router with `logged_in_required`.
- [ ] **Step 3:** `cargo check` passes.

---

### Task 4: UI SetupWizard + API + wiring

**Files:**
- Create: `ui/src/components/SetupWizard.jsx` (or `ui/src/pages/Setup/index.jsx`)
- Modify: `ui/src/api/index.js`, `App.jsx`, `Storages/index.jsx`, `index.css` if needed
- Keep: `StorageCreateForm.jsx` at `/storages/register/manual`

**Behavior:**
- Route `/setup` → wizard; `/storages/register` → wizard (Phase B, Phase A if needed)
- Storages page: if no storages on mount → navigate `/setup`
- New storage button → `/storages/register` (wizard)
- Poll channel every 2s until found or 2 min timeout
- On success → `/storages/:id/files`

- [ ] **Step 1:** API methods for setup endpoints.
- [ ] **Step 2:** Wizard component (Phase A then B).
- [ ] **Step 3:** Wire routes + empty-state redirect.
- [ ] **Step 4:** UI builds (`pnpm build` in `ui/`).

---

### Task 5: Verify

- [ ] `cargo test` / `cargo check` in `sarca/`
- [ ] `pnpm build` in `ui/`
- [ ] Commit when user asks (or single feat commit if already approved to ship)

---

## Spec coverage

| Spec item | Task |
|-----------|------|
| Phase A local API | 2, 3, 4 |
| Phase B bot + getUpdates | 1, 3, 4 |
| First-run + New storage | 4 |
| Atomic create | 3 |
| No token logging | 1, 3 |
| Out of scope docker restart | documented in Phase A copy |
|advanced forms remain | `/storages/register/manual` |
