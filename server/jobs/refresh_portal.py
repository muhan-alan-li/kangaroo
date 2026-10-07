#!/usr/bin/env python3
"""Rebuild the portal feed from the last scan, without running a scan.

The dashboard reads `state/portal_feed.jsonl`, which a scan writes at its end. This does the
same job from `state/last_scan.json`, so the portal can be brought up to date (or re-enriched
after a change to the keyword vocabulary) in seconds instead of a four-minute sweep.

    /usr/bin/python3 -m server refresh
    /usr/bin/python3 -m server refresh --budget 400   # fetch more descriptions this pass
    /usr/bin/python3 -m server refresh --dry-run      # report only, write nothing
    /usr/bin/python3 -m server refresh --summary      # JSON summary on stdout
"""
from __future__ import annotations

import argparse
import json
import os
import sys

from ..scan import portal  # noqa: E402
from ..scan.core import ROOT, load_yaml  # noqa: E402

SCAN = os.path.join(ROOT, "state", "last_scan.json")


def main(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument("--dry-run", action="store_true", help="report only, write nothing")
    ap.add_argument("--budget", type=int, default=portal.DESC_BUDGET,
                    help="descriptions to fetch this pass (default 250)")
    ap.add_argument("--summary", action="store_true", help="print the summary as JSON")
    args = ap.parse_args(argv)

    try:
        with open(SCAN, encoding="utf-8") as fh:
            scan = json.load(fh)
    except FileNotFoundError:
        sys.stderr.write(f"no scan at {SCAN}; run `python3 -m server scan` first\n")
        return 1

    postings = scan.get("matched") or []
    filters = load_yaml("config", "filters.yaml")
    summary = portal.refresh(postings, filters=filters, ran_at=scan.get("ran_at"),
                             no_write=args.dry_run, desc_budget=args.budget)
    if args.summary:
        print(json.dumps(summary, indent=2))
    else:
        print(portal.line(summary))
        print(f"  feed: {portal.FEED_PATH}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
