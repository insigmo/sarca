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

import base64
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

# Writing through the prototype's value setter and firing `input` is what a
# framework-controlled field listens for; assigning `.value` alone leaves Solid
# holding the old signal and the next render wipes the text.
_FILL_JS = """
(() => {
  const el = document.querySelector(%(selector)s);
  if (!el) return 'missing';
  const value = %(value)s;
  const setter = Object.getOwnPropertyDescriptor(
    Object.getPrototypeOf(el), 'value'
  )?.set;
  if (setter) setter.call(el, value); else el.value = value;
  el.dispatchEvent(new Event('input', { bubbles: true }));
  el.dispatchEvent(new Event('change', { bubbles: true }));
  return el.value === value ? 'ok' : 'lost';
})()
"""


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

    def fill(self, selector: str, value: str, timeout_s: float = 20.0) -> None:
        """Put a value into an input and make sure it stayed there.

        `tauri-pilot fill` races the framework twice over: the node it matched
        can be swapped out while the page is still mounting ("No element"), and
        a value written into the old node is simply lost when Solid re-renders
        the form. Writing through the native setter with an `input` event feeds
        the same signal a keystroke would, and the result is read back, so a
        lost value is retried instead of silently submitting an empty form.
        """
        script = _FILL_JS % {"selector": json.dumps(selector), "value": json.dumps(value)}
        deadline = time.monotonic() + timeout_s
        outcome = "missing"
        while time.monotonic() < deadline:
            outcome = self.eval_js(script)
            if outcome == "ok":
                return
            time.sleep(0.25)
        raise PilotError(f"could not fill {selector}: {outcome}")

    # ------------------------------------------------------------- app flows

    def wait_for_styles(self, timeout_s: float = 15.0) -> None:
        """Block until the page's stylesheet is actually applied.

        Any assertion about rendered geometry is meaningless before CSS lands,
        and a webview that came back from a relaunch can briefly present the
        document with `document.styleSheets` still empty — every element then
        measures at its unstyled defaults. One reload recovers it.
        """
        deadline = time.monotonic() + timeout_s
        reloaded = False
        while time.monotonic() < deadline:
            if self.eval_js("document.styleSheets.length"):
                return
            if not reloaded and time.monotonic() > deadline - timeout_s / 2:
                self.eval_js("window.location.reload(); 'ok'")
                reloaded = True
            time.sleep(0.25)
        raise PilotError("the page never applied its stylesheet")

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

    def goto_until(self, path: str, *fragments: str, tries: int = 4) -> str:
        """`goto` until the URL sticks.

        `location.assign` is a full page load, so one issued while the previous
        one is still settling is simply dropped, and the app is left on whatever
        route it booted into. Re-issuing costs a reload; losing the navigation
        costs the whole test.
        """
        wanted = fragments or (path,)
        last = PilotError("never navigated")
        for _ in range(tries):
            self.goto(path)
            try:
                return self.wait_for_url(*wanted, timeout_s=10.0)
            except PilotError as err:
                last = err
        raise last

    def connect(self, base_url: str) -> None:
        """Fill the local connect shim and land on the server UI."""
        self.wait_for("body[data-shim-ready]")
        self.fill("#serverUrl", base_url)
        self.run("click", "#submit")
        # The Rust side navigates the webview once the server answers.
        self.wait_for_url(base_url.split("://", 1)[-1])
        self.wait_for("input[name=email]", timeout_ms=30000)

    def login(self, email: str, password: str) -> None:
        # requestSubmit() instead of clicking: the button lives inside a SUID
        # wrapper, and the form's own submit handler is what does the work.
        # Both fields are re-read right before submitting, so a value the form
        # dropped on a re-render is refilled instead of sent empty.
        submitted = ""
        for _ in range(5):
            self.fill("input[name=email]", email, timeout_s=30)
            self.fill("input[name=password]", password, timeout_s=30)
            submitted = self.eval_js(
                """
                (() => {
                  const form = document.querySelector('form');
                  const email = document.querySelector('input[name=email]');
                  const password = document.querySelector('input[name=password]');
                  if (!form || !email || !password) return 'missing';
                  if (!email.value || !password.value) return 'empty';
                  form.requestSubmit();
                  return 'ok';
                })()
                """
            )
            if submitted == "ok":
                break
            time.sleep(0.5)
        else:
            raise PilotError(f"login form was not ready to submit: {submitted!r}")

        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            if self.eval_js("localStorage.getItem('access_token') ? '1' : ''") == "1":
                break
            time.sleep(0.25)
        else:
            raise PilotError(f"sign-in never stored a token; page says: {self.page_text()[:200]!r}")

        self.pin_locale()

        # The login route leaves the app on whatever it last had; land on a real
        # page so the settings modal has a shell to open in. A brand-new account
        # with no storages is bounced to the setup wizard, which is fine.
        self.goto_until("/storages", "/storages", "/setup")

    def pin_locale(self, code: str = "en") -> None:
        """Force the UI language, then reload so the choice is applied.

        The UI picks its language from the browser, so every assertion on a
        visible label would otherwise depend on the locale of whatever machine
        runs the suite.
        """
        current = self.eval_js("localStorage.getItem('sarca.locale')")
        if current == code:
            return
        self.eval_js(f"localStorage.setItem('sarca.locale', {code!r}); 'ok'")
        self.eval_js("window.location.reload(); 'ok'")
        time.sleep(1.0)

    def page_text(self) -> str:
        return self.eval_js("(document.body.textContent || '').trim()") or ""

    def open_storage(self, storage_id: str) -> None:
        """Land in a storage's file browser.

        Settings only offers the Sync tab while a storage is open — auto-upload
        binds a local folder to *this* storage.
        """
        self.goto_until(f"/storages/{storage_id}/files/", f"/storages/{storage_id}/files")
        self.wait_for(".files-page, .fs-list, .fs-grid, .files-empty", timeout_ms=20000)

    def open_sync_settings(self) -> None:
        """Open Settings on the Sync tab through the client's own deep link."""
        self.open_settings("sync")

    def open_settings(self, tab: str = "general") -> None:
        """Open Settings on `tab` through the client's own deep link.

        The native shell listens for this event (see `common/nativeClient.js`),
        which is a far steadier way in than hunting the gear button across the
        desktop sidebar and the mobile drawer.
        """
        self.eval_js(
            "window.dispatchEvent(new CustomEvent('sarca-open-settings',"
            f" {{ detail: {{ tab: {tab!r} }} }})); 'ok'"
        )
        self.wait_for(".settings-modal", timeout_ms=10000)

    def close_settings(self) -> None:
        self.click('[aria-label="Close settings"]', required=False)
        self.wait_gone(".settings-modal", timeout_s=10)

    # -------------------------------------------------------------- DOM basics

    #: Everything the app makes clickable. `li` covers SUID's menu items, which
    #: render as `<li role="menuitem">` in a portal outside the modal subtree.
    #: `.MuiListItemButton-root` covers the folder picker's rows: SUID draws
    #: them as bare `<div>`s and only adds `role="button"` when there is an
    #: `href`, so nothing else here would match them.
    CLICKABLE = (
        "button, a, li, [role=menuitem], [role=button], .MuiListItemButton-root,"
        " .files-sidebar__item, .settings-nav__item, .theme-picker__option,"
        " .files-view-toggle__btn, .storage-card, .files-breadcrumb__crumb"
    )

    def click(self, selector: str, *, required: bool = True, timeout_s: float = 15.0) -> bool:
        """Click the first *visible* match.

        The sidebar exists twice (desktop rail + mobile drawer) and only one of
        them is on screen, so a blind `querySelector` picks the wrong copy about
        half the time. `offsetParent` is what tells them apart.
        """
        script = """
        (() => {
          for (const el of document.querySelectorAll(%s)) {
            if (el.offsetParent === null && getComputedStyle(el).position !== 'fixed') continue;
            if (el.disabled) return 'disabled';
            el.click();
            return 'ok';
          }
          return 'missing';
        })()
        """ % json.dumps(selector)
        return self._poll_click(script, f"click {selector}", required, timeout_s)

    def click_text(
        self,
        text: str,
        *,
        scope: str | None = None,
        exact: bool = True,
        required: bool = True,
        timeout_s: float = 15.0,
    ) -> bool:
        """Click the visible control whose own label reads `text`.

        Labels are what the product promises; class names are an implementation
        detail that a restyle is free to change. Matching the deepest node that
        carries the text keeps a wrapper `<div>` from swallowing the click.
        """
        script = """
        (() => {
          const wanted = %(text)s, exact = %(exact)s;
          const root = %(scope)s ? document.querySelector(%(scope)s) : document;
          if (!root) return 'missing';
          const hit = [];
          for (const el of root.querySelectorAll(%(clickable)s)) {
            if (el.offsetParent === null && getComputedStyle(el).position !== 'fixed') continue;
            const t = (el.textContent || '').trim();
            if (exact ? t === wanted : t.includes(wanted)) hit.push(el);
          }
          if (!hit.length) return 'missing';
          // Innermost match: a menu item inside a list inside a paper.
          const el = hit.reduce((a, b) => (a.contains(b) ? b : a));
          if (el.disabled || el.getAttribute('aria-disabled') === 'true') return 'disabled';
          el.click();
          return 'ok';
        })()
        """ % {
            "text": json.dumps(text),
            "exact": "true" if exact else "false",
            "scope": json.dumps(scope) if scope else "null",
            "clickable": json.dumps(self.CLICKABLE),
        }
        return self._poll_click(script, f"click {text!r}", required, timeout_s)

    def _poll_click(self, script: str, what: str, required: bool, timeout_s: float) -> bool:
        deadline = time.monotonic() + timeout_s
        outcome = "missing"
        while time.monotonic() < deadline:
            outcome = self.eval_js(script)
            if outcome == "ok":
                return True
            time.sleep(0.25)
        if required:
            raise PilotError(f"could not {what}: {outcome}")
        return False

    def wait_gone(self, selector: str, timeout_s: float = 15.0) -> None:
        script = f"document.querySelector({json.dumps(selector)}) ? 'here' : 'gone'"
        deadline = time.monotonic() + timeout_s
        while time.monotonic() < deadline:
            if self.eval_js(script) == "gone":
                return
            time.sleep(0.2)
        raise PilotError(f"{selector} never disappeared")

    def wait_for_text(self, text: str, timeout_s: float = 20.0) -> None:
        deadline = time.monotonic() + timeout_s
        while time.monotonic() < deadline:
            if text in self.page_text():
                return
            time.sleep(0.25)
        raise PilotError(f"page never showed {text!r}; last text: {self.page_text()[:300]!r}")

    def wait_for_alert(self, fragment: str, timeout_s: float = 40.0) -> None:
        """Wait for a toast to say `fragment`.

        Alerts auto-dismiss, so this polls the whole document rather than a
        snapshot — the toast may appear and go between two reads otherwise.
        """
        self.wait_for_text(fragment, timeout_s=timeout_s)

    def press(self, code: str, *, ctrl: bool = False, shift: bool = False) -> None:
        """Fire a keydown on window, where the Files page installs its handler."""
        key = {"KeyA": "a", "KeyC": "c", "KeyX": "x", "KeyV": "v"}.get(code, code)
        init = json.dumps(
            {
                "code": code,
                "key": key,
                "ctrlKey": ctrl,
                "shiftKey": shift,
                "bubbles": True,
                "cancelable": True,
            }
        )
        self.eval_js(f"window.dispatchEvent(new KeyboardEvent('keydown', {init})); 'ok'")

    def press_in(self, selector: str, key: str) -> None:
        """Fire a keydown on an element, for handlers bound to the field itself."""
        outcome = self.eval_js(
            """
            (() => {
              const el = document.querySelector(%s);
              if (!el) return 'missing';
              el.dispatchEvent(new KeyboardEvent('keydown', {
                key: %s, bubbles: true, cancelable: true,
              }));
              return 'ok';
            })()
            """
            % (json.dumps(selector), json.dumps(key))
        )
        if outcome != "ok":
            raise PilotError(f"could not press {key} in {selector}: {outcome}")

    def stub_prompt(self, value: str | None) -> None:
        """Answer the next `window.prompt` with `value` (None = cancel).

        Rename is a native prompt; a headless run has nobody to type into it.
        """
        self.eval_js(f"window.prompt = () => {json.dumps(value)}; 'ok'")

    def local_storage(self, key: str) -> str | None:
        # A missing key comes back over the bridge as the word "undefined", which
        # is truthy in Python, so normalise every flavour of nothing to None.
        raw = self.eval_js(f"localStorage.getItem({key!r})")
        return None if raw in (None, "", "null", "undefined") else raw

    # ------------------------------------------------------------- file browser

    ROW = ".fs-list-item, .fs-grid-item"
    ROW_NAME = ".fs-list-item__name, .fs-grid-item__name"

    def rows(self) -> list[str]:
        """Names currently drawn in the browser, in on-screen order."""
        return (
            self.eval_js(
                f"[...document.querySelectorAll({json.dumps(self.ROW_NAME)})]"
                ".map((n) => (n.textContent || '').trim())"
            )
            or []
        )

    def wait_for_row(self, name: str, timeout_s: float = 60.0) -> None:
        deadline = time.monotonic() + timeout_s
        seen: list[str] = []
        while time.monotonic() < deadline:
            seen = self.rows()
            if name in seen:
                return
            time.sleep(0.5)
        raise PilotError(f"{name!r} never appeared; rows: {seen}")

    def wait_row_gone(self, name: str, timeout_s: float = 60.0) -> None:
        deadline = time.monotonic() + timeout_s
        seen: list[str] = []
        while time.monotonic() < deadline:
            seen = self.rows()
            if name not in seen:
                return
            time.sleep(0.5)
        raise PilotError(f"{name!r} never went away; rows: {seen}")

    def _row_js(self, name: str, body: str) -> str:
        """Run `body` (with `row` bound) against the row named `name`."""
        return """
        (() => {
          const wanted = %(name)s;
          for (const row of document.querySelectorAll(%(row)s)) {
            const label = row.querySelector(%(rowName)s);
            if (!label || (label.textContent || '').trim() !== wanted) continue;
            %(body)s
          }
          return 'missing';
        })()
        """ % {
            "name": json.dumps(name),
            "row": json.dumps(self.ROW),
            "rowName": json.dumps(self.ROW_NAME),
            "body": body,
        }

    def click_row(self, name: str, *, ctrl: bool = False, shift: bool = False) -> None:
        """Click a row the way a mouse would, modifiers included.

        On a desktop-sized window a single click *selects*; the handler reads
        `ctrlKey`/`shiftKey` straight off the event, so a bare `.click()` can
        only ever mean "select this one".
        """
        body = """
            const box = row.getBoundingClientRect();
            const opts = {
              bubbles: true, cancelable: true, view: window, button: 0,
              clientX: box.left + box.width / 2, clientY: box.top + box.height / 2,
              ctrlKey: %(ctrl)s, shiftKey: %(shift)s,
            };
            row.dispatchEvent(new MouseEvent('mousedown', opts));
            row.dispatchEvent(new MouseEvent('mouseup', opts));
            row.dispatchEvent(new MouseEvent('click', opts));
            return 'ok';
        """ % {"ctrl": str(ctrl).lower(), "shift": str(shift).lower()}
        self._poll_click(self._row_js(name, body), f"click row {name!r}", True, 30.0)

    def open_row(self, name: str) -> None:
        """Double-click a row: enter a folder, or open a file in the viewer.

        Single click is selection on the desktop layout, so opening is always
        the double click — the same thing a file manager does.
        """
        body = """
            const box = row.getBoundingClientRect();
            const opts = {
              bubbles: true, cancelable: true, view: window, button: 0,
              clientX: box.left + box.width / 2, clientY: box.top + box.height / 2,
            };
            row.dispatchEvent(new MouseEvent('mousedown', opts));
            row.dispatchEvent(new MouseEvent('mouseup', opts));
            row.dispatchEvent(new MouseEvent('click', opts));
            row.dispatchEvent(new MouseEvent('dblclick', opts));
            return 'ok';
        """
        self._poll_click(self._row_js(name, body), f"open row {name!r}", True, 30.0)

    def row_action(self, name: str, label: str) -> None:
        """Right-click a row and pick `label` from its context menu."""
        body = """
            const box = row.getBoundingClientRect();
            row.dispatchEvent(new MouseEvent('contextmenu', {
              bubbles: true, cancelable: true, view: window,
              clientX: box.left + 8, clientY: box.top + 8,
            }));
            return 'ok';
        """
        self._poll_click(self._row_js(name, body), f"open menu for {name!r}", True, 30.0)
        # SUID renders the menu in a portal, so it is not inside the row.
        self.click_text(label, timeout_s=10)

    def toggle_star(self, name: str) -> None:
        body = """
            const star = row.querySelector('.fs-list-item__star, .fs-grid-item__star');
            if (!star) return 'no-star';
            // The class sits on a wrapper; the handler lives on the button inside
            // it. Clicking the wrapper only bubbles up and selects the row.
            const btn = star.querySelector('button') || star;
            btn.click();
            return 'ok';
        """
        self._poll_click(self._row_js(name, body), f"star {name!r}", True, 30.0)

    def is_starred(self, name: str) -> bool:
        body = """
            return row.querySelector('.fs-star-icon--active') ? 'yes' : 'no';
        """
        return self.eval_js(self._row_js(name, body)) == "yes"

    def sidebar_click(self, label: str, timeout_s: float = 25.0) -> None:
        """Click a sidebar entry by its aria-label, drawer or rail.

        Below 841px the desktop rail is `display: none` and the only copy of the
        sidebar is a drawer that starts closed, so the entry is in the DOM but
        unclickable. Rather than measure the window, try the entry and fall back
        to the burger — that also covers a rail that is still mounting.
        """
        selector = f'.files-sidebar__item[aria-label="{label}"]'
        deadline = time.monotonic() + timeout_s
        while time.monotonic() < deadline:
            if self.click(selector, required=False, timeout_s=1.0):
                return
            self.click(".files-page__nav-toggle", required=False, timeout_s=1.0)
        raise PilotError(f"the sidebar never offered {label!r}")

    def sidebar_overflow_click(self, label: str, timeout_s: float = 25.0) -> None:
        """Click a sidebar action that lives behind the overflow ("More options") menu.

        Log out and Disconnect were moved out of the always-visible footer so a
        stray click cannot end the session; reaching them now takes two steps.
        """
        deadline = time.monotonic() + timeout_s
        while time.monotonic() < deadline:
            self.sidebar_click("More options", timeout_s=max(1.0, deadline - time.monotonic()))
            if self.click_text(label, required=False, timeout_s=2.0):
                return
        raise PilotError(f"the sidebar overflow menu never offered {label!r}")

    def confirm_dialog(self, timeout_s: float = 10.0) -> None:
        """Accept the ActionConfirmDialog that guards destructive actions."""
        if not self.click_text("Confirm", required=False, timeout_s=timeout_s):
            raise PilotError("no confirmation dialog appeared")

    def open_section(self, label: str) -> None:
        """Switch the file browser to All files / Favorites / Recent / Shared / Trash."""
        self.sidebar_click(label)

    def set_view(self, mode: str) -> None:
        """`mode` is 'list' or 'tiles'."""
        label = "List view" if mode == "list" else "Tiles view"
        self.click(f'.files-view-toggle__btn[aria-label="{label}"]')

    SEARCH_INPUT = ".header-search input"

    def search(self, text: str) -> None:
        """Type into the header pill and run the search.

        The field only asks the server on Enter, so filling it alone changes
        nothing a user would see.
        """
        self.fill(self.SEARCH_INPUT, text)
        self.press_in(self.SEARCH_INPUT, "Enter")

    def upload_bytes(self, files: list[tuple[str, bytes]]) -> None:
        """Hand files to the page's hidden `<input type=file>`.

        This is the same code path a picked file takes: the input's `change`
        handler is what enqueues the upload. There is no way to drive the native
        picker itself from a test, and faking a drop event would exercise a
        different handler.
        """
        payload = [
            {"name": name, "b64": base64.b64encode(data).decode()} for name, data in files
        ]
        outcome = self.eval_js(
            """
            (() => {
              const specs = %s;
              const input = document.querySelector(
                'input[type=file]:not([webkitdirectory])'
              );
              if (!input) return 'no-input';
              if (typeof DataTransfer === 'undefined') return 'no-datatransfer';
              const dt = new DataTransfer();
              for (const spec of specs) {
                const bin = atob(spec.b64);
                const bytes = new Uint8Array(bin.length);
                for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
                dt.items.add(new File([bytes], spec.name, { type: 'application/octet-stream' }));
              }
              input.files = dt.files;
              if (input.files.length !== specs.length) return 'not-set';
              input.dispatchEvent(new Event('change', { bubbles: true }));
              return 'ok';
            })()
            """
            % json.dumps(payload)
        )
        if outcome != "ok":
            raise PilotError(f"could not hand files to the upload input: {outcome}")

    def reset(self, storage_id: str | None = None) -> None:
        """Put the app back in a known state between scenarios.

        The session-scoped client keeps its webview, so a modal left open or a
        remembered view mode would leak into the next test.
        """
        self.eval_js(
            """
            (() => {
              for (const key of Object.keys(localStorage)) {
                if (key.startsWith('sarca.fsLayerCache.')
                  || key === 'sarca.filesViewMode'
                  || key === 'sarca.uploadMgr.size') localStorage.removeItem(key);
              }
              return 'ok';
            })()
            """
        )
        if storage_id:
            self.open_storage(storage_id)
        else:
            self.goto_until("/storages")
