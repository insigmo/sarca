"""In-process fake Telegram Bot API.

Implements exactly the surface Sarca uses (see `sarca/src/common/telegram_api/`):

    POST /bot<token>/sendDocument      multipart: chat_id + document
    GET  /bot<token>/getFile?file_id=
    GET  /file/bot<token>/<file_path>
    GET  /bot<token>/getMe
    GET  /bot<token>/getChat?chat_id=
    GET  /bot<token>/getChatMember?chat_id=&user_id=
    GET  /bot<token>/getUpdates
    POST /bot<token>/deleteWebhook
    POST /bot<token>/copyMessage       form: chat_id, from_chat_id, message_id
    POST /bot<token>/deleteMessage     form: chat_id, message_id

Plus a control surface for tests (not part of the real Bot API):

    GET  /__mock/stats                 call counters + stored document inventory
    POST /__mock/reset                 clear counters (documents kept)
    POST /__mock/latency               {"getFile": 0.25, "download": 0.25, ...}
    POST /__mock/flood                 {"method": "sendDocument", "times": 1, "retry_after": 1}
    POST /__mock/fail                  {"method": "sendDocument", "times": 1, "status": 500}

Documents are kept on disk so a restarted Sarca still resolves old file ids.
"""

from __future__ import annotations

import json
import re
import threading
import time
import uuid
from dataclasses import dataclass, field
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any
from urllib.parse import parse_qs, urlparse

_BOT_RE = re.compile(r"^/bot(?P<token>[^/]+)/(?P<method>[A-Za-z]+)$")
_FILE_RE = re.compile(r"^/file/bot(?P<token>[^/]+)/(?P<path>.+)$")


@dataclass
class _Document:
    file_id: str
    size: int
    chat_id: int
    message_id: int


@dataclass
class MockState:
    """Mutable state shared by the handler threads."""

    root: Path
    lock: threading.Lock = field(default_factory=threading.Lock)
    documents: dict[str, _Document] = field(default_factory=dict)
    # (chat_id, message_id) -> file_id ; deleted messages drop out of here
    messages: dict[tuple[int, int], str] = field(default_factory=dict)
    deleted_messages: set[tuple[int, int]] = field(default_factory=set)
    calls: dict[str, int] = field(default_factory=dict)
    bytes_sent: int = 0
    bytes_received: int = 0
    next_message_id: int = 1000
    latency: dict[str, float] = field(default_factory=dict)
    # method -> requests being served right now / peak seen so far
    in_flight: dict[str, int] = field(default_factory=dict)
    max_in_flight: dict[str, int] = field(default_factory=dict)
    # method -> [(kind, count, extra)] injected failures
    injected: dict[str, list[dict[str, Any]]] = field(default_factory=dict)
    updates: list[dict[str, Any]] = field(default_factory=list)

    def note(self, method: str) -> None:
        self.calls[method] = self.calls.get(method, 0) + 1

    def take_injection(self, method: str) -> dict[str, Any] | None:
        queue = self.injected.get(method)
        if not queue:
            return None
        item = queue[0]
        item["times"] -= 1
        if item["times"] <= 0:
            queue.pop(0)
        return item


