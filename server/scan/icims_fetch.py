#!/usr/bin/env python3
"""Fetch and parse iCIMS job boards. Runs under .venv-icims, not the scan's interpreter.

WHY THIS IS A SEPARATE PROCESS

Every iCIMS host sits behind an AWS WAF that answers a plain HTTP client with a 405
"Human Verification" page -- urllib, and curl with a full set of browser headers, both get
it on every path including /sitemap.xml and /api/jobs. A real browser is served the page
normally and holds no aws-waf-token cookie, so the WAF is deciding on the TLS fingerprint
rather than on a token. Impersonating a browser fingerprint is the only way in: curl_cffi
with `impersonate="chrome124"` returns 200 where curl returns 405. Verified across profiles
-- chrome, chrome120, chrome124 and safari17_0 pass; chrome110 and edge101 do not.

curl_cffi does not belong in the scan's interpreter, so scan.py invokes this as a subprocess
and it speaks JSON on stdin/stdout:

    in   {"hosts": [...], "max_pages": 4, "window_days": 7, "title_words": [...], ...}
    out  {"careers-ice.icims.com": {"ok": true, "pages": 4, "rows": [...]}}

WHY IT ALSO FETCHES DETAIL PAGES

The search view has no date. The card carries none, the header fields are only Position Type
and Remote, and the sort control offers ID, Title, Location, Position Type, Remote and Job
Description with no date option. The board's default order is not a recency proxy either --
`careers-ice` happens to list ids descending, but `careers-sig` does not order by id at all,
so its newest posting sits on the last page. Checking that against the feed's own
`date_posted` values is what ruled the ordering out.

The job DETAIL page does carry schema.org JSON-LD with a real `datePosted`. So the list view
enumerates cheaply and only postings that survive the pre-filters get a detail fetch.

TWO FILTERS RUN HERE, BOTH DELIBERATELY LOOSE:
  * title   -- the same word list scan.py uses, so nothing scan.py would keep is dropped
  * country -- drops only a row whose location EXPLICITLY names a non-US/CA country; a row
               with no country prefix always survives, for scan.py's text filter to judge

scan.py re-applies its own role, region and date filters to whatever comes back. This script
may be looser than scan.py; it must never be tighter.

    ./.venv-icims/bin/python server/scan/icims_fetch.py --selftest <saved-page.html>
"""
import argparse
import html as html_mod
import json
import re
import sys
import threading
import time
import urllib.parse
from concurrent import futures

try:
    from curl_cffi import requests as cr
except ImportError:  # pragma: no cover
    sys.stderr.write("icims_fetch needs curl_cffi; run it with .venv-icims/bin/python\n")
    raise SystemExit(2)

IMPERSONATE = "chrome124"
PAGE_SIZE = 50                      # the search page is fixed at 50 rows
# `pr` is a 0-BASED PAGE INDEX, not a byte offset. Measured on careers-ice: pr=0 gives the
# first 50, pr=4 the last 22, and pr=5 an empty page. pr=50, pr=100 and pr=10 all returned
# zero cards, which is how the offset reading was ruled out.
SEARCH = "https://{host}/jobs/search?ss=1&in_iframe=1&searchRelation=keyword_all&pr={pr}"
ELIGIBLE = ("us", "ca", "pr", "gu", "vi")

TAG = re.compile(r"<[^>]+>")
WS = re.compile(r"\s+")
# Location arrives as an explicit country-state-city string on most sites: "US-FL-Jacksonville",
# "GB-London-...", "IN-KA-Bangalore". The country token is what the region filter wants, and
# having it stated beats guessing from a city name.
COUNTRY_PREFIX = re.compile(r"^([A-Za-z]{2})-[A-Za-z0-9]{1,3}-")
DATEPOSTED = re.compile(r'"datePosted"\s*:\s*"([0-9]{4}-[0-9]{2}-[0-9]{2})')


def text(fragment):
    return WS.sub(" ", html_mod.unescape(TAG.sub(" ", fragment or ""))).strip()


def split_label(fragment):
    """Strip the screen-reader field label iCIMS puts in front of every value."""
    return text(re.sub(r'<span[^>]*field-label[^>]*>.*?</span>', " ", fragment, flags=re.S))


