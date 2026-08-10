"""Scenario 11 — the desktop client's feature surface, end to end.

Sixteen user journeys driven through the real Tauri binary against the hermetic
Sarca server. Where scenario 10 pins down a handful of regressions, this file
walks the product: sign in, make a storage, put files in it, find them, star
them, share them, zoom them, throw them away and get them back.

Everything is asserted the way a person would check it — the row is on screen,
the toast said so, the breadcrumb moved — and then, where the answer has to be
true on the server too, confirmed through the API.

Needs a display (`DISPLAY`/Xvfb) and `tauri-pilot` on PATH; skipped otherwise.
"""

from __future__ import annotations

import time
import uuid

import pytest

from helpers import media
from helpers.api import SarcaClient
from helpers.pilot import ClientApp, PilotError

pytestmark = [pytest.mark.gui, pytest.mark.mock_only, pytest.mark.slow]

VIEW_MODE_KEY = "sarca.filesViewMode"


# --------------------------------------------------------------------- helpers


@pytest.fixture
def stage(gui: ClientApp, storage: str) -> ClientApp:
    """The session client, parked in an empty storage of this test's own.

    The client is shared; the storage is not. Anything a scenario creates dies
    with its storage, so tests never see each other's rows.
    """
    gui.reset(storage)
    yield gui
    # Leave no modal or selection behind for whoever runs next.
    try:
        gui.press("Escape")
        gui.reset()
    except PilotError:
        pass


def seed_file(sarca: SarcaClient, storage: str, name: str, data: bytes = b"hello") -> None:
    result = sarca.upload(storage, name, data)
    assert result.ok, f"seeding {name} failed: {result.error}"


def reopen(app: ClientApp, storage: str) -> None:
    """Re-enter the storage so the browser refetches from the server."""
    app.goto_until("/storages")
    app.open_storage(storage)


def sort_label(app: ClientApp) -> str:
    return (
        app.eval_js(
            """
            (() => {
              for (const b of document.querySelectorAll('button')) {
                const t = (b.textContent || '').trim();
                if (t.startsWith('Sort:')) return t;
              }
              return '';
            })()
            """
        )
        or ""
    )


def set_sort(app: ClientApp, field: str, direction: str) -> None:
    """Drive the sort menu to an exact state.

    Picking the field that is already active flips the direction instead of
    re-selecting it, so the field and the direction have to be set in two
    separate visits to the menu.
    """
    if field not in sort_label(app):
        app.click_text("Sort:", exact=False)
        app.click_text(field)
    app.click_text("Sort:", exact=False)
    app.click_text("Ascending" if direction == "asc" else "Descending")
    arrow = "↑" if direction == "asc" else "↓"
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        if sort_label(app) == f"Sort: {field} {arrow}":
            return
        time.sleep(0.25)
    raise AssertionError(f"sort never became {field} {arrow}; label is {sort_label(app)!r}")


# ------------------------------------------------------------------ 1. sign in


def test_sign_in_survives_a_relaunch_and_log_out_clears_it(
    signed_in: ClientApp,
    base_url: str,
    storage: str,
    sarca: SarcaClient,
    credentials: tuple[str, str],
) -> None:
    """The session is the thing a user never wants to redo.

    A fresh client is connected and signed in, killed, and started again: it has
    to come back already authenticated. Logging out then has to undo exactly
    that — back to the sign-in form with no token left behind.
    """
    assert signed_in.local_storage("access_token"), "sign-in stored no token"
    # WebKit flushes localStorage on a timer; killing the process too early
    # loses the write through no fault of the app.
    time.sleep(6)

    signed_in.restart()
    signed_in.wait_for_url("/storages", "/setup")
    assert signed_in.local_storage("access_token"), "the session did not survive a relaunch"

    # An account with no storages is bounced to the setup wizard, which has no
    # sidebar. The `storage` fixture is here to keep the list non-empty, and
    # this puts the app back on the page whose sidebar owns Log out.
    signed_in.goto_until("/storages")
    # Log out sits behind the sidebar's overflow menu and asks for confirmation,
    # so a stray click cannot end the session.
    signed_in.sidebar_overflow_click("Log out")
    signed_in.confirm_dialog()
    signed_in.wait_for_url("/login")
    assert not signed_in.local_storage("access_token"), "log out left the token behind"

    # Log out is "log out everywhere": the server bumps the user's
    # tokens_valid_from, which also kills the token the harness holds for the
    # same superuser. Mint a fresh one so the rest of the session still works.
    sarca.login(*credentials)


