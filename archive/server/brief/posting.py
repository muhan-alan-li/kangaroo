"""The brief's inputs: the portal row for a posting, and the posting's own text.

The brief generator is the only server-side reader allowed to fetch a description on demand.
The dashboard's read path (`server/api/store.py`) answers in milliseconds and never touches the
network, so this module is the single bridge between the two: it reuses the scan's description
fetchers rather than growing a second set that could disagree about Greenhouse or Workday.
"""
from __future__ import annotations

import json
import os

from ..config import DESC, FEED, ROOT, SCAN
from .state import read_json, write_json

RESUME_PATH = os.path.join(ROOT, "data", "candidate", "resume-text.md")


def find_posting(pid):
    """The portal row for a posting id, falling back to the last scan's list."""
    try:
        with open(FEED, encoding="utf-8") as fh:
            for line in fh:
                line = line.strip()
                if not line:
                    continue
                try:
                    row = json.loads(line)
                except json.JSONDecodeError:
                    continue
                if str(row.get("id")) == str(pid):
                    return row
    except FileNotFoundError:
        pass
    for row in (read_json(SCAN, {}) or {}).get("matched") or []:
        if str(row.get("id")) == str(pid):
            return row
    return None


def description_for(row):
    """The posting's text, from the cache, the row, the last scan, or the board API.

    The fetched text is written back to the shared cache so the next open of this posting is
    instant. Both sides write that file atomically, so a concurrent scan can only mean
    last-writer-wins on identical data.
    """
    pid = row.get("id")
    cache = read_json(DESC, {}) or {}
    if cache.get(pid):
        return cache[pid], "cache"
    if row.get("description"):
        return row["description"], "row"
    # Pre-fix rows: the portal kept `has_text: true` but dropped the source's text, so the
    # feed row and the cache are both empty while the last scan still carries it.
    for scan_row in (read_json(SCAN, {}) or {}).get("matched") or []:
        if str(scan_row.get("id")) == str(pid) and scan_row.get("description"):
            return scan_row["description"], "scan"
    try:
        from ..scan import portal
    except Exception:  # noqa: BLE001
        return None, "none"
    if not portal._description_fetcher(row):
        return None, "unfetchable"
    text = portal._safe_description(row)
    if not text:
        return None, "failed"
    cache[pid] = text
    write_json(DESC, cache)
    return text, "fetched"
