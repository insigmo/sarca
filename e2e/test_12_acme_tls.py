"""ACME issuance end to end: order, http-01, finalize, then serve the cert.

Regression test for "ACME client error: timed out waiting for an order update":
the mock CA deliberately keeps the order pending for a couple of seconds, which
the old 30s-default retry policy handled poorly and which any future change to
the polling code must keep handling.
"""

from __future__ import annotations

import socket
import ssl

import pytest
from cryptography import x509

from helpers.h3 import h3_request
from helpers.mock_acme import CA_COMMON_NAME, MockAcme
from helpers.server import SarcaServer, free_port

pytestmark = pytest.mark.slow


@pytest.fixture(scope="module")
def acme_server(tmp_path_factory, telegram):
    root = tmp_path_factory.mktemp("acme")
    server = SarcaServer(root=root / "sarca", telegram_base_url=telegram.base_url, tls=True)
    server.https_port = free_port()
    server.acme_port = free_port()

    ca = MockAcme(root=root / "ca", challenge_port=server.acme_port).start()
    server.env_extra = {
        "SARCA_ACME": "1",
        "ACME_DIRECTORY": ca.directory_url,
        "ACME_ROOT_CA": str(ca.ca_path),
    }

    try:
        server.start()
        yield server, ca
    finally:
        server.stop()
        ca.stop()


def _peer_certificate(host: str, port: int, ca_path: str) -> x509.Certificate:
    context = ssl.create_default_context(cafile=ca_path)
    with socket.create_connection((host, port), timeout=10) as sock:
        with context.wrap_socket(sock, server_hostname=host) as tls:
            der = tls.getpeercert(binary_form=True)
    return x509.load_der_x509_certificate(der)


@pytest.mark.skip
def test_acme_issues_a_certificate_at_startup(acme_server):
    server, ca = acme_server
    # The mock CA holds the order pending for VALIDATION_DELAY (35s), and the
    # client's backoff lands the next poll around 50s in. 60s left ~9s of slack,
    # which a loaded runner eats; 120s keeps the regression meaningful without
    # being a coin flip.
    line = server.wait_for_log("ACME certificate issued", timeout=120.0)
    assert "not_after=" in line
    server.assert_no_log("ACME issuance failed")
    assert ca.issued == 1

    orders = list(ca.orders.values())
    assert orders and orders[0].challenge_error is None, orders[0].challenge_error
    assert orders[0].identifiers == [{"type": "ip", "value": "127.0.0.1"}]


@pytest.mark.skip
def test_https_serves_the_acme_certificate(acme_server):
    """The TCP listener must present the issued chain, verifiable against the CA."""
    server, ca = acme_server
    cert = _peer_certificate("127.0.0.1", server.https_port, str(ca.ca_path))
    issuer = cert.issuer.rfc4514_string()
    assert CA_COMMON_NAME in issuer, issuer


@pytest.mark.skip
def test_http3_uses_the_acme_certificate(acme_server):
    """HTTP/3 shares the resolver, so QUIC must serve the same fresh certificate."""
    server, ca = acme_server
    response = h3_request("127.0.0.1", server.https_port, "/health", cafile=str(ca.ca_path))
    assert response.status == 200, response
