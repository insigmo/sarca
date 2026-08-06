"""Scenario 8 — a compressed JPEG preview is stored next to every uploaded photo.

Contract under test:
  * upload of an image produces a second, small JPEG document in Telegram;
  * `GET .../files/preview/<path>` serves that JPEG (downscaled, re-encoded);
  * `GET .../files/download/<path>` still returns the untouched original bytes;
  * the preview survives a cache wipe and a server restart, and is rebuilt on demand
    for files uploaded before the feature existed.
"""

from __future__ import annotations

import shutil

import pytest

from helpers import media
from helpers.api import SarcaClient, sha256
from helpers.server import SarcaServer

pytestmark = pytest.mark.mock_only

PREVIEW_MAX_EDGE = 2048


def upload_photo(sarca: SarcaClient, storage: str, name: str = "photo.jpg") -> bytes:
    data = media.big_photo()
    result = sarca.upload(storage, name, data, content_type="image/jpeg")
    assert result.ok, result.error
    sarca.wait_for_file(storage, name)
    return data


def test_uploading_a_photo_stores_a_compressed_preview(
    sarca: SarcaClient, storage: str, mock, server: SarcaServer
) -> None:
    offset = server.log_offset()
    original = upload_photo(sarca, storage)

    line = server.wait_for_log("uploaded preview for file", offset=offset, timeout=30)
    assert "bytes" in line

    r = sarca.preview(storage, "photo.jpg")
    assert r.status_code == 200, r.text
    assert r.headers["content-type"] == "image/jpeg"
    preview = r.content
    assert media.is_jpeg(preview)
    assert len(preview) < len(original), "preview must be smaller than the original"

    width, height = media.image_size(preview)
    assert max(width, height) <= PREVIEW_MAX_EDGE


def test_download_still_returns_the_untouched_original(sarca: SarcaClient, storage: str) -> None:
    original = upload_photo(sarca, storage, "keepme.jpg")
    downloaded = sarca.download_bytes(storage, "keepme.jpg")
    assert sha256(downloaded) == sha256(original)
    assert len(downloaded) == len(original)


def test_preview_is_served_without_touching_the_original_chunks(
    sarca: SarcaClient, storage: str, mock, workdir
) -> None:
    """After a cache wipe the preview comes from its own small Telegram document."""
    original = upload_photo(sarca, storage, "cold.jpg")
    shutil.rmtree(workdir / "preview_cache", ignore_errors=True)
    shutil.rmtree(workdir / "chunk_cache", ignore_errors=True)

    mock.reset_calls()
    r = sarca.preview(storage, "cold.jpg")
    assert r.status_code == 200
    preview = r.content

    # Exactly one getFile + one file download: the preview document, nothing else.
    assert mock.calls("getFile") == 1, mock.stats()
    assert mock.calls("download") == 1, mock.stats()
    assert len(preview) < len(original) // 2


def test_preview_is_cached_on_disk_after_first_read(
    sarca: SarcaClient, storage: str, mock, workdir
) -> None:
    upload_photo(sarca, storage, "warm.jpg")
    sarca.preview(storage, "warm.jpg")

    mock.reset_calls()
    r = sarca.preview(storage, "warm.jpg")
    assert r.status_code == 200
    assert mock.calls("getFile") == 0, "cached preview must not hit Telegram"

    cached = list((workdir / "preview_cache").glob("*.jpg"))
    assert cached, "expected preview_cache entries under WORK_DIR"


def test_preview_survives_a_server_restart(
    sarca: SarcaClient, storage: str, server: SarcaServer, workdir, credentials
) -> None:
    upload_photo(sarca, storage, "persist.jpg")
    before = sarca.preview(storage, "persist.jpg").content

    shutil.rmtree(workdir / "preview_cache", ignore_errors=True)
    server.restart()
    sarca.login(*credentials)

    after = sarca.preview(storage, "persist.jpg")
    assert after.status_code == 200
    assert after.content == before, "stored preview must be byte-identical across restarts"


