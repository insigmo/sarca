# HTTP/3 Clients + Alt-Svc + Boot/CI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prefer HTTP/3 for native clients (one TCP fallback), advertise HTTP/3 to browsers via `Alt-Svc`, fix SQLite superuser boot panic, and bump CI actions off Node 20 where possible.

**Architecture:** Keep dual-listen TCP HTTPS + QUIC HTTP/3. Add one Axum/tower response header layer for `Alt-Svc: h3=":<https_port>"; ma=86400`. Clients keep existing `sarca-sync` prefer-H3 path. Map SQLite unique violations with `is_unique_violation()`.

**Tech Stack:** Rust, Axum, tower-http `SetResponseHeaderLayer`, sqlx SQLite, reqwest HTTP/3 (`http3-client` + `reqwest_unstable`), GitHub Actions.

## Global Constraints

- Prefer HTTP/3; one TCP HTTPS fallback on connect/timeout/request error — do not remove fallback.
- Never hardcode Alt-Svc port `443`; use `config.https_addr.port()`.
- No website JS changes.
- Do not change ACME, dual-listen, or QUIC cert hot-reload behavior.
- Do not invent forks for GH Actions; bump only published Node-24-capable releases.
- GHCR visibility / `PACKAGES_TOKEN` are out of scope.
- Spec: `docs/superpowers/specs/2026-07-31-http3-clients-altsvc-design.md`
- Do not ask the user clarifying questions; implement, test, commit per task.
- Work from repo root `/home/beta/git/sarca`.

---

## File Structure

| Path | Responsibility |
|---|---|
| `sarca/src/repositories/users.rs` | Map SQLite UNIQUE → `AlreadyExists`; permanent unit test |
| `sarca/src/startup.rs` | Idempotent `create_superuser` (AlreadyExists → password sync) |
| `sarca/src/server.rs` | Apply `Alt-Svc` header layer to shared router |
| `sarca/tests/http3_smoke.rs` or new test | Assert TCP response includes `Alt-Svc` with bound port |
| `crates/sarca-sync/Cargo.toml`, `client/src-tauri/Cargo.toml`, `.cargo/config.toml` | Verify H3 flags; fix gaps only |
| `.github/workflows/client.yml`, `release.yml`, `docker-image.yml` | Bump actions |
| `README.md` | Confirm UDP/HTTP/3 note (already present — only edit if missing) |

---

### Task 1: SQLite unique → AlreadyExists + idempotent superuser

**Files:**
- Modify: `sarca/src/repositories/users.rs`
- Modify: `sarca/src/startup.rs` (only if needed; AlreadyExists branch already exists)
- Remove: any `#region agent log` debug NDJSON / `debug_unique_probe` module left from debugging

**Interfaces:**
- Consumes: `sqlx::Error::Database(dbe)`, `dbe.is_unique_violation()`
- Produces: `UsersRepository::create` returns `Err(SarcaError::AlreadyExists(_))` on duplicate email
- Produces: `create_superuser` does not panic when email already exists

- [ ] **Step 1: Replace WIP with a permanent failing-then-passing test**

Remove `mod debug_unique_probe` and all `#region agent log` blocks in `users.rs` / `startup.rs`.

Add a real unit test in `users.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::db::pool::get_pool;
    use crate::startup::{create_superuser, init_db};
    use crate::config::Config;
    use std::time::Duration;

    #[tokio::test]
    async fn create_duplicate_email_maps_to_already_exists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.sqlite");
        let pool = get_pool(path.to_str().unwrap(), 4, Duration::from_secs(5))
            .await
            .unwrap();
        init_db(&pool).await;

        let email = "admin@example.com";
        let u1 = InDBUser::new_password(email.into(), "h1".into());
        UsersRepository::new(&pool).create(u1).await.unwrap();

        let u2 = InDBUser::new_password(email.into(), "h2".into());
        let err = UsersRepository::new(&pool).create(u2).await.unwrap_err();
        assert!(matches!(err, SarcaError::AlreadyExists(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn create_superuser_twice_does_not_panic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.sqlite");
        let pool = get_pool(path.to_str().unwrap(), 4, Duration::from_secs(5))
            .await
            .unwrap();
        init_db(&pool).await;

        // Build a minimal Config the same way other startup tests do, or construct
        // only the fields create_superuser needs if a test helper exists.
        // Prefer: load from env with SUPERUSER_EMAIL / SUPERUSER_PASS set under a mutex
        // if Config::from_env is the project pattern; otherwise set fields explicitly.
        let mut config = /* obtain Config with known superuser_email/pass */;
        create_superuser(&pool, &config).await;
        create_superuser(&pool, &config).await; // must not panic
    }
}
```