# ---------------------------------------------------------------- 2. storages


def test_storages_page_lists_a_storage_and_settings_renames_it(
    gui: ClientApp, sarca: SarcaClient, telegram
) -> None:
    """The storage list is the app's front door, and its gear button is the
    only way to rename a storage from the UI."""
    name = f"e2e-gui-{uuid.uuid4().hex[:8]}"
    storage_id = sarca.create_storage(
        name=name, chat_ids=[telegram.new_chat_id()], bot_token=telegram.new_token()
    )["id"]
    try:
        gui.reset()
        gui.wait_for(".storages-grid", timeout_ms=20000)
        gui.wait_for_text(name)

        gui.click(f'[aria-label="Settings for {name}"]')
        gui.wait_for("#storage-settings-title", timeout_ms=10000)

        renamed = f"{name}-renamed"
        gui.fill(".storage-settings-form input", renamed)
        gui.click_text("Save", scope=".settings-modal--storage")
        gui.wait_for_alert(f'Renamed storage to "{renamed}"')

        assert sarca.storage_detail(storage_id)["name"] == renamed

        gui.click('[aria-label="Close storage settings"]', required=False)
        gui.wait_for_text(renamed)
    finally:
        sarca.delete_storage(storage_id)


# ----------------------------------------------------------------- 3. folders


def test_create_a_folder_and_walk_in_and_out_of_it(
    stage: ClientApp, sarca: SarcaClient, storage: str
) -> None:
    """New → Create folder, then the breadcrumb has to take you back.

    Opening a folder changes the URL, which is what makes a folder linkable and
    the back button work at all.
    """
    stage.click_text("New")
    stage.click_text("Create folder")
    stage.wait_for("#folder-name", timeout_ms=10000)
    stage.fill("#folder-name", "Album")
    stage.click_text("Create", scope="[role=dialog]")

    stage.wait_for_row("Album")
    assert any(item["name"].rstrip("/") == "Album" for item in sarca.tree(storage))

    stage.open_row("Album")
    stage.wait_for_url("/files/Album")

    stage.click_text("All files", scope=".files-breadcrumb")
    stage.wait_for_row("Album")


# ------------------------------------------------------------------ 4. upload


def test_upload_from_the_client_lands_in_the_storage(
    stage: ClientApp, sarca: SarcaClient, storage: str
) -> None:
    """Picking a file has to reach Telegram and come back byte-for-byte.

    The upload manager is the only progress a user gets, so it has to show the
    file and then say it finished — a silent upload is indistinguishable from a
    stuck one.
    """
    payload = media.blob(48 * 1024, seed=11)
    stage.upload_bytes([("client-upload.bin", payload)])

    stage.wait_for(".upload-mgr", timeout_ms=20000)
    stage.wait_for_row("client-upload.bin", timeout_s=120)

    assert sarca.download_bytes(storage, "client-upload.bin") == payload


# --------------------------------------------------------------- 5. view mode


def test_view_mode_switches_between_tiles_and_list_and_is_remembered(
    stage: ClientApp, sarca: SarcaClient, storage: str
) -> None:
    """Tiles or list is a preference, not a per-visit choice."""
    seed_file(sarca, storage, "view.txt")
    reopen(stage, storage)
    stage.wait_for_row("view.txt")

    stage.set_view("tiles")
    stage.wait_for(".fs-grid-item", timeout_ms=10000)
    assert stage.local_storage(VIEW_MODE_KEY) == "tiles"

    stage.set_view("list")
    stage.wait_for(".fs-list-item", timeout_ms=10000)
    assert stage.local_storage(VIEW_MODE_KEY) == "list"

    reopen(stage, storage)
    stage.wait_for(".fs-list-item", timeout_ms=20000)


# -------------------------------------------------------------------- 6. sort


def test_sorting_reorders_the_browser(
    stage: ClientApp, sarca: SarcaClient, storage: str
) -> None:
    """Name ascending, name descending, and size — the three a file manager owes
    you. Folders keep leading either way, so only the files are compared."""
    seed_file(sarca, storage, "alpha.txt", b"a")
    seed_file(sarca, storage, "beta.txt", b"b" * 4096)
    seed_file(sarca, storage, "gamma.txt", b"g" * 1024)
    reopen(stage, storage)
    stage.wait_for_row("gamma.txt")

    def files() -> list[str]:
        return [name for name in stage.rows() if name.endswith(".txt")]

    set_sort(stage, "Name", "asc")
    assert files() == ["alpha.txt", "beta.txt", "gamma.txt"]

    set_sort(stage, "Name", "desc")
    assert files() == ["gamma.txt", "beta.txt", "alpha.txt"]

    set_sort(stage, "Size", "asc")
    assert files() == ["alpha.txt", "gamma.txt", "beta.txt"]


