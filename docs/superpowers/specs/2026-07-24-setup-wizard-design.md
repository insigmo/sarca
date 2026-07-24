# Setup wizard (2026-07-24)

## Problem

Setting up Sarca requires many manual Telegram steps scattered across BotFather, channel admin UI, chat-id bots, then separate Sarca screens for storage and workers (plus Local Bot API credentials in `sarca.conf`). There is no first-run guided path. Env bootstrap only wires an already-created bot + channel.

## Goal

A **two-phase setup wizard** that configures Local Bot API credentials (once) and creates a storage + worker from a bot token + auto-detected channel(s), **without Telegram user login** inside Sarca. Remaining Telegram work is limited to: create bot, create channel(s), add bot as admin, post a message for detection.

## Decisions (locked)

| Topic | Choice |
|-------|--------|
| Telegram login in Sarca | **No** (no MTProto / user session) |
| Channel discovery | User posts in channel → Sarca polls Bot API `getUpdates` |
| Wizard entry | Auto on first login when user has **no storages**; **New storage** always opens Phase B |
| Local Bot API | Phase A once per server when not ready; skip allowed with ~20 MB warning |
| Scope v1 | Local API credentials + verify; bot validate; up to 3 channels; atomic storage+worker create |
| Advanced forms | Keep existing storage/worker forms as fallback |
| `TELEGRAM_LOCAL_API` / base URL | Stay in `sarca.conf`; wizard does **not** flip Docker Compose |
| Auto-restart Local Bot API container | **Out of scope** v1 (instruct user to restart if needed) |

## Flow

```text
Login → has storages?
         ├─ no  → Setup Wizard
         │         ├─ Phase A if local API not ready
         │         └─ Phase B (bot + channels → storage)
         └─ yes → normal UI
                    «New storage» → Phase B only
```

`GET /setup/status` drives auto-open after login (`needs_local_api`, `local_api_ready` / skipped, `has_storages`).

### Phase A — Local Bot API (server-once)

1. Explain why Local API matters (files larger than official ~20 MB limit).
2. Link to https://my.telegram.org → API development tools.
3. Collect `api_id` / `api_hash`; persist in `app_settings`.
4. If the deployment reads credentials from a file used by the Local Bot API entrypoint, write there too when supported; otherwise show “restart `telegram-bot-api`”.
5. **Verify** against `TELEGRAM_API_BASE_URL`.
6. **Skip** sets `local_api_skipped` (or equivalent) and keeps a UI banner about the size limit.

### Phase B — Storage (bot + channels)

Token is held only for the wizard session (UI state + setup endpoints). Persist to `storage_workers` only on final create.

1. Storage name.
2. BotFather deep-link + paste token → validate (`getMe`, `deleteWebhook` so `getUpdates` works).
3. Instructions: private channel → add bot as admin (post + delete messages).
4. “Post any message in the channel” → UI polls channel detect every ~2s (≈2 min timeout).
5. Optional: detect up to **3** channels total (`exclude_chat_ids` so prior hits are ignored).
6. Finish: one transaction creates storage + channel slots + worker → open that storage’s files.

Mid-wizard abandon creates nothing in the DB.

## Backend API

Router `setup` (authenticated; same create-storage authorization as today).

| Endpoint | Purpose |
|----------|---------|
| `GET /setup/status` | `needs_local_api`, `local_api_ready`, `local_api_skipped`, `has_storages` |
| `POST /setup/local-api` | Save `api_id` / `api_hash` to `app_settings` (+ entrypoint file if applicable) |
| `POST /setup/local-api/verify` | Health/ping Local API base URL |
| `POST /setup/bot/validate` | `{ token }` → `getMe` + `deleteWebhook` |
| `POST /setup/channel/poll` | `{ token, exclude_chat_ids? }` → `getUpdates`, return new `chat_id` + title |
| `POST /setup/storages` | `{ name, token, chat_ids[] }` → atomic storage + channels + worker |

### Channel poll semantics

- Prefer `channel_post` updates; also accept relevant `my_chat_member` if enough to resolve chat.
- Confirm with `getChat` when possible.
- Do not write to DB.
- Rate-limit polls server-side.

### Security

- Do not log full bot tokens.
- Setup poll/validate are ephemeral; no worker row until finish.
- Created storages use existing ACL rules.

## UI

- Full-screen or modal `SetupWizard` with Phase A → Phase B.
- Empty state (no storages) and **New storage** open Phase B (Phase A only when status says needed).
- Clear errors: bad token, detect timeout, bot not admin, duplicate `chat_id`; allow retry without wiping the whole wizard.
- After skip Local API: persistent but dismissible warning about ~20 MB until Local API becomes ready.

## Edge cases

- Bot already had a webhook → cleared on validate.
- Multiple channels in one session → `exclude_chat_ids`.
- Env bootstrap already created a storage → no auto wizard.
- Official API path (skip or `TELEGRAM_LOCAL_API=false`) → Phase B still works.

## Testing

- Service: parse `getUpdates` → `chat_id`; atomic create storage+channels+worker.
- API: bad token; poll timeout / no update; setup status flags; auth on setup routes.
- UI smoke: first-run opens wizard; New storage opens Phase B; finish lands in storage files.

## Out of scope (v1)

- Telegram user login / MTProto; programmatic bot or channel creation.
- Auto-restart of the Local Bot API Docker container.
- Removing advanced storage/worker forms.
- Changing how `TELEGRAM_LOCAL_API` / `TELEGRAM_API_BASE_URL` are enabled in compose/conf.
