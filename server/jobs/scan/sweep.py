#!/usr/bin/env python3
"""The Workday sweep, as its own run.

Workday is 1,328 boards and the daily scan's slowest source -- about two minutes of the
three -- for the largest board list in the workspace and, historically, three submitted
applications. Splitting it off does not make the requests cheaper; it moves them off the
critical path, so the daily run spends its time on iCIMS and the JSON vendors (under a
minute together) and reads Workday out of this pass's output file instead.

What it writes, all atomic and all under data/state/:

    workday_sweep.json   the sweep's postings, which server/scan/engine.py reads when fresh
    board_health.json    per-board outcome: answered, did not answer, and how long until
                         it is worth looking at again

The daily scan falls back to sweeping Workday itself when the cache is missing or past
`max_cache_age_hours`, so skipping this pass slows the scan down and never loses coverage.

    python3 -m server sweep                 # sweep, write the cache and the health
    python3 -m server sweep --dry-run       # sweep, print, write nothing
    python3 -m server sweep --workers 8     # a quieter pool
    python3 -m server sweep --limit 100     # part of the fleet
    python3 -m server sweep --json

Run it at an hour the daily scan is not using. Two passes at once is the documented way to
trip Workday's per-IP throttle and to get a failure count that describes neither.
"""

from __future__ import annotations

import argparse
import os
import sys
import time

from ...scan import boardstate, boards  # noqa: E402
from ...scan.sources.workday import fetch_workday_boards  # noqa: E402


def main(argv=None):
    ap = argparse.ArgumentParser(description="Sweep the Workday boards into the cache.")
    ap.add_argument("--workers", type=int, default=0,
                    help="override data/config/sources.yaml's workday workers")
    ap.add_argument("--limit", type=int, default=0, help="sweep only the first N boards")
    ap.add_argument("--dry-run", action="store_true",
                    help="sweep and report, but write no state")
    ap.add_argument("--json", action="store_true", help="machine-readable result")
    args = ap.parse_args(argv)

    opts = dict(boards.workday_opts())
    if args.workers:
        opts["workers"] = args.workers

    all_boards = boards.load_workday_boards()
    partial = bool(args.limit) and len(all_boards) > args.limit
    batch = all_boards[:args.limit] if args.limit else all_boards
    if not batch:
        sys.stderr.write("[workday] no boards configured; nothing to sweep\n")
        return 1

    # The cursor: a board whose keyword page one is identical to the last time its page two
    # was read does not need page two this time. That halves the requests on an unchanged
    # fleet, and it is bounded -- `cursor_max_age_days` caps how stale a page-two read may be,
    # so a posting that lands past position 20 without disturbing page one is still picked up
    # within the bound. Nothing is reused unless the previous sweep is on disk to reuse from.
    cursor_max_age = int(opts.get("cursor_max_age_days", 3))
    state = boardstate.load()
    cursors = boardstate.cursors(state, cursor_max_age)
    prev_postings, prev_age = (boardstate.read_sweep(cursor_max_age * 24 + 24) if cursors
                              else (None, None))
    if prev_postings is None:
        cursors = {}

    sys.stderr.write(f"[workday] sweeping {len(batch)} boards "
                     f"({opts.get('workers')} workers, {opts.get('max_pages')} page(s) each, "
                     f"{len(cursors)} with a usable cursor)\n")
    boardstate.begin_run()
    started = time.perf_counter()
    postings = fetch_workday_boards(batch, {"workday": opts}, cursors=cursors)
    wall = time.perf_counter() - started
    recorded = boardstate.end_run()

    # An unchanged board's page-one rows are the same rows it returned last sweep, so they
    # are replaced wholesale by the previous sweep's set for that board -- which is page one
    # plus the page two that was not re-read, and is a superset of what just came back.
    skipped = {o["key"] for o in recorded if (o.get("cursor") or {}).get("unchanged")}
    reused = []
    if skipped and prev_postings:
        prev_by_board = {}
        for posting in prev_postings:
            prev_by_board.setdefault(posting.get("board"), []).append(posting)
        fresh = [p for p in postings if p.get("board") not in skipped]
        reused = [p for key in sorted(skipped) for p in prev_by_board.get(key, [])]
        postings = fresh + reused

    failed = [o for o in recorded if not o["ok"]]
    quiet = [o for o in recorded if o["ok"] and not o["hits"]]
    productive = [o for o in recorded if o["ok"] and o["hits"]]

    # Trust check before anything is written. A sweep that lost a quarter of the fleet is the
    # throttle, and its cache would be a quiet day the scan did not have, while its failure
    # records would send the retry pass after hundreds of boards that were never broken.
    verdict_ok, reason = boardstate.sweep_verdict(
        recorded, previous_postings=len(prev_postings) if prev_postings else None)
    if not verdict_ok:
        sys.stderr.write(f"[workday] REFUSED this sweep: {reason}\n"
                         f"[workday] nothing written -- the previous cache and board state "
                         f"are unchanged. Re-run when Workday is answering again.\n")
        return 2

    if not args.dry_run:
        boardstate.finish_run(recorded, boards=len(batch), write=True)
        meta = {"boards": len(batch), "outcomes": len(recorded),
                "productive": len(productive), "quiet": len(quiet), "failed": len(failed),
                "page_two_skipped": len(skipped), "carried_over": len(reused),
                "wall_seconds": round(wall, 1), "verdict": reason}
        if partial:
            # A partial sweep must never leave a partial cache behind. The daily scan reads
            # this file as the whole fleet's answer, so a --limit run writing it would narrow
            # Workday coverage to the boards it happened to look at -- silently, because the
            # cache would still look fresh. Health is still recorded for the boards it did
            # check; the cache is not touched.
            print(f"  partial sweep ({len(batch)} of {len(all_boards)} boards): "
                  f"health recorded, cache left as it was")
        else:
            boardstate.write_sweep(postings, meta=meta)
    state = boardstate.load()

    if args.json:
        import json as _json
        print(_json.dumps({
            "boards": len(batch), "postings": len(postings), "wall_seconds": round(wall, 1),
            "productive_boards": len(productive), "quiet_boards": len(quiet),
            "failed_boards": len(failed), "page_two_skipped": len(skipped),
            "postings_carried_over": len(reused), "dry_run": bool(args.dry_run),
            "board_state": dict(zip(("known", "ok", "failing", "broken"),
                                    boardstate.summary(state))),
        }, indent=2))
        return 0

    print(f"workday sweep: {len(batch)} boards in {wall:.1f}s -> {len(postings)} postings")
    print(f"  answered with postings: {len(productive)}")
    print(f"  answered, nothing in window: {len(quiet)}")
    print(f"  did not answer: {len(failed)}")
    if skipped:
        print(f"  page two skipped on {len(skipped)} unchanged board(s); "
              f"{len(reused)} posting(s) carried over from the sweep {prev_age:.1f}h old")
    print(f"  {boardstate.line(state)}")
    if args.dry_run:
        print("  dry run: no cache and no health written")
    elif partial:
        print(f"  wrote {os.path.relpath(boardstate.HEALTH_PATH)}; "
              f"the sweep cache was left alone (partial run)")
    else:
        print(f"  wrote {os.path.relpath(boardstate.SWEEP_PATH)} and "
              f"{os.path.relpath(boardstate.HEALTH_PATH)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
