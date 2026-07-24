"""Extended API e2e: copy/move, favorites/recent, shares, auth extras.

Requires a running Sarca with Postgres. Folder ops do not need Telegram workers.
"""

from __future__ import annotations

from datetime import datetime, timedelta, timezone

import httpx
import pytest

from test_api import _create_folder, _tree_names


# ---------------------------------------------------------------------------
# Auth extras
# ---------------------------------------------------------------------------


def test_auth_me_and_providers(client: httpx.Client, auth_headers: dict[str, str]) -> None:
    r = client.get("/api/auth/providers")
    assert r.status_code == 200, r.text
    body = r.json()
    assert "google" in body and "github" in body and "smtp" in body
    assert isinstance(body["google"], bool)
    assert isinstance(body["github"], bool)
    assert isinstance(body["smtp"], bool)

    r = client.get("/api/auth/me", headers=auth_headers)
    assert r.status_code == 200, r.text
    me = r.json()
    assert "email" in me
    assert "email_verified" in me
    assert me["email_verified"] is True  # superuser / existing users backfilled


def test_login_includes_email_verified(tokens: dict[str, str]) -> None:
    assert "email_verified" in tokens
    assert tokens["email_verified"] is True


def test_password_forgot_always_204(client: httpx.Client) -> None:
    r = client.post(
        "/api/auth/password/forgot",
        json={"email": "nobody-does-not-exist@example.com"},
    )
    assert r.status_code == 204, r.text


def test_password_reset_rejects_bad_token(client: httpx.Client) -> None:
    r = client.post(
        "/api/auth/password/reset",
        json={"token": "not-a-real-token", "new_password": "abcdefgh"},
    )
    assert r.status_code in (400, 401, 404), r.text


def test_verify_rejects_bad_token(client: httpx.Client) -> None:
    r = client.post("/api/auth/verify", json={"token": "not-a-real-token"})
    assert r.status_code in (400, 401, 404), r.text


def test_oauth_start_unconfigured_is_error(client: httpx.Client) -> None:
    # Without OAuth client ids, start should fail (not 500 HTML).
    r = client.get("/api/auth/oauth/google/start", follow_redirects=False)
    assert r.status_code in (400, 404, 502, 503), r.text


# ---------------------------------------------------------------------------
# Copy / move folders
# ---------------------------------------------------------------------------


def test_copy_folder_tree(
    client: httpx.Client, auth_headers: dict[str, str], storage_id: str
) -> None:
    _create_folder(client, auth_headers, storage_id, "srcroot")
    _create_folder(client, auth_headers, storage_id, "nested", path="srcroot")
    _create_folder(client, auth_headers, storage_id, "dest")

    r = client.post(
        f"/api/storages/{storage_id}/files/copy",
        headers=auth_headers,
        json={"path": "srcroot/", "destination_folder": "dest"},
    )
    assert r.status_code == 204, r.text

    names = _tree_names(client, auth_headers, storage_id)
    assert "srcroot" in names
    assert "dest" in names

    r = client.get(
        f"/api/storages/{storage_id}/files/tree/dest",
        headers=auth_headers,
    )
    assert r.status_code == 200, r.text
    dest_names = {e["name"] for e in r.json()}
    assert "srcroot" in dest_names

    r = client.get(
        f"/api/storages/{storage_id}/files/tree/dest/srcroot",
        headers=auth_headers,
    )
    assert r.status_code == 200, r.text
    assert any(e["name"] == "nested" for e in r.json())


def test_copy_folder_conflict_rename(
    client: httpx.Client, auth_headers: dict[str, str], storage_id: str
) -> None:
    _create_folder(client, auth_headers, storage_id, "piece")
    _create_folder(client, auth_headers, storage_id, "box")
    # First copy into box
    r = client.post(
        f"/api/storages/{storage_id}/files/copy",
        headers=auth_headers,
        json={"path": "piece/", "destination_folder": "box"},
    )
    assert r.status_code == 204, r.text

    # Second copy conflicts
    r = client.post(
        f"/api/storages/{storage_id}/files/copy",
        headers=auth_headers,
        json={"path": "piece/", "destination_folder": "box"},
    )
    assert r.status_code == 409, r.text

    r = client.post(
        f"/api/storages/{storage_id}/files/copy",
        headers=auth_headers,
        json={
            "path": "piece/",
            "destination_folder": "box",
            "on_conflict": "rename",
        },
    )
    assert r.status_code == 204, r.text

    r = client.get(
        f"/api/storages/{storage_id}/files/tree/box",
        headers=auth_headers,
    )
    names = {e["name"] for e in r.json()}
    assert "piece" in names
    assert "piece (1)" in names