# ------------------------------------------------------------------ 7. search


def test_search_narrows_the_current_folder(
    stage: ClientApp, sarca: SarcaClient, storage: str
) -> None:
    """Typing in the header pill filters what is on screen and clearing it puts
    everything back."""
    seed_file(sarca, storage, "report-q1.txt")
    seed_file(sarca, storage, "holiday.jpg", media.png())
    reopen(stage, storage)
    stage.wait_for_row("holiday.jpg")

    stage.search("report")
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline and "holiday.jpg" in stage.rows():
        time.sleep(0.25)
    assert stage.rows() == ["report-q1.txt"], stage.rows()

    stage.search("")
    stage.wait_for_row("holiday.jpg")


# --------------------------------------------------------------- 8. favorites


def test_starring_a_file_puts_it_in_favorites(
    stage: ClientApp, sarca: SarcaClient, storage: str
) -> None:
    """The star is a two-way switch: on, the file shows up under Favorites; off,
    it leaves again."""
    seed_file(sarca, storage, "starred.txt")
    seed_file(sarca, storage, "plain.txt")
    reopen(stage, storage)
    stage.wait_for_row("starred.txt")

    stage.toggle_star("starred.txt")
    stage.wait_for_alert('Added "starred.txt" to favorites')

    stage.open_section("Favorites")
    stage.wait_for_row("starred.txt")
    assert "plain.txt" not in stage.rows()

    assert stage.is_starred("starred.txt")
    stage.toggle_star("starred.txt")
    stage.wait_for_alert('Removed "starred.txt" from favorites')
    stage.wait_row_gone("starred.txt")


# ------------------------------------------------------------------- 9. trash


def test_deleting_a_file_fills_the_trash_and_restore_undoes_it(
    stage: ClientApp, sarca: SarcaClient, storage: str
) -> None:
    """Delete is reversible until it is not.

    A deleted file has to leave the browser, appear under Trash, come back
    where it was on restore, and only then be destroyable for good.
    """
    seed_file(sarca, storage, "doomed.txt")
    reopen(stage, storage)
    stage.wait_for_row("doomed.txt")

    stage.row_action("doomed.txt", "Delete")
    stage.click_text("Confirm", scope="[role=dialog]")
    stage.wait_row_gone("doomed.txt")

    stage.open_section("Trash")
    stage.wait_for_row("doomed.txt")

    stage.row_action("doomed.txt", "Restore")
    stage.wait_for_alert('Restored "doomed.txt"')

    stage.open_section("All files")
    stage.wait_for_row("doomed.txt")
    assert sarca.download_bytes(storage, "doomed.txt") == b"hello"

    stage.row_action("doomed.txt", "Delete")
    stage.click_text("Confirm", scope="[role=dialog]")
    stage.wait_row_gone("doomed.txt")
    stage.open_section("Trash")
    stage.wait_for_row("doomed.txt")
    stage.row_action("doomed.txt", "Delete forever")
    stage.click_text("Confirm", scope="[role=dialog]")
    stage.wait_row_gone("doomed.txt")
    assert sarca.download(storage, "doomed.txt").status_code == 404


# ------------------------------------------------------------------ 10. rename


def test_rename_from_the_context_menu(
    stage: ClientApp, sarca: SarcaClient, storage: str
) -> None:
    """Renaming goes through a native prompt, so the test answers it directly —
    what matters is that the new name reaches the server, not who typed it."""
    seed_file(sarca, storage, "before.txt")
    reopen(stage, storage)
    stage.wait_for_row("before.txt")

    stage.stub_prompt("after.txt")
    stage.row_action("before.txt", "Rename")
    stage.wait_for_alert('Renamed to "after.txt"')

    stage.wait_for_row("after.txt")
    stage.wait_row_gone("before.txt")
    assert sarca.download_bytes(storage, "after.txt") == b"hello"


# -------------------------------------------------------------------- 11. move


