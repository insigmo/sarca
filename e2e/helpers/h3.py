"""Minimal HTTP/3 (QUIC) client for the e2e suite, built on aioquic."""

from __future__ import annotations

import asyncio
import ssl
from dataclasses import dataclass, field
from typing import Any

from aioquic.asyncio.client import connect
from aioquic.asyncio.protocol import QuicConnectionProtocol
from aioquic.h3.connection import H3_ALPN, H3Connection
from aioquic.h3.events import DataReceived, HeadersReceived
from aioquic.quic.configuration import QuicConfiguration
from aioquic.quic.events import QuicEvent


@dataclass
class H3Response:
    status: int
    headers: dict[str, str] = field(default_factory=dict)
    body: bytes = b""
    alpn: str | None = None


class _H3Client(QuicConnectionProtocol):
    def __init__(self, *args: Any, **kwargs: Any) -> None:
        super().__init__(*args, **kwargs)
        self._http = H3Connection(self._quic)
        self._responses: dict[int, H3Response] = {}
        self._waiters: dict[int, asyncio.Future] = {}

    async def request(
        self,
        method: str,
        authority: str,
        path: str,
        headers: dict[str, str] | None,
        body: bytes | None,
    ) -> H3Response:
        stream_id = self._quic.get_next_available_stream_id()
        request_headers = [
            (b":method", method.encode()),
            (b":scheme", b"https"),
            (b":authority", authority.encode()),
            (b":path", path.encode()),
        ]
        for key, value in (headers or {}).items():
            request_headers.append((key.lower().encode(), value.encode()))

        self._responses[stream_id] = H3Response(status=0)
        waiter = asyncio.get_event_loop().create_future()
        self._waiters[stream_id] = waiter

        self._http.send_headers(stream_id, request_headers, end_stream=body is None)
        if body is not None:
            self._http.send_data(stream_id, body, end_stream=True)
        self.transmit()

        return await asyncio.shield(waiter)

    def quic_event_received(self, event: QuicEvent) -> None:
        for h3_event in self._http.handle_event(event):
            stream_id = getattr(h3_event, "stream_id", None)
            response = self._responses.get(stream_id)
            if response is None:
                continue
            if isinstance(h3_event, HeadersReceived):
                for key, value in h3_event.headers:
                    name = key.decode()
                    if name == ":status":
                        response.status = int(value)
                    else:
                        response.headers[name] = value.decode()
            elif isinstance(h3_event, DataReceived):
                response.body += h3_event.data
            if getattr(h3_event, "stream_ended", False) or self._body_complete(response):
                self._finish(stream_id, response)

    @staticmethod
    def _body_complete(response: H3Response) -> bool:
        """aioquic does not always flag `stream_ended` when HEADERS and DATA arrive in
        one packet, so fall back to Content-Length (axum always sets it here)."""
        if not response.status:
            return False
        length = response.headers.get("content-length")
        return length is not None and len(response.body) >= int(length)

    def _finish(self, stream_id: int, response: H3Response) -> None:
        waiter = self._waiters.pop(stream_id, None)
        if waiter is not None and not waiter.done():
            response.alpn = self._quic.tls.alpn_negotiated
            waiter.set_result(response)


async def _run(
    host: str,
    port: int,
    method: str,
    path: str,
    headers: dict[str, str] | None,
    body: bytes | None,
    timeout: float,
) -> H3Response:
    config = QuicConfiguration(is_client=True, alpn_protocols=H3_ALPN)
    # Self-signed certificate in dev/e2e: cert identity is not what these tests check.
    config.verify_mode = ssl.CERT_NONE

    async with connect(
        host, port, configuration=config, create_protocol=_H3Client, wait_connected=True
    ) as client:
        return await asyncio.wait_for(
            client.request(method, f"{host}:{port}", path, headers, body),
            timeout=timeout,
        )


def h3_request(
    host: str,
    port: int,
    path: str,
    method: str = "GET",
    headers: dict[str, str] | None = None,
    body: bytes | None = None,
    timeout: float = 15.0,
) -> H3Response:
    """Perform one HTTP/3 request over QUIC and return the full response."""
    return asyncio.run(_run(host, port, method, path, headers, body, timeout))
