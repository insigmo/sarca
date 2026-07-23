"""UI and startup end-to-end checks."""

from __future__ import annotations

import os
import re

import httpx


def test_root_serves_spa_html(client: httpx.Client) -> None:
    r = client.get("/")
    assert r.status_code == 200, r.text
    ctype = r.headers.get("content-type", "")
    assert "text/html" in ctype, ctype
    body = r.text.lower()
    assert "<html" in body or "<!doctype html" in body
    # Vite-built SPA references hashed assets
    assert "/assets/" in r.text or "script" in body


def test_spa_fallback_for_client_route(client: httpx.Client) -> None:
    r = client.get("/storages")
    assert r.status_code == 200, r.text
    assert "text/html" in r.headers.get("content-type", "")


def test_assets_js_or_css_available(client: httpx.Client) -> None:
    home = client.get("/")
    assert home.status_code == 200
    # pick first /assets/... reference from index
    matches = re.findall(r'(/assets/[^"\']+)', home.text)
    assert matches, "expected hashed /assets/… references in index.html"
    asset = matches[0]
    r = client.get(asset)
    assert r.status_code == 200, f"{asset} → {r.status_code}"
    assert len(r.content) > 0


def test_api_unknown_route_is_not_html_spa(client: httpx.Client) -> None:
    # /api/* must not fall through to the SPA index as a soft 200 HTML page.
    r = client.get("/api/this-route-does-not-exist")
    assert r.status_code == 404, r.text
    assert "text/html" not in r.headers.get("content-type", "")


def test_startup_banner_mentions_port(server_log_path: str | None) -> None:
    if not server_log_path or not os.path.isfile(server_log_path):
        # Local runs without CI log file still pass other tests.
        return
    log = open(server_log_path, encoding="utf-8", errors="replace").read()
    assert "Sarca is running" in log, log[-2000:]
    port = os.environ.get("PORT", "8000")
    assert port in log or f":{port}" in log, log[-2000:]
    assert "database ok" in log or "listening on" in log, log[-2000:]
