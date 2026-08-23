"""Drive the real client sync engine (crates/sarca-sync) from the tests.

Uses the crate's `headless` example as a CLI so auto-upload is exercised through
the same code path the desktop/mobile client runs, not a Python re-implementation.
"""

from __future__ import annotations

import json
import os
import subprocess
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from helpers.server import repo_root

ROOT = repo_root()


def build_driver() -> Path:
    """Build (once) and return the headless sync driver binary."""
    name = "headless.exe" if os.name == "nt" else "headless"
    binary = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target")) / "debug/examples" / name
    if binary.is_file() and os.environ.get("SARCA_SKIP_BUILD") == "1":
        return binary
    subprocess.run(
        ["cargo", "build", "-p", "sarca-sync", "--example", "headless"],
        cwd=ROOT,
        check=True,
    )
    if not binary.is_file():
        raise RuntimeError(f"built sync driver missing at {binary}")
    return binary


@dataclass
class SyncRun:
    """Result of one headless sync invocation."""

    raw: dict[str, Any]

    @property
    def statuses(self) -> list[dict[str, Any]]:
        return self.raw.get("statuses", [])

    @property
    def status(self) -> dict[str, Any]:
        assert self.statuses, f"no binding status in {self.raw}"
        return self.statuses[0]

    @property
    def errors(self) -> list[str]:
        errors = list(self.raw.get("tick_errors", []))
        errors += [s["last_error"] for s in self.statuses if s.get("last_error")]
        return errors

    @property
    def scanned(self) -> int:
        return self.status.get("scanned", 0)

    @property
    def pending(self) -> int:
        return self.status.get("pending", 0)

    @property
    def already_synced(self) -> int:
        return self.status.get("already_synced", 0)

    @property
    def uploaded_paths(self) -> set[str]:
        items = self.raw.get("transfers", {}).get("items", [])
        return {
            (f"{i['path']}/{i['name']}" if i["path"] else i["name"])
            for i in items
            if i["direction"] == "upload" and i["status"] == "done"
        }


def run_sync(
    *,
    base_url: str,
    email: str,
    password: str,
    storage_id: str,
    local_dir: Path,
    data_dir: Path,
    remote_root: str = "",
    mode: str = "auto_upload",
    ticks: int = 1,
    binding_id: str = "e2e-binding",
    retry_failed: bool = False,
    timeout: float = 240.0,
) -> SyncRun:
    driver = build_driver()
    result = subprocess.run(
        [
            str(driver),
            "--server", base_url,
            "--email", email,
            "--password", password,
            "--storage-id", storage_id,
            "--local", str(local_dir),
            "--remote-root", remote_root,
            "--mode", mode,
            "--ticks", str(ticks),
            "--data-dir", str(data_dir),
            "--binding-id", binding_id,
            "--retry-failed", "1" if retry_failed else "0",
        ],
        cwd=ROOT,
        capture_output=True,
        text=True,
        # The driver reports paths as UTF-8 JSON; decoding with the host ANSI
        # codepage (Windows) throws on any non-Latin-1 filename.
        encoding="utf-8",
        errors="replace",
        timeout=timeout,
    )
    if result.returncode != 0:
        raise RuntimeError(
            f"sync driver failed ({result.returncode}):\n{result.stdout}\n{result.stderr}"
        )
    last_line = [ln for ln in result.stdout.splitlines() if ln.startswith("{")][-1]
    return SyncRun(raw=json.loads(last_line))
