"""E2E fixtures: live Sarca API + Postgres (no real Telegram needed for FS/auth tests)."""

from __future__ import annotations

import os
import time
import uuid

import httpx
import pytest

BASE_URL = os.environ.get("SARCA_BASE_URL", "http://127.0.0.1:8000").rstrip("/")
SUPERUSER_EMAIL = os.environ.get("SUPERUSER_EMAIL", "e2e@sarca.test")
SUPERUSER_PASS = os.environ.get("SUPERUSER_PASS", "e2e-password")
SERVER_LOG = os.environ.get("SARCA_SERVER_LOG")


@pytest.fixture(scope="session")
def server_log_path() -> str | None:
    return SERVER_LOG


@pytest.fixture(scope="session")
def base_url() -> str:
    return BASE_URL


@pytest.fixture(scope="session")
def wait_for_api(base_url: str) -> None:
    deadline = time.time() + 90
    last_err: Exception | None = None
    while time.time() < deadline:
        try:
            # login endpoint should respond even with bad credentials
            r = httpx.post(
                f"{base_url}/api/auth/login",
                json={"email": "probe@example.com", "password": "x"},
                timeout=2.0,
            )
            if r.status_code in (200, 401, 403, 422):
                return
        except Exception as e:  # noqa: BLE001
            last_err = e
        time.sleep(1)
    raise RuntimeError(f"API not ready at {base_url}: {last_err}")


@pytest.fixture(scope="session")
def client(base_url: str, wait_for_api: None) -> httpx.Client:
    with httpx.Client(base_url=base_url, timeout=30.0) as c:
        yield c


@pytest.fixture(scope="session")
def tokens(client: httpx.Client) -> dict[str, str]:
    r = client.post(
        "/api/auth/login",
        json={"email": SUPERUSER_EMAIL, "password": SUPERUSER_PASS},
    )
    assert r.status_code == 200, r.text
    data = r.json()
    assert "access_token" in data
    assert "refresh_token" in data
    return data


@pytest.fixture(scope="session")
def auth_headers(tokens: dict[str, str]) -> dict[str, str]:
    return {"Authorization": f"Bearer {tokens['access_token']}"}


@pytest.fixture
def storage_id(client: httpx.Client, auth_headers: dict[str, str]) -> str:
    name = f"e2e-{uuid.uuid4().hex[:8]}"
    # Unique negative chat id in Telegram channel range
    chat_id = -1000000000000 - (uuid.uuid4().int % 1_000_000_000)
    r = client.post(
        "/api/storages",
        headers=auth_headers,
        json={"name": name, "chat_id": chat_id},
    )
    assert r.status_code in (200, 201), r.text
    body = r.json()
    sid = body.get("id") or body.get("storage", {}).get("id")
    assert sid, body
    yield sid
    client.delete(f"/api/storages/{sid}", headers=auth_headers)
