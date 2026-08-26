"""Scenario 15 — the backup file is the instance.

The promise the feature makes is narrow and testable: download one file, carry it
to a different Sarca with its own empty database, restore it there, and the same
storages and the same files are reachable — file bytes included, because those
never left Telegram.
"""

from __future__ import annotations

import uuid

import pytest

from helpers import media
from helpers.api import SarcaClient
from helpers.server import SarcaServer

pytestmark = pytest.mark.mock_only

MAGIC = b"SARCABK1"
BACKUP_PASSWORD = "a good long backup password"


def _fresh_server(e2e_tmp, telegram, email: str, password: str) -> SarcaServer:
    """A second Sarca, own WORK_DIR and database, same fake Telegram behind it."""
    server = SarcaServer(
        root=e2e_tmp / f"restore-target-{uuid.uuid4().hex[:6]}",
        telegram_base_url=telegram.base_url,
        email=email,
        password=password,
        env_extra={"TELEGRAM_CHUNK_SIZE_MB": "1", "SARCA_TELEGRAM_PACING_MS": "20"},
    )
    server.start()
    return server


def test_backup_downloads_a_self_describing_archive(sarca: SarcaClient) -> None:
    r = sarca.create_backup()
    assert r.status_code == 200, r.text
    assert r.content.startswith(MAGIC), r.content[:16]
    assert ".sarcabak" in r.headers.get("content-disposition", "")


def test_a_password_changes_the_bytes_on_disk(sarca: SarcaClient) -> None:
    plain = sarca.create_backup().content
    secret = sarca.create_backup(BACKUP_PASSWORD).content
    assert plain.startswith(MAGIC) and secret.startswith(MAGIC)
    # Byte 9 is the flags byte; bit0 says the payload is encrypted.
    assert plain[9] & 1 == 0
    assert secret[9] & 1 == 1


def test_backup_and_restore_require_superuser(sarca: SarcaClient, base_url: str) -> None:
    email = f"plain-{uuid.uuid4().hex[:6]}@sarca.test"
    assert sarca.create_user(email, "plain-pass-123").status_code == 201
    user_id = next(u["id"] for u in sarca.list_users() if u["email"] == email)
    client = SarcaClient(base_url)
    try:
        client.login(email, "plain-pass-123")
        assert client.create_backup().status_code == 403
        assert client.restore_backup(MAGIC + b"junk").status_code == 403
    finally:
        client.close()
        sarca.delete_user(user_id)


def test_a_wrong_password_is_refused_and_says_so(
    sarca: SarcaClient, e2e_tmp, telegram, base_url: str
) -> None:
    """Refused on the target server, so a failed attempt cannot damage the source."""
    archive = sarca.create_backup(BACKUP_PASSWORD).content

    server = _fresh_server(e2e_tmp, telegram, "target@sarca.test", "target-pass-123")
    client = SarcaClient(server.base_url)
    try:
        client.login(server.email, server.password)

        no_password = client.restore_backup(archive)
        assert no_password.status_code == 400, no_password.text
        assert "password" in no_password.text.lower()

        wrong = client.restore_backup(archive, "not the password")
        assert wrong.status_code == 400, wrong.text

        # Nothing was applied: the target still answers as itself.
        assert client.get_trash_settings()["retention_days"] >= 1
    finally:
        client.close()
        server.stop()


def test_a_file_that_is_not_a_backup_is_refused(
    sarca: SarcaClient, e2e_tmp, telegram
) -> None:
    server = _fresh_server(e2e_tmp, telegram, "target@sarca.test", "target-pass-123")
    client = SarcaClient(server.base_url)
    try:
        client.login(server.email, server.password)
        r = client.restore_backup(b"PK\x03\x04 definitely a zip")
        assert r.status_code == 400, r.text
    finally:
        client.close()
        server.stop()


@pytest.mark.slow
def test_a_backup_restored_elsewhere_serves_the_same_storages_and_files(
    sarca: SarcaClient, storage: str, e2e_tmp, telegram, credentials
) -> None:
    payload = media.blob(300_000, seed=15)
    assert sarca.upload(storage, "carried.bin", payload).ok
    sarca.wait_for_file(storage, "carried.bin")
    source_name = next(s["name"] for s in sarca.list_storages() if s["id"] == storage)
    source_events = len(sarca.changelog(storage)["events"])

    archive = sarca.create_backup(BACKUP_PASSWORD)
    assert archive.status_code == 200, archive.text

    # A different install: its own WORK_DIR and database, nothing in it yet. The
    # superuser is configured with the same credentials, which is what the
    # operator would do when moving a server.
    target = _fresh_server(e2e_tmp, telegram, credentials[0], "moved-pass-123")
    client = SarcaClient(target.base_url)
    try:
        client.login(target.email, target.password)
        assert client.list_storages() == [], "the target must start empty"

        result = client.restore_backup(archive.content, BACKUP_PASSWORD)
        assert result.status_code == 200, result.text
        body = result.json()
        assert body["rows"] > 0 and body["tables"] > 0, body
        assert body["skipped_tables"] == [], body

        # The restore replaced the accounts, so the old token is gone with them.
        client.login(target.email, target.password)

        names = {s["name"] for s in client.list_storages()}
        assert source_name in names, names

        tree = client.tree(storage)
        assert "carried.bin" in {e["name"] for e in tree}
        # The bytes were never in the archive — this is the restored bot token
        # and message ids fetching them back out of Telegram.
        assert client.download_bytes(storage, "carried.bin") == payload

        # The sync log is restored as it was, not re-synthesised by the `files`
        # triggers while the rows were being copied in.
        assert len(client.changelog(storage)["events"]) == source_events
    finally:
        client.close()
        target.stop()
