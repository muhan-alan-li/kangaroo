"""Paths, constants, and the two settings the server reads from data/config/.

Everything lives under the workspace root: state files under `data/state/`, config under `data/config/`,
the client under `client/`, and the brief generator under `bin/`. Keeping the paths here means
a module can be imported and tested without pulling in the HTTP handler.
"""
from __future__ import annotations

import os

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CLIENT = os.path.join(ROOT, "client")
SCAN = os.path.join(ROOT, "data", "state", "last_scan.json")
FEED = os.path.join(ROOT, "data", "state", "portal_feed.jsonl")
EXPIRED = os.path.join(ROOT, "data", "state", "portal_expired.json")
HEALTH = os.path.join(ROOT, "data", "state", "board_health.json")
SWEEP = os.path.join(ROOT, "data", "state", "workday_sweep.json")
FILTERS = os.path.join(ROOT, "data", "config", "filters.yaml")
DESC = os.path.join(ROOT, "data", "state", "descriptions.json")
BRIEFS = os.path.join(ROOT, "data", "state", "briefs")
BRIEF_SCRIPT = os.path.join(ROOT, "server", "brief", "generate.py")
PYTHON = "/usr/bin/python3"
BRIEF_STALE_AFTER = 900        # a .running marker older than this is a crashed run

TIER_ORDER = ["vancouver", "remote_canada", "toronto", "seattle",
              "remote_us", "remote_unspecified"]

# Content types for the client directory. Anything unlisted is served as a binary blob rather
# than guessed at, because a wrong MIME type makes the browser refuse the file.
MIME = {
    ".html": "text/html; charset=utf-8",
    ".css": "text/css; charset=utf-8",
    ".js": "text/javascript; charset=utf-8",
    ".mjs": "text/javascript; charset=utf-8",
    ".json": "application/json; charset=utf-8",
    ".map": "application/json; charset=utf-8",
    ".svg": "image/svg+xml",
    ".png": "image/png",
    ".ico": "image/x-icon",
    ".webp": "image/webp",
    ".woff2": "font/woff2",
    ".txt": "text/plain; charset=utf-8",
}


def tier_order():
    """The rank the scan uses, so the dashboard cannot disagree with the scan."""
    try:
        import yaml
        with open(FILTERS, encoding="utf-8") as fh:
            names = [str(t.get("name")) for t in (yaml.safe_load(fh) or {}).get("location_tiers") or []]
        if names:
            return names
    except Exception:  # noqa: BLE001
        pass
    return TIER_ORDER


def window_days():
    """The freshness window the scan and the portal both use, from data/config/filters.yaml."""
    try:
        import yaml
        with open(FILTERS, encoding="utf-8") as fh:
            return int((yaml.safe_load(fh) or {}).get("max_age_days") or 7)
    except Exception:  # noqa: BLE001
        return 7
