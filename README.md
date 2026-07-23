<p align="center">
  <img src="logo.svg" alt="Sarca logo" width="120" />
</p>

<h1 align="center">Sarca</h1>

<p align="center">
  Self-hosted cloud storage that keeps files in Telegram channels — no paid object storage required.
</p>

<p align="center">
  <a href="https://github.com/insigmo/sarca/actions"><img alt="CI" src="https://img.shields.io/github/actions/workflow/status/insigmo/sarca/docker-image.yml?style=flat-square&logo=github"></a>
  <a href="https://github.com/insigmo/sarca/pkgs/container/sarca"><img alt="GHCR" src="https://img.shields.io/badge/ghcr.io-sarca-14635C?style=flat-square&logo=docker"></a>
</p>

## What it does

Sarca turns Telegram into a backend for a personal or multi-user file cloud:

- Each **storage** is a separate filesystem (like a volume)
- Files are split into chunks and uploaded to a Telegram channel/group via bots (**storage workers**)
- Metadata lives in Postgres; the binary is small and does not need a language runtime on the host

You can use it as a personal drive, a shared team vault, or as an API similar to S3 / Nextcloud-style backends.

## Requirements

- **PostgreSQL**
- Optional but recommended for large files: **Telegram Local Bot API** (included in Docker Compose)
- Telegram bots + a channel/group where those bots are admins

## Installation

### Linux / macOS (Apple Silicon)

```sh
curl -fsSL https://raw.githubusercontent.com/insigmo/sarca/refs/heads/master/install.sh | bash
```

1. Edit `~/.local/share/sarca/.env` (`SUPERUSER_*`, `SECRET_KEY`, `DATABASE_*`)
2. Make sure Postgres is reachable
3. Run:

```sh
sarca
```

Open http://127.0.0.1:8000

### Windows (PowerShell)

```powershell
irm https://raw.githubusercontent.com/insigmo/sarca/refs/heads/master/install.ps1 | iex
```

Edit `%LOCALAPPDATA%\Sarca\.env`, then run `sarca.cmd` from the install folder (path is printed by the installer).

### Docker Compose

```sh
curl -fsSL https://raw.githubusercontent.com/insigmo/sarca/refs/heads/master/install.sh | bash -s -- --docker
cd sarca
# edit .env: SUPERUSER_*, SECRET_KEY, TELEGRAM_API_ID, TELEGRAM_API_HASH
docker compose up -d
```

Open http://127.0.0.1:8000

Logs:

```sh
docker logs -f sarca
```

<details>
<summary>Build from source</summary>

Needs Cargo, Node.js, pnpm, and Postgres.

```sh
git clone https://github.com/insigmo/sarca.git
cd sarca
cp .env.example .env   # edit credentials

cd ui && pnpm install && pnpm run build && cd ..
cd sarca && cargo build --release && cd ..

mkdir -p run/ui
cp -a ui/dist/. run/ui/
cp sarca/target/release/sarca run/

cd run
set -a && . ../.env && set +a
./sarca
```

</details>

## Usage

### 1. Sign in

Open the UI and sign in with the superuser from `.env` (or register a new user if registration is enabled for your setup).

### 2. Create a Telegram channel / group

1. Create a channel or group (private is recommended)
2. Create one or more bots with [@BotFather](https://t.me/BotFather)
3. Add the bots as admins with permission to send (and preferably delete) messages/documents
4. Get the `chat_id` (for example by forwarding a message to `@RawDataBot`)

`chat_id` rules:

- Supergroups / channels: usually `-100…`
- Regular groups: negative id without the `-100` prefix
- Private chats (positive ids) are not supported

**Optional auto-setup:** set `TELEGRAM_BOT_TOKEN`, `TELEGRAM_CHANNEL_ID` (channel id **without** the `-100` prefix), and `STORAGE_NAME` in `.env`. On startup Sarca creates the storage and attaches the bot for the superuser. If those are left empty, configure them in the UI instead.

### 3. Create a storage

In the UI go to **Storages → New storage** and provide a name + `chat_id` (skip if you used auto-setup above).

### 4. Attach storage workers

Go to **Storage workers**, create a worker with the bot token, and bind it to your storage (skip if you used auto-setup above).

You can add several bots to raise throughput (Telegram rate-limits each bot).

### 5. Upload and manage files

Open the storage filesystem to:

- Upload / download files
- Create folders
- Search, rename, move, delete
- Share access with other Sarca users (Viewer / Can edit / Admin)

### Local Bot API (large files)

Official Bot API caps downloads around 20 MB. With Local Bot API (compose service `telegram-bot-api`) you can use much larger chunks (up to ~2 GB per document).

In `.env`:

```env
TELEGRAM_LOCAL_API=true
TELEGRAM_API_BASE_URL=http://telegram-bot-api:8081
TELEGRAM_API_ID=...
TELEGRAM_API_HASH=...
TELEGRAM_CHUNK_SIZE_MB=1950
WORK_DIR=/work
```

Get `TELEGRAM_API_ID` / `TELEGRAM_API_HASH` from https://my.telegram.org

## Configuration

See [`.env.example`](.env.example) for the full list. Important keys:

| Variable | Purpose |
|---|---|
| `PORT` | HTTP port (default `8000`) |
| `SUPERUSER_EMAIL` / `SUPERUSER_PASS` | Bootstrap admin |
| `SECRET_KEY` | JWT + encryption material |
| `DATABASE_*` | Postgres connection |
| `TELEGRAM_*` | Bot API endpoint, rate limit, chunk size; optional `TELEGRAM_BOT_TOKEN` / `TELEGRAM_CHANNEL_ID` (+ `STORAGE_NAME`) for startup bootstrap |
| `STORAGE_NAME` | Optional storage name used with bootstrap vars above |
| `WORK_DIR` | Spool directory for uploads |

## Logo

Project logo placeholder: [`logo.svg`](logo.svg). Replace it with the final artwork when ready (also update `ui/src/assets/logo.svg` used by the UI).

## Donations

If Sarca is useful to you, you can support development:

**BTC**: `bc1qyd28yapuutcmfxmrpxtd835z3ds2q260jzh4v7`

**TON**: `UQDw5-4nyIrb1K1waDFH4oGYBIfZYfEoqmS26ix0kKAi6e-Q`

**USDT**: `0x1D3dD608804E1992a37c9b2CA673522c1e17f543`

## Features

- [x] Upload / download files
- [x] Folders
- [x] File info
- [x] Delete / rename / move
- [x] Search
- [x] Access control
- [x] Multiple storage workers

## License

MIT — see [LICENSE](LICENSE).
