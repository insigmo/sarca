# Design: HTTP/3 clients + website Alt-Svc + boot/CI hardening

Date: 2026-07-31  
Approach: **A** (minimal stable)  
Status: approved in chat; awaiting spec file review

## Goal

- Native clients (sarca-sync / Tauri desktop / mobile) talk to the server preferring **HTTP/3**, with **one TCP HTTPS fallback** on connect/timeout/request failure.
- The **website** stays on normal browser HTTPS; when UDP is available the browser upgrades via **`Alt-Svc`**.
- Fix SQLite boot panic when ensuring the superuser on an existing DB.
- Reduce GitHub Actions Node.js 20 deprecation warnings by bumping actions where a Node 24-capable release exists.

## Non-goals

- Strict HTTP/3-only (no TCP fallback for clients).
- Disabling TCP HTTPS on `:443` or requiring HTTPS DNS RR for first browser visit.
- Changing website JS to force QUIC (browsers do not expose that).
- Fixing GHCR package visibility / `PACKAGES_TOKEN` secrets (operational, not code).
- Redesigning ACME, cert hot-reload, or QUIC cert-reload (QUIC still needs process restart on renew).

## Architecture

```
Browser ──TCP HTTPS──► Sarca (:HTTPS_ADDR TCP)
   │                      │ Alt-Svc: h3=":<port>"; ma=86400
   └──(upgrade)──QUIC────► Sarca (:HTTPS_ADDR UDP)  HTTP/3

Native client ──QUIC H3 (prior knowledge)──► Sarca
       │ on connect/timeout/request error (once)
       └──TCP HTTPS (h2/1.1)───────────────► Sarca
```

Server continues dual-listen: TCP TLS + QUIC HTTP/3 on `HTTPS_ADDR`, ACME/redirect on `ACME_HTTP_ADDR`.

## Server: Alt-Svc

1. Add a response header on the shared Axum router:
   - `Alt-Svc: h3=":<port>"; ma=86400`
   - `<port>` is `config.https_addr.port()` (never hardcode `443`; dev uses high ports).
2. Apply via one tower/axum layer so **UI + `/api`** both advertise it.
3. Sending the same header on HTTP/3 responses is fine (idempotent advertisement).
4. Do not change dual-listen, ACME `:80`, or cert reload behavior.

### Test

- Integration/unit: a TCP HTTPS response includes `Alt-Svc` with the bound port.

## Clients: prefer H3 + one TCP fallback

Existing `crates/sarca-sync` path stays the source of truth:

- Feature `http3-client` + `reqwest_unstable` → `HTTP3_PREFERRED`.
- H3 client: `http3_prior_knowledge()`.
- Separate TCP client for fallback.
- `send_preferring_h3`: try H3 once; on connect/timeout/request error → one TCP send.
- Log response protocol at info (`HTTP/3` / `HTTP/2` / …).

Work in this change set:

- Verify flags/`Cargo.toml`/`.cargo/config.toml` so desktop + Android/iOS builds keep H3 preference enabled.
- No protocol redesign; only fix build gaps if a target accidentally disabled H3.

### Website

- No UI code changes.
- Document in README (short): firewall must allow **443/udp** (or the configured HTTPS UDP port) for browser HTTP/3.

## Boot: superuser UNIQUE on SQLite

Root cause: `UsersRepository::create` mapped duplicate email only when `constraint() == Some("users_email_key")` (Postgres-style). SQLite reports `constraint() = None`, `is_unique_violation() = true`, code `2067` → `SarcaError::Unknown` → `create_superuser` panics.

Fix:

- Map `dbe.is_unique_violation()` → `SarcaError::AlreadyExists(...)` (same pattern as `files.rs`).
- Keep `create_superuser` AlreadyExists branch: sync password hash from config, do not panic.
- Remove temporary debug instrumentation / probe module after verification.
- Test: second insert same email → `AlreadyExists`; repeated `create_superuser` does not panic.

## CI: Node 20 deprecation warnings

Bump workflow actions to Node 24-capable releases where available, at least:

| Current | Target (as of 2026-07) |
|---------|-------------------------|
| `android-actions/setup-android@v3` | `@v4` |
| `actions/setup-java@v4` | latest patch that runs on Node 24, if published |
| `softprops/action-gh-release@v2` | latest that runs on Node 24, if published |
| `docker/setup-*`, `docker/login-action`, `docker/build-push-action` | latest majors/minors that advertise Node 24 |

If a dependency has no Node 24 release yet, leave it and note the remaining warning—do not invent forks.

Out of scope for code: GHCR “Change visibility → Public” and `PACKAGES_TOKEN` annotations.

## Error handling / stability

- Clients: one H3 attempt, then TCP; do not loop.
- Server: Alt-Svc misconfiguration must not break TCP serving (header build failure → log + omit header, or fail tests at build time with a valid static construction from port).
- Boot: only non-AlreadyExists create errors may still terminate the process.

## Success criteria

1. Existing SQLite DB with superuser: `task restart` / container boot completes past “ensuring superuser” without panic.
2. TCP HTTPS responses include `Alt-Svc` with the correct port.
3. Default sync client build still has `HTTP3_PREFERRED == true` and prefers H3 with TCP fallback.
4. Website unchanged; README mentions UDP for HTTP/3.
5. CI run annotations for Node 20 reduced for actions we can bump; remaining are documented as upstream-limited.

## Implementation order

1. Land SQLite unique → AlreadyExists (unblocks docker/dev restart).
2. Add Alt-Svc layer + test.
3. Verify client H3 build flags; fix gaps only.
4. Bump CI actions; README one-liner if missing.
5. Remove debug instrumentation after boot verification.
