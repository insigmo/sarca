"""Upload smoke: a real image and a real video go all the way through.

Unlike the scenario suites this one uploads media rather than octet-streams, so
it exercises the photo/video paths of the Telegram sender (thumbnails, video
chunking, streamed progress). It runs everywhere now:

* default (mock Bot API): part of the normal suite and of CI;
* `SARCA_E2E_TELEGRAM=real ...`: same asserts against live Telegram;
* `SARCA_BASE_URL=... pytest -m smoke`: against an already-deployed server,
  using whatever storage its first worker serves.
"""

from __future__ import annotations

import os
import struct
import time
import zlib
from pathlib import Path

import pytest

from helpers.api import SarcaClient

pytestmark = [pytest.mark.smoke, pytest.mark.slow]

FIXTURES = Path(__file__).resolve().parent / "fixtures"
EXTERNAL_BASE_URL = os.environ.get("SARCA_BASE_URL")


def _minimal_png(width: int = 8, height: int = 8) -> bytes:
    def chunk(tag: bytes, data: bytes) -> bytes:
        return (
            struct.pack(">I", len(data))
            + tag
            + data
            + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)
        )

    raw = b"".join(b"\x00" + (b"\x00\xff\x00" * width) for _ in range(height))
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(raw, 9))
        + chunk(b"IEND", b"")
    )


def _minimal_mp4() -> bytes:
    ftyp = b"isom" + struct.pack(">I", 0) + b"isomiso2mp41"
    ftyp_box = struct.pack(">I", 8 + len(ftyp)) + b"ftyp" + ftyp
    mdat_payload = b"\x00" * 64
    mdat_box = struct.pack(">I", 8 + len(mdat_payload)) + b"mdat" + mdat_payload
    return ftyp_box + mdat_box


def _fixture_or(name: str, fallback) -> bytes:
    path = FIXTURES / name
    if path.is_file() and path.stat().st_size > 32:
        return path.read_bytes()
    return fallback()


@pytest.fixture(scope="module")
def smoke_storage(sarca: SarcaClient, shared_storage: str) -> str:
    """Where the smoke uploads land.

    Against a deployed server there is no bot token to hand out, so the first
    configured storage worker is used instead of creating a storage.
    """
    if not EXTERNAL_BASE_URL:
        return shared_storage
    workers = sarca.get("/api/storage_workers")
    if workers.status_code != 200 or not workers.json():
        pytest.skip("no storage workers - attach a Telegram bot before upload smoke")
    return workers.json()[0]["storage_id"]


def test_upload_image_and_video_smoke(sarca: SarcaClient, smoke_storage: str) -> None:
    png = _fixture_or("smoke.png", _minimal_png)
    mp4 = _fixture_or("smoke.mp4", _minimal_mp4)

    stamp = str(int(time.time()))
    img_name = f"smoke-{stamp}.png"
    vid_name = f"smoke-{stamp}.mp4"
    nested_name = f"nested-{stamp}.png"

    for name, blob, content_type, path in (
        (img_name, png, "image/png", ""),
        (vid_name, mp4, "video/mp4", ""),
        (nested_name, png, "image/png", f"smoke-dir-{stamp}/"),
    ):
        result = sarca.upload(smoke_storage, name, blob, path=path, content_type=content_type)
        assert result.ok, f"upload {name} failed: {result.error}"
        assert result.phases[-1] == "done", result.phases

    root = {e["name"]: e["is_file"] for e in sarca.tree(smoke_storage)}
    assert root.get(img_name) is True, root
    assert root.get(vid_name) is True, root
    assert root.get(f"smoke-dir-{stamp}") is False, root

    nested = {e["name"]: e["is_file"] for e in sarca.tree(smoke_storage, f"smoke-dir-{stamp}")}
    assert nested.get(nested_name) is True, nested


def test_uploaded_image_downloads_byte_for_byte(
    sarca: SarcaClient, smoke_storage: str
) -> None:
    """Media takes the photo path through Telegram; the bytes must still return."""
    png = _fixture_or("smoke.png", _minimal_png)
    name = f"smoke-roundtrip-{int(time.time())}.png"

    result = sarca.upload(smoke_storage, name, png, content_type="image/png")
    assert result.ok, result.error
    assert sarca.download_bytes(smoke_storage, name) == png
