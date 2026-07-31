"""Scenario 7 — opening a file in the viewer must feel instant (< 1 s).

These tests inject realistic Telegram latency into the fake Bot API (a real
`getFile` + CDN fetch round trip is ~150-400 ms) so the numbers mean something.
The budget is the wall clock of the request the UI actually issues when a user
clicks a file: `preview` for photos, a first Range read for video/other files.
"""

from __future__ import annotations

import shutil
import time

import pytest

from helpers import media
from helpers.api import SarcaClient

pytestmark = [pytest.mark.mock_only, pytest.mark.slow]

OPEN_BUDGET_SECONDS = 1.0
# One Telegram round trip: getFile metadata + the actual document fetch.
TELEGRAM_RTT = 0.2


def timed(fn, *args, **kwargs):
    started = time.perf_counter()
    result = fn(*args, **kwargs)
    return result, time.perf_counter() - started


@pytest.fixture
def slow_telegram(mock):
    """Every Telegram call costs a realistic round trip for the duration of a test."""
    mock.set_latency(getFile=TELEGRAM_RTT, download=TELEGRAM_RTT, sendDocument=0.0)
    yield mock
    mock.clear_latency()


def test_photo_opens_within_one_second_cold(
    sarca: SarcaClient, storage: str, slow_telegram, workdir
) -> None:
    """Cold cache: the preview is one small document, not the whole original."""
    data = media.big_photo(4000, 3000)
    assert sarca.upload(storage, "holiday.jpg", data, content_type="image/jpeg").ok
    sarca.wait_for_file(storage, "holiday.jpg")

    # Nothing warm anywhere: this is the first open after a server move.
    shutil.rmtree(workdir / "preview_cache", ignore_errors=True)
    shutil.rmtree(workdir / "chunk_cache", ignore_errors=True)
    slow_telegram.reset_calls()

    response, elapsed = timed(sarca.preview, storage, "holiday.jpg")
    assert response.status_code == 200
    assert media.is_jpeg(response.content)
    assert elapsed < OPEN_BUDGET_SECONDS, (
        f"cold preview took {elapsed:.2f}s (>{OPEN_BUDGET_SECONDS}s); "
        f"telegram calls: {slow_telegram.stats()['calls']}"
    )


def test_photo_opens_within_one_second_warm(
    sarca: SarcaClient, storage: str, slow_telegram
) -> None:
    data = media.big_photo(4000, 3000)
    assert sarca.upload(storage, "warm.jpg", data, content_type="image/jpeg").ok
    sarca.wait_for_file(storage, "warm.jpg")

    sarca.preview(storage, "warm.jpg")  # warm the disk cache
    response, elapsed = timed(sarca.preview, storage, "warm.jpg")
    assert response.status_code == 200
    assert elapsed < 0.3, f"warm preview took {elapsed:.2f}s; it should be a disk read"


def test_thumbnail_grid_opens_fast(sarca: SarcaClient, storage: str, slow_telegram) -> None:
    """The gallery grid asks for thumbs; each must stay well inside the budget."""
    for i in range(3):
        data = media.big_photo(2000, 1500)
        assert sarca.upload(storage, f"grid{i}.jpg", data, content_type="image/jpeg").ok
        sarca.wait_for_file(storage, f"grid{i}.jpg")

    for i in range(3):
        response, elapsed = timed(sarca.thumb, storage, f"grid{i}.jpg")
        assert response.status_code == 200
        assert elapsed < OPEN_BUDGET_SECONDS, f"thumb {i} took {elapsed:.2f}s"


def test_video_first_bytes_arrive_within_one_second(
    sarca: SarcaClient, storage: str, slow_telegram
) -> None:
    """Player start = first Range request; only the first 1 MB chunk may be fetched."""
    data = media.blob(3 * 1024 * 1024, seed=21)
    assert sarca.upload(storage, "clip.mp4", data, content_type="video/mp4").ok
    sarca.wait_for_file(storage, "clip.mp4")

    slow_telegram.reset_calls()
    response, elapsed = timed(
        sarca.download, storage, "clip.mp4", headers={"Range": "bytes=0-65535"}
    )
    assert response.status_code == 206
    assert len(response.content) == 65536
    assert elapsed < OPEN_BUDGET_SECONDS, (
        f"first video range took {elapsed:.2f}s; calls: {slow_telegram.stats()['calls']}"
    )


def test_document_download_starts_within_one_second(
    sarca: SarcaClient, storage: str, slow_telegram
) -> None:
    data = media.blob(2 * 1024 * 1024, seed=31)
    assert sarca.upload(storage, "manual.pdf", data, content_type="application/pdf").ok
    sarca.wait_for_file(storage, "manual.pdf")

    started = time.perf_counter()
    with sarca.http.stream(
        "GET",
        f"/api/storages/{storage}/files/download/manual.pdf",
        headers=sarca.headers,
    ) as response:
        assert response.status_code == 200
        next(response.iter_bytes())
        first_byte = time.perf_counter() - started
    assert first_byte < OPEN_BUDGET_SECONDS, f"time to first byte was {first_byte:.2f}s"


def test_preview_does_not_scale_with_original_size(
    sarca: SarcaClient, storage: str, slow_telegram, workdir
) -> None:
    """A 12 MP photo must open as fast as a small one — that is the point of the feature."""
    timings = {}
    for name, size in (("small.jpg", (800, 600)), ("huge.jpg", (4000, 3000))):
        data = media.big_photo(*size)
        assert sarca.upload(storage, name, data, content_type="image/jpeg").ok
        sarca.wait_for_file(storage, name)
        shutil.rmtree(workdir / "preview_cache", ignore_errors=True)
        response, elapsed = timed(sarca.preview, storage, name)
        assert response.status_code == 200
        timings[name] = elapsed

    assert timings["huge.jpg"] < OPEN_BUDGET_SECONDS, timings
    assert timings["huge.jpg"] < timings["small.jpg"] + 0.5, timings