def parse_page(page_html, host):
    """Every job card on one search page, as plain rows."""
    rows = []
    for chunk in page_html.split('class="iCIMS_JobCardItem"')[1:]:
        chunk = chunk[:6000]
        link = re.search(r'href="([^"]*?/jobs/(\d+)/[^"]*?)"', chunk)
        if not link:
            continue
        job_id = link.group(2)
        title_m = (re.search(r"<h3[^>]*>(.*?)</h3>", chunk, re.S)
                   or re.search(r'title="[^"]*?\d+\s*-\s*([^"]+)"', chunk))
        title = text(title_m.group(1)) if title_m else ""
        if not title:
            continue
        # The card lays its fields out in fixed divs before the title.
        head = chunk[:link.start()]
        loc_m = re.search(r'class="col-xs-6 header left"[^>]*>(.*?)</div>', head, re.S)
        location = split_label(loc_m.group(1)) if loc_m else ""
        other = [text(m.group(1)) for m in
                 re.finditer(r'field-label">Other Locations</span>(.*?)</dd>', chunk, re.S)]
        code = ""
        hit = COUNTRY_PREFIX.match(location)
        if hit:
            code = hit.group(1).lower()
        rows.append({
            "id": job_id,
            "title": title,
            "location": location,
            "country_code": code,
            "other_locations": [x for x in other if x],
            "url": "https://%s/jobs/%s/job" % (host, job_id),
        })
    return rows


# One curl handle per worker thread. `cr.get` builds and tears down a session for every
# request, so libcurl's connection pool and DNS cache never survive to the next call: the
# same host is reconnected and re-resolved for each of a board's pages and each of its
# detail pages. A thread-local session keeps them, which matters most for exactly the work
# this process does -- every iCIMS host is one host fetched many times.
_LOCAL = threading.local()


def _session():
    session = getattr(_LOCAL, "session", None)
    if session is None:
        session = cr.Session(impersonate=IMPERSONATE)
        _LOCAL.session = session
    return session


def get(url, timeout, attempts=3):
    """One GET, retried. Returns the body or None."""
    for attempt in range(max(1, attempts)):
        if attempt:
            time.sleep(0.5 * attempt)
        try:
            resp = _session().get(url, timeout=timeout)
            if resp.status_code == 200:
                return resp.text
        except Exception:  # noqa: BLE001 -- retried, then reported by the caller
            pass
    return None


def fetch_pages(host, max_pages, timeout, keywords=()):
    """Enumerate a board, then backfill any board that ran past the page cap.

    A board that fills every page we asked for is flagged `truncated`, because its default
    order is not by date and a recent posting could sit past the cap. For those boards only,
    a keyword search narrows the result set server-side -- on `careers-ice` the plain listing
    needs 5 pages while `searchKeyword=engineer` needs 2 -- so the same small budget reaches
    every engineering posting instead of an arbitrary first slice. The pass is skipped for
    boards that already fit.
    """
    rows, seen, pages = [], set(), 0
    ok, truncated = True, False
    for page in range(max(1, max_pages)):
        body = get(SEARCH.format(host=host, pr=page), timeout)
        if body is None:
            if page == 0:
                ok = False
            break
        found = parse_page(body, host)
        pages += 1
        for row in found:
            if row["id"] not in seen:
                seen.add(row["id"])
                rows.append(row)
        if len(found) < PAGE_SIZE:
            break                      # last page of this board
        if page + 1 == max_pages:
            truncated = True

    if truncated and keywords:
        for keyword in keywords:
            suffix = "&searchKeyword=" + urllib.parse.quote(keyword)
            for page in range(max(1, max_pages)):
                body = get(SEARCH.format(host=host, pr=page) + suffix, timeout)
                if not body:
                    break
                found = parse_page(body, host)
                for row in found:
                    if row["id"] not in seen:
                        seen.add(row["id"])
                        rows.append(row)
                if len(found) < PAGE_SIZE:
                    break
    return {"ok": ok, "pages": pages, "truncated": truncated, "rows": rows}


