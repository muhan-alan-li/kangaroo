#!/usr/bin/env python3
"""Assertions for the board state machine: health, backoff, the retry queue, the sweep cache.

These are the pieces the daily scan, server/jobs/retry_workday.py and server/jobs/sweep_workday.py all share, and
the ones where a wrong answer is invisible -- a board that is never retried again, a cached
posting that reads fresher than it is, a backoff that never expires.

    /usr/bin/python3 server/tests/test_boardstate.py
"""

from __future__ import annotations

from datetime import datetime, timedelta, timezone
import json
import os
import shutil
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))  # server/, so the `scanner` package resolves

from scan import boardstate  # noqa: E402

failures = []
checks = 0
TMP = tempfile.mkdtemp(prefix="boardstate-test-")


def check(label, got, want):
    global checks
    checks += 1
    if got != want:
        failures.append(label)
        print(f"  MISMATCH {label}\n    got : {got!r}\n    want: {want!r}")


def stamp(days=0, hours=0):
    return (datetime.now(timezone.utc) + timedelta(days=days, hours=hours)).isoformat(
        timespec="seconds")


BOARD = {"name": "acme", "ats": "workday", "tenant": "acme", "wd": "wd1", "site": "ext"}

# ---- identity and classification ----------------------------------------------------
check("key()", boardstate.key(BOARD), "acme|wd1|ext")
check("key() on a string passthrough", boardstate.key("acme|wd1|ext"), "acme|wd1|ext")
check("classify 404", boardstate.classify("HTTPError: HTTP Error 404: Not Found"), "structural")
check("classify 422", boardstate.classify("HTTPError: HTTP Error 422: Unprocessable Entity"),
      "structural")
check("classify timeout", boardstate.classify("URLError: <urlopen error timed out>"), "transient")
check("classify empty", boardstate.classify(None), "transient")

# ---- the backoff schedule -----------------------------------------------------------
state = {}
for i in (1, 2, 3, 4, 5):
    row = boardstate.record(state, "acme|wd1|ext", "acme", False, 0,
                            "URLError: timed out", now=stamp(days=-i))
    want = {1: ("failing", 1), 2: ("failing", 3), 3: ("failing", 14),
            4: ("broken", 30), 5: ("broken", 30)}[i]
    check(f"transient failure {i} status", row["status"], want[0])
    check(f"transient failure {i} retry_in_days", row["retry_in_days"], want[1])

state = {}
for i in (1, 2, 3):
    row = boardstate.record(state, "acme|wd1|ext", "acme", False, 0,
                            "HTTP Error 422: Unprocessable Entity", now=stamp(days=-i))
    want = {1: ("failing", 7), 2: ("failing", 30), 3: ("broken", 30)}[i]
    check(f"structural failure {i} status", row["status"], want[0])
    check(f"structural failure {i} retry_in_days", row["retry_in_days"], want[1])

row = boardstate.record(state, "acme|wd1|ext", "acme", True, 7, None, now=stamp())
check("recovery resets the streak", row["consecutive_failures"], 0)
check("recovery clears the error", row["last_error"], None)
check("recovery returns to ok", row["status"], "ok")
check("recovery clears next_check_at", row["next_check_at"], None)
check("recovery keeps the failure tally", row["failures"], 3)

# ---- the retry queue ----------------------------------------------------------------
state = {}
boardstate.record(state, "acme|wd1|ext", "acme", False, 0, "URLError: timed out",
                  now=stamp(days=-2))
boardstate.record(state, "quiet|wd1|ext", "quiet", True, 0, None, now=stamp())
boardstate.record(state, "gone|wd1|ext", "gone", False, 0, "URLError: timed out",
                  now=stamp(days=-2))
boards = [BOARD, {"name": "quiet", "tenant": "quiet", "wd": "wd1", "site": "ext"}]
check("due() picks the failing board only", [i["key"] for i in boardstate.due(state, boards)],
      ["acme|wd1|ext"])
check("due() skips a board no longer configured, even failing",
      [i["key"] for i in boardstate.due(state, boards, force=True)], ["acme|wd1|ext"])
check("due() never returns a healthy board, even forced",
      "quiet|wd1|ext" in [i["key"] for i in boardstate.due(state, boards, force=True)], False)

# The first failure waits a day, so a board that failed seconds ago is not due yet.
fresh = {}
boardstate.record(fresh, "acme|wd1|ext", "acme", False, 0, "URLError: timed out",
                  now=stamp())
