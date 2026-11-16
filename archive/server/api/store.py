"""Reading the scan's state for the API.

The server never fetches anything and never makes a network call: it reads files the scan
wrote under `data/state/`, on every request, so the page is current the moment a scan finishes.
"""
from __future__ import annotations

import json
import os
from datetime import date

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


def _expiry(row):
    """(the date a row dies on, days left) recomputed against today.

    The feed stores its own `expires_in_days`, but that number was computed at scan time: an
    applied snapshot from a week ago would claim days it no longer has, and a shortlisted row
    carried past the window would look alive. Every list therefore refreshes from
    `expires_on`, so the same posting shows the same deadline in all three tabs. A posting the
    feed expired keeps the date it died on, which is what makes its row say "expired".
    """
    end = str(row.get("expires_on") or "")[:10]
    if not end:
        return None, row.get("expires_in_days")
    try:
        return end, (date.fromisoformat(end) - date.today()).days
    except ValueError:
        return end, row.get("expires_in_days")


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
    # `matched` is the scan's number for the health page, so it counts every live row rather
    # than the open table, which the reader empties by generating briefs and applying.
    matched = len(postings)

    secs = scan.get("source_seconds") or {}

    # The applied list lives in its own file, because the feed retires a row after the window
    # and an application outlives it. Where the posting is still live, the live row supplies the
    # fields a scan can refresh (title, location) and the snapshot supplies `applied_at`.
    live_ids = {str(p.get("id")): p for p in postings}
    applied, applied_ids = [], set()
    for snap in triage.rows():
        live = live_ids.get(str(snap.get("id")))
        row = dict(live) if live else dict(snap)
        row["applied_at"] = snap.get("applied_at")
        row["expires_on"], row["expires_in_days"] = _expiry(row)
        applied_ids.add(str(row.get("id")))
        applied.append(row)

    # The shortlisted view: every posting with a brief, whether or not the feed still carries
    # the row. The brief is a durable artifact like the applied snapshot, so a retired
    # posting keeps its row -- marked `live: false`, which is what tells the page it can no
    # longer be opened in the drawer and may only be linked out. In-flight runs are listed
    # too, because a batch of them is exactly what the reader is watching.
    #
    # An applied posting is not shortlisted any more: applying is a decision, a brief is only a
    # read, so the row moves on to the applied tab and out of the pipeline.
    # The import is local: briefs.py reads this module, so the two cannot import at the top.
    from .briefs import brief_index
    shortlisted, briefed = [], {}
    for pid, entry in brief_index().items():
        status = "running" if entry.get("running") else (entry.get("status") or "")
        if not status:
            continue
        live = live_ids.get(str(pid))
        if live:
            briefed[str(pid)] = status
        if str(pid) in applied_ids:
            continue
        row = {"id": str(pid), "live": bool(live), "status": status,
               "generated_at": entry.get("generated_at"),
               "title": (live or {}).get("title") or entry.get("title") or "",
               "company": (live or {}).get("company") or entry.get("company") or "",
               "url": (live or {}).get("url") or entry.get("url") or "",
               "location": (live or {}).get("location") or "",
               "tier": (live or {}).get("tier") or ""}
        if live:
            for key in ("posted", "posted_derived", "scanned_at", "age_days"):
                if live.get(key) is not None:
                    row[key] = live.get(key)
            row["expires_on"] = live.get("expires_on")
        else:
            # The feed has retired it, so the expiry it was retired on is the honest date: an
            # expired row says when it died rather than showing nothing at all.
            row["expires_on"] = (expired.get(str(pid)) or {}).get("expires_on")
        row["expires_on"], row["expires_in_days"] = _expiry(row)
        shortlisted.append(row)
    # Newest brief first, with the runs still in flight ahead of the finished ones: the
    # shortlisted tab is the progress view while a batch is generating.
    shortlisted.sort(key=lambda r: (r.get("status") == "running",
                                    r.get("generated_at") or ""), reverse=True)

    # The pipeline (2026-11-03): a brief moves a posting out of Open and into Shortlisted. Any
    # status that has something to read -- `ok`, or a run in flight -- moves it; a `failed`
    # brief leaves the row where it is, because there is nothing to read yet and the next
    # Generate briefs press retries it. Applying removes a row from every list but Applied.
    # A row that already has a brief still says so on its way out, so a stale page's batch
    # press reports the skip instead of spending the model call.
    for p in postings:
        status = briefed.get(str(p.get("id")))
        if status:
            p["brief_status"] = status
        p["expires_on"], p["expires_in_days"] = _expiry(p)
    moved = {pid for pid, status in briefed.items() if status in ("ok", "running")}
    postings = [p for p in postings
                if str(p.get("id")) not in applied_ids
                and str(p.get("id")) not in moved]

    return {
        "ran_at": scan.get("ran_at"),
        "feed": feed_present,
        "window": window_days(),
        "expired_total": len(expired),
        "collected": sum((scan.get("sources") or {}).values()),
        "matched": matched,
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
        "shortlisted": shortlisted,
        "boards_total": len(boards),
        "boards_ok": len(boards) - len(failing),
        "boards_failing": len(failing),
        "failing": failing[:40],
        "sweep": {k: sweep.get(k) for k in
                  ("swept_at", "count", "boards", "productive", "quiet", "failed",
                   "page_two_skipped", "carried_over", "wall_seconds", "verdict")},
        "tiers": tier_order(),
    }
