"""Scenario 2 — upload and download round trips; the SHA-256 must survive Telegram."""

from __future__ import annotations

import hashlib
import io
import zipfile

import pytest

from helpers import media
from helpers.api import SarcaClient, sha256
from helpers.server import SarcaServer

pytestmark = pytest.mark.mock_only


def roundtrip(sarca: SarcaClient, storage: str, name: str, data: bytes, path: str = "") -> None:
    """Upload, wait for visibility, download, and compare digests."""
    result = sarca.upload(storage, name, data, path=path)
    assert result.ok, result.error
    full_path = f"{path}{name}" if path else name
    info = sarca.wait_for_file(storage, full_path)
    assert info["size"] == len(data), info

    downloaded = sarca.download_bytes(storage, full_path)
    assert sha256(downloaded) == sha256(data), (
        f"hash mismatch for {name}: {len(downloaded)} bytes back, {len(data)} sent"
    )


def test_small_file_roundtrip_keeps_hash(sarca: SarcaClient, storage: str) -> None:
    roundtrip(sarca, storage, "hello.txt", b"hello sarca\n" * 10)


def test_binary_blob_roundtrip_keeps_hash(sarca: SarcaClient, storage: str) -> None:
    roundtrip(sarca, storage, "blob.bin", media.blob(300 * 1024))


def test_empty_file_roundtrip(sarca: SarcaClient, storage: str) -> None:
    roundtrip(sarca, storage, "empty.bin", b"")


def test_one_byte_file_roundtrip(sarca: SarcaClient, storage: str) -> None:
    roundtrip(sarca, storage, "one.bin", b"\x00")


@pytest.mark.slow
def test_multi_chunk_file_roundtrip_keeps_hash(
    sarca: SarcaClient, storage: str, mock, server: SarcaServer
) -> None:
    """2.5 MB at 1 MB chunks: 3 Telegram documents reassembled byte-exactly."""
    data = media.blob(int(2.5 * 1024 * 1024), seed=42)
    before = mock.calls("sendDocument")
    roundtrip(sarca, storage, "big.bin", data)
    assert mock.calls("sendDocument") - before == 3, "expected exactly 3 chunk uploads"

    info = sarca.info(storage, "big.bin")
    assert info["size"] == len(data)


def test_unicode_and_spaces_in_filename(sarca: SarcaClient, storage: str) -> None:
    roundtrip(sarca, storage, "привет мир (1).txt", "содержимое\n".encode())


def test_upload_into_nested_folders(sarca: SarcaClient, storage: str) -> None:
    assert sarca.create_folder(storage, "photos").status_code in (200, 201)
    assert sarca.create_folder(storage, "2026", path="photos").status_code in (200, 201)
    roundtrip(sarca, storage, "note.txt", b"nested", path="photos/2026/")

    names = {e["name"] for e in sarca.tree(storage, "photos/2026")}
    assert "note.txt" in names


def test_client_supplied_content_hash_is_stored(sarca: SarcaClient, storage: str) -> None:
    data = b"hash me" * 100
    digest = f"sha256:{sha256(data)}"
    result = sarca.upload(storage, "hashed.bin", data, content_hash=digest)
    assert result.ok, result.error
    info = sarca.wait_for_file(storage, "hashed.bin")
    assert info.get("content_hash") == digest, info


def test_range_request_returns_exact_slice(sarca: SarcaClient, storage: str) -> None:
    data = media.blob(400 * 1024, seed=9)
    assert sarca.upload(storage, "ranged.bin", data).ok
    sarca.wait_for_file(storage, "ranged.bin")

    r = sarca.download(storage, "ranged.bin", headers={"Range": "bytes=1000-1999"})
    assert r.status_code == 206, r.text
    assert r.content == data[1000:2000]
    assert r.headers["content-length"] == "1000"

    r = sarca.download(storage, "ranged.bin", headers={"Range": "bytes=999999999-"})
    assert r.status_code == 416


def test_range_across_chunk_boundary(sarca: SarcaClient, storage: str) -> None:
    """A range spanning two 1 MB chunks must stitch them without gaps."""
    data = media.blob(int(1.5 * 1024 * 1024), seed=11)
    assert sarca.upload(storage, "spanning.bin", data).ok
    sarca.wait_for_file(storage, "spanning.bin")

    start, end = 1024 * 1024 - 10, 1024 * 1024 + 9
    r = sarca.download(storage, "spanning.bin", headers={"Range": f"bytes={start}-{end}"})
    assert r.status_code == 206, r.text
    assert r.content == data[start : end + 1]


