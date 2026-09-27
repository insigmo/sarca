"""Scenario 9 — upload throughput and retry naming.

Covers two guarantees that are easy to regress and painful for users:

* uploads for different storages reach Telegram **at the same time** instead of
  queuing behind one global lock;
* retrying a file that did not make it keeps its **original name** — no
  "photo (1).jpg" appearing next to "photo.jpg".
"""

from __future__ import annotations

import concurrent.futures
import time
import uuid

import httpx
import pytest

from helpers.api import SarcaClient, sha256
from helpers.sync_client import run_sync

pytestmark = [pytest.mark.mock_only, pytest.mark.slow]

# Long enough that a serialized relay cannot be mistaken for a parallel one, short
# enough to keep the suite quick.
RELAY_LATENCY_S = 1.0


@pytest.fixture
def two_storages(sarca: SarcaClient, telegram) -> list[str]:
    """Two storages, each with its own bot token (one worker per storage)."""
    made = []
    for _ in range(2):
        st = sarca.create_storage(
            name=f"e2e-par-{uuid.uuid4().hex[:8]}",
            chat_ids=[telegram.new_chat_id()],
            bot_token=telegram.new_token(),
        )
        made.append(st["id"])
    yield made
    for sid in made:
        sarca.delete_storage(sid)


def test_uploads_to_different_storages_overlap(
    sarca: SarcaClient, two_storages: list[str], mock
) -> None:
    """The storage manager must not serialize every Telegram relay process-wide."""
    mock.reset_calls()
    mock.set_latency(sendDocument=RELAY_LATENCY_S)
    try:
        with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
            futures = [
                pool.submit(sarca.upload, sid, f"parallel-{i}.bin", b"x" * 4096)
                for i, sid in enumerate(two_storages)
            ]
            results = [f.result() for f in futures]
    finally:
        mock.clear_latency()

    assert all(r.ok for r in results), [r.error for r in results]
    assert mock.max_concurrent("sendDocument") >= 2, (
        "relays for two storages ran one after another; the storage manager is "
        "serializing uploads again"
    )


def test_retry_after_aborted_upload_keeps_original_name(
    sarca: SarcaClient, storage: str, mock
) -> None:
    """A client that dies mid-relay must not cost the file its name.

    The abandoned row still occupies the path, so without the reclaim in
    `upload_anyway_from_path_with_progress` the retry lands as "name (1).bin".
    """
    name = "aborted.bin"
    data = b"retry-me" * 512
    digest = sha256(data)

    mock.set_latency(sendDocument=10.0)
    try:
        with pytest.raises(httpx.TimeoutException):
            sarca.upload(storage, name, data, content_hash=digest, timeout=1.0)
    finally:
        mock.clear_latency()

    info = sarca.info(storage, name)
    assert info["is_uploaded"] is False, (
        "expected the killed upload to leave an unfinished row holding the path"
    )

    result = sarca.upload(storage, name, data, content_hash=digest)
    assert result.ok, result.error

    names = sorted(e["name"] for e in sarca.tree(storage))
    assert names == [name], f"retry renamed the file: {names}"
    assert sarca.download_bytes(storage, name) == data


def test_sync_retry_after_failed_relay_keeps_original_name(
    sarca: SarcaClient, storage: str, mock, tmp_path
) -> None:
    """Same guarantee, end to end through the real client sync engine."""
    gallery = tmp_path / "dcim"
    gallery.mkdir()
    (gallery / "IMG_0001.jpg").write_bytes(b"\xff\xd8\xff" + b"a" * 4096)
    (gallery / "IMG_0002.jpg").write_bytes(b"\xff\xd8\xff" + b"b" * 4096)

    def sync(*, retry_failed: bool = False):
        return run_sync(
            base_url=sarca.base_url,
            email="e2e@sarca.test",
            password="e2e-password-123",
            storage_id=storage,
            local_dir=gallery,
            data_dir=tmp_path / "sync-state",
            remote_root="Camera",
            retry_failed=retry_failed,
        )

    # More than the API layer's own 5xx retry budget, so one file really fails.
    mock.inject_failure("sendDocument", times=99, status=500)
    first = sync()
    assert first.errors, "expected the injected Telegram failure to surface"

    mock.clear_injections()
    # The failure armed the retry backoff (60s on the first strike). A user
    # would press "Upload now"; the driver does the same so the retry is not
    # silently deferred past the test.
    second = sync(retry_failed=True)
    assert not second.errors, second.errors

    names = sorted(e["name"] for e in sarca.tree(storage, "Camera"))
    assert names == ["IMG_0001.jpg", "IMG_0002.jpg"], f"retry renamed a file: {names}"


def test_a_replacement_handed_off_mid_relay_is_recognised_at_its_path(
    sarca: SarcaClient, storage: str, mock
) -> None:
    """A new version whose sender hangs up after `spooled` must still be the
    version the path reports.

    That report is all the sync client has to go on once it has handed a file
    off: its next pass asks the path and sends the file again unless the answer
    is "these bytes, being stored" or "these bytes, stored". When the new
    version landed as "name (1).ext" instead, the path kept answering with the
    old bytes, and a video recorded while it was being synced was sent 180
    times over.
    """
    name = "recording.mp4"
    old, new = b"first" * 1024, b"second" * 4096
    assert sarca.upload(storage, name, old, content_hash=sha256(old)).ok

    mock.set_latency(sendDocument=10.0)
    try:
        with pytest.raises(httpx.TimeoutException):
            sarca.upload(
                storage,
                name,
                new,
                content_hash=sha256(new),
                on_conflict="replace",
                timeout=1.0,
            )
        info = sarca.info(storage, name)
        assert (info["size"], info["is_uploaded"], info["is_relaying"]) == (
            len(new),
            False,
            True,
        ), info
    finally:
        mock.clear_latency()

    deadline = time.time() + 60
    while not (info := sarca.info(storage, name))["is_uploaded"]:
        assert time.time() < deadline, f"the new version never finished: {info}"
        time.sleep(0.2)
    assert info["content_hash"] == sha256(new)

    assert sorted(e["name"] for e in sarca.tree(storage)) == [name]
    assert sarca.download_bytes(storage, name) == new
    trashed = [e["name"] for e in sarca.get(f"/api/storages/{storage}/trash").json()]
    assert any(n.startswith("recording (replaced ") for n in trashed), trashed
