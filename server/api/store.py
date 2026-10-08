"""Reading the scan's state for the API.

The server never fetches anything and never makes a network call: it reads files the scan
wrote under `state/`, on every request, so the page is current the moment a scan finishes.
"""
from __future__ import annotations

import json
import os

from ..config import (DESC, EXPIRED, FEED, HEALTH, SCAN, SWEEP, tier_order, window_days)


def read_json(path, default=None):
    try:
        with open(path, encoding="utf-8") as fh:
            return json.load(fh)
    except Exception:  # noqa: BLE001 -- a missing state file is a cold start, not an error
        return default


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
        if str(row.get("id")) == str(pid):
            return row
    return None


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
    order = {name: i for i, name in enumerate(tier_order())}
    postings.sort(key=lambda p: (order.get(p.get("tier") or "", 99),
                                 -int(p.get("score") or 0),
                                 p.get("age_days") if p.get("age_days") is not None else 999))

    secs = scan.get("source_seconds") or {}
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
        "boards_total": len(boards),
        "boards_ok": len(boards) - len(failing),
        "boards_failing": len(failing),
        "failing": failing[:40],
        "sweep": {k: sweep.get(k) for k in
                  ("swept_at", "count", "boards", "productive", "quiet", "failed",
                   "page_two_skipped", "carried_over", "wall_seconds", "verdict")},
        "tiers": tier_order(),
    }
