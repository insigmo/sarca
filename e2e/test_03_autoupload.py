"""Scenario 3 — auto-upload (автозагрузка) driven by the real client sync engine.

Each test builds a local folder, runs the `sarca-sync` engine against the live
server, and then checks the server side: what landed, what was skipped, and that
a second pass does not re-upload anything.
"""

from __future__ import annotations

import pytest

from helpers import media
from helpers.api import SarcaClient, sha256
from helpers.sync_client import run_sync

pytestmark = [pytest.mark.mock_only, pytest.mark.slow]


@pytest.fixture
def gallery(tmp_path):
    """A local "DCIM" folder with photos, a video and a non-media file."""
    root = tmp_path / "dcim"
    root.mkdir()
    (root / "IMG_0001.jpg").write_bytes(media.big_photo(800, 600))
    (root / "IMG_0002.png").write_bytes(media.png(120, 90, seed=3))
    (root / "notes.txt").write_bytes(b"not a photo")
    return root


def sync(sarca, storage, tmp_path, local, **kwargs):
    return run_sync(
        base_url=sarca.base_url,
        email="e2e@sarca.test",
        password="e2e-password-123",
        storage_id=storage,
        local_dir=local,
        data_dir=tmp_path / "sync-state",
        **kwargs,
    )


def test_auto_upload_sends_media_and_skips_other_files(
    sarca: SarcaClient, storage: str, gallery, tmp_path
) -> None:
    run = sync(sarca, storage, tmp_path, gallery, remote_root="Camera")
    assert not run.errors, run.errors

    names = {e["name"] for e in sarca.tree(storage, "Camera")}
    assert names == {"IMG_0001.jpg", "IMG_0002.png"}, names
    assert run.scanned == 2, "auto-upload discovery is media-only"


def test_auto_upload_preserves_bytes(
    sarca: SarcaClient, storage: str, gallery, tmp_path
) -> None:
    run = sync(sarca, storage, tmp_path, gallery, remote_root="Camera")
    assert not run.errors, run.errors

    local = (gallery / "IMG_0001.jpg").read_bytes()
    sarca.wait_for_file(storage, "Camera/IMG_0001.jpg")
    assert sha256(sarca.download_bytes(storage, "Camera/IMG_0001.jpg")) == sha256(local)


def test_second_pass_uploads_nothing(
    sarca: SarcaClient, storage: str, gallery, tmp_path, mock
) -> None:
    first = sync(sarca, storage, tmp_path, gallery, remote_root="Camera")
    assert first.pending == 2, first.status

    uploads_after_first = mock.calls("sendDocument")
    second = sync(sarca, storage, tmp_path, gallery, remote_root="Camera")
    assert not second.errors, second.errors
    assert second.scanned == 2
    assert second.pending == 0, "nothing changed locally, so nothing should be re-sent"
    assert second.already_synced == 2
    assert mock.calls("sendDocument") == uploads_after_first


def test_new_photo_is_picked_up_by_the_next_pass(
    sarca: SarcaClient, storage: str, gallery, tmp_path
) -> None:
    sync(sarca, storage, tmp_path, gallery, remote_root="Camera")

    (gallery / "IMG_0003.jpg").write_bytes(media.big_photo(640, 480))
    run = sync(sarca, storage, tmp_path, gallery, remote_root="Camera")
    assert not run.errors, run.errors
    assert run.pending == 1, run.status

    names = {e["name"] for e in sarca.tree(storage, "Camera")}
    assert "IMG_0003.jpg" in names


def test_edited_photo_is_re_uploaded_beside_the_original(
    sarca: SarcaClient, storage: str, gallery, tmp_path
) -> None:
    """Editing a synced photo sends it again.

    The server never overwrites an existing path (`create_file_anyway` de-duplicates
    like a browser download), so the new bytes land next to the old ones as
    "IMG_0001 (1).jpg" and the first upload stays untouched.
    """
    original = (gallery / "IMG_0001.jpg").read_bytes()
    sync(sarca, storage, tmp_path, gallery, remote_root="Camera")
    edited = media.big_photo(900, 700)
    (gallery / "IMG_0001.jpg").write_bytes(edited)

    run = sync(sarca, storage, tmp_path, gallery, remote_root="Camera")
    assert not run.errors, run.errors
    assert run.pending == 1, run.status

    names = {e["name"] for e in sarca.tree(storage, "Camera")}
    assert {"IMG_0001.jpg", "IMG_0001 (1).jpg"} <= names, names
    assert sha256(sarca.download_bytes(storage, "Camera/IMG_0001.jpg")) == sha256(original)
    assert sha256(sarca.download_bytes(storage, "Camera/IMG_0001 (1).jpg")) == sha256(edited)


