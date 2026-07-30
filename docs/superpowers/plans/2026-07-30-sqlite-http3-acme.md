# SQLite + HTTP/3 + ACME Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship Sarca as a single binary with SQLite metadata, official Telegram Bot API (≤20MB chunks), in-process HTTP/3+TCP HTTPS+ACME, and QUIC-preferring native clients—no Postgres, Local Bot API, or Caddy.

**Architecture:** One `sarca` process owns SQLite (WAL), Axum router dual-served on QUIC/HTTP3 (UDP) and HTTPS/HTTP1.1 (TCP), ACME http-01 on :80, and existing Telegram upload/replication workers. Wipe-only DB cutover. Binary-first install prompts domain-or-IP for LE `shortlived` certs renewed at `notAfter - 1 day`.

**Tech Stack:** Rust, sqlx (sqlite), Axum (upgrade as needed for H3), quinn + h3 + h3-quinn (or axum-h3/h3-axum), rustls, in-process ACME (instant-acme or equivalent), reqwest with HTTP/3 on `sarca-sync`, SolidJS UI unchanged (browser H3).

## Global Constraints

- Telegram remains the durable blob store; only Local Bot API / large-chunk mode is removed.
- Document and video chunks ≤ 20MB.
- No Caddy; TLS + HTTP/3 + ACME terminate inside `sarca`.
- No Postgres→SQLite data migration (fresh schema / wipe).
- Binary-first install; Compose optional/secondary.
- Clients prefer HTTP/3; TCP HTTPS fallback required.
- LE profile `shortlived` for domain and IP; renew at `notAfter - 1 day`.
- Multi-channel, replication, channel health, bot workers stay.
- Do not ask the user clarifying questions; implement, test, commit per task.
- Acceptance checklist: `.cursor/acceptance/2026-07-30-sqlite-http3-acme.md`
- Spec: `docs/superpowers/specs/2026-07-30-sqlite-http3-acme-design.md`

---

## File Structure

| Path | Responsibility |
|---|---|
| `sarca/src/config.rs` | `SQLITE_PATH`, TLS/ACME addrs, no `DATABASE_*` / local bot flags |
| `sarca/src/common/db/pool.rs` | `SqlitePool` factory + WAL pragmas |
| `sarca/src/startup.rs` | SQLite `init_db`, drop Postgres `create_db` |
| `sarca/src/repositories/*.rs` | All SQL via `SqlitePool` / sqlite dialect |
| `sarca/src/common/telegram_api/bot_api.rs` | Official API only (HTTP getFile) |
| `sarca/src/tls/` (new) | Cert store, ACME client, renew scheduler, hot-reload hook |
| `sarca/src/server.rs` | Dual listen H3+TCP; :80 ACME+redirect |
| `crates/sarca-sync/src/api.rs` | Prefer HTTP/3 client |
| `install.sh`, `sarca.conf.example`, `.github/workflows/e2e.yml`, `README.md` | Binary-first + sqlite CI |

---

### Task 1: SQLite config + pool

**Files:**
- Modify: `sarca/Cargo.toml` (sqlx features)
- Modify: `sarca/src/config.rs`
- Modify: `sarca/src/common/db/pool.rs`
- Modify: `sarca/src/common/db/mod.rs`, `errors.rs` if PG-specific
- Modify: `sarca/src/common/routing/app_state.rs` (`Pool<Sqlite>`)
- Modify: `sarca/src/main.rs` (single pool create; clone for workers; remove `create_db`)
- Test: unit tests in `config.rs` / `pool.rs`

**Interfaces:**
- Produces: `Config { sqlite_path: String, ... }` (remove `db_uri`, `db_uri_without_dbname`, `db_name`)
- Produces: `pub async fn get_pool(path: &str, max_connections: u32, timeout: Duration) -> Result<SqlitePool, String>`
- Produces: after connect, run `PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;`

- [ ] **Step 1: Write failing config test**

```rust
#[test]
fn sqlite_path_from_env() {
    // set SQLITE_PATH via mutex-guarded env test helper used in this crate,
    // or test a pure fn parse_sqlite_path(default_workdir) 
    assert!(Config::default_sqlite_path("work").ends_with("sarca.sqlite"));
}
```

