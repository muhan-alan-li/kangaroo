#!/usr/bin/env python3
"""Rebuild data/config/icims_boards.yaml from the iCIMS links in the Simplify feed.

iCIMS needs a host, not a slug -- every customer gets their own subdomain
("careers-ice.icims.com", "career-schwab.icims.com") and the board lives at that host's
/jobs/search. The hosts are harvested from the feed so that every entry is a company that
has already posted a new-grad role.

    python3 -m server harvest icims              # rewrite data/config/icims_boards.yaml
    python3 -m server harvest icims --report     # counts only, write nothing
"""
import argparse
import json
import os
import re
import sys

from ..feed import load_feed  # noqa: E402

from ...config import ROOT
OUT = os.path.join(ROOT, "data", "config", "icims_boards.yaml")
HOST = re.compile(r"https?://([a-z0-9-]+\.icims\.com)/jobs/", re.I)

HEADER = """# iCIMS job boards, harvested from the Simplify feed.
#
# One entry per customer host; the board is https://{host}/jobs/search. Regenerate with:
#
#     python3 -m server harvest icims
#
# iCIMS is behind an AWS WAF that refuses plain HTTP clients, so the scan reads these
# through server/scan/icims_fetch.py under .venv-icims (curl_cffi, browser TLS impersonation).
# Rebuild that venv with:
#
#     uv venv .venv-icims --python 3.13
#     uv pip install --python .venv-icims/bin/python curl_cffi
#
# The search view carries no date, so the fetcher reads each candidate's job detail page for
# the schema.org `datePosted`. See data/config/source_candidates.md for the evidence.
"""


def main(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument("--report", action="store_true", help="print, do not write")
    ap.add_argument("--refresh", action="store_true", help="ignore the cached feed")
    args = ap.parse_args(argv)

    try:
        records, origin = load_feed(refresh=args.refresh)
    except Exception as exc:  # noqa: BLE001
        sys.stderr.write("icims_harvest: could not load the feed: %s\n" % exc)
        return 1

    hosts, seen = [], set()
    for rec in records:
        hit = HOST.search((rec or {}).get("url") or "")
        if not hit:
            continue
        host = hit.group(1).lower()
        if host not in seen:
            seen.add(host)
            hosts.append(host)
    hosts.sort()

    print("[harvest] feed=%s (%d records) -> %d iCIMS hosts" % (origin, len(records), len(hosts)))
    if args.report:
        return 0

    body = [HEADER, "\n# --- icims (%d boards) ---" % len(hosts), "boards_icims:"]
    for host in hosts:
        body.append("  - {name: %s, ats: icims, slug: %s}" % (json.dumps(host), json.dumps(host)))
    tmp = "%s.%d.tmp" % (OUT, os.getpid())
    with open(tmp, "w", encoding="utf-8") as fh:
        fh.write("\n".join(body) + "\n")
    os.replace(tmp, OUT)
    print("[harvest] wrote %s (%d hosts)" % (OUT, len(hosts)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