def test_retried_upload_with_same_hash_is_not_duplicated(
    sarca: SarcaClient, storage: str
) -> None:
    """A client retry with unchanged bytes must not create a second file.

    sarca-sync can believe an upload failed (e.g. a client-side timeout while
    the server was still relaying to Telegram) and resend the exact same path
    and content_hash on the next pass. Unlike a real edit, this is not new
    content, so the server must recognize it as the same upload instead of
    parking it beside the original as "IMG_0001 (1).jpg".
    """
    data = media.big_photo(800, 600)
    content_hash = sha256(data)

    assert sarca.create_folder(storage, "Camera").status_code in (200, 201)

    first = sarca.upload(storage, "IMG_0001.jpg", data, path="Camera/", content_hash=content_hash)
    assert first.ok, first.events
    sarca.wait_for_file(storage, "Camera/IMG_0001.jpg")

    second = sarca.upload(storage, "IMG_0001.jpg", data, path="Camera/", content_hash=content_hash)
    assert second.ok, second.events

    names = sorted(e["name"] for e in sarca.tree(storage, "Camera"))
    assert names == ["IMG_0001.jpg"], names


def test_nested_folders_are_mirrored(
    sarca: SarcaClient, storage: str, gallery, tmp_path
) -> None:
    nested = gallery / "2026" / "january"
    nested.mkdir(parents=True)
    (nested / "trip.jpg").write_bytes(media.big_photo(500, 400))

    run = sync(sarca, storage, tmp_path, gallery, remote_root="Camera")
    assert not run.errors, run.errors

    assert {e["name"] for e in sarca.tree(storage, "Camera/2026/january")} == {"trip.jpg"}


def test_folder_upload_mode_sends_every_file_type(
    sarca: SarcaClient, storage: str, gallery, tmp_path
) -> None:
    run = sync(
        sarca, storage, tmp_path, gallery, remote_root="Backup", mode="folder_upload"
    )
    assert not run.errors, run.errors

    names = {e["name"] for e in sarca.tree(storage, "Backup")}
    assert names == {"IMG_0001.jpg", "IMG_0002.png", "notes.txt"}, names


def test_upload_into_the_storage_root(
    sarca: SarcaClient, storage: str, gallery, tmp_path
) -> None:
    run = sync(sarca, storage, tmp_path, gallery, remote_root="")
    assert not run.errors, run.errors
    assert {"IMG_0001.jpg", "IMG_0002.png"} <= {e["name"] for e in sarca.tree(storage)}


def test_auto_upload_is_one_way(
    sarca: SarcaClient, storage: str, gallery, tmp_path
) -> None:
    """Files that exist only on the server must not be pulled into the local folder."""
    assert sarca.create_folder(storage, "Camera").status_code in (200, 201)
    assert sarca.upload(storage, "server-only.jpg", media.png(32, 32), path="Camera/").ok
    sarca.wait_for_file(storage, "Camera/server-only.jpg")

    run = sync(sarca, storage, tmp_path, gallery, remote_root="Camera")
    assert not run.errors, run.errors
    assert not (gallery / "server-only.jpg").exists()
    assert run.status["downloading"] == 0


def test_empty_folder_is_a_no_op(sarca: SarcaClient, storage: str, tmp_path) -> None:
    empty = tmp_path / "empty"
    empty.mkdir()
    run = sync(sarca, storage, tmp_path, empty, remote_root="Camera")
    assert not run.errors, run.errors
    assert run.scanned == 0
    assert run.pending == 0


def test_photos_uploaded_by_the_client_get_previews_too(
    sarca: SarcaClient, storage: str, gallery, tmp_path
) -> None:
    """Auto-uploaded photos go through the same server pipeline as browser uploads."""
    run = sync(sarca, storage, tmp_path, gallery, remote_root="Camera")
    assert not run.errors, run.errors
    sarca.wait_for_file(storage, "Camera/IMG_0001.jpg")

    r = sarca.preview(storage, "Camera/IMG_0001.jpg")
    assert r.status_code == 200, r.text
    assert media.is_jpeg(r.content)


def test_sync_reports_a_clear_error_when_the_server_rejects_the_token(
    sarca: SarcaClient, storage: str, gallery, tmp_path
) -> None:
    with pytest.raises(RuntimeError) as excinfo:
        run_sync(
            base_url=sarca.base_url,
            email="e2e@sarca.test",
            password="definitely-wrong",
            storage_id=storage,
            local_dir=gallery,
            data_dir=tmp_path / "sync-state",
            remote_root="Camera",
        )
    assert "sync driver failed" in str(excinfo.value)
