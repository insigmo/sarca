"""Launch a real Sarca server process for e2e tests.

Each instance gets its own WORK_DIR, SQLite file, port and log file, so tests can
restart the server, inspect logs, and never touch the developer's own install.
"""

from __future__ import annotations

import os
import shutil
import signal
import socket
import stat
import subprocess
import sys
import time
from dataclasses import dataclass, field
from pathlib import Path

import httpx

ROOT = Path(__file__).resolve().parents[2]

IS_WINDOWS = os.name == "nt"
BINARY_NAME = "sarca.exe" if IS_WINDOWS else "sarca"


def _rm_readonly(func, path, _exc_info):
    """shutil.rmtree helper: Windows keeps read-only bits on cargo artifacts."""
    os.chmod(path, stat.S_IWRITE)
    func(path)


def repo_root() -> Path:
    return ROOT


def free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def free_udp_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def build_binary() -> Path:
    """Return the sarca binary, building it unless SARCA_BIN points at one."""
    override = os.environ.get("SARCA_BIN")
    if override:
        path = Path(override).resolve()
        if not path.is_file():
            raise RuntimeError(f"SARCA_BIN={path} does not exist")
        return path

    target_dir = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))
    binary = target_dir / "release" / BINARY_NAME
    if os.environ.get("SARCA_SKIP_BUILD") == "1" and binary.is_file():
        return binary

    subprocess.run(
        ["cargo", "build", "--release", "-p", "sarca"],
        cwd=ROOT,
        check=True,
    )
    if not binary.is_file():
        raise RuntimeError(f"build succeeded but {binary} is missing")
    return binary


def ui_dist() -> Path | None:
    dist = ROOT / "ui" / "dist"
    return dist if (dist / "index.html").is_file() else None


