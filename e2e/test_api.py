"""API end-to-end tests covering auth, FS ops, workers, search, rename/move, trash."""

from __future__ import annotations

import io
import uuid

import httpx
import pytest


def _tree_names(client: httpx.Client, auth_headers: dict[str, str], storage_id: str) -> set[str]:
    r = client.get(f"/api/storages/{storage_id}/files/tree/", headers=auth_headers)
    assert r.status_code == 200, r.text
    return {e["name"] for e in r.json()}


def _trash_names(
    client: httpx.Client,
    auth_headers: dict[str, str],
    storage_id: str,
    path: str = "",
) -> set[str]:
    params = {"path": path} if path else None
    r = client.get(
        f"/api/storages/{storage_id}/trash",
        headers=auth_headers,
        params=params,
    )
    assert r.status_code == 200, r.text
    return {e["name"] for e in r.json()}


def _create_folder(
    client: httpx.Client,
    auth_headers: dict[str, str],
    storage_id: str,
    folder_name: str,
    path: str = "",
) -> None:
    r = client.post(
        f"/api/storages/{storage_id}/files/create_folder",
        headers=auth_headers,
        json={"path": path, "folder_name": folder_name},
    )
    assert r.status_code in (200, 201), r.text


def test_login_returns_access_and_refresh(tokens: dict[str, str]) -> None:
    assert tokens["access_token"]
    assert tokens["refresh_token"]
    assert tokens["access_token"] != tokens["refresh_token"]


def test_refresh_token_roundtrip(client: httpx.Client, tokens: dict[str, str]) -> None:
    r = client.post(
        "/api/auth/refresh",
        json={"refresh_token": tokens["refresh_token"]},
    )
    assert r.status_code == 200, r.text
    data = r.json()
    assert data["access_token"]
    assert data["refresh_token"]


def test_refresh_rejects_access_token(client: httpx.Client, tokens: dict[str, str]) -> None:
    r = client.post(
        "/api/auth/refresh",
        json={"refresh_token": tokens["access_token"]},
    )
    assert r.status_code in (401, 403), r.text


def test_list_storages(client: httpx.Client, auth_headers: dict[str, str], storage_id: str) -> None:
    r = client.get("/api/storages", headers=auth_headers)
    assert r.status_code == 200, r.text
    storages = r.json().get("storages", r.json())
    assert isinstance(storages, list)
    assert any(s["id"] == storage_id for s in storages)
    # empty storage must still be listed (regression for #48)
    mine = next(s for s in storages if s["id"] == storage_id)
    assert mine["files_amount"] == 0
    assert mine["size"] == 0


def test_create_and_delete_storage_worker(
    client: httpx.Client, auth_headers: dict[str, str], storage_id: str
) -> None:
    name = f"worker-{uuid.uuid4().hex[:8]}"
    token = f"bot{uuid.uuid4().hex}"
    r = client.post(
        "/api/storage_workers",
        headers=auth_headers,
        json={"name": name, "token": token, "storage_id": storage_id},
    )
    assert r.status_code in (200, 201), r.text
    wid = r.json()["id"]

    r = client.get("/api/storage_workers", headers=auth_headers)
    assert r.status_code == 200
    assert any(w["id"] == wid for w in r.json())

    r = client.delete(f"/api/storage_workers/{wid}", headers=auth_headers)
    assert r.status_code in (200, 204), r.text

    r = client.get("/api/storage_workers", headers=auth_headers)
    assert all(w["id"] != wid for w in r.json())


def test_worker_requires_storage_id(
    client: httpx.Client, auth_headers: dict[str, str]
) -> None:
    r = client.post(
        "/api/storage_workers",
        headers=auth_headers,
        json={"name": f"orphan-{uuid.uuid4().hex[:8]}", "token": f"tok{uuid.uuid4().hex}"},
    )
    assert r.status_code == 400, r.text


