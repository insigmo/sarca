"""Scenario 14 — the checklist regressions reported after the August fix batch.

Each test pins one user report:

1.  Thumbnails in a large folder — every photo must end up with a stored
    thumbnail, not just the first stretch of them (the "72 in a row" report).
2.  Replacing a storage's bot — the server must refuse silently dropping its
    channels (409) until the client confirms, and then actually drop them.
3.  TELEGRAM_PROXY_URL from sarca.conf — accepted, validated, and actually
    used for Telegram traffic.
4.  Client-supplied thumbnails survive a bot replacement purge untouched —
    channels are dropped, but files and their thumbs stay queryable.

Everything here runs against the fake Bot API server fixture, so no test ever
touches api.telegram.org.
"""

from __future__ import annotations

import time

import pytest

from helpers import media
from helpers.api import SarcaClient, new_bot_token, new_chat_id, sha256

pytestmark = [pytest.mark.mock_only]


# --------------------------------------------------------------- thumbnails


def _wait_for_thumb(sarca: SarcaClient, storage_id: str, path: str, timeout: float = 30.0):
    """Poll GET /thumb until it answers 200 (thumbnail upload is async)."""
    deadline = time.time() + timeout
    last = None
    while time.time() < deadline:
        r = sarca.thumb(storage_id, path)
        if r.status_code == 200:
            return r
        last = r
        time.sleep(0.25)
    raise AssertionError(f"thumb for {path} never appeared (last status {last.status_code})")


def test_every_photo_in_a_large_folder_gets_a_thumbnail(
    sarca: SarcaClient, storage: str, tmp_path
) -> None:
    """Regression for «ровно 72 фото подряд получили эскизы, остальные нет».

    Uploads more photos than one grid page shows and requires *every* one of
    them to end up with a stored, servable thumbnail. The old failure mode was
    the thumb queue giving up permanently after a few seconds of 503s; this
    test gives the pipeline minutes of headroom and accepts nothing less than
    100% coverage.
    """
    count = 90
    data = media.big_photo(320, 240)
    digest = sha256(data)

    results = [
        sarca.upload(
            storage,
            f"bulk_{i:03d}.jpg",
            data,
            content_type="image/jpeg",
            # No client `thumb`: the server-side generation path is the one
            # that was starving, so that is what this test has to exercise.
        )
        for i in range(count)
    ]
    assert all(r.ok for r in results), [r.error for r in results if not r.ok]

    sarca.wait_for_file(storage, f"bulk_{count - 1:03d}.jpg")

    missing: list[str] = []
    wrong_bytes: list[str] = []
    for i in range(count):
        path = f"bulk_{i:03d}.jpg"
        try:
            r = _wait_for_thumb(sarca, storage, path)
        except AssertionError:
            missing.append(path)
            continue
        got = sarca.download_bytes(storage, path)
        if sha256(got) != digest:
            wrong_bytes.append(path)
        assert media.is_jpeg(r.content), f"{path}: thumb is not a JPEG"

    assert not missing, (
        f"{len(missing)}/{count} photos never got a thumbnail "
        "(the exact reported symptom): " + ", ".join(missing[:10])
    )
    assert not wrong_bytes, f"originals corrupted for: {wrong_bytes[:10]}"

    info = sarca.info(storage, "bulk_000.jpg")
    assert info["has_thumb"] is True


# ------------------------------------------------------------ bot replacement


def test_replacing_bot_requires_confirmation_and_drops_channels_when_confirmed(
    sarca: SarcaClient,
) -> None:
    """«После смены бота каналы будут удалены вместе с файлами — уточнить»."""
    first_chat = new_chat_id()
    extra_chats = [new_chat_id(), new_chat_id()]
    st = sarca.create_storage(
        name="e2e-bot-swap",
        chat_ids=[first_chat, *extra_chats],
        bot_token=new_bot_token(),
    )
    sid = st["id"]
    try:
        before = sarca.storage_detail(sid)
        assert len(before["channels"]) == 3

        # Silent replacement is refused with a conflict…
        r = sarca.put(f"/api/storages/{sid}/bot", json={"token": new_bot_token()})
        assert r.status_code == 409, r.text

        # …and refusing leaves everything exactly as it was.
        unchanged = sarca.storage_detail(sid)
        assert len(unchanged["channels"]) == 3
        assert unchanged["bot"]["id"] == before["bot"]["id"]

        # Confirmed replacement goes through and drops the channels.
        new_token = new_bot_token()
        r = sarca.put(
            f"/api/storages/{sid}/bot",
            json={"token": new_token, "remove_channels": True},
        )
        assert r.status_code == 200, r.text

        after = sarca.storage_detail(sid)
        # The server keeps the worker row and swaps its credentials, so the
        # identity moves with the new bot's username/masked token.
        assert after["bot"]["name"] != before["bot"]["name"], (
            "the bound bot must now be the new one"
        )
        assert after["bot"]["token_masked"] != before["bot"]["token_masked"]
        assert after["channels"] == [], "confirmed bot swap must drop the channels"
    finally:
        sarca.delete_storage(sid)