- [ ] **Step 2: Run test — expect fail**

Run: `cargo test -p sarca default_sqlite_path --lib`
Expected: compile/link fail or test fail (fn missing)

- [ ] **Step 3: Implement config + pool**

- `SQLITE_PATH` env with default `{work_dir}/sarca.sqlite` or `DATA_DIR/sarca.sqlite` if you introduce `DATA_DIR`; prefer default `work/sarca.sqlite` beside `WORK_DIR` unless `SQLITE_PATH` set.
- sqlx: `features = ["runtime-tokio", "tls-rustls", "sqlite", "uuid", "chrono"]` — remove `postgres`.
- Replace every `PgPool` / `Pool<Postgres>` in **this task's files only** (`app_state`, `pool`, `main` pool construction). Services/repos may not compile until Task 3 — keep Task 1 compiling by temporarily aliasing or completing the mechanical rename of types in signatures that `main` needs. Prefer finishing type rename across crate in Task 1 if `cargo check` demands it, but leave dialect SQL for Task 3.

If `cargo check` fails on hundreds of `PgPool` refs, do mechanical `PgPool`→`SqlitePool` and `Postgres`→`Sqlite` in all files in this task, leaving SQL strings for Task 2–3.

- [ ] **Step 4: `cargo check -p sarca` (or test pool)**

Run: `cargo test -p sarca default_sqlite_path --lib`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add sarca/Cargo.toml sarca/src/config.rs sarca/src/common/db sarca/src/common/routing/app_state.rs sarca/src/main.rs
git commit -m "refactor: switch database config and pool to SQLite"
```

---

### Task 2: SQLite schema in startup.rs

**Files:**
- Modify: `sarca/src/startup.rs` — rewrite `init_db`; delete Postgres `create_db`
- Modify: `sarca/src/main.rs` — call `init_db` only
- Test: `sarca/tests/sqlite_init.rs` (new) or `#[cfg(test)]` in startup

**Interfaces:**
- Produces: `pub async fn init_db(pool: &SqlitePool) -> SarcaResult<()>`
- Tables (same logical model as today): `schema_version`, `users`, `storages`, `storage_workers`, `access`, `files`, `app_settings`, `file_chunks`, `storage_workers_usages`, `storage_channels`, `chunk_replicas`, `favorites`, `recent_files`, `share_links`, `email_tokens`, `file_sync_events`, `storage_purge_jobs`, `storage_purge_messages`
- IDs: `TEXT` UUID strings; timestamps `TEXT` ISO or integer unix — match sqlx `chrono` + existing Rust types
- Partial unique: `CREATE UNIQUE INDEX files_path_storage_id_alive_uidx ON files(path, storage_id) WHERE deleted_at IS NULL`
- Triggers: SQLite `updated_at` + sync-event insert triggers (no plpgsql)
- `file_sync_events.id`: `INTEGER PRIMARY KEY AUTOINCREMENT`

- [ ] **Step 1: Failing test — init creates schema_version**

```rust
#[tokio::test]
async fn init_db_creates_schema_on_empty_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.sqlite");
    let pool = get_pool(path.to_str().unwrap(), 4, Duration::from_secs(5)).await.unwrap();
    init_db(&pool).await.unwrap();
    let v: i64 = sqlx::query_scalar("SELECT version FROM schema_version LIMIT 1")
        .fetch_one(&pool).await.unwrap();
    assert!(v >= 1);
    init_db(&pool).await.unwrap(); // idempotent
}
```

Add `tempfile` dev-dep if needed.

- [ ] **Step 2: Run — expect fail**

Run: `cargo test -p sarca init_db_creates_schema -- --nocapture`
Expected: FAIL

- [ ] **Step 3: Implement SQLite DDL**

Port statements from current `init_db` array; drop `DO $$` / `CREATE EXTENSION` / `CREATE TYPE` / `CREATE DATABASE`. Use `CREATE TABLE IF NOT EXISTS` + `schema_version`.

- [ ] **Step 4: Test pass**

