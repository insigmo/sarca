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

1. Edit `~/.local/share/sarca/sarca.conf` (`SUPERUSER_*`, `SECRET_KEY`, `DATABASE_*`)
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

Edit `%LOCALAPPDATA%\Sarca\sarca.conf`, then run `sarca.cmd` from the install folder (path is printed by the installer).

### Docker Compose

`compose.yml` mounts `./sarca.conf` into the containers. Secrets and Telegram credentials live there. Docker networking (`DATABASE_HOST=db`, `WORK_DIR=/work`) is set in compose. **`PORT` is read from `sarca.conf`** for both the published host port and the process listen port — always pass `--env-file sarca.conf` so Compose can interpolate it.

```sh
curl -fsSL https://raw.githubusercontent.com/insigmo/sarca/refs/heads/master/install.sh | bash -s -- --docker
cd sarca
# edit sarca.conf: SUPERUSER_*, SECRET_KEY, TELEGRAM_API_ID, TELEGRAM_API_HASH, PORT
# keep DATABASE_USER/PASSWORD/NAME = sarca (matches compose db service)
docker compose --env-file sarca.conf up -d
```

Open http://127.0.0.1:$PORT (default `8000` if unset in conf).

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
cp sarca.conf.example sarca.conf   # edit credentials

cd ui && pnpm install && pnpm run build && cd ..
cd sarca && cargo build --release && cd ..

mkdir -p run/ui
cp -a ui/dist/. run/ui/
cp sarca/target/release/sarca run/

cd run
set -a && . ../sarca.conf && set +a
./sarca
```

</details>

## Usage

### 1. Sign in

Open the UI and sign in with the superuser from `sarca.conf` (or register a new user if registration is enabled for your setup).

### 2. Run the setup wizard

On first login (no storages yet), or via **Storages → New storage**, Sarca opens a setup wizard:

1. **Local Bot API (optional once):** paste `api_id` / `api_hash` from [my.telegram.org](https://my.telegram.org) for files larger than ~20 MB, or skip.
2. **Bot:** create a bot with [@BotFather](https://t.me/BotFather) and paste the token.
3. **Channel:** create a private channel, add the bot as admin (post + delete), then post any message — Sarca detects the `chat_id` automatically.
4. Optionally detect up to 3 channels for replication, then finish — storage + worker are created together.

Advanced form (manual `chat_id`): **Storages → New storage → Advanced create**, or `/storages/register/manual`.

**Optional auto-setup:** set `TELEGRAM_BOT_TOKEN`, `TELEGRAM_CHANNEL_ID` (channel id **without** the `-100` prefix), and `STORAGE_NAME` in `sarca.conf`. On startup Sarca creates the storage and attaches the bot for the superuser.

`chat_id` rules (manual / bootstrap):

- Supergroups / channels: usually `-100…`
- Regular groups: negative id without the `-100` prefix
- Private chats (positive ids) are not supported

### 3. Add more workers (optional)

In **Settings → Workers**, add more bot tokens bound to a storage to raise throughput (Telegram rate-limits each bot).

### 4. Upload and manage files

Open the storage filesystem to:

- Upload / download files
- Create folders
- Search, rename, move, delete
- Share access with other Sarca users (Viewer / Can edit / Admin)

### Local Bot API (large files)

Official Bot API caps downloads around 20 MB. With Local Bot API (compose service `telegram-bot-api`) you can use much larger chunks (up to ~2 GB per document).

In `sarca.conf`:

```env
TELEGRAM_LOCAL_API=true
TELEGRAM_API_BASE_URL=http://telegram-bot-api:8081
TELEGRAM_API_ID=...
TELEGRAM_API_HASH=...
TELEGRAM_CHUNK_SIZE_MB=1950
WORK_DIR=/work
```

Get `TELEGRAM_API_ID` / `TELEGRAM_API_HASH` from https://my.telegram.org (or paste them in the setup wizard — it writes `sarca.conf` when possible; restart Local Bot API after changing credentials).

## Releases

Pushes to `master` automatically run e2e, bump the patch version (`v0.0.x`), update `sarca/Cargo.toml` + `ui/package.json`, and publish GitHub Release + GHCR image. You do not need to create tags or edit version fields by hand.

For a minor/major bump: **Actions → Release binaries → Run workflow** and pick the bump type.

## Configuration

See [`sarca.conf.example`](sarca.conf.example) for the full list. Important keys:

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
