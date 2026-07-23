"""API end-to-end tests covering auth, FS ops, workers, search, rename/move."""

from __future__ import annotations

import io
import uuid

import httpx
import pytest


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

    tree = client.get(f"/api/storages/{storage_id}/files/tree/", headers=auth_headers)
    assert tree.status_code == 200
    assert all(e["name"] != "hello.txt" for e in tree.json())

    storages = client.get("/api/storages", headers=auth_headers).json()["storages"]
    mine = next(s for s in storages if s["id"] == storage_id)
    # unfinished uploads must not inflate size (#61/#46)
    assert mine["size"] == 0
