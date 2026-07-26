#!/usr/bin/env python3
"""Regression check: remote Settings ACL must allow Sync/Security commands on LAN :port URLs."""
from __future__ import annotations

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1] / "src-tauri"
CAP = ROOT / "capabilities" / "default.json"

REQUIRED = [
    "allow-default-gallery-path",
    "allow-pick-local-folder",
    "allow-set-client-prefs",
    "allow-get-client-prefs",
    "allow-add-binding",
    "allow-remove-binding",
    "allow-ensure-remote-folder",
    "allow-list-bindings",
    "allow-sync-now",
    "allow-sync-statuses",
    "allow-is-on-wifi",
    "allow-get-about",
    "allow-get-cache-size",
    "allow-clear-local-cache",
    "allow-platform-label",
]


def main() -> int:
    cap = json.loads(CAP.read_text())
    perms = set(cap.get("permissions") or [])
    missing = [p for p in REQUIRED if p not in perms]
    if missing:
        print("FAIL: capabilities missing:", ", ".join(missing), file=sys.stderr)
        return 1
    urls = (cap.get("remote") or {}).get("urls") or []
    if not any("*:*" in u for u in urls):
        print(
            "FAIL: remote.urls must include host:port wildcards (*:*); "
            f"got {urls!r}. Plain http://* does not match http://host:port/",
            file=sys.stderr,
        )
        return 1
    print(f"OK: {len(REQUIRED)} Sync/Security allows present; remote.urls={urls}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
