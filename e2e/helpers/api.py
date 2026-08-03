"""Thin client for the Sarca HTTP API used by the e2e suite."""

from __future__ import annotations

import hashlib
import json
import time
import uuid
from dataclasses import dataclass
from typing import Any, Iterable

import httpx


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


@dataclass
class UploadResult:
    events: list[dict[str, Any]]
    elapsed: float

    @property
    def phases(self) -> list[str]:
        return [e.get("phase", "") for e in self.events]

    @property
    def ok(self) -> bool:
        return "done" in self.phases and "error" not in self.phases

    @property
    def error(self) -> str | None:
        for event in self.events:
            if event.get("phase") == "error":
                return event.get("message", "unknown error")
        return None


class SarcaClient:
    """Authenticated API wrapper. One instance == one logged-in user."""

    def __init__(self, base_url: str, timeout: float = 120.0, verify: bool = True) -> None:
        self.base_url = base_url.rstrip("/")
        self.http = httpx.Client(base_url=self.base_url, timeout=timeout, verify=verify)
        self.access_token: str | None = None
        self.refresh_token: str | None = None
        self.login_payload: dict[str, Any] = {}

    # ------------------------------------------------------------------- auth
    def login(self, email: str, password: str) -> dict[str, Any]:
        r = self.http.post("/api/auth/login", json={"email": email, "password": password})
        r.raise_for_status()
        data = r.json()
        self.access_token = data["access_token"]
        self.refresh_token = data["refresh_token"]
        self.login_payload = data
        return data

    def refresh(self) -> dict[str, Any]:
        r = self.http.post("/api/auth/refresh", json={"refresh_token": self.refresh_token})
        r.raise_for_status()
        data = r.json()
        self.access_token = data["access_token"]
        return data

    @property
    def headers(self) -> dict[str, str]:
        return {"Authorization": f"Bearer {self.access_token}"} if self.access_token else {}

    def request(self, method: str, url: str, **kwargs: Any) -> httpx.Response:
        headers = {**self.headers, **kwargs.pop("headers", {})}
        return self.http.request(method, url, headers=headers, **kwargs)

    def get(self, url: str, **kwargs: Any) -> httpx.Response:
        return self.request("GET", url, **kwargs)

    def post(self, url: str, **kwargs: Any) -> httpx.Response:
        return self.request("POST", url, **kwargs)

    def put(self, url: str, **kwargs: Any) -> httpx.Response:
        return self.request("PUT", url, **kwargs)

    def delete(self, url: str, **kwargs: Any) -> httpx.Response:
        return self.request("DELETE", url, **kwargs)

    # --------------------------------------------------------------- storages
    def create_storage(
        self,
        name: str | None = None,
        chat_ids: Iterable[int] | None = None,
        bot_token: str | None = None,
    ) -> dict[str, Any]:
        name = name or f"e2e-{uuid.uuid4().hex[:8]}"
        chat_ids = list(chat_ids) if chat_ids is not None else [new_chat_id()]
        r = self.post(
            "/api/storages",
            json={"name": name, "channels": [{"chat_id": cid} for cid in chat_ids]},
        )
        r.raise_for_status()
        storage = r.json()
        if bot_token:
            self.set_bot(storage["id"], bot_token)
        return storage

    def set_bot(self, storage_id: str, token: str) -> dict[str, Any]:
        r = self.put(f"/api/storages/{storage_id}/bot", json={"token": token})
        r.raise_for_status()
        return r.json()

    def storage_detail(self, storage_id: str) -> dict[str, Any]:
        r = self.get(f"/api/storages/{storage_id}")
        r.raise_for_status()
        return r.json()

    def list_storages(self) -> list[dict[str, Any]]:
        r = self.get("/api/storages")
        r.raise_for_status()
        return r.json()["storages"]

    def delete_storage(self, storage_id: str) -> httpx.Response:
        return self.delete(f"/api/storages/{storage_id}")

    # ------------------------------------------------------------------ files
    def upload(
        self,
        storage_id: str,
        filename: str,
        data: bytes,
        path: str = "",
        content_type: str = "application/octet-stream",
        content_hash: str | None = None,
        mtime_ms: int | None = None,
        thumb: bytes | None = None,
        timeout: float = 180.0,
    ) -> UploadResult:
        """Upload one file and drain the NDJSON progress stream.

        `thumb` mimics the web client, which builds the 128px grid tile itself
        and ships it alongside the original so the server never decodes it.
        """
        files = {"file": (filename, data, content_type)}
        if thumb is not None:
            files["thumb"] = ("thumb.jpg", thumb, "image/jpeg")
        form: dict[str, str] = {"path": path, "filename": filename}
        if content_hash:
            form["content_hash"] = content_hash
        if mtime_ms is not None:
            form["mtime"] = str(mtime_ms)

        events: list[dict[str, Any]] = []
        started = time.time()
        with self.http.stream(
            "POST",
            f"/api/storages/{storage_id}/files/upload",
            headers=self.headers,
            data=form,
            files=files,
            timeout=timeout,
        ) as response:
            response.raise_for_status()
            for line in response.iter_lines():
                line = line.strip()
                if line:
                    events.append(json.loads(line))
        return UploadResult(events=events, elapsed=time.time() - started)

    def create_folder(self, storage_id: str, folder_name: str, path: str = "") -> httpx.Response:
        return self.post(
            f"/api/storages/{storage_id}/files/create_folder",
            json={"path": path, "folder_name": folder_name},
        )

    def tree(self, storage_id: str, path: str = "") -> list[dict[str, Any]]:
        r = self.get(f"/api/storages/{storage_id}/files/tree/{path}")
        r.raise_for_status()
        return r.json()

    def download(self, storage_id: str, path: str, **kwargs: Any) -> httpx.Response:
        return self.get(f"/api/storages/{storage_id}/files/download/{path}", **kwargs)

    def download_bytes(self, storage_id: str, path: str, **kwargs: Any) -> bytes:
        r = self.download(storage_id, path, **kwargs)
        r.raise_for_status()
        return r.content

    def preview(self, storage_id: str, path: str, **kwargs: Any) -> httpx.Response:
        return self.get(f"/api/storages/{storage_id}/files/preview/{path}", **kwargs)

    def thumb(self, storage_id: str, path: str, **kwargs: Any) -> httpx.Response:
        return self.get(f"/api/storages/{storage_id}/files/thumb/{path}", **kwargs)

    def info(self, storage_id: str, path: str) -> dict[str, Any]:
        r = self.get(f"/api/storages/{storage_id}/files/info/{path}")
        r.raise_for_status()
        return r.json()

    def delete_file(self, storage_id: str, path: str) -> httpx.Response:
        return self.delete(f"/api/storages/{storage_id}/files/{path}")

    def wait_for_file(self, storage_id: str, path: str, timeout: float = 60.0) -> dict[str, Any]:
        """Poll `info` until the file exists (upload finalization is async)."""
        deadline = time.time() + timeout
        last = None
        while time.time() < deadline:
            r = self.get(f"/api/storages/{storage_id}/files/info/{path}")
            if r.status_code == 200:
                return r.json()
            last = r
            time.sleep(0.2)
        raise AssertionError(
            f"file {path!r} not visible within {timeout}s "
            f"(last status {last.status_code if last else 'n/a'})"
        )

    # ------------------------------------------------------------------ users
    def create_user(self, email: str, password: str) -> httpx.Response:
        return self.post("/api/users", json={"email": email, "password": password})

    def list_users(self) -> list[dict[str, Any]]:
        r = self.get("/api/users")
        r.raise_for_status()
        return r.json()["users"]

    def delete_user(self, user_id: str) -> httpx.Response:
        return self.delete(f"/api/users/{user_id}")

    def grant_access(self, storage_id: str, email: str, access_type: str = "W") -> httpx.Response:
        """`access_type` is one of R / W / A (the UI sends these uppercase)."""
        return self.post(
            f"/api/storages/{storage_id}/access",
            json={"user_email": email, "access_type": access_type},
        )

    # --------------------------------------------------------------- settings
    def get_trash_settings(self) -> dict[str, Any]:
        r = self.get("/api/settings/trash")
        r.raise_for_status()
        return r.json()

    def set_trash_settings(self, retention_days: int) -> httpx.Response:
        return self.put("/api/settings/trash", json={"retention_days": retention_days})

    # ------------------------------------------------------------------- sync
    def snapshot(self, storage_id: str) -> dict[str, Any]:
        r = self.get(f"/api/storages/{storage_id}/sync/snapshot")
        r.raise_for_status()
        return r.json()

    def changelog(self, storage_id: str, cursor: int = 0, limit: int = 500) -> dict[str, Any]:
        r = self.get(
            f"/api/storages/{storage_id}/sync/changelog",
            params={"cursor": cursor, "limit": limit},
        )
        r.raise_for_status()
        return r.json()

    def close(self) -> None:
        self.http.close()


_CHAT_SEQ = [0]


def new_chat_id() -> int:
    """Unique Telegram-channel-shaped chat id (globally unique in the DB)."""
    _CHAT_SEQ[0] += 1
    return -1_000_000_000_000 - (uuid.uuid4().int % 1_000_000_000) - _CHAT_SEQ[0]


def new_bot_token() -> str:
    """Mock bot token; shape matches Telegram's `<id>:<secret>`."""
    return f"{uuid.uuid4().int % 10_000_000_000}:AA{uuid.uuid4().hex}"