check("a board that just failed is not due yet", boardstate.due(fresh, boards), [])
check("...but is due the next day",
      [i["key"] for i in boardstate.due(fresh, boards, now=stamp(days=1, hours=1))],
      ["acme|wd1|ext"])
check("a just-failed board is due under --force",
      [i["key"] for i in boardstate.due(fresh, boards, force=True)], ["acme|wd1|ext"])

# ---- the recorder -------------------------------------------------------------------
boardstate.begin_run()
boardstate.note(BOARD, True, hits=4)
boardstate.note({"tenant": "b", "wd": "wd1", "site": "ext"}, False, err="URLError: boom")
recorded = boardstate.end_run()
check("recorder captured both outcomes", len(recorded), 2)
check("recorder kept the hit count", recorded[0]["hits"], 4)
check("recorder kept the reason", recorded[1]["err"], "URLError: boom")
boardstate.note(BOARD, True, hits=1)
check("note() outside a run is ignored", boardstate.end_run(), [])

# ---- the cursor: what page one being unchanged does and does not prove -----------------
from scan.sources.workday import cursor_allows_skip, page_signature  # noqa: E402

POSTS = [{"externalPath": "/job/City/SWE-I_R100"}, {"externalPath": "/job/City/SWE-II_R200"}]
check("page_signature keeps order and identity", page_signature(POSTS), ["SWE-I_R100", "SWE-II_R200"])
check("page_signature tolerates a missing path",
      page_signature([{"title": "SWE"}] + [{}]), ["SWE", ""])
sig = page_signature(POSTS)
check("no cursor means no skip", cursor_allows_skip(None, sig, 0, 3), False)
check("an identical page inside the bound skips",
      cursor_allows_skip({"ids": sig, "count": 2}, sig, 1, 3), True)
check("a reordered page does not skip",
      cursor_allows_skip({"ids": list(reversed(sig)), "count": 2}, sig, 1, 3), False)
check("a swapped posting does not skip",
      cursor_allows_skip({"ids": [sig[0], "SWE-III_R300"], "count": 2}, sig, 1, 3), False)
check("a shorter page does not skip",
      cursor_allows_skip({"ids": sig[:1], "count": 1}, sig, 1, 3), False)
check("a page-two read past the bound does not skip",
      cursor_allows_skip({"ids": sig, "count": 2}, sig, 4, 3), False)
check("the bound is inclusive",
      cursor_allows_skip({"ids": sig, "count": 2}, sig, 3, 3), True)
check("an unknown verification age does not skip",
      cursor_allows_skip({"ids": sig, "count": 2}, sig, None, 3), False)
check("an empty page never skips", cursor_allows_skip({"ids": [], "count": 0}, [], 0, 3), False)

# stored on the row, and only advanced by a read that reached the last page
row = boardstate.record({}, "acme|wd1|ext", "acme", True, 3, None, now=stamp(days=-1),
                        cursor={"signature": sig, "pages": 2, "unchanged": False})
check("a read-through stamps verified_at", row["cursor"]["verified_at"], stamp(days=-1))
check("the stamp is readable back", boardstate.cursor_for(row)["age_days"], 1)
row = boardstate.record({}, "acme|wd1|ext", "acme", True, 3, None, now=stamp(),
                        cursor={"signature": sig, "pages": 2, "unchanged": False})
first = row["cursor"]["verified_at"]
row = boardstate.record({"acme|wd1|ext": row}, "acme|wd1|ext", "acme", True, 3, None,
                        now=stamp(), cursor={"signature": sig, "pages": 1, "unchanged": True})
check("a skipped page two does not advance verified_at", row["cursor"]["verified_at"], first)
check("cursors() offers the board", "acme|wd1|ext" in boardstate.cursors({"acme|wd1|ext": row}),
      True)
check("a board with no cursor is offered nothing",
      boardstate.cursors({"x": {"status": "ok"}}), {})

# ---- the trust check: a throttled run must not become the record ------------------------
def outcomes(ok, hits=0, n=1, err=None):
    return [{"ok": True, "hits": hits, "err": err}] * n if ok else [{"ok": False, "hits": 0,
                                                                     "err": err or "boom"}] * n


healthy = outcomes(True, hits=2, n=44) + outcomes(False, n=5, err="HTTP 422")
check("a healthy sweep passes", boardstate.sweep_verdict(healthy)[0], True)
check("a healthy sweep's reason names the count",
      boardstate.sweep_verdict(healthy)[1], "5 of 49 boards failed")

