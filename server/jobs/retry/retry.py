#!/usr/bin/env python3
"""Re-check only the Workday boards that did not answer, on their own beat.

The daily scan reads 1,328 boards and about fifty of them fail, for two different reasons.
Most are the network or Workday's throttle, and they answer fine an hour later. The rest have
a wrong career-site name in their config -- a Workday 422 -- and will never answer, so no
amount of retrying helps and only a fix to the entry does.

This pass exists so neither case costs the daily run anything:

  - it runs on a different clock from the scan (weekly, at a quiet hour), so the two never
    compete for the same per-IP budget. Running it beside the scan is the documented way to
    make the throttle worse and to get a failure count that describes the probe, not the scan.
  - it looks only at the boards `server/scan/boardstate.py` marked as failing or broken, and
    only when their backoff has come up: a blip is retried tomorrow, a board that failed twice
    in three days, and a board that keeps failing monthly.
  - a board that answers is cleared to `ok` and rejoins the daily sweep by itself. A board
    whose config is wrong is marked `broken` and named in the output once, so a human can fix
    or drop the entry instead of the pass retrying it forever.

It is deliberately a small pool (8 workers by default). The point is a second look at fifty
boards, not a second sweep of the fleet.

    python3 -m server retry                 # the boards whose beat has come up
    python3 -m server retry --dry-run       # say what would be checked, touch nothing
    python3 -m server retry --force         # every failing board, ignoring the backoff
    python3 -m server retry --json
    python3 -m server retry --limit 10      # a quick look
"""

from __future__ import annotations

import argparse
import os
import re
import sys
import time
from concurrent import futures

from ...scan import boardstate, boards  # noqa: E402
from ...scan.sources.workday import _probe_board  # noqa: E402


def role_regex(opts):
    words = tuple(opts.get("title_words") or ("engineer", "engineering", "developer",
                                              "programmer", "swe", "sde"))
    return re.compile(r"\b(" + "|".join(re.escape(w) for w in words) + r")\b", re.I)


def reason_of(err):
    """A short reason for grouping: the HTTP status when there is one, else the exception.

    Grouping on the exception type alone puts a 422 (a wrong career-site name, a config fix)
    in the same bucket as a 403 (refused) and a timeout (the network), which are three
    different jobs for whoever reads the output.
    """
    text = str(err or "no answer")
    code = re.search(r"\b(\d{3})\b", text)
    if code:
        return f"HTTP {code.group(1)}"
    kind = re.match(r"([A-Za-z]+Error)", text)
    return kind.group(1) if kind else text[:48]