def test_move_folder_conflict_rename(
    client: httpx.Client, auth_headers: dict[str, str], storage_id: str
) -> None:
    _create_folder(client, auth_headers, storage_id, "alpha")
    _create_folder(client, auth_headers, storage_id, "beta")
    _create_folder(client, auth_headers, storage_id, "gamma")

    # Move alpha into beta
    r = client.post(
        f"/api/storages/{storage_id}/files/move",
        headers=auth_headers,
        json={"path": "alpha/", "destination_folder": "beta"},
    )
    assert r.status_code == 204, r.text

    # Recreate alpha at root, move into beta → conflict
    _create_folder(client, auth_headers, storage_id, "alpha")
    r = client.post(
        f"/api/storages/{storage_id}/files/move",
        headers=auth_headers,
        json={"path": "alpha/", "destination_folder": "beta"},
    )
    assert r.status_code == 409, r.text

    r = client.post(
        f"/api/storages/{storage_id}/files/move",
        headers=auth_headers,
        json={
            "path": "alpha/",
            "destination_folder": "beta",
            "on_conflict": "rename",
        },
    )
    assert r.status_code == 204, r.text

    live = _tree_names(client, auth_headers, storage_id)
    assert "alpha" not in live  # moved away from root

    r = client.get(
        f"/api/storages/{storage_id}/files/tree/beta",
        headers=auth_headers,
    )
    names = {e["name"] for e in r.json()}
    assert "alpha" in names
    assert "alpha (1)" in names


# ---------------------------------------------------------------------------
# Favorites / recent (folders rejected; empty lists OK)
# ---------------------------------------------------------------------------


def test_favorites_list_empty_and_reject_folder(
    client: httpx.Client, auth_headers: dict[str, str], storage_id: str
) -> None:
    r = client.get(f"/api/storages/{storage_id}/favorites", headers=auth_headers)
    assert r.status_code == 200, r.text
    assert r.json() == []

    _create_folder(client, auth_headers, storage_id, "nofav")
    r = client.put(
        f"/api/storages/{storage_id}/favorites",
        headers=auth_headers,
        json={"path": "nofav/"},
    )
    assert r.status_code in (400, 422), r.text


def test_recent_list_empty_and_reject_folder(
    client: httpx.Client, auth_headers: dict[str, str], storage_id: str
) -> None:
    r = client.get(f"/api/storages/{storage_id}/recent", headers=auth_headers)
    assert r.status_code == 200, r.text
    assert r.json() == []

    _create_folder(client, auth_headers, storage_id, "norecent")
    r = client.post(
        f"/api/storages/{storage_id}/recent",
        headers=auth_headers,
        json={"path": "norecent/"},
    )
    assert r.status_code in (400, 422), r.text


def test_favorites_missing_file(
    client: httpx.Client, auth_headers: dict[str, str], storage_id: str
) -> None:
    r = client.put(
        f"/api/storages/{storage_id}/favorites",
        headers=auth_headers,
        json={"path": "missing.pdf"},
    )
    assert r.status_code in (400, 404), r.text


# ---------------------------------------------------------------------------
# Public share links (folders)
# ---------------------------------------------------------------------------


def test_share_folder_create_list_public_tree_revoke(
    client: httpx.Client, auth_headers: dict[str, str], storage_id: str
) -> None:
    _create_folder(client, auth_headers, storage_id, "shared")
    _create_folder(client, auth_headers, storage_id, "child", path="shared")

    r = client.post(
        f"/api/storages/{storage_id}/shares",
        headers=auth_headers,
        json={"path": "shared/"},
    )
    assert r.status_code == 201, r.text
    link = r.json()
    assert link["token"]
    assert link["path"] in ("shared/", "shared")
    assert link["has_password"] is False
    assert link["url_path"].startswith("/s/")
    token = link["token"]
    share_id = link["id"]

    r = client.get(f"/api/storages/{storage_id}/shares", headers=auth_headers)
    assert r.status_code == 200, r.text
    assert any(s["id"] == share_id for s in r.json())

    # Public metadata (no auth)
    r = client.get(f"/api/public/shares/{token}")
    assert r.status_code == 200, r.text
    meta = r.json()
    assert meta["is_file"] is False
    assert meta["has_password"] is False
    assert meta["name"] == "shared"

    r = client.get(f"/api/public/shares/{token}/tree")
    assert r.status_code == 200, r.text
    children = {e["name"] for e in r.json()}
    assert "child" in children

    # Path traversal rejected
    r = client.get(
        f"/api/public/shares/{token}/tree",
        params={"path": "../"},
    )
    assert r.status_code in (400, 404), r.text

    r = client.delete(
        f"/api/storages/{storage_id}/shares/{share_id}",
        headers=auth_headers,
    )
    assert r.status_code == 204, r.text

    r = client.get(f"/api/public/shares/{token}")
    assert r.status_code == 404, r.text