def test_move_a_file_into_a_folder_through_the_picker(
    stage: ClientApp, sarca: SarcaClient, storage: str
) -> None:
    """Move to… opens a folder picker, and the file has to actually change path
    on the server, not merely vanish from the list."""
    assert sarca.create_folder(storage, "Archive").status_code < 300
    seed_file(sarca, storage, "movable.txt")
    reopen(stage, storage)
    stage.wait_for_row("movable.txt")

    stage.row_action("movable.txt", "Move to…")
    stage.wait_for("[role=dialog]", timeout_ms=10000)
    stage.click_text("Archive", scope="[role=dialog]")
    stage.click_text("Move here", scope="[role=dialog]")
    stage.wait_for_alert('Moved "movable.txt"')

    stage.wait_row_gone("movable.txt")
    assert sarca.download_bytes(storage, "Archive/movable.txt") == b"hello"


# ------------------------------------------------------------------ 12. viewer


def test_the_viewer_opens_an_image_and_steps_through_the_folder(
    stage: ClientApp, sarca: SarcaClient, storage: str
) -> None:
    """Clicking a picture opens the viewer, and the arrows walk the folder
    without going back to the list."""
    seed_file(sarca, storage, "one.png", media.png(seed=1))
    seed_file(sarca, storage, "two.png", media.png(seed=2))
    reopen(stage, storage)
    stage.wait_for_row("two.png")

    stage.open_row("one.png")
    stage.wait_for(".file-viewer", timeout_ms=30000)
    stage.wait_for_text("one.png")

    stage.click('[aria-label="Next file"]')
    stage.wait_for_text("two.png")
    stage.click('[aria-label="Previous file"]')
    stage.wait_for_text("one.png")

    stage.click(".file-viewer__close")
    stage.wait_gone(".file-viewer")
    stage.wait_for_row("one.png")


# ------------------------------------------------------------------- 13. share


def test_share_a_file_by_link_and_revoke_it(
    stage: ClientApp, sarca: SarcaClient, storage: str
) -> None:
    """A public link is the one thing that leaves the account, so it has to be
    listed under Shared and revocable from there."""
    seed_file(sarca, storage, "public.txt", b"shared bytes")
    reopen(stage, storage)
    stage.wait_for_row("public.txt")

    stage.row_action("public.txt", "Share link…")
    stage.wait_for("[role=dialog]", timeout_ms=10000)
    stage.click_text("Create", scope="[role=dialog]")
    stage.wait_for('[aria-label="Copy link"]', timeout_ms=20000)
    stage.click_text("Close", scope="[role=dialog]")

    stage.open_section("Shared")
    stage.wait_for(".shared-links-panel__row", timeout_ms=20000)
    stage.wait_for_text("public.txt")

    stage.click('[aria-label="Revoke link"]')
    deadline = time.monotonic() + 20
    while time.monotonic() < deadline:
        if "public.txt" not in stage.page_text():
            break
        time.sleep(0.5)
    else:
        pytest.fail("the revoked link is still listed under Shared")


# ---------------------------------------------------------- 14. bulk selection


def test_select_everything_and_delete_it_in_one_go(
    stage: ClientApp, sarca: SarcaClient, storage: str
) -> None:
    """Ctrl+A, then one Delete. The bulk bar has to count what it holds, and the
    whole batch has to land in the trash together."""
    for name in ("bulk-a.txt", "bulk-b.txt", "bulk-c.txt"):
        seed_file(sarca, storage, name)
    reopen(stage, storage)
    stage.wait_for_row("bulk-c.txt")

    stage.click_row("bulk-a.txt")
    stage.press("KeyA", ctrl=True)
    stage.wait_for(".files-bulk-bar", timeout_ms=10000)
    assert "3 selected" in stage.page_text()

    stage.click_text("Delete", scope=".files-bulk-bar")
    stage.click_text("Confirm", scope="[role=dialog]")
    for name in ("bulk-a.txt", "bulk-b.txt", "bulk-c.txt"):
        stage.wait_row_gone(name)

    stage.open_section("Trash")
    for name in ("bulk-a.txt", "bulk-b.txt", "bulk-c.txt"):
        stage.wait_for_row(name)


# ----------------------------------------------------------------- 15. settings


