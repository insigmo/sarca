# Sarca end-to-end suite

Full-stack tests: a real `sarca` server process, a real client sync engine, and a
fake (or real) Telegram Bot API. Nothing is stubbed inside the server — uploads go
through multipart → spool → chunking → "Telegram" → SQLite, and downloads come back
the same way.

## Running

```sh
task e2e                       # build + run everything (recommended)
task e2e:gui                   # only the GUI suite, driving the real client
cd e2e && ../.venv/bin/python -m pytest -q          # if the venv already exists
pytest -q test_02_upload_download.py -k hash        # one file / one test
pytest -q -m "not slow"                             # skip the slow scenarios
```

The suite builds `target/release/sarca` on first use. Skip that with
`SARCA_SKIP_BUILD=1` (reuses the existing binary) or point at another one with
`SARCA_BIN=/path/to/sarca`.

### Modes

| Mode | How | What happens |
| --- | --- | --- |
| Hermetic (default) | `pytest` | A fake Bot API starts on an ephemeral port; the server runs with `TELEGRAM_API_BASE_URL` pointing at it, its own `WORK_DIR`, SQLite file and port. |
| Real Telegram | `SARCA_E2E_TELEGRAM=real SARCA_E2E_BOT_TOKEN=123:AA… SARCA_E2E_CHAT_IDS=-1001234567890 pytest` | Same server, real `api.telegram.org`. Tests marked `mock_only` are skipped; expect flood-wait pauses. |
| External server | `SARCA_BASE_URL=http://127.0.0.1:8001 SUPERUSER_EMAIL=… SUPERUSER_PASS=… pytest` | No process management; runs against an already-running deployment. |

Useful knobs: `SARCA_E2E_KEEP_TMP=1` keeps the temp dir (server log, WORK_DIR,
fake-Telegram documents) after the run, `SARCA_E2E_RUST_LOG` overrides `RUST_LOG`.

## Scenarios

| File | Scenario |
| --- | --- |
| `test_01_storages.py` | Storage lifecycle: create, list, bot binding via `getMe`, channel add/remove, name/chat-id conflicts, rename, delete, auth. |
| `test_02_upload_download.py` | Upload → download round trips with SHA-256 equality: small, binary, empty, 1-byte, multi-chunk, unicode names, nested folders, Range requests (incl. across a chunk boundary), folder ZIP, client-supplied `content_hash`, progress stream phases, chunk-cache reuse. |
| `test_03_autoupload.py` | Auto-upload driven by the real `sarca-sync` engine (`cargo run -p sarca-sync --example headless`): media-only discovery, byte integrity, idempotent second pass, new/edited files, nested folders, folder-upload mode, one-way behaviour, previews for client uploads. |
| `test_04_http3.py` | A TLS instance (self-signed, ACME off): HTTPS over TCP, `Alt-Svc`, then real QUIC requests via aioquic — login, authenticated API, UI, bad credentials — plus the ACME/redirect listener. |
| `test_05_users.py` | Create user, log in, share a storage, revoke; `DELETE /api/users/{id}` incl. token invalidation, purge of storages the user alone owned, superuser protection, permission checks. |
| `test_06_settings.py` | Trash retention (get/set/validation/persistence/actual purge via log), chunk-size setting → document count, multi-channel replication via `copyMessage`, `TELEGRAM_RATE_LIMIT` throttling (log-verified). |
| `test_07_open_speed.py` | "Opens in under a second" with injected Telegram latency: cold/warm photo preview, thumbnail grid, first video Range, time-to-first-byte for documents, and that open time does not scale with photo size. |
| `test_08_preview_and_original.py` | The stored-JPEG-preview feature: preview exists and is downscaled, download still returns the untouched original, cold reads cost exactly one Telegram document, disk cache, restart persistence, legacy fallback, PNG→JPEG previews, thumb vs preview, purge of preview documents. |
| `test_api.py`, `test_features.py`, `test_ui.py` | Pre-existing suites (auth, FS ops, trash, shares, favorites, UI serving), now running against this harness. |
| `test_09_upload_resilience.py` | A retried upload keeps its original name (no `(1)` suffix), parallel file uploads actually overlap, and concurrency backs off on flood control. |
| `test_10_client_ui.py` | The desktop client itself, driven through `tauri-pilot`: sync settings paint the auto-upload toggle with no spinner, it defaults to off, the cached state lives in localStorage across a relaunch, and folder rows keep their name tight to the icon. |
| `test_11_gui_flows.py` | Sixteen journeys through the client's own UI: sign in and stay signed in, storages, folders, upload, tile/list view, sort, search, favorites, trash and restore, rename, move, the viewer, sharing by link, bulk select and delete, settings (account/users/trash retention), and photo zoom by magnifier plus swipe between photos. Each one is confirmed on screen and, where it is server state, through the API. |
| `test_13_gui_power_flows.py` | Twenty more client journeys: keyboard file management (select-all, copy/cut/paste, F2), the New menu, breadcrumbs, the copy picker, empty trash, the Info dialog, batch upload, sort by size, Recent, theme persistence across a relaunch, the language switcher, app lock (PIN, relaunch, wrong PIN), changing and disabling another account, granting access to a second client, and a password-protected share link. |
| `test_upload_smoke.py` | Image + video upload smoke. Runs on the mock by default (so it is in CI); point it at real Telegram or a deployed server for a live check. |

## Harness

```
helpers/mock_telegram.py   fake Bot API: sendDocument/getFile/file download/getMe/
                           getChat/getChatMember/getUpdates/copyMessage/deleteMessage,
                           plus latency and flood/failure injection for tests
helpers/server.py          launches sarca with its own port/WORK_DIR/log; log helpers
                           (wait_for_log / assert_no_log) for behaviour verification
helpers/api.py             API client (upload drains the NDJSON progress stream)
helpers/sync_client.py     runs the sarca-sync headless driver
helpers/h3.py              HTTP/3 client (aioquic)
helpers/media.py           deterministic photos / PNGs / blobs / video fixture
helpers/pilot.py           builds and drives the desktop client (tauri-pilot):
                           private DBus session, isolated HOME/XDG dirs, a static
                           server standing in for `pnpm dev`, connect/login flows,
                           and a UI layer for the scenarios — rows, context-menu
                           actions, modals, keyboard, uploads, localStorage,
                           text/alert waits, and `reset()` back to a clean stage
```

Markers: `slow` (multi-second), `mock_only` (needs the fake Bot API), `smoke`
(media upload smoke), `gui` (drives the desktop client). Nothing is deselected by
default; `gui` skips itself without a display or without `tauri-pilot` on PATH.

### GUI tests

`task e2e:gui` installs `tauri-pilot-cli` if missing, builds `client/dist` plus a
debug client with `--features pilot` (the plugin is compiled in only there), and
runs under Xvfb when there is no display.