class _Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    state: MockState  # injected by MockTelegram

    # ---------------------------------------------------------------- plumbing
    def log_message(self, fmt: str, *args: Any) -> None:  # noqa: A002
        line = f"{time.strftime('%H:%M:%S')} mock-telegram {fmt % args}\n"
        log = self.state.root / "mock_telegram.log"
        with log.open("a", encoding="utf-8") as fh:
            fh.write(line)

    def _read_body(self) -> bytes:
        length = self.headers.get("Content-Length")
        if length is not None:
            return self.rfile.read(int(length))
        if (self.headers.get("Transfer-Encoding") or "").lower() == "chunked":
            out = bytearray()
            while True:
                size_line = self.rfile.readline().strip()
                size = int(size_line.split(b";")[0] or b"0", 16)
                if size == 0:
                    self.rfile.readline()
                    break
                out += self.rfile.read(size)
                self.rfile.readline()
            return bytes(out)
        return b""

    def _send_json(self, payload: dict[str, Any], status: int = 200) -> None:
        body = json.dumps(payload).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _send_bytes(self, body: bytes, content_type: str = "application/octet-stream") -> None:
        self.send_response(200)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _error(self, status: int, description: str, extra: dict[str, Any] | None = None) -> None:
        payload: dict[str, Any] = {
            "ok": False,
            "error_code": status,
            "description": description,
        }
        if extra:
            payload.update(extra)
        self._send_json(payload, status=status)

    def _sleep(self, key: str) -> None:
        delay = self.state.latency.get(key, 0.0)
        if delay:
            time.sleep(delay)

    def _maybe_inject(self, method: str) -> bool:
        """Return True when the response was replaced by an injected failure."""
        with self.state.lock:
            item = self.state.take_injection(method)
        if not item:
            return False
        if item["kind"] == "flood":
            retry_after = int(item.get("retry_after", 1))
            self._error(
                429,
                f"Too Many Requests: retry after {retry_after}",
                {"parameters": {"retry_after": retry_after}, "retry_after": retry_after},
            )
        else:
            self._error(int(item.get("status", 500)), item.get("description", "Internal Server Error"))
        return True

    # ------------------------------------------------------------------ routes
    def do_GET(self) -> None:  # noqa: N802
        parsed = urlparse(self.path)
        query = parse_qs(parsed.query)

        if parsed.path.startswith("/__mock/"):
            self._control(parsed.path, query, body=None)
            return

        file_match = _FILE_RE.match(parsed.path)
        if file_match:
            self._download(file_match.group("path"))
            return

        bot_match = _BOT_RE.match(parsed.path)
        if not bot_match:
            self._error(404, "Not Found")
            return

        method = bot_match.group("method")
        token = bot_match.group("token")
        with self.state.lock:
            self.state.note(method)
        if self._maybe_inject(method):
            return
        self._sleep(method)

        if method == "getMe":
            bot_id = abs(hash(token)) % 10_000_000
            self._send_json(
                {
                    "ok": True,
                    "result": {
                        "id": bot_id,
                        "is_bot": True,
                        "first_name": "MockBot",
                        "username": f"mock_{bot_id}_bot",
                    },
                }
            )
        elif method == "getChat":
            chat_id = int(query.get("chat_id", ["0"])[0])
            self._send_json(
                {
                    "ok": True,
                    "result": {"id": chat_id, "title": f"Mock Channel {chat_id}", "type": "channel"},
                }
            )
        elif method == "getChatMember":
            self._send_json({"ok": True, "result": {"status": "administrator"}})
        elif method == "getUpdates":
            with self.state.lock:
                updates = list(self.state.updates)
            self._send_json({"ok": True, "result": updates})
        elif method == "getFile":
            self._get_file(query.get("file_id", [""])[0])
        else:
            self._error(400, f"Bad Request: method not found ({method})")

    def do_POST(self) -> None:  # noqa: N802
        parsed = urlparse(self.path)

        if parsed.path.startswith("/__mock/"):
            body = self._read_body()
            self._control(parsed.path, parse_qs(parsed.query), body=body)
            return

        bot_match = _BOT_RE.match(parsed.path)
        if not bot_match:
            self._error(404, "Not Found")
            return

        method = bot_match.group("method")
        body = self._read_body()
        with self.state.lock:
            self.state.note(method)
            self.state.bytes_received += len(body)
        if self._maybe_inject(method):
            return

        # Peak overlap per method, so tests can tell "Sarca relayed these files in
        # parallel" from "it just did them quickly". Counted around the artificial
        # latency, which is what gives the overlap a window to be observed in.
        with self.state.lock:
            self.state.in_flight[method] = self.state.in_flight.get(method, 0) + 1
            self.state.max_in_flight[method] = max(
                self.state.max_in_flight.get(method, 0), self.state.in_flight[method]
            )
        try:
            self._sleep(method)

            if method == "sendDocument":
                self._send_document(body)
            elif method == "copyMessage":
                self._copy_message(body)
            elif method == "deleteMessage":
                self._delete_message(body)
            elif method == "deleteWebhook":
                self._send_json({"ok": True, "result": True})
            else:
                self._error(400, f"Bad Request: method not found ({method})")
        finally:
            with self.state.lock:
                self.state.in_flight[method] -= 1

    # ---------------------------------------------------------------- handlers
    def _send_document(self, body: bytes) -> None:
        content_type = self.headers.get("Content-Type", "")
        if "multipart/form-data" not in content_type:
            self._error(400, "Bad Request: expected multipart/form-data")
            return
        fields, document = _parse_multipart(body, content_type)
        if document is None:
            self._error(400, "Bad Request: document part missing")
            return
        try:
            chat_id = int(fields.get("chat_id", "0"))
        except ValueError:
            self._error(400, "Bad Request: chat_id is empty")
            return
        if chat_id == 0:
            self._error(400, "Bad Request: chat not found")
            return

        file_id = f"mockfile-{uuid.uuid4().hex}"
        (self.state.root / "documents").mkdir(parents=True, exist_ok=True)
        (self.state.root / "documents" / f"{file_id}.bin").write_bytes(document)

        with self.state.lock:
            self.state.next_message_id += 1
            message_id = self.state.next_message_id
            self.state.documents[file_id] = _Document(file_id, len(document), chat_id, message_id)
            self.state.messages[(chat_id, message_id)] = file_id

        self._send_json(
            {
                "ok": True,
                "result": {
                    "message_id": message_id,
                    "chat": {"id": chat_id, "type": "channel"},
                    "date": int(time.time()),
                    "document": {
                        "file_id": file_id,
                        "file_unique_id": file_id[-16:],
                        "file_name": "sarca_chunk.bin",
                        "file_size": len(document),
                    },
                },
            }
        )

    def _get_file(self, file_id: str) -> None:
        with self.state.lock:
            doc = self.state.documents.get(file_id)
        if doc is None:
            self._error(400, "Bad Request: file not found")
            return
        self._send_json(
            {
                "ok": True,
                "result": {
                    "file_id": file_id,
                    "file_unique_id": file_id[-16:],
                    "file_size": doc.size,
                    "file_path": f"documents/{file_id}.bin",
                },
            }
        )

    def _download(self, rel_path: str) -> None:
        with self.state.lock:
            self.state.note("download")
        if self._maybe_inject("download"):
            return
        self._sleep("download")
        target = (self.state.root / rel_path).resolve()
        if not str(target).startswith(str(self.state.root.resolve())) or not target.is_file():
            self._error(404, "Not Found")
            return
        data = target.read_bytes()
        with self.state.lock:
            self.state.bytes_sent += len(data)
        self._send_bytes(data)

    def _copy_message(self, body: bytes) -> None:
        form = _parse_urlencoded(body)
        from_chat = int(form.get("from_chat_id", "0"))
        to_chat = int(form.get("chat_id", "0"))
        src_message = int(form.get("message_id", "0"))
        with self.state.lock:
            file_id = self.state.messages.get((from_chat, src_message))
            if file_id is None:
                self._error(400, "Bad Request: message to copy not found")
                return
            self.state.next_message_id += 1
            new_message_id = self.state.next_message_id
            self.state.messages[(to_chat, new_message_id)] = file_id
        self._send_json({"ok": True, "result": {"message_id": new_message_id}})

    def _delete_message(self, body: bytes) -> None:
        form = _parse_urlencoded(body)
        chat_id = int(form.get("chat_id", "0"))
        message_id = int(form.get("message_id", "0"))
        with self.state.lock:
            existed = self.state.messages.pop((chat_id, message_id), None)
            self.state.deleted_messages.add((chat_id, message_id))
        if existed is None:
            self._error(400, "Bad Request: message to delete not found")
            return
        self._send_json({"ok": True, "result": True})

    # ---------------------------------------------------------------- control
    def _control(self, path: str, query: dict[str, list[str]], body: bytes | None) -> None:
        action = path[len("/__mock/") :]
        payload: dict[str, Any] = {}
        if body:
            try:
                payload = json.loads(body)
            except json.JSONDecodeError:
                payload = {}

        with self.state.lock:
            if action == "stats":
                self._send_json(
                    {
                        "calls": dict(self.state.calls),
                        "documents": len(self.state.documents),
                        "messages": len(self.state.messages),
                        "deleted_messages": len(self.state.deleted_messages),
                        "bytes_received": self.state.bytes_received,
                        "bytes_sent": self.state.bytes_sent,
                        "document_sizes": sorted(d.size for d in self.state.documents.values()),
                    }
                )
                return
            if action == "reset":
                self.state.calls.clear()
                self.state.max_in_flight.clear()
                self.state.bytes_sent = 0
                self.state.bytes_received = 0
                self.state.injected.clear()
                self.state.latency.clear()
                self._send_json({"ok": True})
                return
            if action == "latency":
                self.state.latency.update({k: float(v) for k, v in payload.items()})
                self._send_json({"ok": True, "latency": self.state.latency})
                return
            if action in {"flood", "fail"}:
                method = payload.get("method", "sendDocument")
                item = {
                    "kind": "flood" if action == "flood" else "fail",
                    "times": int(payload.get("times", 1)),
                    "retry_after": int(payload.get("retry_after", 1)),
                    "status": int(payload.get("status", 500)),
                    "description": payload.get("description", "Internal Server Error"),
                }
                self.state.injected.setdefault(method, []).append(item)
                self._send_json({"ok": True})
                return
            if action == "updates":
                self.state.updates = list(payload.get("updates", []))
                self._send_json({"ok": True})
                return
            if action == "message_exists":
                key = (int(payload.get("chat_id", 0)), int(payload.get("message_id", 0)))
                self._send_json({"exists": key in self.state.messages})
                return
        self._error(404, "unknown control action")