def test_storage_detail_includes_bot(
    client: httpx.Client, auth_headers: dict[str, str], storage_id: str
) -> None:
    name = f"bot-detail-{uuid.uuid4().hex[:8]}"
    token = f"bot{uuid.uuid4().hex}token"
    r = client.post(
        "/api/storage_workers",
        headers=auth_headers,
        json={"name": name, "token": token, "storage_id": storage_id},
    )
    assert r.status_code in (200, 201), r.text
    wid = r.json()["id"]

    try:
        r = client.get(f"/api/storages/{storage_id}", headers=auth_headers)
        assert r.status_code == 200, r.text
        body = r.json()
        assert "bot" in body
        assert body["bot"] is not None
        assert body["bot"]["id"] == wid
        assert body["bot"]["name"] == name
        assert "token_masked" in body["bot"]
        assert token not in body["bot"]["token_masked"]
        assert "channels" in body
    finally:
        client.delete(f"/api/storage_workers/{wid}", headers=auth_headers)


def test_refresh_channels_without_bot_conflicts(
    client: httpx.Client, auth_headers: dict[str, str]
) -> None:
    # Fresh storage with no worker
    r = client.post(
        "/api/storages",
        headers=auth_headers,
        json={
            "name": f"nobot-{uuid.uuid4().hex[:8]}",
            "channels": [{"chat_id": -100_000_000_000 - int(uuid.uuid4().int % 10**9)}],
        },
    )
    assert r.status_code in (200, 201), r.text
    sid = r.json()["id"]
    try:
        r = client.post(f"/api/storages/{sid}/channels/refresh", headers=auth_headers)
        assert r.status_code == 409, r.text
    finally:
        client.delete(f"/api/storages/{sid}", headers=auth_headers)


def test_folder_create_list_search_rename_move_delete(
    client: httpx.Client, auth_headers: dict[str, str], storage_id: str
) -> None:
    # create folder at root
    r = client.post(
        f"/api/storages/{storage_id}/files/create_folder",
        headers=auth_headers,
        json={"path": "", "folder_name": "docs"},
    )
    assert r.status_code in (200, 201), r.text

    r = client.get(f"/api/storages/{storage_id}/files/tree/", headers=auth_headers)
    assert r.status_code == 200, r.text
    layer = r.json()
    names = {e["name"] for e in layer}
    assert "docs" in names
    docs = next(e for e in layer if e["name"] == "docs")
    assert docs["is_file"] is False

    # nested folder
    r = client.post(
        f"/api/storages/{storage_id}/files/create_folder",
        headers=auth_headers,
        json={"path": "docs", "folder_name": "notes"},
    )
    assert r.status_code in (200, 201), r.text

    # search
    r = client.get(
        f"/api/storages/{storage_id}/files/search/",
        headers=auth_headers,
        params={"search_path": "notes"},
    )
    assert r.status_code == 200, r.text
    hits = r.json()
    assert any("notes" in h["path"] for h in hits)
    note_hit = next(h for h in hits if "notes" in h["path"])
    assert note_hit["is_file"] is False  # folders end with /

    # rename folder docs -> documents
    r = client.post(
        f"/api/storages/{storage_id}/files/rename",
        headers=auth_headers,
        json={"path": "docs/", "new_name": "documents"},
    )
    assert r.status_code in (200, 204), r.text

    r = client.get(f"/api/storages/{storage_id}/files/tree/", headers=auth_headers)
    names = {e["name"] for e in r.json()}
    assert "documents" in names
    assert "docs" not in names

    # move notes under root: documents/notes/ -> notes/
    r = client.post(
        f"/api/storages/{storage_id}/files/move",
        headers=auth_headers,
        json={"path": "documents/notes/", "destination_folder": ""},
    )
    assert r.status_code in (200, 204), r.text

    r = client.get(f"/api/storages/{storage_id}/files/tree/", headers=auth_headers)
    names = {e["name"] for e in r.json()}
    assert "notes" in names

    # delete folder without trailing slash (UI regression #61)
    r = client.delete(
        f"/api/storages/{storage_id}/files/notes",
        headers=auth_headers,
    )
    assert r.status_code in (200, 204), r.text

    r = client.get(f"/api/storages/{storage_id}/files/tree/", headers=auth_headers)
    names = {e["name"] for e in r.json()}
    assert "notes" not in names

    # delete documents with trailing slash
    r = client.delete(
        f"/api/storages/{storage_id}/files/documents/",
        headers=auth_headers,
    )
    assert r.status_code in (200, 204), r.text

    r = client.get(f"/api/storages/{storage_id}/files/tree/", headers=auth_headers)
    names = {e["name"] for e in r.json()}
    assert "documents" not in names


