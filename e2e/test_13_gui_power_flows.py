"""Scenario 13 — twenty more journeys through the real desktop client.

Scenario 11 walks the product's happy path; this file takes the parts a power
user reaches for and the parts that only exist because the client is *native*.
Everything runs through the real Tauri binary (tauri-pilot) against the hermetic
Sarca server, and every claim is checked on screen and, where it is server
state, through the API as well.

The twenty scenarios, in file order:

 1. Ctrl+A selects every visible row and Escape drops the selection again.
 2. Ctrl+C then Ctrl+V inside a folder copies the file — both copies survive.
 3. Ctrl+X then Ctrl+V moves it instead — the source path is gone afterwards.
 4. Pasting into the folder a file already lives in duplicates it in place,
    and pasting where a different file holds the name raises the conflict
    dialog, whose "Keep both" lands a "name (1).ext" sibling.
 5. F2 renames whatever is selected, without touching the context menu.
 6. The New menu creates a folder, and the dialog refuses a name with a "/".
 7. Breadcrumbs walk back out of a nested folder, one level and all the way.
 8. "Copy to…" through the folder picker duplicates a file into a subfolder.
 9. Empty trash destroys every trashed item, not just the selected one.
10. The Info dialog reports the same byte size and path the API reports.
11. Handing several files to the upload input at once drives the upload
    manager to completion and every byte lands intact.
12. Sorting by size orders the browser by size, ascending and descending.
13. Opening a file in the viewer puts it in Recent.
14. A theme picked in Settings is still applied after the app is relaunched.
15. The sidebar language switcher relabels the UI and switches back.
16. App lock: enable with a PIN, relaunch into the lock screen, refuse a wrong
    PIN, unlock with the right one, then disable it again.
17. The superuser changes another account's password from Settings, and the new
    password is the one the server accepts.
18. Disabling an account from Settings locks that account out of the API.
19. Access granted from Settings makes the storage visible to a second client
    signed in as that user.
20. A password-protected share link asks for the password at /s/<token>,
    refuses a wrong one, and opens the file with the right one.

Needs a display (`DISPLAY`/Xvfb) and `tauri-pilot` on PATH; skipped otherwise.
"""

from __future__ import annotations

import json
import time
import uuid

import pytest

from helpers import media
from helpers.api import SarcaClient
from helpers.pilot import ClientApp, PilotError

pytestmark = [pytest.mark.gui, pytest.mark.mock_only, pytest.mark.slow]


# --------------------------------------------------------------------- helpers


@pytest.fixture
def stage(gui: ClientApp, storage: str) -> ClientApp:
    """The session client, parked in an empty storage of this test's own."""
    gui.reset(storage)
    yield gui
    try:
        gui.press("Escape")
        gui.reset()
    except PilotError:
        pass


def seed_file(
    sarca: SarcaClient, storage: str, name: str, data: bytes = b"hello", path: str = ""
) -> None:
    result = sarca.upload(storage, name, data, path=path)
    assert result.ok, f"seeding {name} failed: {result.error}"


def reopen(app: ClientApp, storage: str) -> None:
    """Re-enter the storage so the browser refetches from the server."""
    app.goto_until("/storages")
    app.open_storage(storage)


def paths(sarca: SarcaClient, storage: str, path: str = "") -> list[str]:
    return [item["name"] for item in sarca.tree(storage, path)]


_FILL_LABELED_JS = """
(() => {
  const wanted = %(label)s, value = %(value)s;
  for (const lab of document.querySelectorAll('label')) {
    const text = (lab.textContent || '').trim();
    if (!text.startsWith(wanted)) continue;
    let el = lab.control || (lab.htmlFor && document.getElementById(lab.htmlFor));
    if (!el) el = lab.parentElement && lab.parentElement.querySelector('input, textarea');
    if (!el) continue;
    const setter = Object.getOwnPropertyDescriptor(
      Object.getPrototypeOf(el), 'value'
    )?.set;
    if (setter) setter.call(el, value); else el.value = value;
    el.dispatchEvent(new Event('input', { bubbles: true }));
    el.dispatchEvent(new Event('change', { bubbles: true }));
    return el.value === value ? 'ok' : 'lost';
  }
  return 'missing';
})()
"""


