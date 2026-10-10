"""Durable per-board state: which boards answered, when each was last swept, and what the
sweep before that returned.

Three readers, one file:

- the daily scan records every Workday board's outcome as it fetches, and reports one line
  of it. Nothing is written while the sources are running -- the recorder is memory only,
  and `finish_run` persists it afterwards, which is the same rule `harvest_boards` follows.
- `server/jobs/retry/retry.py` reads the file to pick the boards worth another look, and writes back
  what it learned.
- `server/jobs/scan/sweep.py` reads the last sweep to skip a board whose first page has not moved
  since, and to serve the daily scan from its own output.

A board that answers nothing is not the same as a board that answers with nothing. The first
is a request that failed and is worth retrying; the second is a real answer and worth
remembering. `status` is carried per board, and the retry queue is derived from it.
"""

from __future__ import annotations

from datetime import datetime, timedelta, timezone
import json
import os
import re

from .core import ROOT, atomic_write

HEALTH_PATH = os.path.join(ROOT, "data", "state", "board_health.json")
SWEEP_PATH = os.path.join(ROOT, "data", "state", "workday_sweep.json")
# Boards prune removed as structurally dead. Harvest rebuilds the board file from the feed,
# and the dead triples came from feed URLs in the first place, so without this list the
# next harvest resurrects every board prune just buried. Committed with the config so a
# fresh clone remembers too (data/state/ is gitignored and would forget).
TOMBSTONE_PATH = os.path.join(ROOT, "data", "config", "workday_tombstones.yaml")

TOMBSTONE_HEADER = """# Workday boards removed as structurally dead, by server/jobs/retry/prune.py.
#
# A board lands here when it is `broken` on a structural failure (a Workday 404/422 -- a
# wrong career-site name no retry can fix). `server/jobs/harvest/workday.py` reads this
# list and skips these triples when it rebuilds data/config/workday_boards.yaml from the feed,
# because the dead triples came from feed URLs and would otherwise be re-added on the
# next harvest. Remove a line by hand to let a board back in (e.g. the employer fixed
# the career-site name).
#
#     python3 -m server harvest workday
#
"""

# How long to wait before looking at a failed board again, per consecutive failure. The
# schedule is the whole policy: retry a blip tomorrow, a board that failed twice in three
# days, and a board that keeps failing only monthly. Past the end of the list the board is
# `broken` -- still re-checked, but on the slow beat, and named in the report so a human can
# fix the entry rather than waiting on a pass that will never succeed.
RETRY_BACKOFF_DAYS = {
    "transient": [1, 3, 14],
    "structural": [7, 30],
}
BROKEN_RECHECK_DAYS = 30

# A 404 or a 422 is the board's own configuration, not the network: a Workday 422 means the
# career-site name in the triple is wrong. Those are worth naming, and not worth retrying
# hourly. Everything else (timeouts, connection resets, 5xx, 429) is the network or the
# throttle, and is expected to clear.
STRUCTURAL_RE = re.compile(r"\b(404|422)\b|not found|unprocessable", re.I)

_RUN = []
_ACTIVE = False


def key(board):
    """The board's identity, the same string the postings carry as extra['board']."""
    if isinstance(board, str):
        return board
    return f"{board.get('tenant')}|{board.get('wd')}|{board.get('site')}"


def key_of(tenant, wd, site):
    """The board key built from raw parts, normalised the way the YAML parser would.

    PyYAML reads an unquoted `001` as the integer 1, so the config text says `001` while the
    parsed board -- and therefore the health row -- says `1`. Anything that compares text it
    found (a URL, a config line) against parsed identity must fold the parts the same way.
    """
    def norm(part):
        part = str(part)
        return str(int(part)) if part.isdigit() else part
    return f"{norm(tenant)}|{norm(wd)}|{norm(site)}"


def now_iso():
    return datetime.now(timezone.utc).isoformat(timespec="seconds")


def classify(err):
    """"structural" when the board's own config is wrong, "transient" otherwise."""
    text = str(err or "")
    return "structural" if STRUCTURAL_RE.search(text) else "transient"


def _parse(stamp):
    if not stamp:
        return None
    try:
        parsed = datetime.fromisoformat(stamp)
    except ValueError:
        return None
    return parsed if parsed.tzinfo else parsed.replace(tzinfo=timezone.utc)


