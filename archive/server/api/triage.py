"""The applied list: postings whose application link the candidate has opened.

One file, `data/state/applied.json`, mapping posting id -> a snapshot of the row at the moment
it was marked. The snapshot is the point: the portal feed retires a posting after its window,
and an application does not stop existing because the feed let the row go, so the applied list
carries its own copy of the fields it renders.

This is the reader's own list, driven from the page. It is not the retired application ledger:
nothing here submits anything, and no per-application artifact lives beside it.

Writes are read-modify-write, so they are serialized under a lock file and committed with a
pid-tagged temp plus `os.replace` -- the same dance the scan's state writers use. One user, but
two open tabs are two writers.
"""
from __future__ import annotations

import contextlib
import fcntl
import json
import os
from datetime import datetime, timezone

from ..config import APPLIED

LOCK = APPLIED + ".lock"

# What the applied list needs to render a row, and nothing that a scan can regrow.
SNAPSHOT = ("company", "title", "url", "location", "tier", "source", "ats",
            "posted", "posted_derived", "age_days", "score", "expires_on", "level_ii")


def _read_unlocked():
    try:
        with open(APPLIED, encoding="utf-8") as fh:
            data = json.load(fh)
    except FileNotFoundError:
        return {}
    except (OSError, json.JSONDecodeError):  # noqa: BLE001 -- unreadable is an empty list
        return {}
    return data if isinstance(data, dict) else {}


@contextlib.contextmanager
def _locked():
    os.makedirs(os.path.dirname(APPLIED) or ".", exist_ok=True)
    with open(LOCK, "a+") as fh:
        fcntl.flock(fh, fcntl.LOCK_EX)
        try:
            yield
        finally:
            fcntl.flock(fh, fcntl.LOCK_UN)


def _write_unlocked(rows):
    tmp = f"{APPLIED}.{os.getpid()}.tmp"
    with open(tmp, "w", encoding="utf-8") as fh:
        json.dump(rows, fh, ensure_ascii=False, indent=1, sort_keys=True)
        fh.write("\n")
    os.replace(tmp, APPLIED)


def rows():
    """Every applied posting, newest application first."""
    out = [{"id": pid, **(row or {})} for pid, row in _read_unlocked().items()]
    out.sort(key=lambda r: (r.get("applied_at") or ""), reverse=True)
    return out


def has(pid):
    return str(pid) in _read_unlocked()


def mark(row):
    """Record one posting as applied. Idempotent: the first application keeps its timestamp."""
    pid = str((row or {}).get("id") or "").strip()
    if not pid:
        raise ValueError("posting has no id")
    snap = {k: (row or {}).get(k) for k in SNAPSHOT}
    snap["id"] = pid
    with _locked():
        current = _read_unlocked()
        if pid in current:
            return {"id": pid, **(current[pid] or {})}
        snap["applied_at"] = datetime.now(timezone.utc).replace(microsecond=0).isoformat()
        current[pid] = snap
        _write_unlocked(current)
        return snap


def remove(ids):
    """Drop the given ids. Returns how many of them existed."""
    wanted = {str(i).strip() for i in (ids or []) if str(i).strip()}
    with _locked():
        current = _read_unlocked()
        gone = [pid for pid in wanted if pid in current]
        if gone:
            for pid in gone:
                current.pop(pid, None)
            _write_unlocked(current)
    return len(gone)
