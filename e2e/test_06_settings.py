"""Scenario 6 — settings actually change behaviour, verified through the server log.

Covers the app setting exposed over the API (trash retention) and the deployment
settings from sarca.conf that change the Telegram data path: chunk size, rate limit,
multi-channel replication.
"""

from __future__ import annotations

import sqlite3
import time
import uuid

import pytest

from helpers import media
from helpers.api import SarcaClient, new_bot_token, new_chat_id
from helpers.server import SarcaServer

pytestmark = pytest.mark.mock_only


# ------------------------------------------------------------- trash retention


def test_trash_retention_default_and_update(sarca: SarcaClient) -> None:
    original = sarca.get_trash_settings()["retention_days"]
    try:
        assert 1 <= original <= 30
        r = sarca.set_trash_settings(7)
        assert r.status_code == 200, r.text
        assert r.json()["retention_days"] == 7
        assert sarca.get_trash_settings()["retention_days"] == 7
    finally:
        sarca.set_trash_settings(original)


@pytest.mark.parametrize("bad", [0, -1, 31, 9999])
def test_trash_retention_rejects_out_of_range(sarca: SarcaClient, bad: int) -> None:
    r = sarca.set_trash_settings(bad)
    assert r.status_code == 400, f"retention_days={bad} should be refused: {r.text}"


def test_trash_retention_survives_restart(sarca: SarcaClient, server: SarcaServer, credentials) -> None:
    original = sarca.get_trash_settings()["retention_days"]
    try:
        assert sarca.set_trash_settings(3).status_code == 200
        server.restart()
        sarca.login(*credentials)
        assert sarca.get_trash_settings()["retention_days"] == 3
    finally:
        sarca.set_trash_settings(original)


@pytest.mark.slow
def test_expired_trash_is_purged_according_to_the_setting(
    sarca: SarcaClient, storage: str, server: SarcaServer, workdir, mock, credentials
) -> None:
    """Backdate a trashed file past the retention window; the purge loop must eat it."""
    original = sarca.get_trash_settings()["retention_days"]
    try:
        assert sarca.set_trash_settings(1).status_code == 200
        assert sarca.upload(storage, "old.txt", b"stale").ok
        sarca.wait_for_file(storage, "old.txt")
        assert sarca.delete_file(storage, "old.txt").status_code in (200, 204)

        db = sqlite3.connect(workdir / "sarca.sqlite")
        db.execute(
            "UPDATE files SET deleted_at = datetime('now', '-10 days') WHERE path = 'old.txt'"
        )
        db.commit()
        db.close()

        # The purge loop runs once at boot, so a restart triggers it immediately.
        offset = server.log_offset()
        server.restart()
        sarca.login(*credentials)
        server.wait_for_log("[TRASH PURGE] permanently deleting", offset=offset, timeout=30)

        trash = sarca.get(f"/api/storages/{storage}/trash").json()
        assert "old.txt" not in {e["name"] for e in trash}
    finally:
        sarca.set_trash_settings(original)


def test_trash_settings_require_superuser(
    sarca: SarcaClient, base_url: str
) -> None:
    email = f"plain-{uuid.uuid4().hex[:6]}@sarca.test"
    assert sarca.create_user(email, "plain-pass-123").status_code == 201
    user_id = next(u["id"] for u in sarca.list_users() if u["email"] == email)
    client = SarcaClient(base_url)
    try:
        client.login(email, "plain-pass-123")
        assert client.set_trash_settings(5).status_code == 403
    finally:
        client.close()
        sarca.delete_user(user_id)


# ------------------------------------------------------- chunk size (sarca.conf)


def test_chunk_size_setting_drives_the_number_of_telegram_documents(
    sarca: SarcaClient, storage: str, mock
) -> None:
    """TELEGRAM_CHUNK_SIZE_MB=1 in this harness → one document per megabyte."""
    data = media.blob(int(3.2 * 1024 * 1024), seed=77)
    before = mock.calls("sendDocument")
    assert sarca.upload(storage, "chunked.bin", data).ok
    sarca.wait_for_file(storage, "chunked.bin")
    assert mock.calls("sendDocument") - before == 4, mock.stats()

    sizes = mock.stats()["document_sizes"]
    assert 1024 * 1024 in sizes, "expected full 1 MB chunks"


# ---------------------------------------------------------------- replication


@pytest.mark.slow
def test_second_channel_gets_replicas_via_copy_message(
    sarca: SarcaClient, server: SarcaServer, mock
) -> None:
    """A storage with two channels replicates chunks with copyMessage, not re-upload."""
    storage = sarca.create_storage(
        chat_ids=[new_chat_id(), new_chat_id()], bot_token=new_bot_token()
    )["id"]
    try:
        offset = server.log_offset()
        before_copies = mock.calls("copyMessage")
        assert sarca.upload(storage, "replicated.bin", media.blob(200_000, seed=8)).ok
        sarca.wait_for_file(storage, "replicated.bin")

        deadline = time.time() + 40
        while time.time() < deadline and mock.calls("copyMessage") == before_copies:
            time.sleep(0.5)
        assert mock.calls("copyMessage") > before_copies, "replication never copied the chunk"

        detail = sarca.storage_detail(storage)
        assert detail["replication"]["pending"] == 0, detail["replication"]
        server.assert_no_log("replication failed", offset=offset)
    finally:
        sarca.delete_storage(storage)


# -------------------------------------------------------------- rate limiting


@pytest.mark.slow
def test_rate_limit_setting_throttles_telegram_calls(e2e_tmp, telegram, mock) -> None:
    """A dedicated server with TELEGRAM_RATE_LIMIT=1 must wait for a free bot slot."""
    server = SarcaServer(
        root=e2e_tmp / f"ratelimited-{uuid.uuid4().hex[:6]}",
        telegram_base_url=telegram.base_url,
        env_extra={
            "TELEGRAM_RATE_LIMIT": "1",
            "TELEGRAM_CHUNK_SIZE_MB": "1",
            "SARCA_TELEGRAM_PACING_MS": "20",
        },
    )
    server.start()
    client = SarcaClient(server.base_url)
    try:
        client.login(server.email, server.password)
        storage = client.create_storage(
            chat_ids=[new_chat_id()], bot_token=new_bot_token()
        )["id"]

        offset = server.log_offset()
        # Three documents (2 chunks + thumb-ish work) against a 1/min budget: the
        # scheduler has to park after the first one.
        client.upload(
            storage,
            "throttled.bin",
            media.blob(int(1.5 * 1024 * 1024), seed=4),
            timeout=15,
        )
    except Exception:  # the upload is expected to still be waiting when we stop
        pass
    finally:
        server.wait_for_log("waiting for getting a token", offset=offset, timeout=30)
        client.close()
        server.stop()
