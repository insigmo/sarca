"""E2E harness: a real Sarca server process against a fake (or real) Telegram Bot API.

Default mode is hermetic — every session builds/reuses the release binary, starts a
mock Bot API on an ephemeral port, and runs Sarca with its own WORK_DIR + SQLite.

Run against real Telegram instead:

    SARCA_E2E_TELEGRAM=real \
    SARCA_E2E_BOT_TOKEN=123:AA... \
    SARCA_E2E_CHAT_IDS=-1001234567890 \
    pytest e2e

Run against an already-running server (no process management):

    SARCA_BASE_URL=http://127.0.0.1:8001 SUPERUSER_EMAIL=... SUPERUSER_PASS=... pytest e2e
"""

from __future__ import annotations

import os
import shutil
import tempfile
import uuid
from dataclasses import dataclass
from pathlib import Path

import httpx
import pytest

from helpers.api import SarcaClient, new_bot_token, new_chat_id
from helpers.mock_telegram import MockTelegram
from helpers.server import SarcaServer, build_binary, repo_root

E2E_EMAIL = "e2e@sarca.test"
E2E_PASSWORD = "e2e-password-123"

os.environ["NO_AT_BRIDGE"] = "1"
os.environ["WEBKIT_A11Y_BUS_TYPE"] = "none"

EXTERNAL_BASE_URL = os.environ.get("SARCA_BASE_URL")
TELEGRAM_MODE = os.environ.get("SARCA_E2E_TELEGRAM", "mock").lower()
KEEP_TMP = os.environ.get("SARCA_E2E_KEEP_TMP") == "1"


def pytest_configure(config: pytest.Config) -> None:
    config.addinivalue_line("markers", "mock_only: needs the fake Telegram Bot API")
    config.addinivalue_line("markers", "slow: takes more than a few seconds")


@dataclass
class TelegramBackend:
    """Where Sarca sends its Telegram traffic during this session."""

    base_url: str
    mock: MockTelegram | None
    real_tokens: list[str]
    real_chat_ids: list[int]

    @property
    def is_mock(self) -> bool:
        return self.mock is not None

    def new_token(self) -> str:
        if self.mock:
            return new_bot_token()
        assert self.real_tokens, "SARCA_E2E_BOT_TOKEN is required in real mode"
        return self.real_tokens[0]

    def new_chat_id(self) -> int:
        if self.mock:
            return new_chat_id()
        assert self.real_chat_ids, "SARCA_E2E_CHAT_IDS is required in real mode"
        return self.real_chat_ids[0]


# --------------------------------------------------------------------- session


@pytest.fixture(scope="session")
def e2e_tmp() -> Path:
    root = Path(tempfile.mkdtemp(prefix="sarca-e2e-"))
    yield root
    if not KEEP_TMP:
        shutil.rmtree(root, ignore_errors=True)


@pytest.fixture(scope="session")
def telegram(e2e_tmp: Path) -> TelegramBackend:
    if TELEGRAM_MODE == "real":
        tokens = [t for t in os.environ.get("SARCA_E2E_BOT_TOKEN", "").split(",") if t.strip()]
        chat_ids = [
            int(c) for c in os.environ.get("SARCA_E2E_CHAT_IDS", "").split(",") if c.strip()
        ]
        if not tokens or not chat_ids:
            pytest.exit("real mode needs SARCA_E2E_BOT_TOKEN and SARCA_E2E_CHAT_IDS", returncode=2)
        yield TelegramBackend(
            base_url=os.environ.get("TELEGRAM_API_BASE_URL", "https://api.telegram.org"),
            mock=None,
            real_tokens=tokens,
            real_chat_ids=chat_ids,
        )
        return

    mock = MockTelegram(e2e_tmp / "telegram").start()
    yield TelegramBackend(base_url=mock.base_url, mock=mock, real_tokens=[], real_chat_ids=[])
    mock.stop()