Run: `cargo test -p sarca init_db_creates_schema`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git commit -am "feat: initialize SQLite schema instead of Postgres"
```

---

### Task 3: Repository dialect port

**Files:**
- Modify all under `sarca/src/repositories/`: `access.rs`, `app_settings.rs`, `chunk_replicas.rs`, `email_tokens.rs`, `favorites.rs`, `files.rs`, `recent_files.rs`, `share_links.rs`, `storage_channels.rs`, `storage_purge.rs`, `storage_workers.rs`, `storages.rs`, `sync.rs`, `users.rs`
- Modify services/routers still using Postgres-specific SQL or `Transaction<'_, Postgres>`
- Hotspots:
  - `storage_purge.rs`: replace `FOR UPDATE SKIP LOCKED` with claim pattern:
    ```sql
    BEGIN IMMEDIATE;
    UPDATE storage_purge_messages
    SET status = 'in_progress'
    WHERE id = (
      SELECT id FROM storage_purge_messages WHERE status = 'pending' LIMIT 1
    )
    RETURNING *;
    ```
    (or equivalent two-step in Rust transaction)
  - `files.rs`: `SPLIT_PART` → `CASE/instr` or Rust path split; `ILIKE` → `LIKE` + `COLLATE NOCASE`; `ANY($1)` → expand binds or `IN (...)`
  - `chunk_replicas.rs`: `COUNT(*) FILTER` → `SUM(CASE WHEN …)`

- [ ] **Step 1: `cargo check -p sarca` — list remaining PG SQL errors; fix until green**

Run: `cargo check -p sarca 2>&1`
Expected: success

- [ ] **Step 2: Add unit/integration test for purge claim + list_dir path prefix if feasible**

- [ ] **Step 3: `cargo test -p sarca --lib`**

Expected: PASS (skip e2e)

- [ ] **Step 4: Commit**

```bash
git commit -am "fix: port repositories to SQLite dialect"
```

---

### Task 4: Remove Local Bot API

**Files:**
- `sarca/src/common/telegram_api/bot_api.rs` — delete local FS download/cleanup branches
- `sarca/src/config.rs` — remove `TELEGRAM_LOCAL_API` branching; always `https://api.telegram.org`; default chunk 20; **video chunk default 20**
- `sarca/src/services/setup.rs`, `routers/setup.rs`, `schemas/setup.rs` — remove local-api phase/routes
- `sarca/src/repositories/app_settings.rs` — remove api_id/hash keys if only for local API
- `ui/src/pages/Setup/index.jsx`, `ui/src/api/index.js`
- `compose.yml` — remove `telegram-bot-api` service/volume; remove postgres if Task 8 not yet — at least remove bot-api here
- Delete or stop shipping `docker/telegram-bot-api-entrypoint.sh`
- `install.sh`, `sarca.conf.example`, `README.md`

- [ ] **Step 1: Test chunk defaults ≤20**

```rust
#[test]
fn video_chunk_default_capped_at_20() {
    assert!(/* default video chunk */ <= 20);
}
```

- [ ] **Step 2: Implement removals + cap**

- [ ] **Step 3: `cargo test -p sarca video_chunk` and bot_api tests (remove local_bot_api_cleanup_tests)**

- [ ] **Step 4: Commit**

```bash
git commit -am "feat: drop Local Bot API; cap Telegram chunks at 20MB"
```

---

### Task 5: TLS cert store + ACME schedule helpers

**Files:**
- Create: `sarca/src/tls/mod.rs`, `store.rs`, `acme.rs`, `renew.rs`
- Modify: `sarca/src/lib.rs` or `main.rs` module tree; `config.rs` for `HTTPS_ADDR`, `ACME_HTTP_ADDR`, `TLS_HOSTNAME`, `ACME_DIRECTORY`, `CERTS_DIR`
- Dev-deps as needed

**Interfaces:**
- `pub fn renew_at(not_after: DateTime<Utc>) -> DateTime<Utc>` → `not_after - Duration::days(1)`
- `pub enum TlsIdentity { Dns(String), Ip(IpAddr) }` with parse from `TLS_HOSTNAME`
- `pub struct CertStore { dir: PathBuf }` — load/save PEM
- ACME order logic may be stubbed with trait + mock in tests; real client wired in Task 6

- [ ] **Step 1: Failing tests for renew_at and identity parse**

