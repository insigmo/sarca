"""Scenario 5 — user lifecycle: create, log in, share a storage, delete."""

from __future__ import annotations

import uuid

import pytest

from helpers.api import SarcaClient, new_bot_token, new_chat_id
from helpers.server import SarcaServer

pytestmark = pytest.mark.mock_only


@pytest.fixture
def temp_user(sarca: SarcaClient) -> dict[str, str]:
    """A throwaway account, removed afterwards even if the test fails."""
    email = f"user-{uuid.uuid4().hex[:8]}@sarca.test"
    password = "temp-password-123"
    r = sarca.create_user(email, password)
    assert r.status_code == 201, r.text
    user = next(u for u in sarca.list_users() if u["email"] == email)
    yield {"email": email, "password": password, "id": user["id"]}
    sarca.delete_user(user["id"])


def test_created_user_appears_in_the_list_and_can_log_in(
    sarca: SarcaClient, temp_user: dict[str, str], base_url: str
) -> None:
    listed = {u["email"]: u for u in sarca.list_users()}
    assert temp_user["email"] in listed
    assert listed[temp_user["email"]]["is_superuser"] is False
    # Admin-created accounts skip the e-mail verification gate.
    assert listed[temp_user["email"]]["email_verified"] is True

    client = SarcaClient(base_url)
    try:
        tokens = client.login(temp_user["email"], temp_user["password"])
        assert tokens["access_token"]
        assert client.get("/api/storages").json()["storages"] == []
    finally:
        client.close()


def test_duplicate_email_is_rejected(sarca: SarcaClient, temp_user: dict[str, str]) -> None:
    r = sarca.create_user(temp_user["email"], "another-pass-123")
    assert r.status_code == 409, r.text


def test_non_superuser_cannot_manage_users(
    sarca: SarcaClient, temp_user: dict[str, str], base_url: str
) -> None:
    client = SarcaClient(base_url)
    try:
        client.login(temp_user["email"], temp_user["password"])
        assert client.get("/api/users").status_code == 403
        assert client.create_user("sneaky@sarca.test", "pass-123456").status_code == 403
        assert client.delete_user(temp_user["id"]).status_code == 403
    finally:
        client.close()


def test_granted_user_sees_the_shared_storage(
    sarca: SarcaClient, storage: str, temp_user: dict[str, str], base_url: str
) -> None:
    assert sarca.grant_access(storage, temp_user["email"], "W").status_code in (200, 201, 204)

    client = SarcaClient(base_url)
    try:
        client.login(temp_user["email"], temp_user["password"])
        assert [s["id"] for s in client.list_storages()] == [storage]
        assert client.upload(storage, "shared.txt", b"from the other user").ok
        client.wait_for_file(storage, "shared.txt")
    finally:
        client.close()

    holders = {u["email"] for u in sarca.get(f"/api/storages/{storage}/access").json()}
    assert temp_user["email"] in holders


def test_deleting_a_user_revokes_access_and_logins(
    sarca: SarcaClient, storage: str, base_url: str
) -> None:
    email = f"doomed-{uuid.uuid4().hex[:6]}@sarca.test"
    password = "doomed-pass-123"
    assert sarca.create_user(email, password).status_code == 201
    user_id = next(u["id"] for u in sarca.list_users() if u["email"] == email)
    assert sarca.grant_access(storage, email, "W").status_code in (200, 201, 204)

    victim = SarcaClient(base_url)
    victim.login(email, password)
    stale_token = victim.access_token

    assert sarca.delete_user(user_id).status_code == 204
    assert all(u["email"] != email for u in sarca.list_users())

    # The account is gone: no new login, and the still-unexpired token is refused.
    assert victim.http.post(
        "/api/auth/login", json={"email": email, "password": password}
    ).status_code in (401, 403, 404)
    assert victim.http.get(
        "/api/storages", headers={"Authorization": f"Bearer {stale_token}"}
    ).status_code in (401, 403)
    victim.close()

    # The shared storage itself survives — it still has its owner.
    assert any(s["id"] == storage for s in sarca.list_storages())
    holders = {u["email"] for u in sarca.get(f"/api/storages/{storage}/access").json()}
    assert email not in holders


def test_deleting_a_user_purges_the_storages_only_they_owned(
    sarca: SarcaClient, base_url: str, mock, server: SarcaServer
) -> None:
    email = f"owner-{uuid.uuid4().hex[:6]}@sarca.test"
    password = "owner-pass-123"
    assert sarca.create_user(email, password).status_code == 201
    user_id = next(u["id"] for u in sarca.list_users() if u["email"] == email)

    owner = SarcaClient(base_url)
    owner.login(email, password)
    own_storage = owner.create_storage(
        chat_ids=[new_chat_id()], bot_token=new_bot_token()
    )["id"]
    assert owner.upload(own_storage, "mine.txt", b"data").ok
    owner.wait_for_file(own_storage, "mine.txt")
    owner.close()

    offset = server.log_offset()
    assert sarca.delete_user(user_id).status_code == 204
    line = server.wait_for_log("orphaned by user", offset=offset, timeout=15)
    assert own_storage in line

    # Storage rows are gone, and its bot went with the user.
    assert sarca.get(f"/api/storages/{own_storage}").status_code in (403, 404)


def test_superuser_cannot_be_deleted(sarca: SarcaClient) -> None:
    superuser = next(u for u in sarca.list_users() if u["is_superuser"])
    r = sarca.delete_user(superuser["id"])
    assert r.status_code == 403, r.text
    assert any(u["is_superuser"] for u in sarca.list_users())


def test_deleting_an_unknown_user_is_404(sarca: SarcaClient) -> None:
    assert sarca.delete_user(str(uuid.uuid4())).status_code == 404


def test_user_endpoints_require_authentication(base_url: str) -> None:
    anon = SarcaClient(base_url)
    try:
        assert anon.get("/api/users").status_code == 401
        assert anon.delete_user(str(uuid.uuid4())).status_code == 401
    finally:
        anon.close()