# A sweep that loses a large share of its boards is the throttle talking, not a thousand
# broken configurations. The difference matters because a sweep's results feed the cache the
# daily scan reads: a throttled run that writes its cache deletes a day of Workday postings
# silently, and one that writes its health fills the retry queue with boards that were fine.
# Measured on 2026-10-02: a healthy sweep fails ~49 of 1,328 boards (3.7%), a throttled one
# 1,267 (95%). 25% with a floor of 60 sits far from both.
SWEEP_MAX_FAILURE_FRACTION = 0.25
SWEEP_MIN_FAILURES = 60


def sweep_verdict(recorded, previous_postings=None, max_fraction=None, min_failures=None):
    """Whether a sweep's outcomes are trustworthy: (ok, reason).

    Callers use this to refuse to write the sweep cache and to refuse to fold the failures
    into `board_health.json`. A refused sweep changes nothing and says why, which is the only
    safe answer: the alternative is a daily scan that reads an empty cache and reports a quiet
    day it did not have.
    """
    max_fraction = SWEEP_MAX_FAILURE_FRACTION if max_fraction is None else max_fraction
    min_failures = SWEEP_MIN_FAILURES if min_failures is None else min_failures
    outcomes = list(recorded or [])
    total = len(outcomes)
    if not total:
        return False, "no board outcomes recorded"
    failed = sum(1 for o in outcomes if not o.get("ok"))
    if failed >= min_failures and failed / total > max_fraction:
        return False, (f"{failed} of {total} boards failed ({failed / total:.0%}) -- that is "
                       f"the throttle or the network, not {failed} broken configs")
    productive = sum(1 for o in outcomes if o.get("ok") and o.get("hits"))
    if previous_postings and not productive:
        return False, (f"no board returned a posting where the last sweep found "
                       f"{previous_postings}; treating this sweep as lost")
    return True, f"{failed} of {total} boards failed"


def load(path=None):
    """The board-state file as {board key: row}, or {} when there is nothing yet."""
    path = path or HEALTH_PATH
    try:
        with open(path, encoding="utf-8") as fh:
            data = json.load(fh)
    except (OSError, json.JSONDecodeError):
        return {}
    boards = data.get("boards") or {}
    return boards if isinstance(boards, dict) else {}


def save(boards, path=None, extra=None):
    """Write the state atomically. The temp name carries the pid, so two passes may overlap."""
    path = path or HEALTH_PATH
    os.makedirs(os.path.dirname(path), exist_ok=True)
    payload = {"updated_at": now_iso(), "boards": dict(sorted(boards.items()))}
    payload.update(extra or {})
    with atomic_write(path) as fh:
        json.dump(payload, fh, indent=1)
    return len(boards)


# ------------------------------------------------------------------ run-time recording

def begin_run():
    """Start collecting one run's per-board outcomes. Returns the in-memory recorder."""
    global _ACTIVE
    del _RUN[:]
    _ACTIVE = True
    return _RUN


def end_run():
    """Stop collecting, and hand back what the run gathered. Empty if none was begun.

    The recorder is cleared, so a stray `note()` after the run cannot be picked up later and
    `finish_run` cannot be fed the same outcome twice.
    """
    global _ACTIVE
    _ACTIVE = False
    gathered = list(_RUN)
    del _RUN[:]
    return gathered


def note(board, ok, hits=0, err=None, cursor=None, name=None):
    """Record one board's outcome. Does nothing unless a run has begun, so a source called
    on its own -- a probe script, a test -- cannot quietly fill the recorder.

    `board` may be a config dict or an already-built key string; the JSON vendors pass the
    string (`vendor|slug`) with `name`, since their boards carry no tenant/wd/site triple.
    """
    if not _ACTIVE:
        return
    if name is None and isinstance(board, dict):
        name = board.get("name")
    _RUN.append({"key": key(board),
                 "name": name,
                 "ok": bool(ok), "hits": int(hits or 0), "err": (str(err) if err else None),
                 "cursor": cursor})


def finish_run(recorded=None, boards=None, path=None, write=True, now=None):
    """Fold the run's outcomes into the state file and return the updated state."""
    outcomes = recorded if recorded is not None else list(_RUN)
    state = load(path)
    stamp = now or now_iso()
    for outcome in outcomes:
        row = record(state, outcome["key"], outcome.get("name"), outcome["ok"],
                     outcome.get("hits", 0), outcome.get("err"), now=stamp,
                     cursor=outcome.get("cursor"))
        row["last_checked"] = stamp
    if write:
        # `boards_total` is what the FILE holds, which is the only number a reader can check
        # against the file itself. It used to be the fleet the CALLER swept instead, so a
        # sweep run with --limit 300 left a 1,328-board file advertising 300. The run's own
        # fleet size is still worth having, under a name that says so.
        extra = {"boards_total": len(state)}
        if isinstance(boards, int):
            extra["boards_swept"] = boards
        save(state, path, extra=extra)
    return state


