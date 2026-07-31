"""Deterministic test media (images / video / blobs)."""

from __future__ import annotations

import io
import shutil
import struct
import subprocess
import zlib
from pathlib import Path

FIXTURES = Path(__file__).resolve().parents[1] / "fixtures"


def png(width: int = 64, height: int = 64, seed: int = 0) -> bytes:
    """A valid RGB PNG with a reproducible gradient (no Pillow needed)."""

    def chunk(tag: bytes, data: bytes) -> bytes:
        return (
            struct.pack(">I", len(data))
            + tag
            + data
            + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)
        )

    rows = bytearray()
    for y in range(height):
        rows += b"\x00"
        for x in range(width):
            rows += bytes(((x + seed) % 256, (y + seed) % 256, (x * y + seed) % 256))
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(bytes(rows), 6))
        + chunk(b"IEND", b"")
    )


def big_photo(width: int = 4000, height: int = 3000) -> bytes:
    """A multi-megabyte photo-like JPEG (falls back to PNG without Pillow)."""
    try:
        from PIL import Image  # noqa: PLC0415
    except ImportError:
        return png(width // 4, height // 4)

    import random  # noqa: PLC0415

    # Upscaled noise: detailed enough to compress like a real photo (megabytes),
    # and orders of magnitude faster to build than a per-pixel Python loop.
    small = (width // 8, height // 8)
    noise = Image.frombytes(
        "RGB", small, random.Random(1234).randbytes(small[0] * small[1] * 3)
    )
    img = noise.resize((width, height), Image.BICUBIC)
    buf = io.BytesIO()
    img.save(buf, format="JPEG", quality=95)
    return buf.getvalue()


def recompress_jpeg(data: bytes, quality: int) -> bytes:
    """Re-encode a photo at a lower quality (smaller file, same dimensions)."""
    from PIL import Image  # noqa: PLC0415

    with Image.open(io.BytesIO(data)) as img:
        buf = io.BytesIO()
        img.convert("RGB").save(buf, format="JPEG", quality=quality)
    return buf.getvalue()


def image_size(data: bytes) -> tuple[int, int]:
    from PIL import Image  # noqa: PLC0415

    with Image.open(io.BytesIO(data)) as img:
        return img.size


def is_jpeg(data: bytes) -> bool:
    return data[:3] == b"\xff\xd8\xff"


def is_png(data: bytes) -> bool:
    return data[:8] == b"\x89PNG\r\n\x1a\n"


def blob(size: int, seed: int = 7) -> bytes:
    """Deterministic incompressible-ish bytes of an exact size."""
    import random  # noqa: PLC0415

    return random.Random(seed).randbytes(size)


def video(seconds: int = 2) -> bytes:
    """Small mp4; uses the checked-in fixture, else ffmpeg, else skips the caller."""
    fixture = FIXTURES / "smoke.mp4"
    if fixture.is_file() and fixture.stat().st_size > 1024:
        return fixture.read_bytes()
    if shutil.which("ffmpeg"):
        out = FIXTURES / "generated.mp4"
        FIXTURES.mkdir(exist_ok=True)
        subprocess.run(
            [
                "ffmpeg", "-y", "-hide_banner", "-loglevel", "error",
                "-f", "lavfi", "-i", f"testsrc=size=320x240:rate=15:duration={seconds}",
                "-pix_fmt", "yuv420p", str(out),
            ],
            check=True,
        )
        return out.read_bytes()
    raise RuntimeError("no video fixture and ffmpeg is unavailable")