def fill_labeled(app: ClientApp, label: str, value: str, timeout_s: float = 15.0) -> None:
    """Fill a SUID TextField found by its floating label.

    Settings and the dialogs draw their fields through SUID, which gives them
    no name and an id that changes per mount; the visible label is the only
    stable handle a test and a user share.
    """
    script = _FILL_LABELED_JS % {"label": json.dumps(label), "value": json.dumps(value)}
    deadline = time.monotonic() + timeout_s
    outcome = "missing"
    while time.monotonic() < deadline:
        outcome = app.eval_js(script)
        if outcome == "ok":
            return
        time.sleep(0.25)
    raise AssertionError(f"could not fill the {label!r} field: {outcome}")


def wait_for_lock_error(app: ClientApp, text: str, timeout_s: float = 30.0) -> None:
    """Wait for the lock screen's own error line.

    The gate paints its error into a dedicated node, and `verify_app_lock_pin`
    deliberately sleeps before answering (PIN guessing throttle), so reading
    that node is both more precise and more patient than scanning page text.
    """
    deadline = time.monotonic() + timeout_s
    seen = ""
    while time.monotonic() < deadline:
        seen = app.eval_js(
            "document.querySelector('.app-lock-gate__error')?.textContent || ''"
        )
        if text in (seen or ""):
            return
        time.sleep(0.25)
    raise AssertionError(f"the lock screen never said {text!r} (showed {seen!r})")


def new_user(sarca: SarcaClient, password: str = "second-user-123") -> tuple[str, str, str]:
    """Create a plain account; returns (id, email, password)."""
    email = f"e2e-{uuid.uuid4().hex[:8]}@sarca.test"
    response = sarca.create_user(email, password)
    assert response.status_code < 300, response.text
    user = next(u for u in sarca.list_users() if u["email"] == email)
    return user["id"], email, password


def wait_for_access(
    sarca: SarcaClient, storage: str, email: str, timeout_s: float = 30.0
) -> None:
    """Poll the storage's access list until `email` is on it."""
    deadline = time.monotonic() + timeout_s
    seen: list[str] = []
    while time.monotonic() < deadline:
        response = sarca.get(f"/api/storages/{storage}/access")
        if response.status_code == 200:
            payload = response.json()
            rows = payload["users"] if isinstance(payload, dict) else payload
            seen = [row.get("email", "") for row in rows]
            if email in seen:
                return
        time.sleep(0.5)
    raise AssertionError(f"{email} never gained access; list holds {seen}")


def can_log_in(base_url: str, email: str, password: str) -> bool:
    client = SarcaClient(base_url)
    try:
        client.login(email, password)
        return True
    except Exception:
        return False
    finally:
        client.close()


# ------------------------------------------------------- 1. select all / clear


def test_ctrl_a_selects_everything_and_escape_clears_it(
    stage: ClientApp, sarca: SarcaClient, storage: str
) -> None:
    """Select-all is the keyboard's answer to a full folder, and Escape has to
    undo it without touching anything on the server."""
    for name in ("a.txt", "b.txt", "c.txt"):
        seed_file(sarca, storage, name)
    reopen(stage, storage)
    stage.wait_for_row("c.txt")

    stage.press("KeyA", ctrl=True)
    stage.wait_for_text("3 selected")

    stage.press("Escape")
    stage.wait_gone(".files-bulk-bar", timeout_s=10)
    assert sorted(paths(sarca, storage)) == ["a.txt", "b.txt", "c.txt"]


# ------------------------------------------------------------- 2. copy / paste