If constructing full `Config` is awkward, skip the second test and only keep `create_duplicate_email_maps_to_already_exists` plus a focused match-arm test is enough — but prefer both.

- [ ] **Step 2: Run test — expect pass if fix already present, else fail with Unknown**

Run: `cargo test -p sarca --lib repositories::users::tests::create_duplicate_email_maps_to_already_exists -- --nocapture`

Expected (before fix): `got Unknown` assertion failure  
Expected (after fix already in tree): PASS

- [ ] **Step 3: Implement mapping (if not already)**

In `UsersRepository::create` `map_err`:

```rust
.map_err(|e| {
    match e {
        sqlx::Error::Database(dbe) if dbe.is_unique_violation() => {
            SarcaError::AlreadyExists("user with given email".into())
        },
        _ => {
            tracing::error!("{e}");
            SarcaError::Unknown
        },
    }
})?;
```

Do **not** match on `constraint() == Some("users_email_key")`.

Keep `create_superuser` AlreadyExists → `update_password_hash` branch; remove debug file I/O.

- [ ] **Step 4: Run tests + fmt**

```bash
cargo +nightly fmt -p sarca
cargo test -p sarca --lib repositories::users::tests -- --nocapture
```

Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add sarca/src/repositories/users.rs sarca/src/startup.rs
git commit -m "$(cat <<'EOF'
fix: map SQLite unique email to AlreadyExists for superuser boot

EOF
)"
```

---

### Task 2: Alt-Svc response header

**Files:**
- Modify: `sarca/src/server.rs`
- Modify: `sarca/src/main.rs` (pass `https_addr.port()` into router build if needed)
- Modify: `sarca/tests/http3_smoke.rs` (or add `sarca/tests/alt_svc.rs`)

**Interfaces:**
- Consumes: `u16` HTTPS listen port
- Produces: every response from the shared router includes  
  `Alt-Svc: h3=":<port>"; ma=86400`  
  (exact spacing: `h3=":PORT"; ma=86400`)

- [ ] **Step 1: Write failing test**

Extend TLS smoke so a **TCP** HTTPS GET to `/health` asserts the header. Pattern after existing `http3_smoke.rs` (reqwest rustls client against `Server::health_router` dual-served, or a smaller helper that only TCP-serves the health router with the Alt-Svc layer).

```rust
#[tokio::test]
async fn tcp_https_response_advertises_alt_svc() {
    // bind ephemeral port, serve health router + Alt-Svc layer over TCP TLS
    // GET https://127.0.0.1:{port}/health
    let alt = resp.headers().get("alt-svc").expect("Alt-Svc missing");
    let expected = format!("h3=\":{}\"; ma=86400", port);
    assert_eq!(alt.to_str().unwrap(), expected);
}
```

- [ ] **Step 2: Run test — expect fail**

Run: `cargo test -p sarca --test http3_smoke tcp_https_response_advertises_alt_svc -- --nocapture`  
(or `--test alt_svc` if separate file)

Expected: FAIL — header missing

- [ ] **Step 3: Implement layer**

Change `Server::build_server` (and `health_router` used in TLS tests) to accept the HTTPS port, or add:

```rust
pub fn with_alt_svc(router: Router, https_port: u16) -> Router {
    let value = HeaderValue::from_str(&format!("h3=\":{https_port}\"; ma=86400"))
        .expect("Alt-Svc header");
    router.layer(SetResponseHeaderLayer::overriding(
        axum::http::header::HeaderName::from_static("alt-svc"),
        value,
    ))
}
```

Apply in `build_server` and `run_tls` path so UI + `/api` both get it. For plain HTTP `run()` (e2e without TLS), either omit Alt-Svc or still set it from `config.https_addr.port()` — prefer **only attach when serving TLS** so plain e2e is unchanged: apply in `run_tls` before `serve_dual_tls`, and on `health_router` in tests.

```rust
// in run_tls:
let router = with_alt_svc(self.router, runtime.https_addr.port());
serve_dual_tls(router, ui_dir, runtime, acme_task).await;
```

If `HeaderValue::from_str` cannot fail for a `u16` port, `.expect` is fine.

- [ ] **Step 4: Run test — expect pass**

```bash
cargo +nightly fmt -p sarca
cargo test -p sarca --test http3_smoke -- --nocapture
```

Expected: PASS (H3 probe may skip in some envs — TCP Alt-Svc test must pass)

- [ ] **Step 5: Commit**

```bash
git add sarca/src/server.rs sarca/src/main.rs sarca/tests/
git commit -m "$(cat <<'EOF'
feat: advertise HTTP/3 via Alt-Svc on TLS responses