def record(state, board_key, name, ok, hits=0, err=None, now=None, cursor=None):
    """Update one board's row in place and return it."""
    stamp = now or now_iso()
    row = state.setdefault(board_key, {
        "name": name, "first_seen": stamp, "attempts": 0, "failures": 0,
        "consecutive_failures": 0, "last_ok": None, "last_error": None,
        "error_kind": None, "status": "ok", "next_check_at": None,
    })
    if name and not row.get("name"):
        row["name"] = name
    row["attempts"] = int(row.get("attempts") or 0) + 1
    row["last_checked"] = stamp
    if cursor:
        # `verified_at` moves only when the board was read through to its last page. A sweep
        # that skipped page two has not looked at page two, and the cursor's whole safety
        # argument is that its age is the age of that read.
        previous = row.get("cursor") or {}
        ids = list(cursor.get("signature") or [])
        row["cursor"] = {
            "ids": ids,
            "count": len(ids),
            "pages": cursor.get("pages"),
            "verified_at": (previous.get("verified_at") if cursor.get("unchanged")
                            else stamp),
            "unchanged": bool(cursor.get("unchanged")),
        }
    if ok:
        row["last_ok"] = stamp
        row["consecutive_failures"] = 0
        row["last_error"] = None
        row["error_kind"] = None
        row["status"] = "ok"
        row["next_check_at"] = None
        row["hits_last_sweep"] = int(hits or 0)
        return row
    kind = classify(err)
    row["failures"] = int(row.get("failures") or 0) + 1
    row["consecutive_failures"] = int(row.get("consecutive_failures") or 0) + 1
    row["last_error"] = str(err) if err else "no answer"
    row["error_kind"] = kind
    schedule = RETRY_BACKOFF_DAYS[kind]
    streak = row["consecutive_failures"]
    if streak <= len(schedule):
        row["status"] = "failing"
        days = schedule[streak - 1]
    else:
        row["status"] = "broken"
        days = BROKEN_RECHECK_DAYS
    row["next_check_at"] = (now_dt(stamp) + timedelta(days=days)).isoformat(timespec="seconds")
    row["retry_in_days"] = days
    return row


def now_dt(stamp=None):
    return _parse(stamp) or datetime.now(timezone.utc)


# ------------------------------------------------------------------------- querying

def due(state, boards=None, force=False, now=None):
    """The boards worth re-checking: every row that is not `ok` and whose beat has come up.

    `force` ignores the schedule, which is what a manual run wants -- but it still returns
    only failing and broken boards. A board that answered is not a candidate, whenever the
    pass runs; re-checking healthy boards is the daily sweep's job.
    """
    stamp = now_dt(now)
    by_key = {key(b): b for b in (boards or [])}
    out = []
    for board_key, row in sorted(state.items()):
        if row.get("status") == "ok":
            continue
        if not force:
            due_at = _parse(row.get("next_check_at"))
            if due_at is not None and due_at > stamp:
                continue
        board = by_key.get(board_key)
        if board is None and boards:
            continue                      # the board is no longer in the list; skip it
        out.append({"key": board_key, "board": board, "state": row})
    return out


def summary(state):
    """Counts for a one-line report: (known, ok, failing, broken)."""
    ok = sum(1 for r in state.values() if r.get("status") == "ok")
    failing = sum(1 for r in state.values() if r.get("status") == "failing")
    broken = sum(1 for r in state.values() if r.get("status") == "broken")
    return len(state), ok, failing, broken


def load_tombstones(path=None):
    """{board key: {reason, pruned_at}} for every board prune removed, or {}.

    A missing or unreadable file is no tombstones, not an error: harvest must keep
    working on a checkout from before tombstones existed.
    """
    path = path or TOMBSTONE_PATH
    try:
        with open(path, encoding="utf-8") as fh:
            text = fh.read()
    except OSError:
        return {}
    try:
        import yaml
        data = yaml.safe_load(text) or {}
    except Exception:  # noqa: BLE001 -- PyYAML missing or a hand edit broke the file
        data = None
    if isinstance(data, dict):
        rows = data.get("tombstones") or []
        out = {}
        for row in rows:
            if isinstance(row, dict) and row.get("key"):
                out[str(row["key"])] = {"reason": row.get("reason") or "structural failure",
                                        "pruned_at": row.get("pruned_at")}
        return out
    # No YAML parser (or not a mapping): fall back to the key lines, which are the
    # contract -- one `- {key: ...}` or `key: ...` per tombstone.
    out = {}
    for hit in re.finditer(r"key:\s*([^\s,}]+)", text):
        out[hit.group(1)] = {"reason": "structural failure", "pruned_at": None}
    return out