def test_copy_and_paste_duplicates_a_file_into_a_folder(
    stage: ClientApp, sarca: SarcaClient, storage: str
) -> None:
    """Ctrl+C / Ctrl+V is a copy, so the original has to still be where it was."""
    assert sarca.create_folder(storage, "Copies").status_code < 300
    seed_file(sarca, storage, "doc.txt", b"copy me")
    reopen(stage, storage)
    stage.wait_for_row("doc.txt")

    stage.click_row("doc.txt")
    stage.press("KeyC", ctrl=True)
    stage.wait_for_alert('Copied "doc.txt"')

    stage.open_row("Copies")
    stage.wait_for_url("Copies")
    stage.press("KeyV", ctrl=True)
    stage.wait_for_row("doc.txt")

    assert sarca.download_bytes(storage, "Copies/doc.txt") == b"copy me"
    assert sarca.download_bytes(storage, "doc.txt") == b"copy me"


# -------------------------------------------------------------- 3. cut / paste


def test_cut_and_paste_moves_a_file_into_a_folder(
    stage: ClientApp, sarca: SarcaClient, storage: str
) -> None:
    """Ctrl+X / Ctrl+V is a move: the destination gains the file and the source
    loses it."""
    assert sarca.create_folder(storage, "Moved").status_code < 300
    seed_file(sarca, storage, "note.txt", b"move me")
    reopen(stage, storage)
    stage.wait_for_row("note.txt")

    stage.click_row("note.txt")
    stage.press("KeyX", ctrl=True)
    stage.wait_for_alert('Cut "note.txt"')

    stage.open_row("Moved")
    stage.wait_for_url("Moved")
    stage.press("KeyV", ctrl=True)
    stage.wait_for_row("note.txt")

    assert sarca.download_bytes(storage, "Moved/note.txt") == b"move me"
    assert sarca.download(storage, "note.txt").status_code == 404


# ---------------------------------------------------------- 4. paste conflict


def test_paste_duplicates_in_place_and_offers_keep_both_elsewhere(
    stage: ClientApp, sarca: SarcaClient, storage: str
) -> None:
    """Two flavours of the same collision.

    Pasting into the folder the file already lives in cannot mean "replace it
    with itself", so it silently keeps both. Pasting where a *different* file
    holds the name is a real conflict, and the dialog's "Keep both" has to add
    a sibling instead of overwriting.
    """
    assert sarca.create_folder(storage, "Dest").status_code < 300
    seed_file(sarca, storage, "dup.txt", b"root copy")
    seed_file(sarca, storage, "dup.txt", b"folder copy", path="Dest")
    reopen(stage, storage)
    stage.wait_for_row("dup.txt")

    stage.click_row("dup.txt")
    stage.press("KeyC", ctrl=True)
    stage.wait_for_alert('Copied "dup.txt"')

    stage.press("KeyV", ctrl=True)
    stage.wait_for_row("dup (1).txt")
    assert sarca.download_bytes(storage, "dup (1).txt") == b"root copy"

    stage.click_row("dup.txt")
    stage.press("KeyC", ctrl=True)
    stage.wait_for_alert('Copied "dup.txt"')
    stage.open_row("Dest")
    stage.wait_for_url("Dest")
    stage.press("KeyV", ctrl=True)

    stage.wait_for("[role=dialog]", timeout_ms=15000)
    stage.wait_for_text("Path already exists")
    stage.click_text("Keep both", scope="[role=dialog]")

    stage.wait_for_row("dup (1).txt")
    assert sarca.download_bytes(storage, "Dest/dup (1).txt") == b"root copy"
    assert sarca.download_bytes(storage, "Dest/dup.txt") == b"folder copy"


# ------------------------------------------------------------------- 5. F2


def test_f2_renames_the_selected_row(
    stage: ClientApp, sarca: SarcaClient, storage: str
) -> None:
    """F2 is the file-manager reflex; it has to rename the selection with no
    menu in between."""
    seed_file(sarca, storage, "old-name.txt", b"same bytes")
    reopen(stage, storage)
    stage.wait_for_row("old-name.txt")

    stage.click_row("old-name.txt")
    stage.stub_prompt("new-name.txt")
    stage.press("F2")
    stage.wait_for_alert('Renamed to "new-name.txt"')

    stage.wait_for_row("new-name.txt")
    assert sarca.download_bytes(storage, "new-name.txt") == b"same bytes"


# ------------------------------------------------------------ 6. New → folder


