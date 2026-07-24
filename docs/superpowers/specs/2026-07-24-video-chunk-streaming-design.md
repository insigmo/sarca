# Progressive video chunk streaming (2026-07-24)

## Problem

Sarca splits files into Telegram document chunks sized by `TELEGRAM_CHUNK_SIZE_MB` (often ~1950 MB with Local Bot API). Range downloads map byte offsets to chunk indexes using that same global size. For large videos, the player cannot start until the first huge chunk is fully uploaded and downloadable.

## Decision

Use a smaller Telegram chunk size for videos so progressive Range playback can begin after the first chunk is ready.

| Kind | Chunk size |
|------|------------|
| Video | `TELEGRAM_VIDEO_CHUNK_SIZE_MB` (default **48**) |
| Everything else | `TELEGRAM_CHUNK_SIZE_MB` (unchanged) |

**Video detection:** extension in `{mp4, webm, mkv, mov, m4v, avi, mpeg, mpg, ogv, 3gp}` and/or multipart `Content-Type: video/*`.

**Per-file persistence:** store `files.chunk_size_bytes` at upload time. Downloads use `file.chunk_size_bytes` for Range→chunk mapping, falling back to config `telegram_chunk_size_mb` for rows without the column set (legacy uploads).

## Out of scope

- No HLS / DASH
- No re-encoding or remuxing
- No migration of existing video rows (they keep large chunks; still play, just not progressive until first chunk is available)
- Folder ZIP download streams whole Telegram documents sequentially and does not need Range mapping

## Verify

1. Restart Sarca so schema `ALTER` adds `chunk_size_bytes`.
2. Upload an MP4 larger than ~100 MB.
3. After the first ~48 MB Telegram chunk finishes, open inline playback — seeking/start should work via Range + existing `getInlineMediaUrl` player.