EOF
)"
```

---

### Task 3: Verify client HTTP/3 build flags

**Files:**
- Read/verify: `crates/sarca-sync/Cargo.toml`, `crates/sarca-sync/src/api.rs`, `client/src-tauri/Cargo.toml`, `.cargo/config.toml`
- Modify: only if a target disables H3 contrary to the spec

**Interfaces:**
- Produces: `HTTP3_PREFERRED == true` in default `sarca-sync` build (`http3-client` + `reqwest_unstable`)

- [ ] **Step 1: Run existing preference test**

```bash
cargo test -p sarca-sync http3_preference_enabled_in_default_build -- --nocapture
```

Expected: PASS

- [ ] **Step 2: Confirm Cargo defaults**

- `sarca-sync` default features include `http3-client`
- `.cargo/config.toml` has `reqwest_unstable` for host + android/ios targets
- `sarca-client` depends on `sarca-sync` with defaults (no `default-features = false`)

If all true → no code change; write a short note in the commit body or skip commit and mark task complete with evidence in the report.

- [ ] **Step 3: Fix gaps only if Step 1/2 fail**

Restore feature / rustflags; re-run Step 1.

- [ ] **Step 4: Commit only if files changed**

```bash
git add crates/sarca-sync/Cargo.toml client/src-tauri/Cargo.toml .cargo/config.toml
git commit -m "$(cat <<'EOF'
fix: keep HTTP/3 preference enabled on client targets

EOF
)"
```

---

### Task 4: CI action bumps + README check

**Files:**
- Modify: `.github/workflows/client.yml`
- Modify: `.github/workflows/release.yml`
- Modify: `.github/workflows/docker-image.yml`
- Modify: `README.md` only if UDP/HTTP/3 firewall note is missing (it already mentions `443/udp`)

**Interfaces:**
- Produces: `android-actions/setup-android@v4` (Node 24)
- Produces: latest available Node-24-capable pins for `actions/setup-java`, `softprops/action-gh-release`, `docker/*` if published; otherwise leave and note in commit message

- [ ] **Step 1: Bump setup-android**

In `client.yml` and `release.yml`:

```yaml
- uses: android-actions/setup-android@v4
```

- [ ] **Step 2: Check other actions for Node 24 releases**

Look up current tags for:
- `actions/setup-java` (stay `@v4` if already Node 24, else bump patch)
- `softprops/action-gh-release@v2` → newest v2.x if Node 24
- `docker/setup-qemu-action`, `docker/setup-buildx-action`, `docker/login-action`, `docker/build-push-action`

Bump only when a release notes Node 24 / removes Node 20. Do not pin arbitrary SHAs unless the repo already does.

- [ ] **Step 3: README**

If README already has `443/udp` (HTTP/3), do not edit. Otherwise add one sentence under production TLS ports.

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/client.yml .github/workflows/release.yml .github/workflows/docker-image.yml README.md
git commit -m "$(cat <<'EOF'
ci: bump Actions off Node 20 where upstream allows

EOF
)"
```

---

## Spec coverage (self-review)

| Spec requirement | Task |
|---|---|
| SQLite unique → AlreadyExists; no boot panic | Task 1 |
| Remove debug instrumentation | Task 1 |
| Alt-Svc with dynamic port | Task 2 |
| Clients prefer H3 + one TCP fallback | Task 3 (verify existing) |
| Website unchanged; README UDP note | Task 4 |
| CI Node 20 warnings | Task 4 |
| No strict H3-only / no TCP disable | Global Constraints |

---