def test_storage_still_listed_after_folder_ops(
    client: httpx.Client, auth_headers: dict[str, str], storage_id: str
) -> None:
    client.post(
        f"/api/storages/{storage_id}/files/create_folder",
        headers=auth_headers,
        json={"path": "", "folder_name": "tmp"},
    )
    client.delete(f"/api/storages/{storage_id}/files/tmp/", headers=auth_headers)

    r = client.get("/api/storages", headers=auth_headers)
    storages = r.json().get("storages", r.json())
    assert any(s["id"] == storage_id for s in storages)


def test_upload_without_worker_fails_cleanly(
    client: httpx.Client, auth_headers: dict[str, str], storage_id: str
) -> None:
    # No storage worker registered → upload must fail without leaving visible file
    files = {"file": ("hello.txt", io.BytesIO(b"hello e2e"), "text/plain")}
    data = {"path": ""}
    r = client.post(
        f"/api/storages/{storage_id}/files/upload",
        headers=auth_headers,
        files=files,
        data=data,
    )
    assert r.status_code >= 400, r.text
    # Must not look like a folder/directory permission error.
    assert "upload directory" not in r.text.lower()

    tree = client.get(f"/api/storages/{storage_id}/files/tree/", headers=auth_headers)
    assert tree.status_code == 200
    assert all(e["name"] != "hello.txt" for e in tree.json())

    storages = client.get("/api/storages", headers=auth_headers).json()["storages"]
    mine = next(s for s in storages if s["id"] == storage_id)
    # unfinished uploads must not inflate size (#61/#46)
    assert mine["size"] == 0


def test_upload_parent_trailing_slash_without_worker(
    client: httpx.Client, auth_headers: dict[str, str], storage_id: str
) -> None:
    """Trailing slash on parent path must not be treated as uploading a folder."""
    files = {"file": ("pic.png", io.BytesIO(b"\x89PNG\r\n"), "image/png")}
    data = {"path": "album/"}
    r = client.post(
        f"/api/storages/{storage_id}/files/upload",
        headers=auth_headers,
        files=files,
        data=data,
    )
    assert r.status_code >= 400, r.text
    tree = client.get(f"/api/storages/{storage_id}/files/tree/", headers=auth_headers)
    assert tree.status_code == 200
    assert all(e["name"] not in ("pic.png", "album") for e in tree.json())


def test_trash_soft_delete_list_and_restore(
    client: httpx.Client, auth_headers: dict[str, str], storage_id: str
) -> None:
    _create_folder(client, auth_headers, storage_id, "keep")
    _create_folder(client, auth_headers, storage_id, "gone")
    _create_folder(client, auth_headers, storage_id, "nested", path="gone")

    r = client.delete(f"/api/storages/{storage_id}/files/gone/", headers=auth_headers)
    assert r.status_code in (200, 204), r.text

    live = _tree_names(client, auth_headers, storage_id)
    assert "keep" in live
    assert "gone" not in live

    trash = _trash_names(client, auth_headers, storage_id)
    assert "gone" in trash
    assert "keep" not in trash

    # Folder container is browsable in trash
    trash_inner = _trash_names(client, auth_headers, storage_id, path="gone")
    assert "nested" in trash_inner

    r = client.post(
        f"/api/storages/{storage_id}/trash/restore",
        headers=auth_headers,
        json={"path": "gone/"},
    )
    assert r.status_code == 204, r.text

    live = _tree_names(client, auth_headers, storage_id)
    assert "gone" in live
    assert "keep" in live
    assert "gone" not in _trash_names(client, auth_headers, storage_id)

    nested = client.get(
        f"/api/storages/{storage_id}/files/tree/gone",
        headers=auth_headers,
    )
    assert nested.status_code == 200, nested.text
    assert any(e["name"] == "nested" for e in nested.json())


