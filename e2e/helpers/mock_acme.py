"""A tiny ACME (RFC 8555) server for the e2e suite.

It speaks just enough of the protocol for instant-acme to complete one order:
directory, nonces, account, order, http-01 authorization, finalize, download.
Signatures are not verified, but the http-01 challenge is: the mock really
fetches `/.well-known/acme-challenge/<token>` from the server under test before
marking the authorization valid, so a broken challenge listener fails the test.

Served over HTTPS because instant-acme refuses plain HTTP directories. The CA
that signs the mock's own certificate is written to disk and handed to the
server under test through `ACME_ROOT_CA`.
"""

from __future__ import annotations

import base64
import json
import ssl
import threading
import time
from dataclasses import dataclass, field
from datetime import datetime, timedelta, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from ipaddress import ip_address
from pathlib import Path
from typing import Any

import httpx
from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import ec
from cryptography.x509.oid import NameOID

CA_COMMON_NAME = "Sarca E2E ACME CA"

# The order stays pending for this long after the challenge is answered. It is
# deliberately longer than instant-acme's 30s default RetryPolicy timeout, which
# is what produced "timed out waiting for an order update" against the real
# Let's Encrypt: a client that does not raise that budget fails this test.
VALIDATION_DELAY = 35.0


def _b64url_decode(data: str) -> bytes:
    return base64.urlsafe_b64decode(data + "=" * (-len(data) % 4))


@dataclass
class _Order:
    identifiers: list[dict[str, str]]
    status: str = "pending"
    certificate_pem: str | None = None
    ready_at: float | None = None
    token: str = "e2e-challenge-token"
    authz_status: str = "pending"
    challenge_error: str | None = None