def _parse_urlencoded(body: bytes) -> dict[str, str]:
    return {k: v[0] for k, v in parse_qs(body.decode("utf-8", "replace")).items()}


def _parse_multipart(body: bytes, content_type: str) -> tuple[dict[str, str], bytes | None]:
    """Minimal multipart/form-data parser: text fields + the `document` part."""
    marker = "boundary="
    idx = content_type.find(marker)
    if idx < 0:
        return {}, None
    boundary = content_type[idx + len(marker) :].strip().strip('"')
    sep = b"--" + boundary.encode()

    fields: dict[str, str] = {}
    document: bytes | None = None

    for raw in body.split(sep):
        if raw in (b"", b"--", b"--\r\n", b"\r\n"):
            continue
        raw = raw.lstrip(b"\r\n")
        head, _, data = raw.partition(b"\r\n\r\n")
        if not _:
            continue
        data = data[:-2] if data.endswith(b"\r\n") else data
        headers = head.decode("utf-8", "replace")
        name_match = re.search(r'name="([^"]+)"', headers)
        if not name_match:
            continue
        name = name_match.group(1)
        if name == "document":
            document = data
        else:
            fields[name] = data.decode("utf-8", "replace").strip()

    return fields, document


class MockTelegram:
    """Threaded fake Bot API bound to an ephemeral port."""

    def __init__(self, root: Path) -> None:
        root.mkdir(parents=True, exist_ok=True)
        (root / "documents").mkdir(exist_ok=True)
        self.state = MockState(root=root)
        handler = type("BoundHandler", (_Handler,), {"state": self.state})
        self._server = ThreadingHTTPServer(("127.0.0.1", 0), handler)
        self._server.daemon_threads = True
        self._thread = threading.Thread(target=self._server.serve_forever, daemon=True)

    @property
    def base_url(self) -> str:
        host, port = self._server.server_address[:2]
        return f"http://{host}:{port}"

    def start(self) -> MockTelegram:
        self._thread.start()
        return self

    def stop(self) -> None:
        self._server.shutdown()
        self._server.server_close()

    # convenience helpers used directly by tests (no HTTP round trip needed)
    def stats(self) -> dict[str, Any]:
        with self.state.lock:
            return {
                "calls": dict(self.state.calls),
                "documents": len(self.state.documents),
                "messages": len(self.state.messages),
                "document_sizes": sorted(d.size for d in self.state.documents.values()),
            }

    def calls(self, method: str) -> int:
        with self.state.lock:
            return self.state.calls.get(method, 0)

    def reset_calls(self) -> None:
        with self.state.lock:
            self.state.calls.clear()
            self.state.max_in_flight.clear()

    def max_concurrent(self, method: str = "sendDocument") -> int:
        """Highest number of `method` requests served at the same time."""
        with self.state.lock:
            return self.state.max_in_flight.get(method, 0)

    def set_latency(self, **kwargs: float) -> None:
        with self.state.lock:
            self.state.latency.update(kwargs)

    def clear_latency(self) -> None:
        with self.state.lock:
            self.state.latency.clear()

    def inject_flood(self, method: str = "sendDocument", times: int = 1, retry_after: int = 1) -> None:
        with self.state.lock:
            self.state.injected.setdefault(method, []).append(
                {"kind": "flood", "times": times, "retry_after": retry_after}
            )

    def inject_failure(self, method: str, times: int = 1, status: int = 500) -> None:
        with self.state.lock:
            self.state.injected.setdefault(method, []).append(
                {"kind": "fail", "times": times, "status": status, "description": "boom"}
            )

    def clear_injections(self) -> None:
        """Drop queued flood/failure injections (they are consumed per request)."""
        with self.state.lock:
            self.state.injected.clear()

    def document_bytes(self, file_id: str) -> bytes:
        return (self.state.root / "documents" / f"{file_id}.bin").read_bytes()

    def document_count(self) -> int:
        with self.state.lock:
            return len(self.state.documents)

    def live_messages(self) -> set[tuple[int, int]]:
        with self.state.lock:
            return set(self.state.messages)


if __name__ == "__main__":  # manual runs: python -m helpers.mock_telegram
    import tempfile

    mock = MockTelegram(Path(tempfile.mkdtemp(prefix="mock-telegram-"))).start()
    print(f"mock telegram on {mock.base_url} (root={mock.state.root})")
    try:
        while True:
            time.sleep(3600)
    except KeyboardInterrupt:
        mock.stop()
