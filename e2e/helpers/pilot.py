"""Drive the real Tauri desktop client through tauri-pilot.

The client is built once with `--features pilot`, which embeds
`tauri-plugin-pilot` (debug builds only). The plugin listens on
`$XDG_RUNTIME_DIR/tauri-pilot-app.sarca.client.sock`, so pointing
XDG_RUNTIME_DIR at a per-test directory gives every app instance its own
control socket with no cross-talk.

HOME and the XDG dirs are redirected too: that is what makes "a client that has
never been configured" reproducible, since both the Rust session file and the
WebKit localStorage live under them.
"""

from __future__ import annotations

import functools
import json
import os
import shutil
import signal
import subprocess
import tempfile
import threading
import time
from dataclasses import dataclass, field
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

APP_IDENTIFIER = "app.sarca.client"
BUILD_TIMEOUT_S = 1800
# Cold start pays for GTK + WebKit + the sync engine's first scan.
START_TIMEOUT_S = 60.0


class PilotError(RuntimeError):
    """A tauri-pilot command failed."""


class ShimServer:
    """Serve `client/dist` at the app's `devUrl`.

    A debug build of the client loads `build.devUrl` (http://localhost:1420),
    which normally comes from `pnpm dev`. Serving the already-built shim there
    keeps the test on the same binary layout as `task client:build` without
    dragging a Vite process into the suite.
    """

    def __init__(self, dist: Path, port: int = 1420) -> None:
        self.dist = dist
        handler = functools.partial(SimpleHTTPRequestHandler, directory=str(dist))
        self._server = ThreadingHTTPServer(("127.0.0.1", port), handler)
        self._server.daemon_threads = True
        self._thread = threading.Thread(target=self._server.serve_forever, daemon=True)

    def start(self) -> ShimServer:
        self._thread.start()
        return self

    def stop(self) -> None:
        self._server.shutdown()
        self._server.server_close()
        self._thread.join(timeout=5)


def pilot_binary() -> str | None:
    return shutil.which("tauri-pilot")


def repo_root() -> Path:
    return Path(__file__).resolve().parents[2]


def build_client(*, skip: bool = False) -> Path:
    """Build (or locate) the debug client with the pilot plugin compiled in."""
    binary = repo_root() / "target" / "debug" / "sarca-client"
    if skip or os.environ.get("SARCA_SKIP_BUILD") == "1":
        if binary.exists():
            return binary
    subprocess.run(
        ["cargo", "build", "-p", "sarca-client", "--features", "pilot"],
        cwd=repo_root(),
        check=True,
        timeout=BUILD_TIMEOUT_S,
    )
    if not binary.exists():
        raise PilotError(f"client binary missing after build: {binary}")
    return binary