def test_trash_restore_conflict_replace(
    client: httpx.Client, auth_headers: dict[str, str], storage_id: str
) -> None:
    _create_folder(client, auth_headers, storage_id, "alpha")
    r = client.delete(f"/api/storages/{storage_id}/files/alpha/", headers=auth_headers)
    assert r.status_code in (200, 204), r.text

    _create_folder(client, auth_headers, storage_id, "alpha")  # live collision

    r = client.post(
        f"/api/storages/{storage_id}/trash/restore",
        headers=auth_headers,
        json={"path": "alpha/"},
    )
    assert r.status_code == 409, r.text
    assert "already exists" in r.text.lower()

    r = client.post(
        f"/api/storages/{storage_id}/trash/restore",
        headers=auth_headers,
        json={"path": "alpha/", "on_conflict": "replace"},
    )
    assert r.status_code == 204, r.text

    live = _tree_names(client, auth_headers, storage_id)
    assert "alpha" in live
    assert "alpha" not in _trash_names(client, auth_headers, storage_id)


def test_trash_restore_conflict_rename(
    client: httpx.Client, auth_headers: dict[str, str], storage_id: str
) -> None:
    """Folder restore with on_conflict=rename must keep trailing-slash semantics."""
    _create_folder(client, auth_headers, storage_id, "beta")
    r = client.delete(f"/api/storages/{storage_id}/files/beta/", headers=auth_headers)
    assert r.status_code in (200, 204), r.text

    _create_folder(client, auth_headers, storage_id, "beta")  # live collision

    r = client.post(
        f"/api/storages/{storage_id}/trash/restore",
        headers=auth_headers,
        json={"path": "beta/", "on_conflict": "rename"},
    )
    assert r.status_code == 204, r.text

    live = _tree_names(client, auth_headers, storage_id)
    assert "beta" in live
    assert "beta (1)" in live
    assert "beta" not in _trash_names(client, auth_headers, storage_id)


def test_trash_delete_forever(
    client: httpx.Client, auth_headers: dict[str, str], storage_id: str
) -> None:
    _create_folder(client, auth_headers, storage_id, "doomed")
    r = client.delete(f"/api/storages/{storage_id}/files/doomed/", headers=auth_headers)
    assert r.status_code in (200, 204), r.text
    assert "doomed" in _trash_names(client, auth_headers, storage_id)

    r = client.delete(
        f"/api/storages/{storage_id}/trash/doomed/",
        headers=auth_headers,
    )
    assert r.status_code == 204, r.text

    assert "doomed" not in _trash_names(client, auth_headers, storage_id)
    assert "doomed" not in _tree_names(client, auth_headers, storage_id)


def test_trash_empty(
    client: httpx.Client, auth_headers: dict[str, str], storage_id: str
) -> None:
    _create_folder(client, auth_headers, storage_id, "one")
    _create_folder(client, auth_headers, storage_id, "two")
    client.delete(f"/api/storages/{storage_id}/files/one/", headers=auth_headers)
    client.delete(f"/api/storages/{storage_id}/files/two/", headers=auth_headers)

    trash = _trash_names(client, auth_headers, storage_id)
    assert "one" in trash and "two" in trash

    r = client.delete(f"/api/storages/{storage_id}/trash", headers=auth_headers)
    assert r.status_code == 204, r.text

    assert _trash_names(client, auth_headers, storage_id) == set()
    live = _tree_names(client, auth_headers, storage_id)
    assert "one" not in live and "two" not in live


def test_trash_retention_settings(
    client: httpx.Client, auth_headers: dict[str, str]
) -> None:
    r = client.get("/api/settings/trash", headers=auth_headers)
    assert r.status_code == 200, r.text
    original = r.json()["retention_days"]
    assert isinstance(original, int)
    assert 1 <= original <= 30

    new_days = 7 if original != 7 else 14
    r = client.put(
        "/api/settings/trash",
        headers=auth_headers,
        json={"retention_days": new_days},
    )
    assert r.status_code == 200, r.text
    assert r.json()["retention_days"] == new_days

    r = client.get("/api/settings/trash", headers=auth_headers)
    assert r.status_code == 200, r.text
    assert r.json()["retention_days"] == new_days

    # restore prior value so local/dev instances stay unchanged
    r = client.put(
        "/api/settings/trash",
        headers=auth_headers,
        json={"retention_days": original},
    )
    assert r.status_code == 200, r.text
    assert r.json()["retention_days"] == original

    r = client.put(
        "/api/settings/trash",
        headers=auth_headers,
        json={"retention_days": 0},
    )
    assert r.status_code == 400, r.text