def test_download_headers_are_sane(sarca: SarcaClient, storage: str) -> None:
    assert sarca.upload(storage, "report.pdf", b"%PDF-1.4\n%fake\n").ok
    sarca.wait_for_file(storage, "report.pdf")

    r = sarca.download(storage, "report.pdf")
    assert r.status_code == 200
    assert r.headers["content-type"] == "application/pdf"
    assert "report.pdf" in r.headers["content-disposition"]
    assert r.headers["accept-ranges"] == "bytes"


def test_folder_download_returns_zip_with_intact_files(sarca: SarcaClient, storage: str) -> None:
    assert sarca.create_folder(storage, "bundle").status_code in (200, 201)
    payloads = {"a.txt": b"alpha" * 100, "b.bin": media.blob(50_000, seed=3)}
    for name, data in payloads.items():
        assert sarca.upload(storage, name, data, path="bundle/").ok
        sarca.wait_for_file(storage, f"bundle/{name}")

    r = sarca.download(storage, "bundle/")
    assert r.status_code == 200, r.text
    assert r.headers["content-type"] == "application/zip"

    with zipfile.ZipFile(io.BytesIO(r.content)) as zf:
        assert set(zf.namelist()) == set(payloads)
        for name, data in payloads.items():
            assert hashlib.sha256(zf.read(name)).hexdigest() == sha256(data)


def test_download_of_missing_file_is_404(sarca: SarcaClient, storage: str) -> None:
    assert sarca.download(storage, "nope.txt").status_code == 404


def test_upload_without_write_access_is_rejected(
    sarca: SarcaClient, storage: str, base_url: str
) -> None:
    email = "reader@sarca.test"
    sarca.create_user(email, "reader-pass-123")
    assert sarca.grant_access(storage, email, "R").status_code in (200, 201, 204)

    reader = SarcaClient(base_url)
    try:
        reader.login(email, "reader-pass-123")
        result_status = reader.post(
            f"/api/storages/{storage}/files/upload",
            files={"file": ("x.txt", b"nope", "text/plain")},
            data={"path": "", "filename": "x.txt"},
        ).status_code
        # check_access reports a missing grant as "storage does not exist" (404) rather
        # than 403, so a probe can't confirm a storage id it may not touch.
        assert result_status in (403, 404), result_status
        # …but reading works.
        assert reader.get(f"/api/storages/{storage}/files/tree/").status_code == 200
    finally:
        reader.close()
        users = {u["email"]: u["id"] for u in sarca.list_users()}
        if email in users:
            sarca.delete_user(users[email])


def test_upload_is_visible_in_tree_and_deletable(sarca: SarcaClient, storage: str) -> None:
    assert sarca.upload(storage, "temp.txt", b"bye").ok
    sarca.wait_for_file(storage, "temp.txt")
    assert "temp.txt" in {e["name"] for e in sarca.tree(storage)}

    assert sarca.delete_file(storage, "temp.txt").status_code in (200, 204)
    assert "temp.txt" not in {e["name"] for e in sarca.tree(storage)}
    # Soft delete: it lands in the trash rather than vanishing.
    trash = sarca.get(f"/api/storages/{storage}/trash").json()
    assert "temp.txt" in {e["name"] for e in trash}


def test_upload_progress_stream_reports_phases(sarca: SarcaClient, storage: str) -> None:
    result = sarca.upload(storage, "progress.bin", media.blob(1_500_000, seed=5))
    assert result.ok, result.error
    phases = result.phases
    assert phases[0] == "spooled", phases
    assert "telegram" in phases, phases
    assert phases[-1] == "done", phases

    telegram_events = [e for e in result.events if e.get("phase") == "telegram"]
    assert telegram_events[-1]["uploaded"] == telegram_events[-1]["total"]
    assert telegram_events[-1]["chunks"] == 2


def test_download_reads_through_the_local_chunk_cache(
    sarca: SarcaClient, storage: str, mock
) -> None:
    """The second download of the same file must not hit Telegram again."""
    data = media.blob(200_000, seed=13)
    assert sarca.upload(storage, "cached.bin", data).ok
    sarca.wait_for_file(storage, "cached.bin")

    assert sarca.download_bytes(storage, "cached.bin") == data
    before = mock.calls("getFile")
    assert sarca.download_bytes(storage, "cached.bin") == data
    assert mock.calls("getFile") == before, "second download should be served from disk cache"