@dataclass
class SarcaServer:
    """A running `sarca` process under test."""

    root: Path
    telegram_base_url: str
    email: str = "e2e@sarca.test"
    password: str = "e2e-password-123"
    env_extra: dict[str, str] = field(default_factory=dict)

    https_port: int = 0
    acme_port: int = 0
    process: subprocess.Popen | None = None
    log_path: Path = field(init=False)
    work_dir: Path = field(init=False)
    runtime_dir: Path = field(init=False)

    def __post_init__(self) -> None:
        self.root.mkdir(parents=True, exist_ok=True)
        self.work_dir = self.root / "work"
        self.runtime_dir = self.root / "runtime"
        self.log_path = self.root / "server.log"
        self.work_dir.mkdir(exist_ok=True)
        self.runtime_dir.mkdir(exist_ok=True)

    # ------------------------------------------------------------------ launch
    @property
    def base_url(self) -> str:
        return self.https_base_url

    @property
    def https_base_url(self) -> str:
        return f"https://127.0.0.1:{self.https_port}"

    def _env(self) -> dict[str, str]:
        env = {
            k: v
            for k, v in os.environ.items()
            # e2e shells often export SUPERUSER_* for the dev instance
            if k
            not in {
                "SUPERUSER_EMAIL",
                "SUPERUSER_PASS",
                "WORK_DIR",
                "SQLITE_PATH",
                "TLS_HOSTNAME",
                "TELEGRAM_API_BASE_URL",
            }
        }
        env.update(
            {
                "WORKERS": "4",
                "CHANNEL_CAPACITY": "32",
                "SUPERUSER_EMAIL": self.email,
                "SUPERUSER_PASS": self.password,
                "ACCESS_TOKEN_EXPIRE_IN_SECS": "1800",
                "REFRESH_TOKEN_EXPIRE_IN_DAYS": "14",
                "SECRET_KEY": "e2e" * 40,
                "TELEGRAM_API_BASE_URL": self.telegram_base_url,
                # requests/min/bot, u16 — the fake Bot API has no flood control
                "TELEGRAM_RATE_LIMIT": "10000",
                "TELEGRAM_CHUNK_SIZE_MB": "20",
                "WORK_DIR": str(self.work_dir),
                "SQLITE_PATH": str(self.work_dir / "sarca.sqlite"),
                "CERTS_DIR": str(self.work_dir / "certs"),
                "RUST_LOG": os.environ.get("SARCA_E2E_RUST_LOG", "sarca=debug,info"),
                "RUST_BACKTRACE": "1",
                "TLS_HOSTNAME": "127.0.0.1",
                "HTTPS_ADDR": f"127.0.0.1:{self.https_port}",
                "ACME_HTTP_ADDR": f"127.0.0.1:{self.acme_port}",
                "SARCA_ACME": "0",
            }
        )
        env.update(self.env_extra)
        return env

    def start(self, binary: Path | None = None, wait: bool = True) -> SarcaServer:
        binary = binary or build_binary()
        if self.https_port == 0:
            self.https_port = free_port()
            self.acme_port = free_port()

        # The server resolves its UI dir next to the binary.
        target_bin = self.runtime_dir / BINARY_NAME
        if not target_bin.exists():
            shutil.copy2(binary, target_bin)
            target_bin.chmod(0o755)
        dist = ui_dist()
        ui_link = self.runtime_dir / "ui"
        if dist and not ui_link.exists():
            # Windows has no directory symlinks without developer mode; a
            # junction is transparent to every filesystem caller.
            if IS_WINDOWS:
                subprocess.run(
                    ["cmd", "/c", "mklink", "/J", str(ui_link), str(dist)],
                    check=True,
                    capture_output=True,
                )
            else:
                ui_link.symlink_to(dist)

        log = self.log_path.open("ab")
        creationflags = subprocess.CREATE_NEW_PROCESS_GROUP if IS_WINDOWS else 0
        self.process = subprocess.Popen(
            [str(target_bin)],
            cwd=self.runtime_dir,
            env=self._env(),
            stdout=log,
            stderr=subprocess.STDOUT,
            **({"creationflags": creationflags} if IS_WINDOWS else {"start_new_session": True}),
        )
        if wait:
            self.wait_ready()
        return self

    def wait_ready(self, timeout: float = 90.0) -> None:
        deadline = time.time() + timeout
        url = f"{self.https_base_url}/api/auth/login"
        last: Exception | None = None
        while time.time() < deadline:
            if self.process and self.process.poll() is not None:
                raise RuntimeError(
                    f"sarca exited with code {self.process.returncode}\n{self.tail(60)}"
                )
            try:
                r = httpx.post(
                    url,
                    json={"email": "probe@example.com", "password": "x"},
                    timeout=2.0,
                    verify=False,
                )
                if r.status_code in (200, 401, 403, 422):
                    return
            except Exception as e:  # noqa: BLE001
                last = e
            time.sleep(0.2)
        raise RuntimeError(f"server not ready at {url}: {last}\n{self.tail(60)}")

    def stop(self, timeout: float = 15.0) -> None:
        if not self.process:
            return
        if self.process.poll() is None:
            if IS_WINDOWS:
                # CTRL_BREAK goes to the whole process group created with
                # CREATE_NEW_PROCESS_GROUP; graceful exit lets SQLite flush.
                self.process.send_signal(signal.CTRL_BREAK_EVENT)
            else:
                os.killpg(os.getpgid(self.process.pid), signal.SIGTERM)
            try:
                self.process.wait(timeout=timeout)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=timeout)
        self.process = None

    def restart(self) -> None:
        self.stop()
        self.start()

    # -------------------------------------------------------------------- logs
    def read_log(self) -> str:
        if not self.log_path.is_file():
            return ""
        return self.log_path.read_text(encoding="utf-8", errors="replace")

    def tail(self, lines: int = 40) -> str:
        return "\n".join(self.read_log().splitlines()[-lines:])

    def log_offset(self) -> int:
        return self.log_path.stat().st_size if self.log_path.is_file() else 0

    def log_since(self, offset: int) -> str:
        if not self.log_path.is_file():
            return ""
        with self.log_path.open("rb") as fh:
            fh.seek(offset)
            return fh.read().decode("utf-8", errors="replace")

    def wait_for_log(self, needle: str, timeout: float = 20.0, offset: int = 0) -> str:
        """Block until `needle` shows up in the log after `offset`; return the matching line."""
        deadline = time.time() + timeout
        while time.time() < deadline:
            text = self.log_since(offset)
            for line in text.splitlines():
                if needle in line:
                    return line
            time.sleep(0.2)
        raise AssertionError(
            f"log line containing {needle!r} not found within {timeout}s\n"
            f"--- log tail ---\n{self.tail(40)}"
        )

    def assert_no_log(self, needle: str, offset: int = 0) -> None:
        hits = [ln for ln in self.log_since(offset).splitlines() if needle in ln]
        assert not hits, f"unexpected log lines for {needle!r}:\n" + "\n".join(hits[:10])
