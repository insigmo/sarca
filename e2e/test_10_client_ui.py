"""Scenario 10 — the desktop client's own UI, driven by tauri-pilot.

These run the real Tauri binary (debug, `--features pilot`) against the
hermetic Sarca server, so what is asserted here is what a user sees:

* Sync settings draw the auto-upload toggle immediately, never a spinner;
* a client that was never told otherwise shows auto-upload **off**;
* the remembered state lives in localStorage, so it survives a relaunch;
* folder rows and file rows put their name at the same distance from the icon.

Needs a display (`DISPLAY`/Xvfb) and `tauri-pilot` on PATH; skipped otherwise.
"""

from __future__ import annotations

import time
import uuid

import pytest

from helpers.api import SarcaClient
from helpers.pilot import ClientApp

pytestmark = [pytest.mark.gui, pytest.mark.mock_only, pytest.mark.slow]

CAMERA_CACHE_KEY = "sarca.client.cameraAutoUploadEnabled"
VIEW_MODE_KEY = "sarca.filesViewMode"
SPINNER = '[aria-label="Loading auto-upload state"]'
SWITCH = "#settings-camera-switch"


def _switch_state(app: ClientApp) -> dict:
    return app.eval_js(
        """
        (() => {
          const el = document.querySelector('#settings-camera-switch');
          if (!el) return { present: false };
          const input = el.matches('input') ? el : el.querySelector('input');
          const node = input || el;
          return {
            present: true,
            checked: node.checked === true
              || node.getAttribute('aria-checked') === 'true'
              || node.getAttribute('data-checked') === 'true',
            disabled: node.disabled === true
              || node.getAttribute('aria-disabled') === 'true',
            spinner: !!document.querySelector('[aria-label="Loading auto-upload state"]'),
          };
        })()
        """
    )


def test_sync_settings_show_the_toggle_without_a_spinner(
    signed_in: ClientApp, storage: str
) -> None:
    """The panel must paint its real answer at once, not a pending spinner.

    The old code gated the switch behind `list_bindings`, which is exactly the
    IPC that is slow while the sync engine is scanning — the "hang" users saw.
    """
    signed_in.open_storage(storage)
    signed_in.open_sync_settings()
    signed_in.wait_for(SWITCH, timeout_ms=5000)

    state = _switch_state(signed_in)
    assert state["present"], "auto-upload switch never rendered"
    assert not state["spinner"], "settings still render the loading spinner"
    assert not state["disabled"], "switch is not interactive while bindings load"


def test_auto_upload_defaults_to_off_on_a_fresh_client(
    signed_in: ClientApp, storage: str
) -> None:
    """Auto-upload is opt-in: a client nobody configured shows it off."""
    signed_in.open_storage(storage)
    signed_in.open_sync_settings()
    signed_in.wait_for(SWITCH, timeout_ms=5000)

    assert _switch_state(signed_in)["checked"] is False


def test_toggle_state_is_remembered_across_a_relaunch(
    signed_in: ClientApp, storage: str
) -> None:
    """The cache moved from sessionStorage to localStorage.

    sessionStorage is wiped when the webview is torn down, so every relaunch
    used to start from "unknown" and wait on IPC again.
    """
    signed_in.open_storage(storage)
    signed_in.open_sync_settings()
    signed_in.wait_for(SWITCH, timeout_ms=5000)
    signed_in.run("storage", "set", CAMERA_CACHE_KEY, "1")
    # WebKit keeps localStorage in memory and flushes it on a timer, so a kill
    # right after the write loses it through no fault of the app.
    time.sleep(6)

    signed_in.restart()
    signed_in.wait_for_url("/storages")
    stored = signed_in.eval_js(f"localStorage.getItem({CAMERA_CACHE_KEY!r})")
    # The key itself is what has to survive; its value may already have been
    # corrected to "0" by the background probe, since this client has no
    # camera binding. sessionStorage would have come back as None.
    assert stored in {"0", "1"}, f"cached toggle state did not survive the relaunch: {stored!r}"

    signed_in.open_storage(storage)
    signed_in.open_sync_settings()
    signed_in.wait_for(SWITCH, timeout_ms=5000)
    assert not _switch_state(signed_in)["spinner"], (
        "a client with a cached state still renders the spinner"
    )


def test_folder_name_sits_close_to_its_icon(
    signed_in: ClientApp, sarca: SarcaClient, telegram
) -> None:
    """Less dead space between a folder's icon and its name.

    Two things had to change: the row gap itself (14px to 10px) and the ~6%
    transparent margin that `object-fit: contain` bakes into every glyph SVG.
    Measuring the rendered boxes catches either one being reverted.
    """
    storage_id = sarca.create_storage(
        name=f"e2e-gui-{uuid.uuid4().hex[:8]}",
        chat_ids=[telegram.new_chat_id()],
        bot_token=telegram.new_token(),
    )["id"]
    try:
        assert sarca.create_folder(storage_id, "Album").status_code < 300
        result = sarca.upload(storage_id, "note.txt", b"hello world")
        assert result.ok, result.error

        signed_in.run("storage", "set", VIEW_MODE_KEY, "list")
        signed_in.eval_js(f"window.location.assign('/storages/{storage_id}/files/'); 'ok'")
        signed_in.wait_for_url(f"/storages/{storage_id}/files")
        signed_in.wait_for(".fs-list-item", timeout_ms=20000)
        # Everything below measures rendered boxes, so the stylesheet has to be
        # live first — otherwise the rows report their unstyled defaults.
        signed_in.wait_for_styles()

        measured = signed_in.eval_js(
            """
            (() => {
              const rows = {};
              let gap = null;
              for (const row of document.querySelectorAll('.fs-list-item')) {
                const name = row.querySelector('.fs-list-item__name');
                const icon = row.querySelector('.file-type-icon');
                if (!name || !icon) continue;
                if (gap === null) gap = getComputedStyle(row).columnGap;
                rows[(name.textContent || '').trim()] = Math.round(
                  (name.getBoundingClientRect().left - icon.getBoundingClientRect().right) * 100
                ) / 100;
              }
              return { gap, rows };
            })()
            """
        )
        gaps = measured["rows"]
        assert "Album" in gaps and "note.txt" in gaps, measured
        assert measured["gap"] == "10px", f"row gap is back to {measured['gap']}"

        folder_gap, file_gap = gaps["Album"], gaps["note.txt"]
        # Both are glyph icons, so both get the same negative margin; a folder
        # must not be the odd one out.
        assert abs(folder_gap - file_gap) <= 0.5, f"folder row is out of line: {gaps}"
        # 10px gap minus the 3px glyph compensation, plus the row body's own
        # padding, which is 0 here.
        assert folder_gap <= 7.5, f"glyph margin compensation is gone: {gaps}"
    finally:
        sarca.delete_storage(storage_id)
