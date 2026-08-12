
<p align="center">
  Self-hosted cloud storage that keeps files in Telegram — no paid object storage required.
</p>
<p align="center">
  <a href="https://github.com/insigmo/sarca/actions/workflows/release.yml"><img alt="CI" src="https://img.shields.io/github/actions/workflow/status/insigmo/sarca/release.yml?style=flat-square&logo=github"></a>
  <a href="https://github.com/insigmo/sarca/pkgs/container/sarca"><img alt="GHCR" src="https://img.shields.io/badge/ghcr.io-sarca-14635C?style=flat-square&logo=docker"></a>
</p>


Personal / multi-user file cloud with zero storage bill: file bytes are chunked and pushed into Telegram channels via bots, metadata stays in SQLite on your own server.

## Contents

- [What this is](#what-this-is)
- [Server](#server)
- [Clients](#clients)
- [Usage](#usage)
- [Configuration](#configuration)
- [How Sarca compares](#how-sarca-compares)
- [Donations](#donations)
- [License](#license)

## What this is

The repo has two parts:

| Part        | Path            | Role                                                       |
|-------------|-----------------|------------------------------------------------------------|
| **Server**  | `sarca/`, `ui/` | HTTP API + web UI (binary or Docker)                       |
| **Clients** | `client/`       | Native apps (desktop / mobile) that connect to your server |

Needs: Telegram bots + a channel/group where the bots are admins. For production TLS, open firewall ports **80/tcp** (ACME), **443/tcp** (HTTPS), and **443/udp** (HTTP/3).

## Server

Installers ask for admin email/password, generate `SECRET_KEY` with `openssl rand -hex 256`, prompt for a public domain (or detect your IP for ACME), write `sarca.conf`, then start the server.

### Linux / macOS (Apple Silicon)

```sh
curl -fsSL https://raw.githubusercontent.com/insigmo/sarca/refs/heads/master/install.sh | bash
```

Binary → `~/.local/share/sarca`. SQLite database and certs live under `work/` beside the binary.

### Windows

```powershell
irm https://raw.githubusercontent.com/insigmo/sarca/refs/heads/master/install.ps1 | iex
```

### Docker Compose (optional)

Single-container deploy (SQLite + in-process TLS/ACME):

```sh
curl -fsSL https://raw.githubusercontent.com/insigmo/sarca/refs/heads/master/install.sh | bash -s -- --docker
```

Open `https://your-domain` when `TLS_HOSTNAME` is set. Unset, the server detects its public IP at startup and serves HTTPS on that identity; only a host with no reachable address falls back to `http://127.0.0.1:$PORT` (default `8000`). Logs: `docker logs -f sarca`.

<details>
<summary>Build from source</summary>

Needs Cargo and Node.js/pnpm.

```sh
git clone https://github.com/insigmo/sarca.git
cd sarca
cp sarca.conf.example sarca.conf   # edit credentials

cd ui && pnpm install && pnpm run build && cd ..
cargo build --release -p sarca

mkdir -p run/ui
cp -a ui/dist/. run/ui/
cp target/release/sarca run/

cd run
set -a && . ../sarca.conf && set +a
./sarca
```

</details>

## Clients

Latest release assets ([releases/latest](https://github.com/insigmo/sarca/releases/latest)):

| Platform            | Download                                                                                                    |
|---------------------|-------------------------------------------------------------------------------------------------------------|
| Linux x86_64        | [`.deb`](https://github.com/insigmo/sarca/releases/latest/download/sarca_client_linux_amd64.deb)            |
| Linux aarch64       | [`.deb`](https://github.com/insigmo/sarca/releases/latest/download/sarca_client_linux_arm64.deb)            |
| macOS Apple Silicon | [`.dmg`](https://github.com/insigmo/sarca/releases/latest/download/sarca_client_macos_arm64.dmg)            |
| Windows x86_64      | [installer](https://github.com/insigmo/sarca/releases/latest/download/sarca_client_windows_amd64-setup.exe) |
| Windows ARM64       | [installer](https://github.com/insigmo/sarca/releases/latest/download/sarca_client_windows_arm64-setup.exe) |
| Android arm64       | [`.apk`](https://github.com/insigmo/sarca/releases/latest/download/sarca_client_android_arm64.apk)          |

Open the app, enter your server URL, sign in. See [`client/`](client/) for building from source.

No iOS build ships today — the iOS job in CI is disabled (`if: false` in `client.yml`), pending Apple signing setup. The Android APK is unsigned unless you supply keystore secrets; Tauri still produces an installable APK without them.

## Usage

1. Sign in with the admin email/password you set during install. More users: **Settings → Users**.
2. Setup wizard (**Storages → New storage**): bot token from [@BotFather](https://t.me/BotFather) → private channel with the bot as admin → finish.
3. Optional: **Settings → Workers** — more bot tokens on a storage for throughput.
4. Upload / download, folders, search, trash, shares, ACLs.

Official Bot API supports up to ~20 MB per document chunk. Files larger than the chunk size are split automatically.

## Configuration

Full list: [`sarca.conf.example`](sarca.conf.example).

| Variable                             | Purpose                                                                |
|--------------------------------------|------------------------------------------------------------------------|
| `SUPERUSER_EMAIL` / `SUPERUSER_PASS` | Bootstrap admin                                                        |
| `SECRET_KEY`                         | JWT + encryption (installer generates this)                            |
| `SQLITE_PATH`                        | SQLite metadata database (default `{WORK_DIR}/sarca.sqlite`)           |
| `TLS_HOSTNAME`                       | Public domain or IP for ACME certificate (default: detected public IP) |
| `HTTPS_ADDR` / `ACME_HTTP_ADDR`      | HTTPS (443) and ACME (80) listen addresses                             |
| `CERTS_DIR`                          | PEM store for issued certificates                                      |
| `TELEGRAM_*`                         | Bot API URL, rate limit, chunk size (≤20 MB)                           |
| `WORK_DIR`                           | Upload spool + SQLite + certs directory                                |

## How Sarca compares

Sarca's actual trade-off: no storage bill, because Telegram is the storage backend — at the cost of trusting Telegram with your bytes and living inside the Bot API's limits.

|                      | Sarca                                                                                                                   | Nextcloud                                                     | Seafile                                          | Syncthing                                                                   | rclone                                                                           | Telegram-drive clones (TgDrive etc.)                                    |
|----------------------|-------------------------------------------------------------------------------------------------------------------------|---------------------------------------------------------------|--------------------------------------------------|-----------------------------------------------------------------------------|----------------------------------------------------------------------------------|-------------------------------------------------------------------------|
| Storage backend      | Telegram channels (chunked, replicated)                                                                                 | Your disk/object storage                                      | Your disk/object storage                         | Peer devices, no server                                                     | Whatever remote you point it at (S3, Drive, Telegram-style community remotes, …) | Telegram channels                                                       |
| Storage cost         | Free (Telegram's), but subject to their ToS/rate limits                                                                 | You pay for disks/S3                                          | You pay for disks/S3                             | You pay for your own disks                                                  | Depends on chosen remote                                                         | Free (Telegram's)                                                       |
| Self-hosting burden  | One Rust binary + SQLite, built-in ACME TLS                                                                             | Full LAMP-ish stack, PHP, DB, more moving parts               | App server + DB (MySQL/SQLite) + optional search | None — no server at all                                                     | None — it's a CLI/mount tool, not a service                                      | Varies, usually similarly light                                         |
| Sync model           | Client uploads/downloads via server API (`sarca-sync` engine)                                                           | Client-server sync (official desktop/mobile apps)             | Client-server sync, block-level dedup            | True P2P, no central point                                                  | One-shot/scheduled copy, not continuous sync                                     | Client-server, similar to Sarca                                         |
| Mobile support       | Android APK (Tauri); iOS build exists in CI but is disabled                                                             | Official iOS + Android apps                                   | Official iOS + Android apps                      | Official Android app; no iOS                                                | None (CLI only)                                                                  | Varies by project, often Telegram itself as the "client"                |
| Encryption           | TLS in transit (server↔client, server↔Telegram); file bytes are **not** end-to-end encrypted before they reach Telegram | Optional server-side encryption; E2E encryption app available | Optional per-library client-side encryption      | Encrypted in transit by default (TLS); untrusted-device "sending only" mode | Encryption is opt-in via the `crypt` remote overlay                              | Varies; several add their own AES layer, which Sarca currently does not |
| File size limits     | No hard cap — files are split into ≤20 MB chunks per the Bot API, so very large files mean many chunks                  | Limited by your disk/storage backend only                     | Limited by your disk/storage backend only        | Limited by your disk only                                                   | Limited by chosen remote                                                         | Same Bot API chunking as Sarca                                          |
| Redundancy           | Built-in: `ReplicationService` copies chunks across multiple channels/bots                                              | Whatever your storage/backup layer provides                   | Optional external replication                    | Each device is itself a replica                                             | None built-in                                                                    | Varies                                                                  |
| Maturity / ecosystem | Young, small project                                                                                                    | Very mature, large ecosystem, plugins/marketplace             | Mature, established                              | Mature, established                                                         | Mature, huge backend list                                                        | Mostly young hobby projects, similar risk profile to Sarca              |

Where Sarca is genuinely worse:

- **No end-to-end encryption of file content.** Anyone with access to the bot tokens or the destination channels can read raw chunks; Sarca relies on TLS and Telegram's own storage security, not client-side crypto.
- **Tied to a third party's API and ToS.** Rate limits, chunk size (20 MB), and channel/bot bans are Telegram's call, not yours — unlike Nextcloud/Seafile/Syncthing where you own the whole stack.
- **No iOS client**, and the Android APK build is unsigned by default.
- **Single SQLite database, no clustering.** Nextcloud/Seafile scale out with proper databases and object storage; Sarca is single-node.
- **Small, young project.** Less battle-tested than Nextcloud, Seafile, or Syncthing, with a much smaller user base and ecosystem.
- **Not a true P2P sync tool** like Syncthing — a central Sarca server is always in the path, so it inherits classic client-server availability concerns.

## Donations

**GitHub Sponsors**: [github.com/sponsors/insigmo](https://github.com/sponsors/insigmo)
**BTC**: `bc1qyd28yapuutcmfxmrpxtd835z3ds2q260jzh4v7`
**TON**: `UQDw5-4nyIrb1K1waDFH4oGYBIfZYfEoqmS26ix0kKAi6e-Q`
**USDT**: `0x1D3dD608804E1992a37c9b2CA673522c1e17f543`

## License

MIT — see [LICENSE](LICENSE).
