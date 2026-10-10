#!/usr/bin/env python3
"""Rebuild data/config/json_boards.yaml from the vendor links in the Simplify feed.

Seven vendors share one shape -- a slug in a URL template that answers with JSON -- so they
live in one board file and one fetcher. Every slug here is harvested from a link in the
Simplify new-grad feed, which means each one is a company that has already posted a new-grad
role; none is guessed.

    python3 -m server harvest jsonboards              # rewrite data/config/json_boards.yaml
    python3 -m server harvest jsonboards --report     # counts only, write nothing

The vendors and why each is worth a slot:

    workable          apply.workable.com    one response, all jobs, real `published_on`
    smartrecruiters   api.smartrecruiters   paginated, date-sorted, real `releasedDate`
    rippling          ats.rippling.com      no publish date anywhere in the feed
    bamboohr          {co}.bamboohr.com     no publish date anywhere in the feed
    recruitee         {co}.recruitee.com    real `published_at`
    breezy            {co}.breezy.hr        real `published_date`
    teamtailor        {co}.teamtailor.com   JSON Feed, real `date_published`

BambooHR and Rippling are the two that carry no date at all: their postings arrive with
`age_days: None`, are labelled as such, and are reported once and then absorbed by the
ledger, because there is nothing to age them by.
"""
import argparse
import json
import os
import re
import sys

from ..feed import load_feed  # noqa: E402

from ...config import ROOT
OUT = os.path.join(ROOT, "data", "config", "json_boards.yaml")

# vendor -> (regex capturing the slug, human name)
VENDORS = {
    "workable": (re.compile(r"apply\.workable\.com/([^/\"'?#]+)/j/", re.I), None),
    "smartrecruiters": (re.compile(r"jobs\.smartrecruiters\.com/([^/\"'?#]+)/", re.I), None),
    "rippling": (re.compile(r"ats\.rippling\.com/([^/\"'?#]+)/jobs/", re.I), None),
    "bamboohr": (re.compile(r"https?://([a-z0-9-]+)\.bamboohr\.com/", re.I), None),
    "recruitee": (re.compile(r"https?://([a-z0-9-]+)\.recruitee\.com/", re.I), None),
    "breezy": (re.compile(r"https?://([a-z0-9-]+)\.breezy\.hr/", re.I), None),
    "teamtailor": (re.compile(r"https?://([a-z0-9-]+)\.teamtailor\.com/", re.I), None),
}

HEADER = """# JSON-vendor job boards, harvested from the Simplify feed.
#
# Seven vendors, one shape: a slug in a URL template that answers with JSON. All of them are
# public and unauthenticated. Regenerate with:
#
#     python3 -m server harvest jsonboards
#
# Endpoints, and the two that carry no date:
#
#     workable         https://apply.workable.com/api/v1/widget/accounts/{slug}?details=true
#     smartrecruiters  https://api.smartrecruiters.com/v1/companies/{slug}/postings
#     rippling         https://ats.rippling.com/api/v1/board/{slug}/jobs        (no date)
#     bamboohr         https://{slug}.bamboohr.com/careers/list                 (no date)
#     recruitee        https://{slug}.recruitee.com/api/offers/
#     breezy           https://{slug}.breezy.hr/json
#     teamtailor       https://{slug}.teamtailor.com/jobs.json
#
# SmartRecruiters answers 0 for some slugs that plainly have jobs, so treat an empty result
# as "check the slug casing" rather than "this company has nothing". Rippling slugs often
# keep a `-careers` suffix; do not strip it.
"""


def harvest(records):
    out = {}
    for vendor, (pattern, _) in VENDORS.items():
        seen, rows = set(), []
        for rec in records:
            url = (rec or {}).get("url") or ""
            hit = pattern.search(url)
            if not hit:
                continue
            slug = hit.group(1)
            if slug.lower() in seen:
                continue
            seen.add(slug.lower())
            rows.append(slug)
        out[vendor] = sorted(rows, key=str.lower)
    return out


def main(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument("--report", action="store_true", help="print, do not write")
    ap.add_argument("--refresh", action="store_true", help="ignore the cached feed")
    args = ap.parse_args(argv)

    try:
        records, origin = load_feed(refresh=args.refresh)
    except Exception as exc:  # noqa: BLE001
        sys.stderr.write("jsonboards_harvest: could not load the feed: %s\n" % exc)
        return 1

    boards = harvest(records)
    total = sum(len(v) for v in boards.values())
    print("[harvest] feed=%s (%d records) -> %d boards across %d vendors"
          % (origin, len(records), total, len(boards)))
    for vendor, slugs in boards.items():
        print("   %-16s %4d" % (vendor, len(slugs)))

    if args.report:
        return 0
    body = [HEADER]
    for vendor, slugs in boards.items():
        body.append("\n# --- %s (%d boards) ---" % (vendor, len(slugs)))
        body.append("boards_%s:" % vendor)
        for slug in slugs:
            body.append("  - {name: %s, ats: %s, slug: %s}"
                        % (json.dumps(slug), vendor, json.dumps(slug)))
    tmp = "%s.%d.tmp" % (OUT, os.getpid())
    with open(tmp, "w", encoding="utf-8") as fh:
        fh.write("\n".join(body) + "\n")
    os.replace(tmp, OUT)
    print("[harvest] wrote %s (%d boards)" % (OUT, total))
    return 0


if __name__ == "__main__":
    sys.exit(main())
