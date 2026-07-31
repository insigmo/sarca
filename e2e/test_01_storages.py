"""Scenario 1 — storage lifecycle: create, inspect, bind a bot, edit channels, delete."""

from __future__ import annotations

import uuid

import pytest

from helpers.api import SarcaClient, new_bot_token, new_chat_id
from helpers.server import SarcaServer

pytestmark = pytest.mark.mock_only


def test_create_storage_returns_id_and_shows_up_in_list(sarca: SarcaClient, mock) -> None:
    name = f"e2e-create-{uuid.uuid4().hex[:6]}"
    storage = sarca.create_storage(name=name, chat_ids=[new_chat_id()])
    try:
        assert storage["id"]
        assert storage["name"] == name
        assert any(s["id"] == storage["id"] for s in sarca.list_storages())
    finally:
        sarca.delete_storage(storage["id"])


def test_channel_name_is_resolved_from_telegram(sarca: SarcaClient, mock) -> None:
    """With a bot bound, Sarca calls getChat to auto-fill a channel's display name."""
    storage = sarca.create_storage(chat_ids=[new_chat_id()], bot_token=new_bot_token())
    sid = storage["id"]
    try:
        # The first channel is created before any bot exists, so it keeps a fallback name.
        assert sarca.storage_detail(sid)["channels"][0]["name"] == "Channel 1"

        chat_id = new_chat_id()
        before = mock.calls("getChat")
        r = sarca.post(f"/api/storages/{sid}/channels", json={"chat_id": chat_id})
        assert r.status_code in (200, 201), r.text
        assert r.json()["name"] == f"Mock Channel {chat_id}"
        assert mock.calls("getChat") > before
    finally:
        sarca.delete_storage(sid)


def test_bot_binding_validates_token_via_get_me(sarca: SarcaClient, mock) -> None:
    storage = sarca.create_storage(chat_ids=[new_chat_id()])
    try:
        before = mock.calls("getMe")
        bot = sarca.set_bot(storage["id"], new_bot_token())
        assert mock.calls("getMe") == before + 1
        assert bot["name"].startswith("mock_")
        assert "•" in bot["token_masked"], "raw token must never leave the server"

        detail = sarca.storage_detail(storage["id"])
        assert detail["bot"]["id"] == bot["id"]
    finally:
        sarca.delete_storage(storage["id"])


def test_invalid_bot_token_is_rejected(sarca: SarcaClient) -> None:
    storage = sarca.create_storage(chat_ids=[new_chat_id()])
    try:
        r = sarca.put(f"/api/storages/{storage['id']}/bot", json={"token": "not-a-token"})
        assert r.status_code == 400, r.text
        assert "invalid" in r.text.lower()
    finally:
        sarca.delete_storage(storage["id"])


def test_storage_name_must_be_unique_per_user(sarca: SarcaClient) -> None:
    name = f"e2e-dup-{uuid.uuid4().hex[:6]}"
    first = sarca.create_storage(name=name, chat_ids=[new_chat_id()])
    try:
        r = sarca.post(
            "/api/storages",
            json={"name": name, "channels": [{"chat_id": new_chat_id()}]},
        )
        assert r.status_code == 409, r.text
    finally:
        sarca.delete_storage(first["id"])


def test_chat_id_cannot_be_reused_by_two_storages(sarca: SarcaClient) -> None:
    chat_id = new_chat_id()
    first = sarca.create_storage(chat_ids=[chat_id])
    try:
        r = sarca.post(
            "/api/storages",
            json={"name": f"e2e-{uuid.uuid4().hex[:6]}", "channels": [{"chat_id": chat_id}]},
        )
        assert r.status_code == 409, r.text
    finally:
        sarca.delete_storage(first["id"])


def test_storage_requires_at_least_one_channel_and_allows_at_most_three(
    sarca: SarcaClient,
) -> None:
    r = sarca.post("/api/storages", json={"name": f"e2e-{uuid.uuid4().hex[:6]}", "channels": []})
    assert r.status_code == 400, r.text

    r = sarca.post(
        "/api/storages",
        json={
            "name": f"e2e-{uuid.uuid4().hex[:6]}",
            "channels": [{"chat_id": new_chat_id()} for _ in range(4)],
        },
    )
    assert r.status_code == 409, r.text


def test_add_and_remove_channels(sarca: SarcaClient) -> None:
    storage = sarca.create_storage(chat_ids=[new_chat_id()], bot_token=new_bot_token())
    sid = storage["id"]
    try:
        extra = new_chat_id()
        r = sarca.post(f"/api/storages/{sid}/channels", json={"chat_id": extra})
        assert r.status_code in (200, 201), r.text
        channel_id = r.json()["id"]
        assert len(sarca.storage_detail(sid)["channels"]) == 2

        r = sarca.delete(f"/api/storages/{sid}/channels/{channel_id}")
        assert r.status_code in (200, 204), r.text
        assert len(sarca.storage_detail(sid)["channels"]) == 1

        # The last active channel must stay: a storage without one cannot serve files.
        last_id = sarca.storage_detail(sid)["channels"][0]["id"]
        r = sarca.delete(f"/api/storages/{sid}/channels/{last_id}")
        assert r.status_code == 409, r.text
    finally:
        sarca.delete_storage(sid)


def test_rename_storage(sarca: SarcaClient) -> None:
    storage = sarca.create_storage(chat_ids=[new_chat_id()])
    sid = storage["id"]
    try:
        new_name = f"renamed-{uuid.uuid4().hex[:6]}"
        r = sarca.put(f"/api/storages/{sid}", json={"name": new_name})
        assert r.status_code == 200, r.text
        assert sarca.storage_detail(sid)["name"] == new_name
    finally:
        sarca.delete_storage(sid)


def test_delete_storage_removes_it_from_the_list(sarca: SarcaClient) -> None:
    storage = sarca.create_storage(chat_ids=[new_chat_id()])
    sid = storage["id"]
    assert sarca.delete_storage(sid).status_code in (200, 204)
    assert all(s["id"] != sid for s in sarca.list_storages())
    assert sarca.get(f"/api/storages/{sid}").status_code in (403, 404)


def test_storage_endpoints_require_authentication(sarca: SarcaClient, base_url: str) -> None:
    anon = SarcaClient(base_url)
    try:
        assert anon.get("/api/storages").status_code == 401
        assert anon.post("/api/storages", json={"name": "x", "channels": []}).status_code == 401
    finally:
        anon.close()


def test_creating_storage_is_logged(sarca: SarcaClient, server: SarcaServer) -> None:
    offset = server.log_offset()
    storage = sarca.create_storage(chat_ids=[new_chat_id()])
    try:
        line = server.wait_for_log("[STORAGES SERVICE] Created storage", offset=offset, timeout=10)
        assert storage["id"] in line
        server.wait_for_log("[ACCESS REPO] granted access", offset=offset, timeout=10)
    finally:
        sarca.delete_storage(storage["id"])