def test_share_folder_with_password(
    client: httpx.Client, auth_headers: dict[str, str], storage_id: str
) -> None:
    _create_folder(client, auth_headers, storage_id, "vault")

    r = client.post(
        f"/api/storages/{storage_id}/shares",
        headers=auth_headers,
        json={"path": "vault/", "password": "s3cret-pass"},
    )
    assert r.status_code == 201, r.text
    token = r.json()["token"]
    assert r.json()["has_password"] is True

    r = client.get(f"/api/public/shares/{token}")
    assert r.status_code == 401, r.text
    assert r.json().get("need_password") is True

    # Wrong password
    r = client.post(
        f"/api/public/shares/{token}/unlock",
        json={"password": "wrong"},
    )
    assert r.status_code == 401, r.text

    # Correct password → unlock cookie
    r = client.post(
        f"/api/public/shares/{token}/unlock",
        json={"password": "s3cret-pass"},
    )
    assert r.status_code == 204, r.text
    assert "set-cookie" in {k.lower() for k in r.headers.keys()}

    # Reuse client cookies for gated access
    r = client.get(f"/api/public/shares/{token}")
    assert r.status_code == 200, r.text
    assert r.json()["name"] == "vault"


def test_share_past_expiry_unavailable(
    client: httpx.Client, auth_headers: dict[str, str], storage_id: str
) -> None:
    _create_folder(client, auth_headers, storage_id, "oldshare")
    past = (datetime.now(timezone.utc) - timedelta(hours=1)).isoformat()

    r = client.post(
        f"/api/storages/{storage_id}/shares",
        headers=auth_headers,
        json={"path": "oldshare/", "expires_at": past},
    )
    # Either rejected at create or created-but-unavailable
    if r.status_code == 201:
        token = r.json()["token"]
        r2 = client.get(f"/api/public/shares/{token}")
        assert r2.status_code == 404, r2.text
    else:
        assert r.status_code == 400, r.text


def test_share_unknown_token_404(client: httpx.Client) -> None:
    r = client.get("/api/public/shares/definitely-not-a-token")
    assert r.status_code == 404, r.text


def test_share_file_download_and_inline_no_trailing_slash(
    client: httpx.Client, auth_headers: dict[str, str]
) -> None:
    """Single-file shares must hit /download and /inline without a trailing slash."""
    import uuid

    workers = client.get("/api/storage_workers", headers=auth_headers)
    if workers.status_code != 200 or not workers.json():
        pytest.skip("needs a storage worker to upload a shareable file")
    storage_id = workers.json()[0]["storage_id"]

    name = f"share-file-{uuid.uuid4().hex[:8]}.txt"
    content = b"hello public share\n"
    r = client.post(
        f"/api/storages/{storage_id}/files/upload",
        headers=auth_headers,
        files={"file": (name, content, "text/plain")},
        data={"path": ""},
        timeout=120.0,
    )
    assert r.status_code == 201, r.text
    if b'"phase":"error"' in r.content or b'"phase": "error"' in r.content:
        pytest.skip(f"upload failed: {r.text[:300]}")
    assert b'"phase":"done"' in r.content or b'"phase": "done"' in r.content, r.text

    r = client.post(
        f"/api/storages/{storage_id}/shares",
        headers=auth_headers,
        json={"path": name},
    )
    assert r.status_code == 201, r.text
    token = r.json()["token"]

    r = client.get(f"/api/public/shares/{token}")
    assert r.status_code == 200, r.text
    assert r.json()["is_file"] is True
    assert r.json()["name"] == name

    # Trailing slash must 404 (UI used to generate this and stuck on Loading)
    r = client.get(f"/api/public/shares/{token}/download/")
    assert r.status_code == 404, r.text

    r = client.get(f"/api/public/shares/{token}/download")
    assert r.status_code == 200, r.text
    assert r.content == content

    r = client.get(f"/api/public/shares/{token}/inline")
    assert r.status_code == 200, r.text
    assert r.content == content


# ---------------------------------------------------------------------------
# Misc regressions
# ---------------------------------------------------------------------------


def test_unauthenticated_storages_rejected(client: httpx.Client) -> None:
    r = client.get("/api/storages")
    assert r.status_code in (401, 403), r.text


def test_create_folder_and_search(
    client: httpx.Client, auth_headers: dict[str, str], storage_id: str
) -> None:
    _create_folder(client, auth_headers, storage_id, "findme")
    r = client.get(
        f"/api/storages/{storage_id}/files/search/",
        headers=auth_headers,
        params={"search_path": "findme"},
    )
    assert r.status_code == 200, r.text
    assert any("findme" in h["path"] for h in r.json())
