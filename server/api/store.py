"""Reading the scan's state for the API.

The server never fetches anything and never makes a network call: it reads files the scan
wrote under `data/state/`, on every request, so the page is current the moment a scan finishes.
"""
from __future__ import annotations

import json
import os

from ..config import (DESC, EXPIRED, FEED, HEALTH, SCAN, SWEEP, tier_order, window_days)
from . import triage


_JSON_CACHE = {}


def read_json(path, default=None):
    """Parse one state file, reusing the parse while the file has not changed.

    `board_health.json` (1.8 MB) and `workday_sweep.json` (1.9 MB) are re-read only when the
    scan rewrites them, and the API answers a request by re-reading state. Without this, every
    click and every 7s brief poll re-parsed ~4 MB of JSON to produce a 183 KB payload.
    """
    try:
        st = os.stat(path)
        stamp = (st.st_mtime_ns, st.st_size)
    except OSError:
        return default
    hit = _JSON_CACHE.get(path)
    if hit and hit[0] == stamp:
        return hit[1]
    try:
        with open(path, encoding="utf-8") as fh:
            value = json.load(fh)
    except Exception:  # noqa: BLE001 -- a missing state file is a cold start, not an error
        return default
    _JSON_CACHE[path] = (stamp, value)
    return value


def read_feed():
    """The live portal rows, oldest write last. The feed is the portal's list of postings.

    It is written by the scan (scan/portal.py): it accumulates postings across runs so a
    live posting cannot vanish because one run's per-tier cap did not reach it, and it retires
    a posting after max_age_days. Falls back to the last scan when the feed does not exist yet,
    so a fresh clone still shows something.
    """
    rows = []
    try:
        with open(FEED, encoding="utf-8") as fh:
            for line in fh:
                line = line.strip()
                if not line:
                    continue
                try:
                    rows.append(json.loads(line))
                except json.JSONDecodeError:
                    continue
    except FileNotFoundError:
        return [], False
    return rows, True


def find_row(pid):
    """One portal row by posting id, falling back to the last scan."""
    rows, present = read_feed()
    if not present:
        rows = (read_json(SCAN, {}) or {}).get("matched") or []
    for row in rows:
        if str(row.get("id")) == str(pid) and _tier_allowed(row):
            return row
    return None


def _tier_allowed(row, allowed=None):
    """A row is shown only if its tier still exists in filters.yaml's location_tiers.

    Tiers get retired (bay_area, 2026-10-26); retired-tier rows left over in the
    state files must not reach the page, so this checks the tier list itself rather
    than hard-coding names. `allowed` lets a caller that is about to run this over
    every row read the list once instead of once per row.
    """
    if allowed is None:
        allowed = set(tier_order())
    return not row.get("tier") or row["tier"] in allowed


def description_for(pid, row=None):
    """The posting's text, from the cache the scan fills. Never fetched here: this process
    answers a request in milliseconds and must not start making network calls."""
    text = (read_json(DESC, {}) or {}).get(str(pid))
    if text:
        return text
    if (row or {}).get("description"):
        return row["description"]
    # Pre-fix rows: source text that the portal dropped before it reached the cache still
    # sits on the last scan's matched list.
    for scan_row in (read_json(SCAN, {}) or {}).get("matched") or []:
        if str(scan_row.get("id")) == str(pid) and scan_row.get("description"):
            return scan_row["description"]
    return ""


def collect():
    """Everything the page shows, assembled per request."""
    allowed = set(tier_order())
    scan = read_json(SCAN, {}) or {}
    health = read_json(HEALTH, {}) or {}
    sweep = read_json(SWEEP, {}) or {}
    expired = read_json(EXPIRED, {}) or {}
    boards = health.get("boards") or {}
    failing = [v for v in boards.values() if (v or {}).get("status") not in (None, "ok")]
    failing.sort(key=lambda b: (-(b.get("consecutive_failures") or 0), b.get("name") or ""))

    postings, feed_present = read_feed()
    if not feed_present:
        postings = scan.get("matched") or []
    postings = [p for p in postings if _tier_allowed(p, allowed)]
    order = {name: i for i, name in enumerate(tier_order())}
    postings.sort(key=lambda p: (order.get(p.get("tier") or "", 99),
                                 -int(p.get("score") or 0),
                                 p.get("age_days") if p.get("age_days") is not None else 999))

    secs = scan.get("source_seconds") or {}

    # The applied list lives in its own file, because the feed retires a row after the window
    # and an application outlives it. Where the posting is still live, the live row supplies the
    # fields a scan can refresh (title, location) and the snapshot supplies `applied_at`.
    live_ids = {str(p.get("id")): p for p in postings}
    applied = []
    for snap in triage.rows():
        live = live_ids.get(str(snap.get("id")))
        row = dict(live) if live else dict(snap)
        row["applied_at"] = snap.get("applied_at")
        applied.append(row)

    return {
        "ran_at": scan.get("ran_at"),
        "feed": feed_present,
        "window": window_days(),
        "expired_total": len(expired),
        "collected": sum((scan.get("sources") or {}).values()),
        "matched": len(postings),
        "with_email": sum(1 for p in postings if p.get("emails") or p.get("verified_emails")),
        "with_keywords": sum(1 for p in postings if p.get("keywords")),
        "with_text": sum(1 for p in postings if p.get("has_text")),
        "sources": scan.get("sources") or {},
        "source_seconds": secs,
        "source_total": round(sum(secs.values()), 1),
        "tier_counts": scan.get("tier_counts") or {},
        "dropped": scan.get("dropped") or {},
        "postings": postings,
        "applied": applied,
        "boards_total": len(boards),
        "boards_ok": len(boards) - len(failing),
        "boards_failing": len(failing),
        "failing": failing[:40],
        "sweep": {k: sweep.get(k) for k in
                  ("swept_at", "count", "boards", "productive", "quiet", "failed",
                   "page_two_skipped", "carried_over", "wall_seconds", "verdict")},
        "tiers": tier_order(),
    }
