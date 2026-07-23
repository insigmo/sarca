"""Live Telegram upload smoke (opt-in).

Skipped in CI / when no storage workers are configured.
Run manually: `task smoke` or `pytest -m smoke e2e/test_upload_smoke.py`
"""

from __future__ import annotations

import io
import os
import struct
import time
import zlib
from pathlib import Path

import httpx
import pytest

pytestmark = pytest.mark.smoke

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = Path(__file__).resolve().parent / "fixtures"


def _load_conf_value(key: str) -> str | None:
    for conf in (
        ROOT / "sarca.conf",
        Path.home() / ".local/share/sarca/sarca.conf",
    ):
        if not conf.is_file():
            continue
        for line in conf.read_text(encoding="utf-8").splitlines():
            line = line.strip()
            if not line or line.startswith("#") or "=" not in line:
                continue
            k, v = line.split("=", 1)
            if k.strip() == key:
                return v.strip().strip('"').strip("'")
    return None


BASE_URL = os.environ.get("SARCA_BASE_URL", "http://127.0.0.1:8001").rstrip("/")
# Prefer sarca.conf over ambient env (agent shells often export e2e@… credentials).
EMAIL = _load_conf_value("SUPERUSER_EMAIL") or os.environ.get("SUPERUSER_EMAIL")
PASSWORD = _load_conf_value("SUPERUSER_PASS") or os.environ.get("SUPERUSER_PASS")


def _minimal_png(width: int = 8, height: int = 8) -> bytes:
    def chunk(tag: bytes, data: bytes) -> bytes:
        return (
            struct.pack(">I", len(data))
            + tag
            + data
            + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)
        )

    raw = b"".join(b"\x00" + (b"\x00\xff\x00" * width) for _ in range(height))
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(raw, 9))
        + chunk(b"IEND", b"")
    )


def _minimal_mp4() -> bytes:
    fixture = FIXTURES / "smoke.mp4"
    if fixture.is_file() and fixture.stat().st_size > 32:
        return fixture.read_bytes()
    ftyp = b"isom" + struct.pack(">I", 0) + b"isomiso2mp41"
    ftyp_box = struct.pack(">I", 8 + len(ftyp)) + b"ftyp" + ftyp
    mdat_payload = b"\x00" * 64
    mdat_box = struct.pack(">I", 8 + len(mdat_payload)) + b"mdat" + mdat_payload
    return ftyp_box + mdat_box


@pytest.fixture(scope="module")
def client() -> httpx.Client:
    if not EMAIL or not PASSWORD:
        pytest.skip("SUPERUSER_EMAIL/PASS missing (env or sarca.conf)")
    deadline = time.time() + 60
    last = None
    while time.time() < deadline:
        try:
            r = httpx.post(
                f"{BASE_URL}/api/auth/login",
                json={"email": "probe@example.com", "password": "x"},
                timeout=2.0,
            )
            if r.status_code in (200, 401, 403, 422):
                break
        except Exception as e:  # noqa: BLE001
            last = e
        time.sleep(1)
    else:
        pytest.skip(f"API not ready at {BASE_URL}: {last}")

    with httpx.Client(base_url=BASE_URL, timeout=120.0) as c:
        yield c


@pytest.fixture(scope="module")
def auth_headers(client: httpx.Client) -> dict[str, str]:
    r = client.post("/api/auth/login", json={"email": EMAIL, "password": PASSWORD})
    if r.status_code != 200:
        pytest.skip(f"login failed: {r.status_code} {r.text}")
    return {"Authorization": f"Bearer {r.json()['access_token']}"}


@pytest.fixture(scope="module")
def storage_id(client: httpx.Client, auth_headers: dict[str, str]) -> str:
    workers = client.get("/api/storage_workers", headers=auth_headers)
    if workers.status_code != 200 or not workers.json():
        pytest.skip("no storage workers — attach a Telegram bot before upload smoke")
    sid = workers.json()[0]["storage_id"]
    return sid


def _upload(
    client: httpx.Client,
    headers: dict[str, str],
    storage_id: str,
    filename: str,
    content: bytes,
    content_type: str,
    parent: str = "",
) -> None:
    files = {"file": (filename, io.BytesIO(content), content_type)}
    data = {"path": parent}
    r = client.post(
        f"/api/storages/{storage_id}/files/upload",
        headers=headers,
        files=files,
        data=data,
    )
    assert r.status_code == 201, f"upload {filename} failed: {r.status_code} {r.text}"


def test_upload_image_and_video_smoke(
    client: httpx.Client, auth_headers: dict[str, str], storage_id: str
) -> None:
    png = (
        (FIXTURES / "smoke.png").read_bytes()
        if (FIXTURES / "smoke.png").is_file()
        else _minimal_png()
    )
    mp4 = (
        (FIXTURES / "smoke.mp4").read_bytes()
        if (FIXTURES / "smoke.mp4").is_file()
        else _minimal_mp4()
    )
    stamp = str(int(time.time()))
    img_name = f"smoke-{stamp}.png"
    vid_name = f"smoke-{stamp}.mp4"

    _upload(client, auth_headers, storage_id, img_name, png, "image/png")
    _upload(client, auth_headers, storage_id, vid_name, mp4, "video/mp4")

    nested = f"nested-{stamp}.png"
    _upload(
        client,
        auth_headers,
        storage_id,
        nested,
        png,
        "image/png",
        parent="smoke-dir/",
    )

    r = client.get(f"/api/storages/{storage_id}/files/tree/", headers=auth_headers)
    assert r.status_code == 200, r.text
    names = {e["name"]: e["is_file"] for e in r.json()}
    assert names.get(img_name) is True, names
    assert names.get(vid_name) is True, names
    assert names.get("smoke-dir") is False, names

    r = client.get(
        f"/api/storages/{storage_id}/files/tree/smoke-dir", headers=auth_headers
    )
    assert r.status_code == 200, r.text
    nested_names = {e["name"]: e["is_file"] for e in r.json()}
    assert nested_names.get(nested) is True, nested_names
