#!/usr/bin/env python3
"""Regression check on the remote Settings ACL.

Sarca's client webview loads the *server's* UI, so the remote page sits on the
far side of a trust boundary. Two things must stay true at once, and this
script guards both directions:

1. The Sync/Security commands are still reachable, so the server's Settings
   page keeps working -- they are declared in the local capability and listed
   in `REMOTE_SETTINGS_COMMANDS`.
2. No capability file grants those commands to a *static URL wildcard*. Remote
   access is handed out at runtime by `grant_remote_capability`, scoped to the
   single origin the user actually connected to. A `"urls": ["http://*:*"]` in
   `capabilities/*.json` would hand the same commands to any http origin the
   webview can be pointed at, which is the hole this check now exists to catch.

Until 2026-08 this script asserted the opposite of (2): it *required* the
`*:*` wildcard. That was correct only while the ACL was static, and it kept
failing CI after the grant moved into `remote_ipc.rs`.
"""
from __future__ import annotations

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1] / "src-tauri"
CAP_DIR = ROOT / "capabilities"
CAP = CAP_DIR / "default.json"
REMOTE_IPC = ROOT / "src" / "remote_ipc.rs"

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
    "allow-set-binding-enabled",
    "allow-update-binding-local-path",
]

# Commands that must never become remote-reachable: they re-point the client at
# a different server, or read where it has already been.
FORBIDDEN_REMOTE = ["connect", "get_url_history"]


def remote_settings_commands() -> list[str]:
    """Parse `REMOTE_SETTINGS_COMMANDS` out of remote_ipc.rs."""
    src = REMOTE_IPC.read_text()
    m = re.search(r"REMOTE_SETTINGS_COMMANDS:\s*&\[&str\]\s*=\s*&\[(.*?)\];", src, re.S)
    if not m:
        return []
    return re.findall(r'"([a-z0-9_]+)"', m.group(1))


def main() -> int:
    failures: list[str] = []

    cap = json.loads(CAP.read_text())
    perms = set(cap.get("permissions") or [])
    missing = [p for p in REQUIRED if p not in perms]
    if missing:
        failures.append(f"capabilities missing: {', '.join(missing)}")

    # (2) No static remote grant, in this or any other capability file.
    for path in sorted(CAP_DIR.glob("*.json")):
        data = json.loads(path.read_text())
        urls = (data.get("remote") or {}).get("urls") or []
        if urls:
            failures.append(
                f"{path.name} declares a static remote.urls grant {urls!r}. "
                "Remote access must be granted at runtime by "
                "grant_remote_capability, scoped to the connected origin."
            )

    # (1) The runtime allow-list still covers the Settings surface...
    cmds = remote_settings_commands()
    if not cmds:
        failures.append("could not parse REMOTE_SETTINGS_COMMANDS from remote_ipc.rs")
    else:
        for want in REQUIRED:
            cmd = want.removeprefix("allow-").replace("-", "_")
            if cmd not in cmds:
                failures.append(f"REMOTE_SETTINGS_COMMANDS missing {cmd!r}")
        # ...and nothing that would let a remote page move the trust boundary.
        for bad in FORBIDDEN_REMOTE:
            if bad in cmds:
                failures.append(
                    f"REMOTE_SETTINGS_COMMANDS must not expose {bad!r} to remote pages"
                )

    if failures:
        for f in failures:
            print(f"FAIL: {f}", file=sys.stderr)
        return 1

    print(
        f"OK: {len(REQUIRED)} Sync/Security allows present; "
        f"{len(cmds)} runtime remote commands; no static remote.urls grant"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