@dataclass
class MockAcme:
    """Mock ACME CA. `challenge_port` is the server's ACME http-01 port."""

    root: Path
    challenge_port: int
    challenge_host: str = "127.0.0.1"
    port: int = 0
    orders: dict[str, _Order] = field(default_factory=dict)
    issued: int = 0

    def __post_init__(self) -> None:
        self.root.mkdir(parents=True, exist_ok=True)
        self._build_ca()
        self._server: ThreadingHTTPServer | None = None
        self._thread: threading.Thread | None = None

    # ------------------------------------------------------------------ certs
    def _build_ca(self) -> None:
        self.ca_key = ec.generate_private_key(ec.SECP256R1())
        subject = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, CA_COMMON_NAME)])
        now = datetime.now(timezone.utc)
        self.ca_cert = (
            x509.CertificateBuilder()
            .subject_name(subject)
            .issuer_name(subject)
            .public_key(self.ca_key.public_key())
            .serial_number(x509.random_serial_number())
            .not_valid_before(now - timedelta(minutes=5))
            .not_valid_after(now + timedelta(days=1))
            .add_extension(x509.BasicConstraints(ca=True, path_length=None), critical=True)
            .add_extension(
                x509.SubjectKeyIdentifier.from_public_key(self.ca_key.public_key()),
                critical=False,
            )
            .add_extension(
                x509.KeyUsage(
                    digital_signature=True,
                    content_commitment=False,
                    key_encipherment=False,
                    data_encipherment=False,
                    key_agreement=False,
                    key_cert_sign=True,
                    crl_sign=True,
                    encipher_only=False,
                    decipher_only=False,
                ),
                critical=True,
            )
            .sign(self.ca_key, hashes.SHA256())
        )
        self.ca_path = self.root / "acme-ca.pem"
        self.ca_path.write_bytes(self.ca_cert.public_bytes(serialization.Encoding.PEM))

        # TLS certificate for the mock's own HTTPS listener.
        key = ec.generate_private_key(ec.SECP256R1())
        cert = self._sign(
            key.public_key(),
            x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, "acme.local")]),
            [x509.IPAddress(ip_address("127.0.0.1"))],
        )
        self._tls_cert_path = self.root / "acme-server.pem"
        self._tls_key_path = self.root / "acme-server.key"
        self._tls_cert_path.write_bytes(cert.public_bytes(serialization.Encoding.PEM))
        self._tls_key_path.write_bytes(
            key.private_bytes(
                serialization.Encoding.PEM,
                serialization.PrivateFormat.PKCS8,
                serialization.NoEncryption(),
            )
        )

    def _sign(self, public_key: Any, subject: x509.Name, sans: list[x509.GeneralName]) -> Any:
        now = datetime.now(timezone.utc)
        return (
            x509.CertificateBuilder()
            .subject_name(subject)
            .issuer_name(self.ca_cert.subject)
            .public_key(public_key)
            .serial_number(x509.random_serial_number())
            .not_valid_before(now - timedelta(minutes=5))
            .not_valid_after(now + timedelta(days=6))
            .add_extension(x509.SubjectAlternativeName(sans), critical=False)
            .add_extension(x509.BasicConstraints(ca=False, path_length=None), critical=True)
            .add_extension(
                x509.AuthorityKeyIdentifier.from_issuer_public_key(self.ca_key.public_key()),
                critical=False,
            )
            .add_extension(
                x509.SubjectKeyIdentifier.from_public_key(public_key),
                critical=False,
            )
            .sign(self.ca_key, hashes.SHA256())
        )

    def issue_from_csr(self, csr_der: bytes) -> str:
        csr = x509.load_der_x509_csr(csr_der)
        try:
            sans = list(csr.extensions.get_extension_for_class(x509.SubjectAlternativeName).value)
        except x509.ExtensionNotFound:
            sans = []
        subject = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, "sarca-e2e")])
        cert = self._sign(csr.public_key(), subject, sans)
        self.issued += 1
        return (
            cert.public_bytes(serialization.Encoding.PEM).decode()
            + self.ca_cert.public_bytes(serialization.Encoding.PEM).decode()
        )

    # ----------------------------------------------------------------- server
    @property
    def directory_url(self) -> str:
        return f"https://127.0.0.1:{self.port}/directory"

    def start(self) -> MockAcme:
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.load_cert_chain(self._tls_cert_path, self._tls_key_path)

        mock = self
        handler = type("Handler", (_AcmeHandler,), {"mock": mock})
        self._server = ThreadingHTTPServer(("127.0.0.1", self.port), handler)
        self._server.socket = context.wrap_socket(self._server.socket, server_side=True)
        self.port = self._server.server_address[1]
        self._thread = threading.Thread(target=self._server.serve_forever, daemon=True)
        self._thread.start()
        return self

    def stop(self) -> None:
        if self._server:
            self._server.shutdown()
            self._server.server_close()
            self._server = None

    # ------------------------------------------------------------ validation
    def validate_challenge(self, order: _Order) -> None:
        """Fetch the token from the server under test, exactly as a real CA does."""
        url = (
            f"http://{self.challenge_host}:{self.challenge_port}"
            f"/.well-known/acme-challenge/{order.token}"
        )
        try:
            body = httpx.get(url, timeout=5.0).text.strip()
        except Exception as e:  # noqa: BLE001
            order.authz_status = "invalid"
            order.status = "invalid"
            order.challenge_error = f"fetch failed: {e}"
            return

        if not body.startswith(f"{order.token}."):
            order.authz_status = "invalid"
            order.status = "invalid"
            order.challenge_error = f"unexpected key authorization: {body!r}"
            return

        order.ready_at = time.monotonic() + VALIDATION_DELAY

    def refresh(self, order: _Order) -> None:
        if order.ready_at is not None and time.monotonic() >= order.ready_at:
            order.authz_status = "valid"
            if order.status == "pending":
                order.status = "ready"