def fetch_date(url, timeout):
    """The publish date from a job detail page's schema.org JSON-LD, or None.

    `?in_iframe=1` is mandatory here: without it the page is a 129 KB JavaScript shell with
    no JSON-LD block at all, while with it the same page is 38 KB and carries `datePosted`.
    The plain URL is what the posting should link to, so the parameter is added only for
    this fetch rather than baked into the row.
    """
    detail = url if "in_iframe=" in url else url + "?in_iframe=1"
    body = get(detail, timeout, attempts=2)
    if not body:
        return None
    hit = DATEPOSTED.search(body)
    return hit.group(1) if hit else None


def title_ok(title, words):
    if not words:
        return True
    return re.search(r"\b(" + "|".join(re.escape(w) for w in words) + r")\b",
                     title, re.I) is not None


def country_ok(row):
    """Drop only a row that explicitly names a non-US/CA country."""
    code = (row.get("country_code") or "").strip().lower()
    if not code:
        return True                    # nothing stated: let scan.py's text filter decide
    return code in ELIGIBLE


def main(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument("--selftest", metavar="HTML", help="parse a saved page, no network")
    args = ap.parse_args(argv)

    if args.selftest:
        with open(args.selftest, encoding="utf-8") as fh:
            rows = parse_page(fh.read(), "selftest.icims.com")
        print("parsed %d rows from %s" % (len(rows), args.selftest))
        for row in rows[:3]:
            print("   %-7s %-44s %-24s cc=%s" % (row["id"], row["title"][:44],
                                                 row["location"][:24], row["country_code"]))
        return 0

    try:
        spec = json.load(sys.stdin)
    except ValueError as exc:
        sys.stderr.write("icims_fetch: bad stdin: %s\n" % exc)
        return 2

    hosts = spec.get("hosts") or []
    max_pages = int(spec.get("max_pages") or 4)
    workers = max(1, int(spec.get("workers") or 8))
    timeout = int(spec.get("timeout") or 25)
    words = tuple(spec.get("title_words") or ())
    keywords = tuple(spec.get("keywords") or ())
    details = bool(spec.get("details", True))
    window = int(spec.get("window_days") or 7)

    # Phase one: enumerate every board cheaply.
    boards, dead, truncated = {}, 0, 0
    with futures.ThreadPoolExecutor(max_workers=workers) as pool:
        pending = {pool.submit(fetch_pages, h, max_pages, timeout, keywords): h
                   for h in hosts}
        for future in futures.as_completed(pending):
            host = pending[future]
            try:
                got = future.result()
            except Exception as exc:  # noqa: BLE001
                got = {"ok": False, "pages": 0, "truncated": False, "rows": [],
                       "error": str(exc)[:120]}
            boards[host] = got
            if not got["ok"]:
                dead += 1
            if got.get("truncated"):
                truncated += 1

    # Phase two: keep what could matter, then read those detail pages for a date.
    candidates = []
    for got in boards.values():
        got["rows"] = [r for r in got["rows"]
                       if title_ok(r["title"], words) and country_ok(r)]
        candidates.extend(got["rows"])

    if details and candidates:
        with futures.ThreadPoolExecutor(max_workers=workers) as pool:
            pending = {pool.submit(fetch_date, r["url"], timeout): r for r in candidates}
            for future in futures.as_completed(pending):
                row = pending[future]
                try:
                    row["posted"] = future.result()
                except Exception:  # noqa: BLE001
                    row["posted"] = None

    # Phase three: drop what is provably older than the window. A candidate with no date is
    # kept and handed to scan.py, which treats a missing date as missing, never as fresh.
    cutoff = time.time() - window * 86400
    for got in boards.values():
        final = []
        for row in got["rows"]:
            posted = row.get("posted")
            if posted:
                try:
                    stamp = time.mktime(time.strptime(posted, "%Y-%m-%d"))
                except ValueError:
                    stamp = None
                if stamp is not None and stamp < cutoff:
                    continue
            final.append(row)
        got["rows"] = final

    total = sum(len(v["rows"]) for v in boards.values())
    dated = sum(1 for v in boards.values() for r in v["rows"] if r.get("posted"))
    sys.stderr.write(
        "[icims] %d hosts, %d candidates, %d kept within %dd (%d dated, %d undated), "
        "%d boards did not answer, %d hit the page cap\n"
        % (len(hosts), len(candidates), total, window, dated, total - dated, dead, truncated))
    json.dump(boards, sys.stdout)
    return 0


if __name__ == "__main__":
    sys.exit(main())
