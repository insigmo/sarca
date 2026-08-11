"""Scenario 4 — is the API really reachable over HTTP/3?

A dedicated server instance (self-signed, ACME disabled) so the suite can talk to
it over QUIC with aioquic, next to the instance the other tests use.
"""

from __future__ import annotations

import json
import uuid

import httpx
import pytest

from helpers.h3 import h3_request
from helpers.server import SarcaServer

pytestmark = pytest.mark.slow


@pytest.fixture(scope="module")
def tls_server(e2e_tmp, telegram) -> SarcaServer:
    server = SarcaServer(
        root=e2e_tmp / f"tls-{uuid.uuid4().hex[:6]}",
        telegram_base_url=telegram.base_url,
        env_extra={"SARCA_TELEGRAM_PACING_MS": "20"},
    )
    server.start()
    yield server
    server.stop()


@pytest.fixture(scope="module")
def tls_tokens(tls_server: SarcaServer) -> dict[str, str]:
    r = httpx.post(
        f"{tls_server.https_base_url}/api/auth/login",
        json={"email": tls_server.email, "password": tls_server.password},
        verify=False,
        timeout=30.0,
    )
    r.raise_for_status()
    return r.json()


def test_tls_mode_serves_https_over_tcp(tls_server: SarcaServer) -> None:
    r = httpx.post(
        f"{tls_server.https_base_url}/api/auth/login",
        json={"email": tls_server.email, "password": tls_server.password},
        verify=False,
        timeout=10.0,
    )
    assert r.status_code == 200, r.text
    assert "access_token" in r.json()


def test_responses_advertise_http3_via_alt_svc(tls_server: SarcaServer) -> None:
    r = httpx.get(f"{tls_server.https_base_url}/", verify=False, timeout=10.0)
    alt_svc = r.headers.get("alt-svc")
    assert alt_svc, "Alt-Svc missing: browsers would never upgrade to HTTP/3"
    assert alt_svc == f'h3=":{tls_server.https_port}"; ma=86400', alt_svc


def test_api_answers_over_http3(tls_server: SarcaServer) -> None:
    """An unauthenticated API call must round trip over QUIC, ALPN "h3"."""
    response = h3_request("127.0.0.1", tls_server.https_port, "/api/storages")
    assert response.alpn == "h3", f"negotiated ALPN was {response.alpn!r}"
    assert response.status == 401, response.body


def test_api_login_works_over_http3(tls_server: SarcaServer) -> None:
    payload = json.dumps(
        {"email": tls_server.email, "password": tls_server.password}
    ).encode()
    response = h3_request(
        "127.0.0.1",
        tls_server.https_port,
        "/api/auth/login",
        method="POST",
        headers={"content-type": "application/json", "content-length": str(len(payload))},
        body=payload,
    )
    assert response.status == 200, response.body
    assert "access_token" in json.loads(response.body)


def test_authenticated_api_call_works_over_http3(
    tls_server: SarcaServer, tls_tokens: dict[str, str]
) -> None:
    response = h3_request(
        "127.0.0.1",
        tls_server.https_port,
        "/api/storages",
        headers={"authorization": f"Bearer {tls_tokens['access_token']}"},
    )
    assert response.status == 200, response.body
    assert "storages" in json.loads(response.body)


def test_http3_rejects_bad_credentials_like_tcp_does(tls_server: SarcaServer) -> None:
    payload = json.dumps({"email": "nobody@sarca.test", "password": "wrong"}).encode()
    response = h3_request(
        "127.0.0.1",
        tls_server.https_port,
        "/api/auth/login",
        method="POST",
        headers={"content-type": "application/json", "content-length": str(len(payload))},
        body=payload,
    )
    assert response.status in (401, 403, 404), response.status


def test_plain_http_listener_redirects_to_https(tls_server: SarcaServer) -> None:
    r = httpx.get(
        f"http://127.0.0.1:{tls_server.acme_port}/anything",
        follow_redirects=False,
        timeout=10.0,
    )
    assert r.status_code == 301
    assert r.headers["location"].startswith("https://")


def test_acme_challenge_path_is_served_over_plain_http(tls_server: SarcaServer) -> None:
    """The http-01 path must stay reachable on :80 for certificate renewal."""
    r = httpx.get(
        f"http://127.0.0.1:{tls_server.acme_port}/.well-known/acme-challenge/does-not-exist",
        follow_redirects=False,
        timeout=10.0,
    )
    # Unknown token → 404, but crucially not a redirect to HTTPS.
    assert r.status_code == 404, r.status_code


def test_ui_is_served_over_http3(tls_server: SarcaServer) -> None:
    response = h3_request("127.0.0.1", tls_server.https_port, "/")
    assert response.status == 200
    assert b"<!doctype html" in response.body.lower() or b"<html" in response.body.lower()
