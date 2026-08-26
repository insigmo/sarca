
<p align="center">
  <img src="logo.svg" width="120" alt="Sarca logo">
</p>
<h1 align="center">🗿 Sarca</h1>

<p align="center"><b>Cloud storage. Zero bill. Telegram pay, not you.</b></p>

<p align="center">
  <a href="https://github.com/insigmo/sarca/actions/workflows/release.yml"><img alt="CI" src="https://img.shields.io/github/actions/workflow/status/insigmo/sarca/release.yml?style=flat-square&logo=github"></a>
  <a href="https://github.com/insigmo/sarca/pkgs/container/sarca"><img alt="GHCR" src="https://img.shields.io/badge/ghcr.io-sarca-14635C?style=flat-square&logo=docker"></a>
</p>

<p>Files go in. Files get chopped small.</p>
<p>Small file live in Telegram channel, free, forever.</p>
<p>Metadata stay home, in SQLite, on your box.</p>
<p>No S3 bill. No disk bill. Just tribe's own server and Telegram's good will.</p>

## Why Sarca, not other solutions?
| Feature                | **Sarca** | Nextcloud | ownCloud | Other Telegram-storage scripts |
|------------------------|-----------|-----------|----------|--------------------------------|
| Free storage backend   | ✅        | ❌        | ❌       | ✅                             |
| HTTP3                  | ✅        | ❌        | ❌       | ❌                             |
| Auto HTTPS             | ✅        | ❌        | ❌       | ❌                             |
| Web UI                 | ✅        | ✅        | ✅       | ✅                             |
| Show preview           | ✅        | ✅        | ✅       | ❌                             |
| Share files by link    | ✅        | ✅        | ✅       | ❌                             |
| Native desktop app     | ✅        | ✅        | ✅       | ❌                             |
| Native mobile app      | ✅        | ✅        | ✅       | ❌                             |
| Background sync client | ✅        | ✅        | ✅       | ❌                             |
| Multi-user accounts    | ✅        | ✅        | ✅       | ❌                             |

Sarca small. Sarca fast. Sarca no ask money.

## Clients

Latest release assets ([releases/latest](https://github.com/insigmo/sarca/releases/latest)):

| Platform            | Download                                                                                                    |
|---------------------|-------------------------------------------------------------------------------------------------------------|
| Linux x86_64        | [`.deb`](https://github.com/insigmo/sarca/releases/latest/download/sarca_client_linux_amd64.deb)            |
| Linux aarch64       | [`.deb`](https://github.com/insigmo/sarca/releases/latest/download/sarca_client_linux_arm64.deb)            |
| macOS Apple Silicon | [`.dmg`](https://github.com/insigmo/sarca/releases/latest/download/sarca_client_macos_arm64.dmg)            |
| macOS Intel         | [`.dmg`](https://github.com/insigmo/sarca/releases/latest/download/sarca_client_macos_amd64.dmg)            |
| Windows x86_64      | [installer](https://github.com/insigmo/sarca/releases/latest/download/sarca_client_windows_amd64-setup.exe) |
| Windows ARM64       | [installer](https://github.com/insigmo/sarca/releases/latest/download/sarca_client_windows_arm64-setup.exe) |
| Android arm64       | [`.apk`](https://github.com/insigmo/sarca/releases/latest/download/sarca_client_android_arm64.apk)          |

Open the app, enter your server URL, sign in. See [`client/`](client/) for building from source.

No iOS build ships today. No money for developer account. 
The iOS job in CI is disabled (`if: false` in `client.yml`).

## Grab rock, install servers

**Linux / macOS**

```sh
curl -fsSL https://raw.githubusercontent.com/insigmo/sarca/refs/heads/master/install.sh | bash
```

Binary land in `~/.local/share/sarca`. Database and certs sit in `work/` next to it. Simple.

**Docker, prefer container**

```sh
docker run -d --name sarca -p 8000:8000 -v sarca-data:/app/work ghcr.io/insigmo/sarca:latest
```

Open `https://your-domain` when `TLS_HOSTNAME` set. Not set? 
Server sniff own public IP at startup, serve HTTPS on that. 
No reachable address, no worry — fall to `http://127.0.0.1:8000`. 
Watch fire: `docker logs -f sarca`.

## Usage

1. Sign in with the admin email/password you set during install. More users: **Settings → Users**
2. Setup wizard (**Storages → New storage**): 
   - create a bot token with [@BotFather](https://t.me/BotFather)
   - create private(s) channel(s) (1 necessary and 2 optional)
   - add as admin a bot to the channel(s)

## Move the rock

**Settings -> General -> Backup** (admin only) downloads one `.sarcabak` file:
settings, storages with their bots and channels, users and access, and the whole
file tree. Password optional -- without one the file is plain gzip and anyone
holding it reads every bot token inside.

**Restore** on another Sarca replaces that server's database with the archive.
Same storages, same files, same tree; the file bytes never left Telegram. Every
session is invalidated, so sign in again after. The database being replaced is
copied to `WORK_DIR/backups/pre-restore-*.sqlite` first (last three kept).

## License

See `LICENSE`. Read before fire spread.