def test_new_menu_creates_a_folder_and_rejects_a_slash(
    stage: ClientApp, sarca: SarcaClient, storage: str
) -> None:
    """The New button is the only way to make a folder from the client, and the
    dialog is the only thing standing between a user and an invalid name."""
    stage.click(".files-new-fab")
    stage.click_text("Create folder")
    stage.wait_for("[role=dialog]", timeout_ms=10000)

    fill_labeled(stage, "New folder name", "Reports")
    stage.click_text("Create", scope="[role=dialog]")
    stage.wait_for_alert('Created folder "Reports"')
    stage.wait_for_row("Reports")
    folders = [
        item["name"].rstrip("/")
        for item in sarca.tree(storage)
        if not item.get("is_file", True)
    ]
    assert "Reports" in folders, folders

    stage.click(".files-new-fab")
    stage.click_text("Create folder")
    stage.wait_for("[role=dialog]", timeout_ms=10000)
    fill_labeled(stage, "New folder name", "bad/name")
    stage.wait_for_text('Folder name cannot have a "/" symbol')
    assert not stage.click_text("Create", scope="[role=dialog]", required=False, timeout_s=3)
    stage.click_text("Cancel", scope="[role=dialog]")


# ------------------------------------------------------------ 7. breadcrumbs


def test_breadcrumbs_walk_back_out_of_a_nested_folder(
    stage: ClientApp, sarca: SarcaClient, storage: str
) -> None:
    """Two levels deep, the breadcrumb has to be able to go one level up and
    straight back to the root."""
    assert sarca.create_folder(storage, "Outer").status_code < 300
    assert sarca.create_folder(storage, "Inner", path="Outer").status_code < 300
    seed_file(sarca, storage, "deep.txt", b"deep", path="Outer/Inner")
    reopen(stage, storage)

    stage.open_row("Outer")
    stage.wait_for_row("Inner")
    stage.open_row("Inner")
    stage.wait_for_row("deep.txt")

    stage.click_text("Outer", scope=".files-breadcrumb")
    stage.wait_for_row("Inner")

    stage.click_text("All files", scope=".files-breadcrumb")
    stage.wait_for_row("Outer")


# ------------------------------------------------------------- 8. copy picker


def test_copy_to_through_the_folder_picker(
    stage: ClientApp, sarca: SarcaClient, storage: str
) -> None:
    """"Copy to…" is the mouse-driven twin of Ctrl+C: same result, different
    road, and the original stays put."""
    assert sarca.create_folder(storage, "Backup").status_code < 300
    seed_file(sarca, storage, "keep.txt", b"keep me")
    reopen(stage, storage)
    stage.wait_for_row("keep.txt")

    stage.row_action("keep.txt", "Copy to…")
    stage.wait_for("[role=dialog]", timeout_ms=10000)
    stage.click_text("Backup", scope="[role=dialog]")
    stage.click_text("Copy here", scope="[role=dialog]")
    stage.wait_for_alert('Copied "keep.txt"')

    assert sarca.download_bytes(storage, "Backup/keep.txt") == b"keep me"
    assert sarca.download_bytes(storage, "keep.txt") == b"keep me"


# ------------------------------------------------------------ 9. empty trash


def test_empty_trash_destroys_everything_in_it(
    stage: ClientApp, sarca: SarcaClient, storage: str
) -> None:
    """Empty trash is the one irreversible bulk action; it has to take the whole
    trash, not the selection."""
    for name in ("gone1.txt", "gone2.txt"):
        seed_file(sarca, storage, name)
        assert sarca.delete_file(storage, name).status_code < 300
    reopen(stage, storage)

    stage.open_section("Trash")
    stage.wait_for_row("gone1.txt")
    stage.wait_for_row("gone2.txt")

    stage.click_text("Empty trash")
    stage.click_text("Confirm", scope="[role=dialog]")
    stage.wait_for_alert("Trash emptied")

    stage.wait_row_gone("gone1.txt")
    assert stage.rows() == []
    assert sarca.download(storage, "gone1.txt").status_code == 404


