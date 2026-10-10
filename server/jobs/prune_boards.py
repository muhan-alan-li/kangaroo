#!/usr/bin/env python3
"""Remove dead boards from config/workday_boards.yaml.

The user's rule, 2026-10-07: a board that keeps failing is just noise, so remove it. "Keep
failing" cannot mean "any failure", because Workday throttles by IP and a bad afternoon makes
every board fail at once -- `server/jobs/retry_workday.py` proved exactly that on the day this was written
(59 of 59 boards failed, 100%, on a targeted probe). Removing on that signal would delete the
whole fleet.

So the trigger is narrower, and is the one thing a retry can never fix: a board whose failure
is **structural** -- a Workday 404 or 422, which by `boardstate.STRUCTURAL_RE` means the
career-site name in the tenant/wd/site triple is wrong. Those have already lost their backoff
(they are `broken`), they fail every sweep and every retry, and no amount of re-checking will
change that. Transient failures (429/403/timeouts) are left alone; the weekly `job-retry`
keeps working them.

Both `config/workday_boards.yaml` and the board's row in `state/board_health.json` are pruned,
so the removed board stops counting as "failing" on the dashboard and never enters the retry
queue again. The config is rewritten in place; prior board configs are not archived.

Removed keys are also appended to `config/workday_tombstones.yaml`, which
`server/jobs/harvest/workday.py` consults when it rebuilds the board file: the dead triples
came from feed URLs, so without the tombstones the next harvest would re-add them.
Delete a tombstone line by hand to let a board back in.

    /usr/bin/python3 -m server prune --dry-run   # what would go, and why
    /usr/bin/python3 -m server prune             # structural-broken boards
    /usr/bin/python3 -m server prune --key dell|wd1|External   # one, by hand
"""
from __future__ import annotations

import argparse
import json
import os
import re
import sys

from ..config import ROOT

from ..scan import boardstate  # noqa: E402

CONFIG = os.path.join(ROOT, "config", "workday_boards.yaml")
HEALTH = os.path.join(ROOT, "state", "board_health.json")


def _field(line, name):
    match = re.search(rf'\b{name}:\s*"?([^",}}\s]+)"?', line)
    return match.group(1) if match else None


def key_of_line(line):
    """The `tenant|wd|site` key an entry line carries, or None for a non-entry line.

    Normalisation is `boardstate.key_of`'s, so the key built from config text always
    matches the key the parser (and the health row) uses -- e.g. unquoted `001` parses
    as the integer 1.
    """
    if not line.lstrip().startswith("-"):
        return None
    tenant, wd, site = _field(line, "tenant"), _field(line, "wd"), _field(line, "site")
    if tenant and wd and site:
        return boardstate.key_of(tenant, wd, site)
    return None


def structural_dead(health):
    """{key: last_error} for boards that are broken on a structural failure.

    `broken` + `structural` is the conjunction that matters: the board exhausted its retry
    backoff AND the failure is a config error, not the network.
    """
    out = {}
    for key, row in (health or {}).items():
        if row.get("status") == "broken" and row.get("error_kind") == "structural":
            out[key] = row.get("last_error") or "structural failure"
    return out


def main(argv=None):
    ap = argparse.ArgumentParser(description="Prune dead Workday boards.")
    ap.add_argument("--dry-run", action="store_true", help="list what would go, change nothing")
    ap.add_argument("--key", action="append", default=[],
                    help="remove one board by tenant|wd|site (repeatable)")
    args = ap.parse_args(argv)

    try:
        with open(HEALTH, encoding="utf-8") as fh:
            health_payload = json.load(fh)
    except (OSError, json.JSONDecodeError):
        sys.stderr.write(f"cannot read {HEALTH}; run a scan first\n")
        return 1
    health = health_payload.get("boards") or {}

    dead = structural_dead(health)
    # An explicit key is honoured even when it is not structural-broken: the operator asked
    # for it by name. It is still reported and backed up the same way.
    targets = set(dead) | set(args.key)

    if not targets:
        sys.stderr.write("[prune] no structural-broken boards; nothing to remove\n")
        return 0

    with open(CONFIG, encoding="utf-8") as fh:
        text = fh.read()
    lines = text.splitlines()

    kept, removed = [], []
    for line in lines:
        key = key_of_line(line)
        if key and key in targets:
            removed.append(key)
            continue
        kept.append(line)

    missing = sorted(targets - set(removed))
    if args.dry_run:
        for key in sorted(removed):
            sys.stderr.write(f"[prune] would remove {key}  ({dead.get(key, 'requested')})\n")
        for key in missing:
            sys.stderr.write(f"[prune] not found in config: {key}\n")
        sys.stderr.write(f"[prune] {len(removed)} board(s) would be removed; dry run\n")
        return 0

    if not removed:
        sys.stderr.write("[prune] nothing matched in the config; no change\n")
        return 0

    with open(CONFIG, "w", encoding="utf-8") as fh:
        fh.write("\n".join(kept) + "\n")

    # Remember what went, so the next `harvest workday` does not re-add it from the feed
    # it originally came from. An explicit --key is remembered too: the operator asked for
    # it to be gone, and harvest cannot tell it apart from a live board.
    boardstate.add_tombstones({k: dead.get(k, "requested") for k in removed})

    # Drop the health rows too, so a removed board cannot keep reporting as "failing" on the
    # dashboard or be offered to the retry queue. board_health is written atomically, and
    # save() stamps its own `updated_at`. A target missing from the config text is already
    # gone from the fleet, so its row goes as well -- otherwise it counts as failing on the
    # dashboard forever while the retry queue skips it.
    for key in removed:
        health.pop(key, None)
    for key in missing:
        health.pop(key, None)
    boardstate.save(health, extra={"boards_total": len(health)})

    print(f"pruned {len(removed)} board(s) from {os.path.relpath(CONFIG)}")
    for key in sorted(removed):
        print(f"  - {key}  ({dead.get(key, 'requested')})")
    if missing:
        print(f"  not found in config (health row dropped): {', '.join(missing)}")
    print(f"  board_health now holds {len(health)} boards")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
