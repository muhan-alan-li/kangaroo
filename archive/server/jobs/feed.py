#!/usr/bin/env python3
"""Shared cache for the SimplifyJobs new-grad feed.

Two harvesters read the same 13 MB listings.json -- server/jobs/harvest/workday.py for Workday
boards and server/jobs/harvest/jsonboards.py for the JSON-vendor boards -- and the `feed`
command reads it for a quick human look, so the fetch and the on-disk cache live here
rather than in each of them. Callers that run right after a scan pass a long TTL (a copy
under a day old is reused, so a harvest straight after a scan costs nothing); the human
`feed` command passes ten minutes.
"""
import json
import os
import time
import urllib.request

from ..config import ROOT
from ..scan.core import atomic_write
FEED = ("https://raw.githubusercontent.com/SimplifyJobs/New-Grad-Positions"
        "/dev/.github/scripts/listings.json")
CACHE = os.path.join(ROOT, "data", "state", "simplify_listings.json")
CACHE_SECONDS = 24 * 3600
UA = ("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 "
      "(KHTML, like Gecko) Chrome/124.0 Safari/537.36")


def load_feed(refresh=False, max_age_seconds=CACHE_SECONDS):
    """(records, origin) where origin is 'cache' or 'download'."""
    if not refresh and os.path.exists(CACHE):
        if time.time() - os.path.getmtime(CACHE) < max_age_seconds:
            try:
                with open(CACHE, encoding="utf-8") as fh:
                    return json.load(fh), "cache"
            except (OSError, ValueError):
                pass
    req = urllib.request.Request(FEED, headers={"User-Agent": UA, "Accept": "*/*"})
    with urllib.request.urlopen(req, timeout=90) as resp:
        records = json.loads(resp.read().decode("utf-8", "replace"))
    if isinstance(records, dict):
        records = records.get("listings") or []
    os.makedirs(os.path.dirname(CACHE), exist_ok=True)
    with atomic_write(CACHE) as fh:
        json.dump(records, fh)
    return records, "download"