# ------------------------------------------------------------- 10. info panel


def test_info_dialog_matches_what_the_api_reports(
    stage: ClientApp, sarca: SarcaClient, storage: str
) -> None:
    """The Info dialog is the client's own account of a file; it must agree with
    the server down to the byte."""
    payload = media.blob(4321)
    seed_file(sarca, storage, "facts.bin", payload)
    info = sarca.info(storage, "facts.bin")
    reopen(stage, storage)
    stage.wait_for_row("facts.bin")

    stage.row_action("facts.bin", "Info")
    stage.wait_for("[role=dialog]", timeout_ms=10000)
    stage.wait_for_text("facts.bin")
    stage.wait_for_text(f"{info['size']:,} bytes")

    text = stage.page_text()
    assert "facts.bin" in text
    stage.press("Escape")


# ----------------------------------------------------------- 11. bulk upload


def test_uploading_several_files_at_once_completes(
    stage: ClientApp, sarca: SarcaClient, storage: str
) -> None:
    """The upload manager is the only feedback a user gets for a batch; it has
    to appear, finish, and leave three intact files behind."""
    batch = [
        ("batch-1.bin", bytes(range(256)) * 3),
        ("batch-2.bin", b"second payload"),
        ("batch-3.bin", b""),
    ]
    stage.upload_bytes(batch)
    stage.wait_for(".upload-mgr", timeout_ms=20000)
    stage.wait_for_alert("3 uploads complete", timeout_s=90)

    for name, data in batch:
        sarca.wait_for_file(storage, name)
        assert sarca.download_bytes(storage, name) == data


# -------------------------------------------------------------- 12. sort size


def test_sorting_by_size_orders_the_browser(
    stage: ClientApp, sarca: SarcaClient, storage: str
) -> None:
    """Sorting by size is the one sort a name-sorted list cannot fake."""
    seed_file(sarca, storage, "small.bin", b"x" * 10)
    seed_file(sarca, storage, "medium.bin", b"x" * 1000)
    seed_file(sarca, storage, "large.bin", b"x" * 50_000)
    reopen(stage, storage)
    stage.wait_for_row("large.bin")

    set_sort(stage, "Size", "asc")
    assert stage.rows() == ["small.bin", "medium.bin", "large.bin"]

    set_sort(stage, "Size", "desc")
    assert stage.rows() == ["large.bin", "medium.bin", "small.bin"]


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
    """Drive the sort menu to an exact state (field, then direction)."""
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


# ----------------------------------------------------------------- 13. recent


def test_opening_a_file_puts_it_in_recent(
    stage: ClientApp, sarca: SarcaClient, storage: str
) -> None:
    """Recent is built from what the viewer opened, so opening one of two files
    has to separate them."""
    seed_file(sarca, storage, "seen.png", media.png(seed=3))
    seed_file(sarca, storage, "unseen.png", media.png(seed=4))
    reopen(stage, storage)
    stage.wait_for_row("seen.png")

    stage.open_row("seen.png")
    stage.wait_for(".file-viewer", timeout_ms=30000)
    stage.click(".file-viewer__close")
    stage.wait_gone(".file-viewer")

    stage.open_section("Recent")
    stage.wait_for_row("seen.png")
    assert "unseen.png" not in stage.rows()


# ------------------------------------------------------------------ 14. theme


def test_theme_choice_survives_a_relaunch(
    signed_in: ClientApp, storage: str
) -> None:
    """A theme is a per-device preference; picking dark once has to be enough."""
    signed_in.open_storage(storage)
    signed_in.open_settings("general")
    signed_in.click_text("Dark", scope=".settings-modal")

    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        if signed_in.eval_js("document.documentElement.dataset.theme") == "dark":
            break
        time.sleep(0.25)
    else:
        pytest.fail("picking Dark never themed the document")

    # WebKit flushes localStorage on a timer; a kill too soon loses the write.
    time.sleep(6)
    signed_in.restart()
    signed_in.wait_for("body", timeout_ms=30000)
    signed_in.wait_for_styles()

    deadline = time.monotonic() + 20
    while time.monotonic() < deadline:
        if signed_in.eval_js("document.documentElement.dataset.theme") == "dark":
            return
        time.sleep(0.5)
    pytest.fail("the relaunched client came back on the light theme")