def add_tombstones(entries, path=None, now=None):
    """Remember freshly pruned boards. `entries` is {key: reason}; existing rows keep
    their original pruned_at. Returns the full set."""
    path = path or TOMBSTONE_PATH
    os.makedirs(os.path.dirname(path), exist_ok=True)
    known = load_tombstones(path)
    stamp = now or now_iso()
    for key, reason in (entries or {}).items():
        if key in known:
            continue
        known[key] = {"reason": reason or "structural failure", "pruned_at": stamp}
    lines = [TOMBSTONE_HEADER, "tombstones:"]
    for key in sorted(known):
        row = known[key]
        tenant, wd, site = (key.split("|") + ["", "", ""])[:3]
        lines.append(f"  - {{key: {key}, tenant: {tenant}, wd: {wd}, site: {site}, "
                     f"pruned_at: {row.get('pruned_at') or stamp}, "
                     f"reason: {json.dumps(row.get('reason') or 'structural failure')}}}")
    with atomic_write(path) as fh:
        fh.write("\n".join(lines) + "\n")
    return known


def line(state):
    """The human line the daily scan and the retry pass both print."""
    total, ok, failing, broken = summary(state)
    if not total:
        return "board state: empty"
    return (f"board state: {total} known, {ok} ok, {failing} failing, {broken} broken")


def cursor_for(row, max_age_days=3, now=None):
    """The cursor to hand the sweep for one board, or None when there is nothing to trust.

    `age_days` is the age of the last read that actually reached the board's second page,
    which is the only read that licenses skipping it.
    """
    stored = (row or {}).get("cursor") or {}
    if not stored.get("ids"):
        return None
    verified = _parse(stored.get("verified_at"))
    if verified is None:
        return None
    return {
        "ids": list(stored.get("ids") or []),
        "count": stored.get("count", len(stored.get("ids") or [])),
        "age_days": (now_dt(now) - verified).days,
        "max_age_days": int(max_age_days),
        "pages": stored.get("pages"),
    }


def cursors(state, max_age_days=3, now=None):
    """{board key: cursor} for every board the sweep may skip page two on."""
    out = {}
    for board_key, row in state.items():
        cur = cursor_for(row, max_age_days, now=now)
        if cur is not None:
            out[board_key] = cur
    return out


# ------------------------------------------------------------------ the sweep cache

def write_sweep(postings, path=None, meta=None):
    """Write one Workday sweep's postings, for the daily scan to read instead of re-sweeping.

    The rows are `make_posting` dicts, so they round-trip as JSON as they are. `swept_at`
    travels with them: Workday postings carry no `posted` date (their age comes from the
    board's `postedOn` text at fetch time), so the only way to keep the age honest after a
    few hours on disk is to know when the sweep happened.
    """
    path = path or SWEEP_PATH
    os.makedirs(os.path.dirname(path), exist_ok=True)
    payload = {"swept_at": now_iso(), "count": len(postings)}
    payload.update(meta or {})
    payload["postings"] = postings
    with atomic_write(path) as fh:
        json.dump(payload, fh)
    return path


def read_sweep(max_age_hours=30, path=None, now=None):
    """The cached sweep as (postings, age_hours), or (None, age_hours) when it is too old.

    Ages are advanced by the whole days the cache has sat there, so a posting that was six
    days old when the sweep ran reads as seven the next day and `max_age_days` still drops
    it. Without that, a cache would quietly make every workday posting look fresher than it
    is -- and the window is the one filter a cache must not bend.
    """
    path = path or SWEEP_PATH
    stamp = now_dt(now)
    try:
        with open(path, encoding="utf-8") as fh:
            payload = json.load(fh)
    except (OSError, json.JSONDecodeError):
        return None, None
    swept = _parse(payload.get("swept_at"))
    if swept is None:
        return None, None
    age_hours = (stamp - swept).total_seconds() / 3600.0
    if max_age_hours is not None and age_hours > float(max_age_hours):
        return None, age_hours
    drift = (stamp.date() - swept.date()).days
    postings = []
    for posting in payload.get("postings") or []:
        row = dict(posting)
        age = row.get("age_days")
        if age is not None and drift:
            row["age_days"] = int(age) + drift
        postings.append(row)
    return postings, age_hours
