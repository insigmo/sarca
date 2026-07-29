<p align="center">
  <img src="logo.svg" alt="Sarca logo" width="120" />
</p>

<h1 align="center">Sarca</h1>

<p align="center">
  Self-hosted cloud storage that keeps files in Telegram channels — no paid object storage required.
</p>

<p align="center">
  <a href="https://github.com/insigmo/sarca/actions/workflows/release.yml"><img alt="CI" src="https://img.shields.io/github/actions/workflow/status/insigmo/sarca/release.yml?style=flat-square&logo=github"></a>
  <a href="https://github.com/insigmo/sarca/pkgs/container/sarca"><img alt="GHCR" src="https://img.shields.io/badge/ghcr.io-sarca-14635C?style=flat-square&logo=docker"></a>
</p>

## What this is

Sarca is a personal / multi-user file cloud. Metadata lives in Postgres; file bytes are chunked and stored in Telegram via bots. The repo has two parts:

| Part | Path | Role |
| --- | --- | --- |
| **Server** | `sarca/`, `ui/` | HTTP API + web UI (binary or Docker) |
| **Clients** | `client/` | Native apps (desktop / mobile) that connect to your server |

Needs: PostgreSQL, Telegram bots + a channel/group where the bots are admins. Docker Compose also runs Local Bot API (larger files).

## Server

Installers ask for admin email/password and Telegram `api_id` / `api_hash` (from [my.telegram.org](https://my.telegram.org) → **API development tools**), generate `SECRET_KEY` with `openssl rand -hex 512`, write `sarca.conf`, then start the server.

### Linux / macOS (Apple Silicon)

```sh
curl -fsSL https://raw.githubusercontent.com/insigmo/sarca/refs/heads/master/install.sh | bash
```

Binary → `~/.local/share/sarca`. Needs a reachable Postgres (`DATABASE_*` in `sarca.conf`).

### Windows

```powershell
irm https://raw.githubusercontent.com/insigmo/sarca/refs/heads/master/install.ps1 | iex
```

### Docker Compose (recommended)

Postgres + Local Bot API included:

```sh
curl -fsSL https://raw.githubusercontent.com/insigmo/sarca/refs/heads/master/install.sh | bash -s -- --docker
```

Open `http://127.0.0.1:$PORT` (default `8000`). Logs: `docker logs -f sarca`.

<details>
<summary>Build from source</summary>

Needs Cargo, Node.js, pnpm, and Postgres.

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

| Platform | Download |
| --- | --- |
| Linux x86_64 | [`.deb`](https://github.com/insigmo/sarca/releases/latest/download/sarca_client_linux_amd64.deb) |
| Linux aarch64 | [`.deb`](https://github.com/insigmo/sarca/releases/latest/download/sarca_client_linux_arm64.deb) |
| macOS Apple Silicon | [`.dmg`](https://github.com/insigmo/sarca/releases/latest/download/sarca_client_macos_arm64.dmg) |
| Windows x86_64 | [installer](https://github.com/insigmo/sarca/releases/latest/download/sarca_client_windows_amd64-setup.exe) |
| Windows ARM64 | [installer](https://github.com/insigmo/sarca/releases/latest/download/sarca_client_windows_arm64-setup.exe) |
| Android arm64 | [`.apk`](https://github.com/insigmo/sarca/releases/latest/download/sarca_client_android_arm64.apk) |

Open the app, enter your server URL, sign in. See [`client/`](client/) for building from source.

## Usage

1. Sign in with the admin email/password you set during install. More users: **Settings → Users**.
2. Setup wizard (**Storages → New storage**): optional Local Bot API credentials → bot token from [@BotFather](https://t.me/BotFather) → private channel with the bot as admin → finish.
3. Optional: **Settings → Workers** — more bot tokens on a storage for throughput.
4. Upload / download, folders, search, trash, shares, ACLs.

Official Bot API ~20 MB download cap; with Local Bot API (Compose) chunks can go up to ~2 GB per document. Files larger than the chunk size are split automatically.

## Configuration

Full list: [`sarca.conf.example`](sarca.conf.example).

| Variable | Purpose |
| --- | --- |
| `PORT` | HTTP port (default `8000`) |
| `SUPERUSER_EMAIL` / `SUPERUSER_PASS` | Bootstrap admin |
| `SECRET_KEY` | JWT + encryption (installer generates this) |
| `DATABASE_*` | Postgres |
| `TELEGRAM_API_ID` / `TELEGRAM_API_HASH` | Local Bot API ([my.telegram.org](https://my.telegram.org)) |
| `TELEGRAM_*` | Bot API URL, rate limit, chunk size |
| `WORK_DIR` | Upload spool directory |

## Donations

**BTC**: `bc1qyd28yapuutcmfxmrpxtd835z3ds2q260jzh4v7`

**TON**: `UQDw5-4nyIrb1K1waDFH4oGYBIfZYfEoqmS26ix0kKAi6e-Q`

**USDT**: `0x1D3dD608804E1992a37c9b2CA673522c1e17f543`

## License

MIT — see [LICENSE](LICENSE).