# --------------------------------------------------------------- 15. language


def test_the_sidebar_switches_language_and_back(stage: ClientApp) -> None:
    """Every label in the product goes through i18n; the switcher is the one
    control that proves it end to end."""
    stage.click('.files-sidebar__item[title="English"]')
    stage.click_text("Русский")
    stage.wait_for_text("Все файлы")

    stage.click('.files-sidebar__item[title="Русский"]')
    stage.click_text("English")
    stage.wait_for_text("All files")


# --------------------------------------------------------------- 16. app lock


def test_app_lock_guards_the_relaunched_client(signed_in: ClientApp, storage: str) -> None:
    """App lock is native-only state: the PIN lives in Rust, and a relaunch is
    the only way to find out whether it is really enforced."""
    signed_in.open_storage(storage)
    signed_in.open_settings("general")
    signed_in.click("#settings-app-lock-switch")
    fill_labeled(signed_in, "PIN (4–8 digits)", "4821")
    fill_labeled(signed_in, "Confirm new PIN", "4821")
    signed_in.click_text("Enable lock", scope=".settings-modal")
    signed_in.wait_for_alert("App lock enabled")
    signed_in.close_settings()

    time.sleep(6)
    signed_in.restart()
    signed_in.wait_for(".app-lock-gate", timeout_ms=60000)

    fill_labeled(signed_in, "PIN", "9999")
    signed_in.click_text("Unlock", scope=".app-lock-gate")
    wait_for_lock_error(signed_in, "Incorrect PIN")

    fill_labeled(signed_in, "PIN", "4821")
    signed_in.click_text("Unlock", scope=".app-lock-gate")
    signed_in.wait_gone(".app-lock-gate", timeout_s=20)

    signed_in.open_settings("general")
    fill_labeled(signed_in, "Current PIN", "4821")
    signed_in.click_text("Disable", scope=".settings-modal")
    signed_in.wait_for_alert("App lock disabled")


# ------------------------------------------------------ 17. password of a user


def test_superuser_changes_another_accounts_password(
    stage: ClientApp, sarca: SarcaClient, base_url: str
) -> None:
    """Whoever administers the server has to be able to hand out a new password
    — and the server has to honour exactly that one."""
    user_id, email, password = new_user(sarca)
    try:
        stage.open_settings("access")
        stage.wait_for_text(email)
        stage.click_text("Change password", scope=f'[data-user="{user_id}"]', required=False)
        if not stage.eval_js("document.querySelector('.settings-users__password-form') ? 1 : 0"):
            open_row_password(stage, email)
        fill_labeled(stage, "New password", "brand-new-pass-9")
        stage.click_text("Save", scope=".settings-users__password-form")
        stage.wait_for_alert("Password changed")

        assert can_log_in(base_url, email, "brand-new-pass-9")
        assert not can_log_in(base_url, email, password)
    finally:
        stage.close_settings()
        sarca.delete_user(user_id)


def open_row_password(app: ClientApp, email: str) -> None:
    """Click the "Change password" button in the account row for `email`."""
    outcome = app.eval_js(
        """
        (() => {
          const wanted = %s;
          for (const row of document.querySelectorAll('.settings-users__row')) {
            if (!(row.textContent || '').includes(wanted)) continue;
            for (const btn of row.querySelectorAll('button')) {
              if ((btn.textContent || '').trim() === 'Change password') {
                btn.click();
                return 'ok';
              }
            }
          }
          return 'missing';
        })()
        """
        % json.dumps(email)
    )
    assert outcome == "ok", f"no Change password button for {email}: {outcome}"


# ------------------------------------------------------------ 18. disable user