class _AcmeHandler(BaseHTTPRequestHandler):
    mock: MockAcme
    protocol_version = "HTTP/1.1"

    def log_message(self, *_args: Any) -> None:  # keep pytest output clean
        pass

    # ------------------------------------------------------------------ utils
    def _base(self) -> str:
        return f"https://127.0.0.1:{self.mock.port}"

    def _send(
        self,
        status: int,
        body: dict[str, Any] | str | None = None,
        headers: dict[str, str] | None = None,
        content_type: str = "application/json",
    ) -> None:
        payload = b""
        if isinstance(body, dict):
            payload = json.dumps(body).encode()
        elif isinstance(body, str):
            payload = body.encode()

        self.send_response(status)
        self.send_header("Replay-Nonce", base64.urlsafe_b64encode(str(time.time_ns()).encode()).decode().rstrip("="))
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(payload)))
        for key, value in (headers or {}).items():
            self.send_header(key, value)
        self.end_headers()
        if payload:
            self.wfile.write(payload)

    def _payload(self) -> dict[str, Any]:
        length = int(self.headers.get("Content-Length", "0"))
        raw = self.rfile.read(length) if length else b"{}"
        jws = json.loads(raw or b"{}")
        payload = jws.get("payload", "")
        if not payload:
            return {}
        return json.loads(_b64url_decode(payload))

    def _order_state(self, order_id: str, order: _Order) -> dict[str, Any]:
        self.mock.refresh(order)
        state: dict[str, Any] = {
            "status": order.status,
            "expires": "2100-01-01T00:00:00Z",
            "identifiers": order.identifiers,
            "authorizations": [f"{self._base()}/authz/{order_id}"],
            "finalize": f"{self._base()}/finalize/{order_id}",
        }
        if order.certificate_pem:
            state["certificate"] = f"{self._base()}/cert/{order_id}"
        return state

    # ----------------------------------------------------------------- routes
    def do_HEAD(self) -> None:  # noqa: N802
        self._send(200)

    def do_GET(self) -> None:  # noqa: N802
        if self.path == "/directory":
            base = self._base()
            self._send(
                200,
                {
                    "newNonce": f"{base}/new-nonce",
                    "newAccount": f"{base}/new-account",
                    "newOrder": f"{base}/new-order",
                    "revokeCert": f"{base}/revoke-cert",
                    "keyChange": f"{base}/key-change",
                },
            )
            return
        if self.path == "/new-nonce":
            self._send(200)
            return
        self._send(404, {"type": "urn:ietf:params:acme:error:malformed"})

    def do_POST(self) -> None:  # noqa: N802
        path = self.path
        payload = self._payload()
        base = self._base()

        if path == "/new-account":
            self._send(201, {"status": "valid"}, {"Location": f"{base}/acct/1"})
            return

        if path == "/new-order":
            order_id = str(len(self.mock.orders) + 1)
            order = _Order(identifiers=payload.get("identifiers", []))
            order.token = f"e2e-token-{order_id}"
            self.mock.orders[order_id] = order
            self._send(
                201,
                self._order_state(order_id, order),
                {"Location": f"{base}/order/{order_id}"},
            )
            return

        if path.startswith("/authz/"):
            order_id = path.rsplit("/", 1)[1]
            order = self.mock.orders[order_id]
            self.mock.refresh(order)
            self._send(
                200,
                {
                    "status": order.authz_status,
                    "identifier": order.identifiers[0],
                    "challenges": [
                        {
                            "type": "http-01",
                            "url": f"{base}/chall/{order_id}",
                            "token": order.token,
                            "status": order.authz_status,
                        }
                    ],
                },
            )
            return

        if path.startswith("/chall/"):
            order_id = path.rsplit("/", 1)[1]
            order = self.mock.orders[order_id]
            self.mock.validate_challenge(order)
            self._send(
                200,
                {
                    "type": "http-01",
                    "url": f"{base}/chall/{order_id}",
                    "token": order.token,
                    "status": "processing",
                },
            )
            return

        if path.startswith("/order/"):
            order_id = path.rsplit("/", 1)[1]
            self._send(200, self._order_state(order_id, self.mock.orders[order_id]))
            return

        if path.startswith("/finalize/"):
            order_id = path.rsplit("/", 1)[1]
            order = self.mock.orders[order_id]
            csr_der = _b64url_decode(payload["csr"])
            order.certificate_pem = self.mock.issue_from_csr(csr_der)
            order.status = "valid"
            self._send(200, self._order_state(order_id, order))
            return

        if path.startswith("/cert/"):
            order_id = path.rsplit("/", 1)[1]
            order = self.mock.orders[order_id]
            self._send(
                200,
                order.certificate_pem or "",
                content_type="application/pem-certificate-chain",
            )
            return

        self._send(404, {"type": "urn:ietf:params:acme:error:malformed"})
