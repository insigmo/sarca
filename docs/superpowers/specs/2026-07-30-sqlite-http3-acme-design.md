# Design: SQLite + HTTP/3 + in-process ACME (no Local Bot API)

Date: 2026-07-30  
Status: approved for implementation planning  
Related goals: lower RAM, faster client↔server path, fewer runtime dependencies

## Summary

Replace Postgres with SQLite, remove Local Telegram Bot API (keep Telegram as the durable blob store via official Bot API with 20MB chunks), and terminate TLS/HTTP/3 inside the `sarca` binary (no Caddy). Primary deploy path is a bare-metal binary; Docker Compose is optional. Clients prefer QUIC/HTTP/3 with TCP HTTPS fallback. Certificates come from Let's Encrypt via in-process ACME (`shortlived` profile for both domain and IP), renewed one day before expiry.

**Telegram is not removed.** Only the Local Bot API container and large-chunk mode go away. Multi-channel storage, bot workers, replication, and channel health stay as designed today.

## Goals

- Cut memory and moving parts: no Postgres process, no `telegram-bot-api` process, no reverse proxy required.
- Serve the same Axum API + UI over HTTP/3 (UDP/443) and HTTPS HTTP/1.1 fallback (TCP/443).
- Prefer QUIC on native clients; browsers use standard HTTPS and pick H3 when available.
- Automate trusted TLS for public domain **or** public IP (LE IP certs + short-lived certs).
- Cover the change with automated tests and verify before calling done.

## Non-goals (this stage)

- Replacing Telegram with local/S3 blob storage.
- Removing multi-channel replication / channel health / multiple bot workers.
- Caddy or other external HTTP/3 proxy.
- Migrating existing Postgres data (wipe / fresh install only).
- Application-level custom CID protocol (use QUIC connection migration from the stack).

## Target runtime

```
Client (UI / Tauri)
  --prefer--> HTTP/3 (QUIC, UDP/443)
  --fallback--> HTTPS HTTP/1.1 (TCP/443)

ACME http-01 + HTTPS redirect <--- TCP/80

sarca (single binary)
├── Axum Router (API + static UI) on H3 + TCP
├── In-process ACME (LE shortlived; domain or IP)
├── SQLite (WAL) — metadata only
├── Official Telegram Bot API (api.telegram.org, ≤20MB chunks)
└── Background workers: upload, replication, channel health, trash/storage purge
```

## 1. Data layer: SQLite

### Storage

- File: configurable `SQLITE_PATH`, default under data dir (e.g. `{DATA_DIR}/sarca.sqlite`).
- PRAGMAs: `journal_mode=WAL`, `foreign_keys=ON`, sensible `busy_timeout`.
- Fresh schema on first start. No Postgres import path.

### Access model

- Replace multiple `PgPool` instances with **one** shared `SqlitePool` used by HTTP and background tasks.
- Keep `max_connections` modest (e.g. 4–8). Writers serialize naturally; avoid pretending many concurrent DB writers exist.

### Dialect mapping

| Postgres today | SQLite approach |
|---|---|
| ENUM types | `TEXT` + CHECK (or plain TEXT) |
| `gen_random_uuid()` | generate in Rust (`uuid`) |
| plpgsql `updated_at` triggers | SQLite `UPDATE OF …` triggers |
| Partial unique index on live files | SQLite partial unique index |
| `FOR UPDATE … SKIP LOCKED` (purge claim) | short `BEGIN IMMEDIATE` + claim `UPDATE … WHERE id=(SELECT … LIMIT 1)` |
| `ILIKE` | `LIKE` + `COLLATE NOCASE` / `lower()` |
| `SPLIT_PART` / `FILTER` / `ANY($1)` | rewrite in SQL or Rust |
| `sqlx` `postgres` feature | `sqlite` feature |

- Keep inline `init_db` style; add `schema_version` table for forward migrations.
- Remove `DATABASE_USER/PASSWORD/HOST/PORT/NAME` from config.

### Performance expectation

- Home / small multi-user: metadata DB is not the bottleneck vs Telegram I/O; SQLite is acceptable.
- Concurrent upload + purge + listing may contend on the write lock; mitigate with short transactions and the single-pool model.

## 2. Telegram: official Bot API only

### Keep

- Telegram channels as durable blob store (`sendDocument` / `getFile` / `copyMessage` / `deleteMessage` / `getChat`).
- Multi-channel replication, channel health, storage workers / rate limiting.
- Chunking of large files into multiple documents.

### Remove

- `telegram-bot-api` container, entrypoint, data volume.
- `TELEGRAM_LOCAL_API`, `TELEGRAM_API_ID`, `TELEGRAM_API_HASH`, and local base URL wiring.
- Local filesystem download path / `cleanup_local_bot_api_copy` / related permission hacks.
- Install prompts for `api_id` / `api_hash`.

### Defaults

- Always `https://api.telegram.org`.
- Default (and effective) document chunk size **20MB**; video chunks also capped at **≤20MB** (no 1950MB / Local API path). Official `getFile` limit is the constraint.

## 3. Transport: HTTP/3 + TCP fallback + CID

### Server listeners