def test_disabling_an_account_locks_it_out(
    stage: ClientApp, sarca: SarcaClient, base_url: str
) -> None:
    """The disable switch is an access-control decision, so the proof is that
    the server stops accepting that account's credentials."""
    user_id, email, password = new_user(sarca)
    try:
        assert can_log_in(base_url, email, password)

        stage.open_settings("access")
        stage.wait_for_text(email)
        stage.click(f'#settings-users-disabled-{user_id}')
        stage.wait_for_alert("User disabled")

        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            if not can_log_in(base_url, email, password):
                break
            time.sleep(0.5)
        else:
            pytest.fail("a disabled account can still sign in")
    finally:
        stage.close_settings()
        sarca.delete_user(user_id)


# ----------------------------------------------------------- 19. grant access


def test_granted_access_shows_the_storage_to_the_other_user(
    stage: ClientApp,
    app: ClientApp,
    sarca: SarcaClient,
    base_url: str,
    storage: str,
) -> None:
    """Sharing a storage is worth nothing until the other person's own client
    lists it, so this scenario ends in a second, freshly installed client."""
    user_id, email, password = new_user(sarca)
    storage_name = sarca.storage_detail(storage)["name"]
    try:
        stage.open_settings("access")
        select_storage(stage, storage)
        stage.click_text("Grant access", scope=".settings-access")
        stage.wait_for("[role=dialog]", timeout_ms=10000)
        # The Accounts panel below has its own "Email" field and comes first in
        # the DOM, so this one is addressed by the id the dialog gives it.
        stage.fill("#email", email)
        stage.click(".access-type-option--w")
        # No scope: the settings modal is also a [role=dialog] and comes first in
        # the document, and its own button reads "Grant access", so an exact
        # "Grant" already picks out the dialog's submit.
        stage.click_text("Grant")
        wait_for_access(sarca, storage, email)
        stage.wait_for_text(email)
        stage.close_settings()

        app.connect(base_url)
        app.login(email, password)
        app.goto_until("/storages", "/storages", "/setup")
        app.wait_for_text(storage_name, timeout_s=30)
    finally:
        sarca.delete_user(user_id)


def select_storage(app: ClientApp, storage_id: str) -> None:
    """Point the Access tab's storage picker at `storage_id`."""
    outcome = app.eval_js(
        """
        (() => {
          const sel = document.querySelector('.settings-select');
          if (!sel) return 'missing';
          const setter = Object.getOwnPropertyDescriptor(
            Object.getPrototypeOf(sel), 'value'
          )?.set;
          if (setter) setter.call(sel, %s); else sel.value = %s;
          sel.dispatchEvent(new Event('change', { bubbles: true }));
          return sel.value === %s ? 'ok' : 'lost';
        })()
        """
        % (json.dumps(storage_id), json.dumps(storage_id), json.dumps(storage_id))
    )
    assert outcome == "ok", f"could not select the storage: {outcome}"


# ------------------------------------------------------- 20. protected share


def test_a_password_protected_link_asks_for_the_password(
    stage: ClientApp, sarca: SarcaClient, storage: str
) -> None:
    """A password on a share is the only thing between a public URL and the
    file, so it has to be enforced on the page a guest actually lands on."""
    seed_file(sarca, storage, "secret.txt", b"classified")
    reopen(stage, storage)
    stage.wait_for_row("secret.txt")

    stage.row_action("secret.txt", "Share link…")
    stage.wait_for("[role=dialog]", timeout_ms=10000)
    fill_labeled(stage, "Password (optional)", "open-sesame")
    stage.click_text("Create", scope="[role=dialog]")
    stage.wait_for('[aria-label="Copy link"]', timeout_ms=20000)
    stage.click_text("Close", scope="[role=dialog]")

    links = sarca.get(f"/api/storages/{storage}/shares").json()
    token = (links[0] if isinstance(links, list) else links["shares"][0])["token"]

    stage.goto_until(f"/s/{token}", f"/s/{token}")
    stage.wait_for_text("Password required")

    fill_labeled(stage, "Password", "wrong-one")
    stage.click_text("Unlock")
    stage.wait_for_text("Incorrect password")

    fill_labeled(stage, "Password", "open-sesame")
    stage.click_text("Unlock")
    stage.wait_for_text("secret.txt", timeout_s=30)