@pytest.fixture(scope="session")
def sarca_binary() -> Path:
    if EXTERNAL_BASE_URL:
        pytest.skip("external server mode")
    return build_binary()


@pytest.fixture(scope="session")
def server(e2e_tmp: Path, telegram: TelegramBackend) -> SarcaServer:
    """The Sarca process under test (skipped when SARCA_BASE_URL points elsewhere)."""
    if EXTERNAL_BASE_URL:
        pytest.skip("external server mode: no managed process")
    # tauri-pilot drives the GUI with webview eval; the shipped CSP
    # (script-src 'self') refuses it, so the server opts into 'unsafe-eval'.
    env_extra: dict[str, str] = {"SARCA_ALLOW_EVAL": "1"}
    if telegram.is_mock:
        # 1 MB chunks keep multi-chunk tests small; the fake Bot API has no flood
        # control, so the proactive 2.2s inter-send gap is pure test latency.
        env_extra.update(
            {
                "TELEGRAM_CHUNK_SIZE_MB": "1",
                "TELEGRAM_VIDEO_CHUNK_SIZE_MB": "1",
                "SARCA_TELEGRAM_PACING_MS": "20",
            }
        )
    srv = SarcaServer(
        root=e2e_tmp / "sarca",
        telegram_base_url=telegram.base_url,
        email=E2E_EMAIL,
        password=E2E_PASSWORD,
        env_extra=env_extra,
    )
    srv.start(build_binary())
    yield srv
    srv.stop()


@pytest.fixture(scope="session")
def base_url(server: SarcaServer | None) -> str:
    if EXTERNAL_BASE_URL:
        return EXTERNAL_BASE_URL.rstrip("/")
    return server.base_url


@pytest.fixture(scope="session")
def credentials() -> tuple[str, str]:
    if EXTERNAL_BASE_URL:
        return (
            os.environ.get("SUPERUSER_EMAIL", E2E_EMAIL),
            os.environ.get("SUPERUSER_PASS", E2E_PASSWORD),
        )
    return (E2E_EMAIL, E2E_PASSWORD)


@pytest.fixture(scope="session")
def sarca(base_url: str, credentials: tuple[str, str]) -> SarcaClient:
    """Superuser API client."""
    client = SarcaClient(base_url)
    client.login(*credentials)
    yield client
    client.close()


@pytest.fixture
def mock(telegram: TelegramBackend) -> MockTelegram:
    """Fake Bot API handle; skips the test when running against real Telegram."""
    if telegram.mock is None:
        pytest.skip("requires the mock Telegram Bot API")
    return telegram.mock


# --------------------------------------------------------------------- storage


@pytest.fixture(scope="session")
def shared_storage(sarca: SarcaClient, telegram: TelegramBackend) -> str:
    """One storage with a bound bot, reused across tests (works in real mode too)."""
    storage = sarca.create_storage(
        name=f"e2e-shared-{uuid.uuid4().hex[:6]}",
        chat_ids=[telegram.new_chat_id()],
        bot_token=telegram.new_token(),
    )
    yield storage["id"]
    sarca.delete_storage(storage["id"])


@pytest.fixture
def storage(sarca: SarcaClient, telegram: TelegramBackend) -> str:
    """A throwaway storage with its own bot (mock mode only: needs fresh chat ids)."""
    if telegram.mock is None:
        pytest.skip("per-test storages need the mock Telegram Bot API")
    st = sarca.create_storage(
        name=f"e2e-{uuid.uuid4().hex[:8]}",
        chat_ids=[telegram.new_chat_id()],
        bot_token=telegram.new_token(),
    )
    yield st["id"]
    sarca.delete_storage(st["id"])


@pytest.fixture
def workdir(server: SarcaServer) -> Path:
    return server.work_dir


# --------------------------------------------------- back-compat (legacy tests)


@pytest.fixture(scope="session")
def wait_for_api(base_url: str) -> None:
    return None


