#!/usr/bin/env python3
"""Recent postings from the SimplifyJobs new-grad feed, one line each, greppable.

This reads .github/scripts/listings.json -- the file the repo generates its README from --
so it can tell a live posting from a closed one, and it knows the exact post date instead
of a rounded "1d". It does no scoring, no tiering and no ledger: it is the quick look.

    python3 -m server feed                       # last 7 days, everything
    python3 -m server feed --days 1              # posted since yesterday
    python3 -m server feed --where 'canada'      # Canada only
    python3 -m server feed --where 'vancouver|burnaby|richmond'
    python3 -m server feed --grep 'new grad|graduate|junior'
    python3 -m server feed --json                # for jq

The raw feed is ~13 MB, so a copy is cached in data/state/ for ten minutes. Pass --refresh to
force a new download.

Output is one posting per line, newest first:

    <age>d | <company> | <title> | <locations> | [sponsor] | <url>

so the usual shell tools work on it:

    python3 -m server feed --days 30 | grep -i canada | sort -t'|' -k1 -n
"""

import argparse
import json
import re
import sys
import time

from .feed import load_feed

# The feed's only explicit "no". Everything else is "Other" or an offer.
NO_SPONSORSHIP = {"does not offer sponsorship"}


def rows(records, days, where, grep, include_closed):
    today = time.time()
    pat_where = re.compile(where, re.I) if where else None
    pat_grep = re.compile(grep, re.I) if grep else None
    out = []
    for rec in records:
        if not isinstance(rec, dict):
            continue
        if not include_closed and not (rec.get("active") and rec.get("is_visible")):
            continue
        if (rec.get("sponsorship") or "").strip().lower() in NO_SPONSORSHIP:
            continue
        stamp = rec.get("date_posted")
        if not isinstance(stamp, (int, float)) or stamp <= 0:
            continue
        age = (today - stamp) / 86400.0
        if days is not None and age > days:
            continue
        locations = " | ".join(x for x in (rec.get("locations") or []) if x)
        if pat_where and not pat_where.search(locations):
            continue
        title = (rec.get("title") or "").strip()
        company = (rec.get("company_name") or "").strip()
        if pat_grep and not pat_grep.search(company + " " + title):
            continue
        sponsor = (rec.get("sponsorship") or "").strip()
        out.append({
            "age_days": round(age, 1),
            "company": company,
            "title": title,
            "locations": locations,
            "sponsorship": sponsor if sponsor and sponsor != "Other" else "",
            "degrees": rec.get("degrees") or [],
            "posted": time.strftime("%Y-%m-%d", time.gmtime(stamp)),
            "url": rec.get("url") or "",
            "company_url": rec.get("company_url") or "",
            "id": rec.get("id") or "",
        })
    out.sort(key=lambda r: r["age_days"])
    return out


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--days", type=float, default=7,
                    help="only postings at most this many days old (default 7; 0 = no limit)")
    ap.add_argument("--where", help="regex on the locations field")
    ap.add_argument("--grep", help="regex on company and title")
    ap.add_argument("--json", action="store_true", help="emit JSON instead of lines")
    ap.add_argument("--include-closed", action="store_true",
                    help="include postings the feed has marked inactive")
    ap.add_argument("--refresh", action="store_true", help="ignore the cached copy")
    ap.add_argument("--limit", type=int, default=0, help="cap the output (0 = no cap)")
    args = ap.parse_args(argv)

    try:
        records, _origin = load_feed(refresh=args.refresh, max_age_seconds=600)
    except Exception as exc:  # noqa: BLE001
        sys.stderr.write("simplify_recent: could not load the feed: %s\n" % exc)
        return 1

    window = None if not args.days or args.days <= 0 else args.days
    found = rows(records, window, args.where, args.grep, args.include_closed)
    if args.limit:
        found = found[:args.limit]

    if args.json:
        print(json.dumps(found, indent=1))
        return 0
    label = "all" if window is None else "last %gd" % window
    sys.stderr.write("# %d postings, %s\n" % (len(found), label))
    for r in found:
        bits = ["%gd" % r["age_days"], r["company"], r["title"], r["locations"]]
        if r["sponsorship"]:
            bits.append("[%s]" % r["sponsorship"])
        bits.append(r["url"])
        print(" | ".join(bits))
    return 0


if __name__ == "__main__":
    sys.exit(main())