@dataclass
class ClientApp:
    """One running desktop client, driven over its pilot socket."""

    binary: Path
    root: Path
    log_path: Path = field(init=False)
    proc: subprocess.Popen | None = field(default=None, init=False)

    def __post_init__(self) -> None:
        self.root.mkdir(parents=True, exist_ok=True)
        self.log_path = self.root / "client.log"
        # Unix socket paths cap at ~108 bytes, and pytest's tmp_path is already
        # deep, so the runtime dir has to live somewhere short.
        self._runtime_dir = Path(tempfile.mkdtemp(prefix="sarca-pilot-", dir="/tmp"))

    # ---------------------------------------------------------------- process

    @property
    def home(self) -> Path:
        return self.root / "home"

    @property
    def runtime_dir(self) -> Path:
        return self._runtime_dir

    @property
    def socket(self) -> Path:
        return self.runtime_dir / f"tauri-pilot-{APP_IDENTIFIER}.sock"

    def _env(self) -> dict[str, str]:
        env = dict(os.environ)
        home = self.home
        for sub in ("", ".local/share", ".config", ".cache"):
            (home / sub).mkdir(parents=True, exist_ok=True)
        self.runtime_dir.mkdir(parents=True, exist_ok=True)
        # 0700: the plugin refuses to bind a world-accessible runtime dir.
        self.runtime_dir.chmod(0o700)
        env.update(
            {
                "HOME": str(home),
                "XDG_DATA_HOME": str(home / ".local/share"),
                "XDG_CONFIG_HOME": str(home / ".config"),
                "XDG_CACHE_HOME": str(home / ".cache"),
                "XDG_RUNTIME_DIR": str(self.runtime_dir),
                # WebKit's sandbox needs more setup than a test container has, and
                # DMABUF rendering fails on virtual displays (Xvfb in CI).
                "WEBKIT_DISABLE_COMPOSITING_MODE": "1",
                "WEBKIT_DISABLE_DMABUF_RENDERER": "1",
            }
        )
        env.pop("SARCA_BASE_URL", None)
        # Let dbus-run-session hand out its own bus instead of inheriting the
        # desktop one (see `_argv`).
        env.pop("DBUS_SESSION_BUS_ADDRESS", None)
        return env

    def _argv(self) -> list[str]:
        """Launch under a private DBus session when one can be had.

        `tauri-plugin-single-instance` claims a well-known name on the session
        bus, so a developer's installed Sarca would swallow the test launch and
        the new process would exit 0 before ever binding a pilot socket.
        """
        dbus = shutil.which("dbus-run-session")
        if dbus:
            return [dbus, "--", str(self.binary)]
        return [str(self.binary)]

    def start(self) -> ClientApp:
        if self.proc is not None:
            raise PilotError("client already running")
        if self.socket.exists():
            self.socket.unlink()
        log = self.log_path.open("ab")
        self.proc = subprocess.Popen(
            self._argv(),
            cwd=str(repo_root()),
            env=self._env(),
            stdout=log,
            stderr=subprocess.STDOUT,
            # Own process group so stop() can take the dbus wrapper and the app
            # down together.
            start_new_session=True,
        )
        self._wait_for_socket()
        self._wait_for_window()
        return self

    def _wait_for_socket(self) -> None:
        deadline = time.monotonic() + START_TIMEOUT_S
        while time.monotonic() < deadline:
            if self.proc is not None and self.proc.poll() is not None:
                raise PilotError(
                    f"client exited with {self.proc.returncode}\n{self.tail_log()}"
                )
            if self.socket.exists():
                try:
                    self.run("ping", timeout=10)
                    return
                except PilotError:
                    pass
            time.sleep(0.2)
        raise PilotError(f"pilot socket never appeared at {self.socket}\n{self.tail_log()}")

    def _wait_for_window(self) -> None:
        """The plugin binds its socket during setup, before Tauri builds the
        window from the config, so `ping` succeeding does not mean there is a
        webview to drive yet."""
        deadline = time.monotonic() + START_TIMEOUT_S
        while time.monotonic() < deadline:
            try:
                payload = self.json("windows", timeout=10) or {}
            except PilotError:
                payload = {}
            if any(w.get("label") == "main" for w in payload.get("windows", [])):
                return
            time.sleep(0.2)
        raise PilotError(f"window 'main' never appeared\n{self.tail_log()}")

    def stop(self) -> None:
        if self.proc is None:
            return
        try:
            pgid = os.getpgid(self.proc.pid)
        except ProcessLookupError:
            self.proc.wait(timeout=5)
            self.proc = None
            return
        os.killpg(pgid, signal.SIGTERM)
        try:
            self.proc.wait(timeout=15)
        except subprocess.TimeoutExpired:
            os.killpg(pgid, signal.SIGKILL)
            self.proc.wait(timeout=10)
        self.proc = None

    def close(self) -> None:
        self.stop()
        shutil.rmtree(self._runtime_dir, ignore_errors=True)

    def restart(self) -> ClientApp:
        """Stop and start again, keeping HOME — i.e. a real app relaunch."""
        self.stop()
        # The socket guard unlinks on drop, but a killed process leaves it behind.
        if self.socket.exists():
            self.socket.unlink()
        return self.start()

    def tail_log(self, lines: int = 40) -> str:
        if not self.log_path.exists():
            return "<no client log>"
        text = self.log_path.read_text(errors="replace").splitlines()
        return "\n".join(text[-lines:])

    # ------------------------------------------------------------------ pilot

    def run(self, *args: str, timeout: float = 30.0) -> str:
        binary = pilot_binary()
        if binary is None:
            raise PilotError("tauri-pilot is not on PATH")
        proc = subprocess.run(
            [binary, "--socket", str(self.socket), "--window", "main", *args],
            capture_output=True,
            text=True,
            timeout=timeout,
        )
        if proc.returncode != 0:
            raise PilotError(
                f"tauri-pilot {' '.join(args)} failed ({proc.returncode}): "
                f"{proc.stderr.strip() or proc.stdout.strip()}"
            )
        return proc.stdout.strip()

    def json(self, *args: str, timeout: float = 30.0):
        out = self.run(*args, "--json", timeout=timeout)
        if not out:
            return None
        return json.loads(out)

    def eval_js(self, script: str, timeout: float = 30.0):
        """Evaluate JS and return the decoded result.

        The script is passed on stdin so quoting never has to survive a shell.
        """
        binary = pilot_binary()
        if binary is None:
            raise PilotError("tauri-pilot is not on PATH")
        proc = subprocess.run(
            [binary, "--socket", str(self.socket), "--window", "main", "eval", "--json", "-"],
            input=script,
            capture_output=True,
            text=True,
            timeout=timeout,
        )
        if proc.returncode != 0:
            raise PilotError(
                f"eval failed ({proc.returncode}): {proc.stderr.strip() or proc.stdout.strip()}"
            )
        payload = json.loads(proc.stdout) if proc.stdout.strip() else None
        if isinstance(payload, dict) and "result" in payload:
            return payload["result"]
        return payload

    def wait_for(self, selector: str, timeout_ms: int = 15000) -> None:
        self.run("wait", selector, "--timeout", str(timeout_ms), timeout=timeout_ms / 1000 + 10)

    def url(self) -> str:
        return self.run("url")

    # ------------------------------------------------------------- app flows

    def wait_for_url(self, *fragments: str, timeout_s: float = 30.0) -> str:
        deadline = time.monotonic() + timeout_s
        last = ""
        while time.monotonic() < deadline:
            try:
                last = self.url()
            except PilotError:
                # The bridge is gone for a moment during a navigation.
                last = ""
            if any(f in last for f in fragments):
                return last
            time.sleep(0.25)
        raise PilotError(f"url never contained any of {fragments} (last: {last!r})")

    def goto(self, path: str) -> None:
        self.eval_js(f"window.location.assign({path!r}); 'ok'")

    def connect(self, base_url: str) -> None:
        """Fill the local connect shim and land on the server UI."""
        self.wait_for("body[data-shim-ready]")
        self.run("fill", "#serverUrl", base_url)
        self.run("click", "#submit")
        # The Rust side navigates the webview once the server answers.
        self.wait_for_url(base_url.split("://", 1)[-1])
        self.wait_for("input[name=email]", timeout_ms=30000)

    def login(self, email: str, password: str) -> None:
        self.run("fill", "input[name=email]", email)
        self.run("fill", "input[name=password]", password)
        # requestSubmit() instead of clicking: the button lives inside a SUID
        # wrapper, and the form's own submit handler is what does the work.
        self.eval_js("document.querySelector('form').requestSubmit(); 'ok'")

        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            if self.eval_js("localStorage.getItem('access_token') ? '1' : ''") == "1":
                break
            time.sleep(0.25)
        else:
            raise PilotError(f"sign-in never stored a token; page says: {self.page_text()[:200]!r}")

        # The login route leaves the app on whatever it last had; land on a real
        # page so the settings modal has a shell to open in. A brand-new account
        # with no storages is bounced to the setup wizard, which is fine.
        self.goto("/storages")
        self.wait_for_url("/storages", "/setup")

    def page_text(self) -> str:
        return self.eval_js("(document.body.textContent || '').trim()") or ""

    def open_storage(self, storage_id: str) -> None:
        """Land in a storage's file browser.

        Settings only offers the Sync tab while a storage is open — auto-upload
        binds a local folder to *this* storage.
        """
        self.goto(f"/storages/{storage_id}/files/")
        self.wait_for_url(f"/storages/{storage_id}/files")
        self.wait_for(".files-page, .fs-list, .fs-grid, .files-empty", timeout_ms=20000)

    def open_sync_settings(self) -> None:
        """Open Settings on the Sync tab through the client's own deep link."""
        self.eval_js(
            "window.dispatchEvent(new CustomEvent('sarca-open-settings',"
            " { detail: { tab: 'sync' } })); 'ok'"
        )
