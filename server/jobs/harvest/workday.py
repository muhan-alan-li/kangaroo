#!/usr/bin/env python3
"""Rebuild config/workday_boards.yaml from the Workday links in the Simplify feed.

Workday needs three things per employer -- a tenant, a wd number and a career site -- not a
single slug, so the board list cannot be hand-written the way ats_boards.yaml is. It is
harvested instead: every Workday link in the Simplify new-grad feed is somebody who has
already posted a new-grad role, which makes the list self-selecting.

    python3 -m server harvest workday              # rewrite config/workday_boards.yaml
    python3 -m server harvest workday --report     # print the counts, write nothing

Two URL shapes appear in the feed and both are handled:

    https://{tenant}.{wd}.myworkdayjobs.com/[en-US/]{site}/job/...
    https://{wd}.myworkdaysite.com/[en-US/]recruiting/{tenant}/{site}/job/...

The feed is read from state/simplify_listings.json when that copy is under a day old, so a
run straight after a scan costs nothing.
"""
import argparse
import json
import os
import re
import sys

from ..feed import load_feed  # noqa: E402

from ...config import ROOT
from ...scan import boardstate  # noqa: E402
OUT = os.path.join(ROOT, "config", "workday_boards.yaml")

PATTERNS = (
    re.compile(r"https?://([a-z0-9-]+)\.(wd\d+)\.myworkdayjobs\.com/"
               r"(?:en-US/)?([^/]+)/job/", re.I),
    re.compile(r"https?://(wd\d+)\.myworkdaysite\.com/(?:en-US/)?recruiting/"
               r"([^/]+)/([^/]+)/job/", re.I),
)

HEADER = """# Workday boards, harvested from the Simplify feed.
#
# Workday is about 46% of that feed, and it is the largest source this workspace can read
# without a feed in front of it. The tenant + wd-number + site triple comes out of the
# feed's own URLs, so every entry here is a company that has already posted a new-grad
# role. Regenerate with:
#
#     python3 -m server harvest workday
#
# The board API is date-sorted newest first and answers with a real publish field
# ("Posted Today", "Posted 3 Days Ago"), so the scan reads the first page of each board and
# stops -- it never pages through the whole board, which for CVS alone is 19,392 postings.
#
# A board whose site name is wrong answers 422 and is skipped. Verify with:
#
#     python3 -m server probe-ats --report
"""


def harvest(records):
    """[(tenant, wd, site, url_count)] from every Workday link in the feed."""
    counts = {}
    for rec in records:
        url = (rec or {}).get("url") or ""
        for i, pattern in enumerate(PATTERNS):
            hit = pattern.search(url)
            if not hit:
                continue
            if i == 0:
                tenant, wd, site = hit.group(1), hit.group(2), hit.group(3)
            else:
                wd, tenant, site = hit.group(1), hit.group(2), hit.group(3)
            key = (tenant.lower(), wd.lower(), site)
            counts[key] = counts.get(key, 0) + 1
            break
    rows = [{"tenant": t, "wd": w, "site": s, "urls": n}
            for (t, w, s), n in sorted(counts.items(), key=lambda kv: -kv[1])]
    return rows


def main(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument("--report", action="store_true", help="print, do not write")
    ap.add_argument("--refresh", action="store_true", help="ignore the cached feed")
    args = ap.parse_args(argv)

    try:
        records, origin = load_feed(refresh=args.refresh)
    except Exception as exc:  # noqa: BLE001
        sys.stderr.write("workday_harvest: could not load the feed: %s\n" % exc)
        return 1

    rows = harvest(records)
    # Prune's tombstones win over the feed: the structurally-dead triples below came from
    # feed URLs, so rebuilding blindly would resurrect them into a fresh 30-day broken
    # cycle. Compared case-insensitively -- the feed lowercases tenants while the keys
    # were built from config text.
    tombstones = boardstate.load_tombstones()
    buried = {k.lower() for k in tombstones}
    live_rows, resurrected = [], 0
    for r in rows:
        key = boardstate.key_of(r["tenant"], r["wd"], r["site"])
        if key.lower() in buried:
            resurrected += 1
            continue
        live_rows.append(r)
    rows = live_rows
    covered = sum(r["urls"] for r in rows)
    print("[harvest] feed=%s (%d records) -> %d Workday boards, %d links"
          % (origin, len(records), len(rows), covered))
    if resurrected:
        print("[harvest] skipped %d pruned board(s) still in the feed "
              "(config/workday_tombstones.yaml)" % resurrected)
    for r in rows[:10]:
        print("   %-22s %-6s %-34s %d" % (r["tenant"], r["wd"], r["site"][:34], r["urls"]))

    if args.report:
        return 0
    body = [HEADER, "\nboards_workday:"]
    for r in rows:
        body.append('  - {name: %s, ats: workday, tenant: %s, wd: %s, site: %s, urls: %d}'
                    % (json.dumps(r["tenant"]), r["tenant"], r["wd"], r["site"], r["urls"]))
    tmp = "%s.%d.tmp" % (OUT, os.getpid())
    with open(tmp, "w", encoding="utf-8") as fh:
        fh.write("\n".join(body) + "\n")
    os.replace(tmp, OUT)
    print("[harvest] wrote %s (%d boards)" % (OUT, len(rows)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