@pytest.fixture(scope="session")
def client(base_url: str, wait_for_api: None) -> httpx.Client:
    with httpx.Client(base_url=base_url, timeout=60.0) as c:
        yield c


@pytest.fixture(scope="session")
def tokens(sarca: SarcaClient) -> dict[str, str]:
    return sarca.login_payload


@pytest.fixture(scope="session")
def auth_headers(tokens: dict[str, str]) -> dict[str, str]:
    return {"Authorization": f"Bearer {tokens['access_token']}"}


@pytest.fixture
def storage_id(sarca: SarcaClient, telegram: TelegramBackend) -> str:
    """Legacy fixture: a storage with channels but *no* bot bound.

    The older suites (test_api / test_features) assert that uploads fail cleanly
    without a storage worker, so this one deliberately stays bot-less.
    """
    if telegram.mock is None:
        pytest.skip("per-test storages need the mock Telegram Bot API")
    st = sarca.create_storage(
        name=f"e2e-legacy-{uuid.uuid4().hex[:8]}", chat_ids=[telegram.new_chat_id()]
    )
    yield st["id"]
    sarca.delete_storage(st["id"])


@pytest.fixture(scope="session")
def server_log_path(server: SarcaServer | None) -> str | None:
    if EXTERNAL_BASE_URL:
        return os.environ.get("SARCA_SERVER_LOG")
    return str(server.log_path)


@pytest.fixture(scope="session")
def repo() -> Path:
    return repo_root()


# --------------------------------------------------------------------------- gui
# Desktop-client fixtures. Everything here is lazy: importing pilot helpers or
# building the client only happens once a `gui`-marked test asks for them.


@pytest.fixture(scope="session")
def gui_available() -> None:
    from helpers.pilot import pilot_binary

    if not os.environ.get("DISPLAY") and not os.environ.get("WAYLAND_DISPLAY"):
        pytest.skip("no display: run under Xvfb (task e2e:gui)")
    if pilot_binary() is None:
        pytest.skip("tauri-pilot not installed (cargo install tauri-pilot-cli)")


@pytest.fixture(scope="session")
def client_binary(gui_available: None) -> Path:
    from helpers.pilot import build_client

    return build_client()


@pytest.fixture(scope="session")
def shim(gui_available: None):
    """Serves client/dist at the debug build's devUrl."""
    from helpers.pilot import ShimServer

    dist = repo_root() / "client" / "dist"
    if not (dist / "index.html").exists():
        pytest.skip("client/dist not built (task client:ui)")
    server = ShimServer(dist).start()
    yield server
    server.stop()


@pytest.fixture
def app(client_binary: Path, shim, tmp_path: Path):
    """A client with an empty HOME: nothing configured, nothing remembered."""
    from helpers.pilot import ClientApp, PilotError

    instance = ClientApp(binary=client_binary, root=tmp_path / "client")

    try:
        instance.start()
    except PilotError:
        instance.close()
        raise
    yield instance
    instance.close()


@pytest.fixture
def signed_in(app, base_url: str, credentials: tuple[str, str]):
    app.connect(base_url)
    app.login(*credentials)
    return app


@pytest.fixture(scope="session")
def gui_app(client_binary: Path, shim, e2e_tmp: Path):
    """One client for the whole session.

    A cold start costs GTK + WebKit + the sync engine's first scan, so paying it
    per scenario would put the flow suite well past a minute of pure startup.
    Scenarios that need a never-configured client keep using `app`/`signed_in`.
    """
    from helpers.pilot import ClientApp, PilotError

    instance = ClientApp(binary=client_binary, root=e2e_tmp / "gui-session-client")
    try:
        instance.start()
    except PilotError:
        instance.close()
        raise
    yield instance
    instance.close()


@pytest.fixture(scope="session")
def gui(gui_app, base_url: str, credentials: tuple[str, str]):
    """The session client, signed in as the superuser."""
    gui_app.connect(base_url)
    gui_app.login(*credentials)
    return gui_app