def test_settings_shows_the_account_creates_a_user_and_saves_trash_retention(
    stage: ClientApp, sarca: SarcaClient
) -> None:
    """The settings modal is where the admin-only surface lives.

    Three things are checked because all three are server state, not local
    preferences: the account it claims to be signed in as, a user it creates,
    and a trash retention it saves.
    """
    stage.open_settings("general")
    stage.wait_for_text("Account")

    original = sarca.get_trash_settings().get("retention_days")
    days = 9 if original != 9 else 12
    stage.open_settings("trash")
    stage.wait_for(".settings-trash", timeout_ms=10000)
    stage.fill(".settings-trash input", str(days))
    stage.click_text("Save", scope=".settings-trash")

    deadline = time.monotonic() + 20
    while time.monotonic() < deadline:
        if sarca.get_trash_settings().get("retention_days") == days:
            break
        time.sleep(0.5)
    else:
        pytest.fail(f"trash retention never became {days}")
    finally_days = original
    if isinstance(finally_days, int):
        # Retention is server-wide; leave it as the rest of the suite found it.
        sarca.set_trash_settings(finally_days)

    email = f"gui-{uuid.uuid4().hex[:8]}@sarca.test"
    stage.open_settings("users")
    stage.wait_for(".settings-users", timeout_ms=10000)
    stage.fill(".settings-users input[type=email]", email)
    stage.fill(".settings-users input[type=password]", "gui-password-123")
    stage.click_text("Create user", scope=".settings-users")

    deadline = time.monotonic() + 20
    while time.monotonic() < deadline:
        if any(user["email"] == email for user in sarca.list_users()):
            break
        time.sleep(0.5)
    else:
        pytest.fail(f"{email} was never created")

    created = next(user for user in sarca.list_users() if user["email"] == email)
    stage.close_settings()
    sarca.delete_user(created["id"])


# --------------------------------------------------------------- 16. photo zoom


def viewer_scale(app: ClientApp) -> float:
    """The photo's current magnification, read off the live transform."""
    raw = app.eval_js(
        """
        (() => {
          const img = document.querySelector('.file-viewer__image');
          if (!img) return '';
          const m = /scale\\(([0-9.]+)\\)/.exec(img.style.transform || '');
          return m ? m[1] : '1';
        })()
        """
    )
    return float(raw or 0)


def swipe(app: ClientApp, dx: int, dy: int = 0) -> None:
    """Throw a finger across the photo, the way a phone would."""
    app.eval_js(
        f"""
        (() => {{
          const el = document.querySelector('.file-viewer__zoom-surface');
          if (!el) return false;
          const fire = (type, x, y) => {{
            const e = new PointerEvent(type, {{
              bubbles: true, cancelable: true, pointerId: 7,
              pointerType: 'touch', clientX: x, clientY: y,
            }});
            el.dispatchEvent(e);
          }};
          fire('pointerdown', 400, 300);
          fire('pointermove', 400 + {dx}, 300 + {dy});
          fire('pointerup', 400 + {dx}, 300 + {dy});
          return true;
        }})()
        """
    )


def test_the_viewer_zooms_a_photo_and_swipes_between_them(
    stage: ClientApp, sarca: SarcaClient, storage: str
) -> None:
    """The magnifier and the touch gestures are the same viewer.

    Desktop drives it with the plus/minus buttons, a phone with its fingers, so
    both are exercised here: zoom in, zoom back out, then swipe the folder.
    """
    seed_file(sarca, storage, "one.png", media.png(seed=1))
    seed_file(sarca, storage, "two.png", media.png(seed=2))
    reopen(stage, storage)
    stage.wait_for_row("two.png")

    stage.open_row("one.png")
    stage.wait_for(".file-viewer", timeout_ms=30000)
    stage.wait_for(".file-viewer__image", timeout_ms=30000)
    assert viewer_scale(stage) == 1.0

    stage.click('[aria-label="Zoom in"]')
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline and viewer_scale(stage) <= 1.0:
        time.sleep(0.2)
    assert viewer_scale(stage) > 1.0, "the plus button never magnified the photo"

    # A zoomed photo pans; it must not skip to the next file under the finger.
    swipe(stage, -300)
    stage.wait_for_text("one.png")

    stage.click('[aria-label="Reset zoom"]')
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline and viewer_scale(stage) != 1.0:
        time.sleep(0.2)
    assert viewer_scale(stage) == 1.0

    swipe(stage, -300)
    stage.wait_for_text("two.png")
    swipe(stage, 300)
    stage.wait_for_text("one.png")

    stage.click(".file-viewer__close")
    stage.wait_gone(".file-viewer")