def main(argv=None):
    ap = argparse.ArgumentParser(description="Re-check the Workday boards that did not answer.")
    ap.add_argument("--workers", type=int, default=8,
                    help="pool size (default 8: this is a second look, not a sweep)")
    ap.add_argument("--limit", type=int, default=0, help="cap how many boards to check")
    ap.add_argument("--force", action="store_true",
                    help="check every failing board now, ignoring the backoff")
    ap.add_argument("--dry-run", action="store_true", help="list the boards, touch nothing")
    ap.add_argument("--json", action="store_true", help="machine-readable result")
    args = ap.parse_args(argv)

    state = boardstate.load()
    if not state:
        sys.stderr.write(
            "[retry] no board state yet (data/state/board_health.json).\n"
            "        The daily scan writes it on its next run; nothing to retry until then.\n")
        return 0

    configured = boards.load_workday_boards()
    queue = boardstate.due(state, configured, force=args.force)
    if args.limit:
        queue = queue[:args.limit]

    total, ok, failing, broken = boardstate.summary(state)
    sys.stderr.write(f"[retry] {boardstate.line(state)}\n")
    if not queue:
        sys.stderr.write("[retry] nothing is due; every failing board is inside its backoff\n")
        return 0

    if args.dry_run:
        for item in queue:
            row = item["state"]
            sys.stderr.write(f"[retry] would check {item['key']} "
                             f"({row.get('status')}, {row.get('retry_in_days')}d beat, "
                             f"last: {row.get('last_error')})\n")
        sys.stderr.write(f"[retry] {len(queue)} board(s) due, dry run, nothing touched\n")
        return 0

    opts = boards.workday_opts()
    page_size = int(opts.get("page_size") or 20)
    max_pages = int(opts.get("max_pages") or 2)
    window = int(opts.get("window_days") or 7)
    timeout = int(opts.get("timeout") or 25)
    search_text = str(opts.get("search_text", "engineer"))
    role_re = role_regex(opts)

    started = time.perf_counter()
    results = []
    workers = max(1, args.workers)
    with futures.ThreadPoolExecutor(max_workers=workers) as pool:
        pending = {
            pool.submit(_probe_board, item["board"], page_size, max_pages, window, timeout,
                        role_re, search_text): item
            for item in queue
        }
        for future in futures.as_completed(pending):
            item = pending[future]
            try:
                # _probe_board returns five values (rows, ok, dropped, err, stat). Unpacking
                # four made every due board raise here and be recorded as a failed probe with
                # a bogus ValueError -- the whole pass would silently "fail" board by board.
                rows, board_ok, _dropped, err, _stat = future.result()
            except Exception as exc:  # noqa: BLE001
                rows, board_ok, err = [], False, f"{type(exc).__name__}: {exc}"
            results.append({"key": item["key"], "name": item["state"].get("name"),
                            "ok": bool(board_ok), "hits": len(rows), "err": err,
                            "was": item["state"].get("status")})
    wall = time.perf_counter() - started

    # A run that lost every board it checked is the throttle, not a pile of dead configs. The
    # bar is higher here than in a sweep: this pass only ever looks at boards that already
    # failed, so a high failure rate is expected and only a total washout is a signal. Its
    # failures are not recorded -- the retry queue and the board list are health, and poisoning
    # them with a bad afternoon would take a week to unwind.
    verdict_ok, reason = boardstate.sweep_verdict(results, max_fraction=0.999, min_failures=25)
    if not verdict_ok:
        sys.stderr.write(f"[retry] not recording this run: {reason}\n"
                         f"[retry] board state is unchanged. Re-run when Workday answers again.\n")
        return 2

    recovered = [r for r in results if r["ok"]]
    still = [r for r in results if not r["ok"]]
    for outcome in results:
        boardstate.record(state, outcome["key"], outcome.get("name"),
                          outcome["ok"], outcome["hits"], outcome.get("err"))
    boardstate.save(state)

    for r in recovered:
        sys.stderr.write(f"[retry] recovered {r['key']} ({r['name']}): {r['hits']} hit(s)\n")

    payload = {
        "checked": len(results),
        "recovered": len(recovered),
        "still_failing": len(still),
        "wall_seconds": round(wall, 1),
        "board_state": {"known": total, "ok": ok, "failing": failing, "broken": broken},
        "now_ok": [r["key"] for r in recovered],
        "now_failing": [{"key": r["key"], "name": r.get("name"), "error": r.get("err")}
                        for r in still],
    }
    if args.json:
        import json as _json
        print(_json.dumps(payload, indent=2))
        return 0

    print(f"retry pass: {len(results)} board(s) checked in {wall:.1f}s")
    print(f"  recovered: {len(recovered)}")
    print(f"  still failing: {len(still)}")
    if still:
        groups = {}
        for r in still:
            groups.setdefault(reason_of(r.get("err")), []).append(r["key"])
        print("  by reason:")
        for reason, keys in sorted(groups.items(), key=lambda kv: -len(kv[1])):
            print(f"    {len(keys):3}  {reason}")
            for k in keys[:5]:
                print(f"         {k}")
            if len(keys) > 5:
                print(f"         ... and {len(keys) - 5} more")
    total, ok, failing, broken = boardstate.summary(state)
    print(f"  board state now: {total} known, {ok} ok, {failing} failing, {broken} broken")
    broken_now = [k for k, r in sorted(state.items()) if r.get("status") == "broken"]
    if broken_now:
        print(f"  broken entries needing a fix or a drop ({len(broken_now)}):")
        for k in broken_now[:15]:
            row = state[k]
            print(f"    {k}  ({row.get('name')})  {row.get('last_error')}")
        if len(broken_now) > 15:
            print(f"    ... and {len(broken_now) - 15} more")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