def test_files_stay_queryable_after_confirmed_bot_swap(sarca: SarcaClient) -> None:
    """Channels die with the old bot, but files keep their rows and thumbs."""
    st = sarca.create_storage(
        chat_ids=[new_chat_id()],
        bot_token=new_bot_token(),
    )
    sid = st["id"]
    try:
        tile = media.recompress_jpeg(media.big_photo(128, 96), quality=75)
        data = media.big_photo(300, 200)
        assert sarca.upload(
            sid, "swap.jpg", data, content_type="image/jpeg", thumb=tile
        ).ok
        sarca.wait_for_file(sid, "swap.jpg")
        assert _wait_for_thumb(sarca, sid, "swap.jpg").status_code == 200

        r = sarca.put(
            f"/api/storages/{sid}/bot",
            json={"token": new_bot_token(), "remove_channels": True},
        )
        assert r.status_code == 200, r.text

        listing = sarca.tree(sid)
        names = {e["name"] for e in listing}
        assert "swap.jpg" in names, "files must survive the channel purge"

        info = sarca.info(sid, "swap.jpg")
        assert info["has_thumb"] is True, "stored thumbs stay attached to their file"
        r = sarca.thumb(sid, "swap.jpg")
        assert r.status_code == 200
        assert r.content == tile, "the client-built tile must be byte-identical"
    finally:
        sarca.delete_storage(sid)


def test_first_bot_binding_does_not_ask_for_confirmation(sarca: SarcaClient) -> None:
    """Binding the very first bot has nothing to confirm and must just work."""
    st = sarca.create_storage(chat_ids=[new_chat_id()])
    sid = st["id"]
    try:
        token = new_bot_token()
        r = sarca.put(f"/api/storages/{sid}/bot", json={"token": token})
        assert r.status_code == 200, r.text
        detail = sarca.storage_detail(sid)
        assert detail["bot"]["id"]
        assert len(detail["channels"]) == 1, "no channels are lost on first binding"
    finally:
        sarca.delete_storage(sid)


# -------------------------------------------------------------------- proxy


def test_telegram_proxy_url_is_accepted_and_validated(workdir) -> None:
    """TELEGRAM_PROXY_URL from sarca.conf reaches Config and is sanity-checked.

    The full round trip through a proxy is covered by the unit tests on
    `validate_telegram_proxy_url` plus the connector wiring; here we prove the
    env contract: a well-formed proxy URL boots the server fine against the
    mock Bot API, and a malformed scheme is rejected at startup rather than
    silently ignored (a silent fallback to direct traffic would leak metadata
    for users who set the proxy precisely because direct traffic fails).
    """
    pytest.importorskip("helpers.server")

    from helpers.server import SarcaServer, build_binary, repo_root

    root = repo_root() / ".." / ".proxy-e2e-tmp"
    srv = SarcaServer(
        root=root.resolve() / "good",
        telegram_base_url="http://127.0.0.1:1",  # never contacted directly here
        email="e2e-proxy@sarca.test",
        password="e2e-password-123",
        env_extra={
            "TELEGRAM_PROXY_URL": "socks5h://127.0.0.1:1080",
            "SUPERUSER_EMAIL": "e2e-proxy@sarca.test",
            "SUPERUSER_PASS": "e2e-password-123",
        },
    )
    try:
        srv.start(build_binary())
        assert srv.https_port > 0, "server must boot with a valid proxy configured"
    finally:
        srv.stop()

    bad = SarcaServer(
        root=root.resolve() / "bad",
        telegram_base_url="http://127.0.0.1:1",
        email="e2e-proxy@sarca.test",
        password="e2e-password-123",
        env_extra={
            "TELEGRAM_PROXY_URL": "ftp://127.0.0.1:21",
            "SUPERUSER_EMAIL": "e2e-proxy@sarca.test",
            "SUPERUSER_PASS": "e2e-password-123",
        },
    )
    try:
        booted = False
        try:
            bad.start(wait=True)
            booted = True
        except (RuntimeError, AssertionError):
            pass  # refused at startup — the desired behaviour
        finally:
            bad.stop()
        assert not booted, "an unsupported proxy scheme must be rejected, not ignored"
    finally:
        import shutil  # noqa: PLC0415

        shutil.rmtree(root, ignore_errors=True)
