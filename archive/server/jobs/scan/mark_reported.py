#!/usr/bin/env python3
"""Mark postings as reported in the dedupe ledger.

The no-agent scan job calls this as `--all-unreported` once `server/scan/engine.py` has written
`data/state/last_scan.json` and the portal feed, so the next run's "new" set stays small. Until an
id is marked, scan.py keeps re-emitting it -- so a failed scan re-reports rather than silently
dropping a posting.

The ledger reader and writer are `server/scan/ledger.py`'s, not a second copy: one parser, and
the pid-tagged temp name that keeps a bare `mark-reported` concurrent with a scan from
renaming a half-written ledger over the scan's own save.

    python3 -m server mark-reported abc123 def456
    python3 -m server mark-reported --all-unreported
"""
from __future__ import annotations

import argparse
import sys
from datetime import datetime, timezone

from ...scan.ledger import load_ledger, save_ledger


def main(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument("ids", nargs="*", help="posting ids to mark as reported")
    ap.add_argument("--all-unreported", action="store_true")
    args = ap.parse_args(argv)

    entries = load_ledger()
    now = datetime.now(timezone.utc).isoformat(timespec="seconds")

    if args.all_unreported:
        targets = [k for k, v in entries.items() if not v.get("reported")]
    else:
        targets = args.ids

    # `--all-unreported` is idempotent: a quiet scan that found nothing new still leaves the
    # ledger fully marked, so "nothing to mark" is success, not an error. The no-agent scan
    # job runs under `set -e` and would otherwise report a failed run on every quiet slot.
    # Passing explicit ids and getting none is still a usage error.
    if not targets and not args.all_unreported:
        print("nothing to mark (pass ids or --all-unreported)", file=sys.stderr)
        return 1

    marked, missing = 0, []
    for pid in targets:
        rec = entries.get(pid)
        if rec is None:
            missing.append(pid)
            continue
        rec["reported"] = True
        rec["reported_at"] = now
        marked += 1

    save_ledger(entries)
    print(f"marked {marked} posting(s) as reported")
    if missing:
        print(f"unknown ids ignored: {', '.join(missing)}", file=sys.stderr)
    unreported = sum(1 for v in entries.values() if not v.get("reported"))
    print(f"ledger: {len(entries)} total, {unreported} still unreported")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