def test_preview_for_legacy_file_falls_back_to_re_encoding(
    sarca: SarcaClient, storage: str, workdir, server: SarcaServer
) -> None:
    """Files uploaded before this feature have no preview document; rebuild from the original."""
    upload_photo(sarca, storage, "legacy.jpg")

    import sqlite3  # noqa: PLC0415

    db = sqlite3.connect(workdir / "sarca.sqlite")
    db.execute(
        "UPDATE files SET preview_telegram_file_id = NULL, "
        "preview_telegram_message_id = NULL WHERE path = 'legacy.jpg'"
    )
    db.commit()
    db.close()
    shutil.rmtree(workdir / "preview_cache", ignore_errors=True)

    r = sarca.preview(storage, "legacy.jpg")
    assert r.status_code == 200, r.text
    assert media.is_jpeg(r.content)


@pytest.mark.slow
def test_stored_preview_beats_re_encoding_the_original(
    sarca: SarcaClient, storage: str, mock, workdir
) -> None:
    """The stored preview is the reason a photo opens fast; measure both paths."""
    import sqlite3  # noqa: PLC0415
    import time  # noqa: PLC0415

    upload_photo(sarca, storage, "race.jpg")
    mock.set_latency(getFile=0.2, download=0.2)
    try:
        shutil.rmtree(workdir / "preview_cache", ignore_errors=True)
        started = time.perf_counter()
        assert sarca.preview(storage, "race.jpg").status_code == 200
        with_preview = time.perf_counter() - started

        # Same photo, but as if it had been uploaded before this feature existed.
        db = sqlite3.connect(workdir / "sarca.sqlite")
        db.execute(
            "UPDATE files SET preview_telegram_file_id = NULL, "
            "preview_telegram_message_id = NULL WHERE path = 'race.jpg'"
        )
        db.commit()
        db.close()
        shutil.rmtree(workdir / "preview_cache", ignore_errors=True)
        shutil.rmtree(workdir / "chunk_cache", ignore_errors=True)

        started = time.perf_counter()
        assert sarca.preview(storage, "race.jpg").status_code == 200
        without_preview = time.perf_counter() - started
    finally:
        mock.clear_latency()

    assert with_preview < without_preview, (
        f"stored preview {with_preview:.2f}s vs rebuild {without_preview:.2f}s"
    )
    assert with_preview < 1.0


def test_already_compact_photo_does_not_get_a_second_copy(
    sarca: SarcaClient, storage: str, mock, workdir
) -> None:
    """When re-encoding buys nothing, skip the extra Telegram document — but still open."""
    data = media.recompress_jpeg(media.big_photo(1200, 900), quality=25)
    before = mock.document_count()
    assert sarca.upload(storage, "compact.jpg", data, content_type="image/jpeg").ok
    sarca.wait_for_file(storage, "compact.jpg")
    # original chunk + thumbnail, and no preview document on top of those
    assert mock.document_count() == before + 2, mock.stats()

    shutil.rmtree(workdir / "preview_cache", ignore_errors=True)
    r = sarca.preview(storage, "compact.jpg")
    assert r.status_code == 200, r.text
    assert media.is_jpeg(r.content)
    assert sha256(sarca.download_bytes(storage, "compact.jpg")) == sha256(data)


def test_preview_is_rejected_for_non_images(sarca: SarcaClient, storage: str) -> None:
    assert sarca.upload(storage, "notes.txt", b"just text").ok
    sarca.wait_for_file(storage, "notes.txt")
    assert sarca.preview(storage, "notes.txt").status_code == 415


def test_png_upload_also_gets_a_jpeg_preview(sarca: SarcaClient, storage: str) -> None:
    data = media.png(2400, 1600)
    assert sarca.upload(storage, "wide.png", data, content_type="image/png").ok
    sarca.wait_for_file(storage, "wide.png")

    r = sarca.preview(storage, "wide.png")
    assert r.status_code == 200, r.text
    assert media.is_jpeg(r.content), "previews are always JPEG, whatever the source format"

    downloaded = sarca.download_bytes(storage, "wide.png")
    assert media.is_png(downloaded), "the original PNG must come back as PNG"
    assert sha256(downloaded) == sha256(data)


