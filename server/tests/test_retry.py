#!/usr/bin/env python3
"""Assertions for the weekly retry pass: the probe-result contract and the unpack.

server/jobs/retry/retry.py unpacks what server/scan/sources/workday.py:_probe_board
returns. Those two disagreed once -- four names for five values -- so every due board
raised ValueError inside the worker, was caught one line later, and was recorded as a
failed probe with a bogus error. Nothing was due for weeks, so no run ever showed it.
These pin both ends of the contract without touching the network.

    /usr/bin/python3 server/tests/test_retry.py
"""

from __future__ import annotations

import contextlib
import io
import os
import sys
from datetime import datetime, timedelta, timezone

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, ROOT)  # the workspace root, so the `server` package resolves

from server.jobs.retry import retry as retry_workday  # noqa: E402
from server.scan import boardstate  # noqa: E402
from server.scan.sources import workday  # noqa: E402

failures = []
checks = 0


def check(label, got, want):
    global checks
    checks += 1
    if got != want:
        failures.append(label)
        print(f"  MISMATCH {label}\n    got : {got!r}\n    want: {want!r}")


def stamp(days=0):
    return (datetime.now(timezone.utc) + timedelta(days=days)).isoformat(timespec="seconds")


BOARD = {"name": "acme", "ats": "workday", "tenant": "acme", "wd": "wd1", "site": "ext"}

# ---- the producer end: _probe_board answers five values ------------------------------
real_workday_board = workday._workday_board
workday._workday_board = lambda *a, **k: ([], True, 0)  # noqa: E731 -- no network
try:
    result = workday._probe_board(BOARD, 20, 2, 7, 25, None, "engineer")
finally:
    workday._workday_board = real_workday_board
check("_probe_board returns five values", len(result), 5)
rows, ok, dropped, err, stat = result
check("_probe_board passes rows through", (rows, ok, dropped, err), ([], True, 0, None))
check("_probe_board always carries a stat dict", isinstance(stat, dict), True)

# ---- the consumer end: the retry pass unpacks five ------------------------------------
state = {}
boardstate.record(state, "acme|wd1|ext", "acme", False, 0, "URLError: timed out",
                  now=stamp(days=-2))

real_load, real_save = boardstate.load, boardstate.save
real_boards, real_opts = retry_workday.workday_boards, retry_workday.workday_opts
real_probe = retry_workday._probe_board
saved = {}


def fake_load(path=None):
    return state


def fake_save(boards, path=None, extra=None):
    saved.clear()
    saved.update(boards)
    return len(boards)


boardstate.load = fake_load
boardstate.save = fake_save
retry_workday.workday_boards = lambda: [BOARD]  # noqa: E731
retry_workday.workday_opts = lambda: {}  # noqa: E731
retry_workday._probe_board = lambda *a, **k: ([{"id": "x"}], True, 0, None, {})  # noqa: E731
try:
    with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
        rc = retry_workday.main(["--force"])
finally:
    boardstate.load, boardstate.save = real_load, real_save
    retry_workday.workday_boards, retry_workday.workday_opts = real_boards, real_opts
    retry_workday._probe_board = real_probe

check("a pass over one recovered board exits 0", rc, 0)
row = saved.get("acme|wd1|ext") or {}
check("the recovered board is ok again", row.get("status"), "ok")
check("no ValueError masquerades as a probe error",
      "ValueError" in str(row.get("last_error") or ""), False)

# A failing probe must still be recorded as failing, with its real reason kept.
state2 = {}
boardstate.record(state2, "acme|wd1|ext", "acme", False, 0, "URLError: timed out",
                  now=stamp(days=-2))
saved2 = {}


def fake_load2(path=None):
    return state2


def fake_save2(boards, path=None, extra=None):
    saved2.clear()
    saved2.update(boards)
    return len(boards)


boardstate.load, boardstate.save = fake_load2, fake_save2
retry_workday.workday_boards = lambda: [BOARD]  # noqa: E731
retry_workday.workday_opts = lambda: {}  # noqa: E731
retry_workday._probe_board = (  # noqa: E731
    lambda *a, **k: ([], False, 0, "HTTP Error 422: Unprocessable Entity", {}))
try:
    with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
        rc2 = retry_workday.main(["--force"])
finally:
    boardstate.load, boardstate.save = real_load, real_save
    retry_workday.workday_boards, retry_workday.workday_opts = real_boards, real_opts
    retry_workday._probe_board = real_probe

check("a pass over a still-dead board exits 0", rc2, 0)
row2 = saved2.get("acme|wd1|ext") or {}
check("the still-dead board stays failing", row2.get("status") in ("failing", "broken"), True)
check("the real reason is kept, not the unpack",
      row2.get("last_error"), "HTTP Error 422: Unprocessable Entity")

print(f"\n{checks} checks, {len(failures)} mismatches")
if failures:
    print("FAILED:")
    for f in failures:
        print("  -", f)
    raise SystemExit(1)
print("the retry pass unpacks the probe result it is actually given")