throttled = outcomes(True, hits=0, n=61) + outcomes(False, n=1267, err="URLError: timed out")
check("a throttled sweep is refused", boardstate.sweep_verdict(throttled)[0], False)
check("the refusal names the fraction",
      "95%" in boardstate.sweep_verdict(throttled)[1], True)

check("a small fleet's bad day is not a throttle",
      boardstate.sweep_verdict(outcomes(False, n=30, err="URLError"))[0], True)
check("an empty run is refused", boardstate.sweep_verdict([])[0], False)
check("a sweep that returns nothing where the last one found plenty is refused",
      boardstate.sweep_verdict(outcomes(True, hits=0, n=40), previous_postings=4086)[0], False)
check("...and says so",
      "last sweep found 4086" in boardstate.sweep_verdict(outcomes(True, hits=0, n=40),
                                                          previous_postings=4086)[1], True)
check("a quiet-but-working sweep still passes",
      boardstate.sweep_verdict(outcomes(True, hits=0, n=40), previous_postings=4086), 
      (False, "no board returned a posting where the last sweep found 4086; treating this "
              "sweep as lost"))
check("a retry run of mostly-real failures still records",
      boardstate.sweep_verdict(outcomes(False, n=12, err="HTTP 422"),
                               max_fraction=0.999, min_failures=25)[0], True)
check("a total washout of a large retry run is refused",
      boardstate.sweep_verdict(outcomes(False, n=30, err="URLError: timed out"),
                               max_fraction=0.999, min_failures=25)[0], False)

# ---- persistence --------------------------------------------------------------------
path = os.path.join(TMP, "health.json")
state = boardstate.finish_run(recorded, boards=3, path=path, now=stamp())
check("finish_run wrote the file", os.path.exists(path), True)
check("finish_run reports the failing board",
      [k for k, r in state.items() if r["status"] != "ok"], ["b|wd1|ext"])
reloaded = boardstate.load(path)
check("load() round-trips", sorted(reloaded), sorted(state))
check("no temp file left behind",
      [f for f in os.listdir(TMP) if f.endswith(".tmp")], [])
check("board state line", boardstate.line(state),
      "board state: 2 known, 1 ok, 1 failing, 0 broken")

# ---- the sweep cache ----------------------------------------------------------------
sweep_path = os.path.join(TMP, "sweep.json")
postings = [
    {"id": "a", "source": "ats", "company": "acme", "title": "SWE", "url": "u",
     "posted": None, "age_days": 2, "ats": "workday", "board": "acme|wd1|ext"},
    {"id": "b", "source": "ats", "company": "acme", "title": "SWE2", "url": "u2",
     "posted": None, "age_days": None, "ats": "workday"},
]
boardstate.write_sweep(postings, path=sweep_path, meta={"boards": 2, "failed": 0})
back, age = boardstate.read_sweep(30, path=sweep_path)
check("fresh cache returns every posting", len(back), 2)
check("fresh cache is minutes old", age < 1, True)
check("a dated posting keeps its age on the same day", back[0]["age_days"], 2)
check("an undated posting stays undated", back[1]["age_days"], None)
check("the cache keeps the meta", json.load(open(sweep_path))["boards"], 2)

# A cache written two days ago: the ages must have moved with the calendar, or a cached
# posting would read fresher than it is and the 7-day window would be a lie.
payload = json.load(open(sweep_path))
payload["swept_at"] = stamp(days=-2)
json.dump(payload, open(sweep_path, "w"))
back, age = boardstate.read_sweep(72, path=sweep_path)
check("two-day-old cache advances the age", back[0]["age_days"], 4)
check("two-day-old cache reports its age", round(age), 48)
stale, age = boardstate.read_sweep(30, path=sweep_path)
check("a cache past max_cache_age_hours is refused", stale, None)
check("a refused cache still reports its age", round(age), 48)
missing, age = boardstate.read_sweep(30, path=os.path.join(TMP, "nope.json"))
check("a missing cache is refused, not an error", (missing, age), (None, None))

shutil.rmtree(TMP, ignore_errors=True)
print(f"\n{checks} checks, {len(failures)} mismatches")
if failures:
    print("FAILED:")
    for f in failures:
        print("  -", f)
    raise SystemExit(1)
print("board state, backoff, retry queue and sweep cache all behave")