- [ ] **Step 2: Implement helpers**

- [ ] **Step 3: Tests pass**

- [ ] **Step 4: Commit**

```bash
git commit -am "feat: add TLS identity and ACME renew scheduling helpers"
```

---

### Task 6: Dual listeners HTTP/3 + TCP + ACME :80

**Files:**
- `sarca/Cargo.toml` — add quinn, h3, h3-quinn (or axum-h3), rustls, instant-acme; bump axum if required
- `sarca/src/server.rs` — refactor `Server::run`
- `sarca/src/tls/acme.rs` — real ACME http-01 solver integrated with :80 listener
- `sarca/tests/http3_smoke.rs` — self-signed cert, bind high ports, GET over H3 and TCP

**Behavior:**
- UDP HTTPS_ADDR → HTTP/3
- TCP HTTPS_ADDR → HTTP/1.1 TLS
- TCP ACME_HTTP_ADDR → `/.well-known/acme-challenge/*` + redirect elsewhere to HTTPS
- Enable QUIC connection migration on server endpoint config
- Hot-reload certs on renew

- [ ] **Step 1: Integration test scaffolding with rcgen self-signed**

- [ ] **Step 2: Implement dual serve**

- [ ] **Step 3: Test H3 + TCP both return 200 for health or `/`**

Run: `cargo test -p sarca --test http3_smoke`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git commit -am "feat: serve Axum over HTTP/3 and TCP HTTPS with ACME port"
```

---

### Task 7: Native client prefer HTTP/3

**Files:**
- `crates/sarca-sync/Cargo.toml` — enable reqwest `http3` if available, or dedicated H3 client path
- `crates/sarca-sync/src/api.rs` — build client preferring H3; fallback TCP; record protocol
- Tests: feature-gated or mock

- [ ] **Step 1: Test that client builder enables HTTP/3 preference (assert config flags / try H3 against Task 6 smoke server if shared)**

- [ ] **Step 2: Implement**

- [ ] **Step 3: `cargo test -p sarca-sync`**

- [ ] **Step 4: Commit**

```bash
git commit -am "feat: prefer HTTP/3 in sarca-sync with TCP fallback"
```

---

### Task 8: install.sh + conf + CI/e2e

**Files:**
- `install.sh` — binary default; prompt domain or detect IP; write `SQLITE_PATH`, `TLS_HOSTNAME`, `HTTPS_ADDR`, `ACME_HTTP_ADDR`; remove api_id/hash and Postgres requirements
- `sarca.conf.example` — match new config
- `compose.yml` / `compose.dev.yml` — optional single-service or mark secondary; no postgres/bot-api required for primary docs
- `.github/workflows/e2e.yml` — remove Postgres service; point app at sqlite file; `TELEGRAM_CHUNK_SIZE_MB=20`
- `README.md`, `scripts/db-reset.sh` (sqlite delete file), `Taskfile.yml` as needed

- [ ] **Step 1: Update conf example + install prompts**

- [ ] **Step 2: Fix e2e workflow**

- [ ] **Step 3: Commit**

```bash
git commit -am "chore: binary-first install with SQLite and ACME config"
```

---

### Task 9: Verification gate

- [ ] **Step 1: Run**

```bash
cargo test -p sarca
cargo test -p sarca-sync
```

- [ ] **Step 2: Fill acceptance report evidence into `.cursor/acceptance/2026-07-30-sqlite-http3-acme.md` (status verifying→done or leave fail notes)**

- [ ] **Step 3: Commit any test fixes**

```bash
git commit -am "test: verify SQLite HTTP/3 ACME acceptance criteria"
```

---

## Spec coverage checklist

| Spec item | Task |
|---|---|
| SQLite WAL + single pool | 1–2 |
| Schema wipe / no PG migration | 2 |
| Repo dialect | 3 |
| Remove Local Bot API; chunks ≤20 | 4 |
| Keep Telegram multi-channel | 4 (do not remove) |
| ACME shortlived renew −1 day | 5–6 |
| Domain or IP identity | 5, 8 |
| H3 + TCP fallback; CID migration | 6 |
| Client prefer QUIC | 7 |
| Binary-first install | 8 |
| Tests / acceptance | 9 |