| Bind | Role |
|---|---|
| UDP/443 | HTTP/3 (`quinn` + `h3` + TLS 1.3) |
| TCP/443 | HTTPS HTTP/1.1 fallback (same Axum router) |
| TCP/80 | ACME `http-01` + redirect to `https://` |

- Terminate TLS in-process (no Caddy).
- Likely Axum upgrade from 0.6 to a version compatible with the chosen H3 adapter.
- Enable QUIC connection migration (Connection ID) so **client IP changes** can continue the connection without a full new handshake.
- **Server IP change** is not solved by CID: re-issue cert for the new IP and clients reconnect to the new address.

### Clients

- Native (`sarca-sync` / Tauri): HTTPS client with HTTP/3 preference; fall back to TCP HTTPS on QUIC failure/timeout; log selected protocol.
- Web UI: same-origin `https://` + `fetch`; browser selects H3 when UDP works and cert is valid. No raw QUIC in JS.
- Production URLs are `https://` (HTTP only for ACME/:80 redirect and explicit dev/test).

### Degradation

- If UDP/443 is blocked, product remains usable over TCP HTTPS.
- Optional logging/metrics: `protocol=h3|http1`.

## 4. ACME / TLS / install

### Identity selection (install)

1. Ask whether a public domain exists.
2. If yes → `TLS_HOSTNAME=<domain>` (DNS SAN).
3. If no → detect external IP → `TLS_HOSTNAME=<ip>` (IP SAN).

### Certificate policy

- Let's Encrypt **`shortlived`** profile for **both** domain and IP (~6 days / 160 hours).
- Renew **one day before `notAfter`** (≈ every ~5 days for short-lived certs).
- Challenge: **`http-01`** on port 80 (required for IP; also used for domain).
- ACME client lives **inside `sarca`** (not certbot).
- Persist account/cert/key under `{DATA_DIR}/certs/`.
- Hot-reload cert into H3 + TCP TLS stacks after renew without process restart.
- If running in IP mode and the public IP changes, obtain a new cert for the new IP and update `TLS_HOSTNAME` / `PUBLIC_BASE_URL` accordingly.

### Config shape (illustrative)

```
HTTPS_ADDR=0.0.0.0:443
ACME_HTTP_ADDR=0.0.0.0:80
TLS_HOSTNAME=example.com   # or dotted IP
ACME_DIRECTORY=https://acme-v02.api.letsencrypt.org/directory
SQLITE_PATH=/var/lib/sarca/sarca.sqlite
WORK_DIR=/var/lib/sarca/work
PUBLIC_BASE_URL=https://example.com
TELEGRAM_CHUNK_SIZE_MB=20
TELEGRAM_RATE_LIMIT=18
# no DATABASE_*, TELEGRAM_LOCAL_*, TELEGRAM_API_ID/HASH
```

Dev may bind high ports without capabilities; production documents `CAP_NET_BIND_SERVICE` / systemd ambient caps / root as needed.

### Install path (primary = binary)

1. Install binary + UI assets.
2. Prompt admin email/password.
3. Prompt domain or auto IP.
4. Write `sarca.conf`, create data dirs.
5. Document firewall: `80/tcp`, `443/tcp`, **`443/udp`**.
6. Start `sarca` (optional systemd unit).
7. Docker Compose remains optional/secondary; not the default mental model.

## 5. Testing and acceptance

### Automated

| Layer | Coverage |
|---|---|
| Unit | SQLite init/schema; chunk size ≤20MB without local API; renew schedule `notAfter - 1 day`; DNS vs IP ACME identifier |
| Integration | Same router over H3 and TCP; metadata writes on SQLite; purge claim without PG `SKIP LOCKED` |
| ACME | Pebble or LE staging / mock ACME directory in CI |
| Client | Prefer H3; fall back to TCP when QUIC unavailable |
| E2E | Binary smoke: start → login → upload file requiring multiple 20MB chunks → download; no Postgres / Local Bot API |

### Manual / documented

- Client network change (QUIC migration) when feasible.
- Server public IP change → new cert + client URL update.

### Acceptance criteria

- Primary path runs as one binary + SQLite file + work/certs dirs; no Postgres or Local Bot API required.
- Telegram remains the durable blob backend (official API, 20MB chunks, existing multi-channel behavior).
- In a normal network, native sync prefers HTTP/3; TCP fallback works when UDP is blocked.
- Certs auto-renew one day before expiry (`shortlived`).
- Memory footprint lower than previous multi-service stack on a comparable workload (record before/after).
- Tests above pass before release claims.

## Implementation order

1. SQLite cutover + wipe/fresh schema + repository dialect changes.
2. Remove Local Bot API paths; pin 20MB official API.
3. TLS + in-process ACME + dual listeners (H3 + TCP) + :80.
4. Native client prefer-H3 + fallback; UI stays same-origin HTTPS.
5. Install script / config / docs for binary-first deploy.
6. Full test pass and memory/smoke verification.

## Open implementation notes (not open product questions)

- Exact Rust crates for H3↔Axum bridging and ACME client — choose during planning for Axum compatibility and maintenance.
- Whether TCP fallback also offers HTTP/2 — optional, not required for acceptance.
- Optional Compose file may be rewritten later as a thin single-service wrapper; not blocking binary path.