def test_thumbnail_and_preview_are_separate_documents(
    sarca: SarcaClient, storage: str, mock
) -> None:
    before = mock.document_count()
    upload_photo(sarca, storage, "both.jpg")
    # original chunk(s) + thumbnail + preview
    assert mock.document_count() >= before + 3

    thumb = sarca.thumb(storage, "both.jpg")
    preview = sarca.preview(storage, "both.jpg")
    assert thumb.status_code == 200 and preview.status_code == 200
    assert media.is_jpeg(thumb.content) and media.is_jpeg(preview.content)
    assert len(thumb.content) < len(preview.content), "thumb (128px) < preview (1920px)"
    assert max(media.image_size(thumb.content)) <= 128


def test_thumb_is_cached_on_disk_after_first_read(
    sarca: SarcaClient, storage: str, mock, workdir
) -> None:
    upload_photo(sarca, storage, "warm_thumb.jpg")
    sarca.thumb(storage, "warm_thumb.jpg")

    mock.reset_calls()
    r = sarca.thumb(storage, "warm_thumb.jpg")
    assert r.status_code == 200
    assert mock.calls("getFile") == 0, "cached thumb must not hit Telegram"

    cached = list((workdir / "thumb_cache").glob("*.jpg"))
    assert cached, "expected thumb_cache entries under WORK_DIR"


def test_client_supplied_thumb_is_stored_verbatim(sarca: SarcaClient, storage: str) -> None:
    """The web client builds the grid tile itself; the server must not re-encode it."""
    tile = media.recompress_jpeg(media.big_photo(128, 96), quality=75)
    data = media.big_photo()
    assert sarca.upload(storage, "client.jpg", data, content_type="image/jpeg", thumb=tile).ok
    sarca.wait_for_file(storage, "client.jpg")

    r = sarca.thumb(storage, "client.jpg")
    assert r.status_code == 200
    assert r.content == tile, "the stored thumb must be the exact bytes the client sent"


def test_junk_client_thumb_falls_back_to_server_generation(
    sarca: SarcaClient, storage: str
) -> None:
    """A non-JPEG `thumb` field is dropped, not stored: the server builds its own."""
    data = media.big_photo()
    assert sarca.upload(
        storage, "junk.jpg", data, content_type="image/jpeg", thumb=b"not a jpeg at all"
    ).ok
    sarca.wait_for_file(storage, "junk.jpg")

    r = sarca.thumb(storage, "junk.jpg")
    assert r.status_code == 200
    assert media.is_jpeg(r.content)
    assert max(media.image_size(r.content)) <= 320


def test_small_image_preview_is_not_upscaled(sarca: SarcaClient, storage: str) -> None:
    data = media.png(64, 48)
    assert sarca.upload(storage, "tiny.png", data, content_type="image/png").ok
    sarca.wait_for_file(storage, "tiny.png")

    r = sarca.preview(storage, "tiny.png")
    assert r.status_code == 200
    assert media.image_size(r.content) == (64, 48)


def test_deleting_a_photo_removes_its_preview_document(
    sarca: SarcaClient, storage: str, mock
) -> None:
    upload_photo(sarca, storage, "gone.jpg")
    live_before = len(mock.live_messages())

    assert sarca.delete_file(storage, "gone.jpg").status_code in (200, 204)
    r = sarca.delete(f"/api/storages/{storage}/trash/gone.jpg")
    assert r.status_code in (200, 204, 404), r.text

    deadline_ok = False
    import time  # noqa: PLC0415

    for _ in range(50):
        if len(mock.live_messages()) < live_before:
            deadline_ok = True
            break
        time.sleep(0.2)
    assert deadline_ok, "purging a photo must delete its Telegram documents"
