"""Work at a Startup (Y Combinator): the anonymous job feed, read out of the
Inertia page payload."""

from __future__ import annotations

import html as htmlmod
import json
import re
import sys
import time
import urllib.error
import urllib.request

from ..core import UA
from ..posting import make_posting

BASE = "https://www.workatastartup.com"

# This host answers 406 to any request whose Accept header is not browser-shaped, so it can
# never go through core.http_get, which sends `Accept: */*`. The request is otherwise an
# ordinary GET: no cookie, no token, no account.
ACCEPT = "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"

# Every page is an Inertia app, and Inertia embeds the page's props as JSON in a `data-page`
# attribute on the root element. Reading that is the whole scraper. Asking for the JSON API
# instead (X-Inertia: true) needs an X-Inertia-Version header and answers 409 without one,
# and that version changes whenever YC rebuilds the front end -- so the HTML is the stable
# door, not the JSON one.
PAGE_RE = re.compile(r'data-page="([^"]+)"')

# The feed is paginated only for signed-in users: a page returns 30 jobs, `?page=2` returns
# the same 30, and every filter in the query string (role, minExperience, usVisaNotRequired,
# jobType, sortBy) is ignored for an anonymous visitor. What does vary is WHICH 30, per
# page type, so a sweep of these paths is what turns a teaser into coverage. Measured
# 2026-10-04: these twelve paths return 285 distinct jobs, 66 of them engineering-titled.
# /jobs/l/software-engineer is included for completeness even though it duplicates /jobs.
DEFAULT_PATHS = (
    "/jobs",
    "/companies?role=eng",
    "/jobs/l/software-engineer",
    "/jobs/l/designer",
    "/jobs/l/recruiting",
    "/jobs/l/science",
    "/jobs/l/product-manager",
    "/jobs/l/operations",
    "/jobs/l/sales-manager",
    "/jobs/l/marketing",
    "/jobs/l/legal",
    "/jobs/l/finance",
)


def _get(path, timeout=45, retries=1):
    """GET one path with browser-shaped headers. Never raises; returns text or None."""
    url = BASE + path if path.startswith("/") else path
    last = None
    for attempt in range(retries + 1):
        try:
            req = urllib.request.Request(url, headers={
                "User-Agent": UA,
                "Accept": ACCEPT,
                "Accept-Language": "en-CA,en;q=0.9",
            })
            with urllib.request.urlopen(req, timeout=timeout) as resp:
                return resp.read().decode("utf-8", "replace")
        except Exception as exc:  # noqa: BLE001
            last = exc
            if attempt < retries:
                time.sleep(1.5 * (attempt + 1))
    sys.stderr.write(f"[scan] fetch failed: {url} ({last})\n")
    return None


def _payload(page):
    """The page's Inertia props, or None when the page is not the shape we expect."""
    match = PAGE_RE.search(page or "")
    if not match:
        return None
    try:
        return json.loads(htmlmod.unescape(match.group(1)))
    except json.JSONDecodeError:
        return None


def src_workatastartup(cfg, filters):
    """Work at a Startup: the newest YC startup roles, deduped across a sweep of page types.

    Every posting carries `companySlug` and the company's YC batch, which is what makes this
    source worth having beside the other startup feed: it is the whole early-stage market,
    not one board.

    Two honest limits, both recorded in data/config/source_candidate.md:

    - **No publish date exists anywhere on the site.** The list payload has no date field and
      neither does the job page, so every posting carries `date_missing` -- the same
      treatment Rippling and BambooHR get. The feed is served newest-first, so the sweep is
      the recent slice, but that is not a date and the source does not pretend it is.
    - **The apply link needs a YC account.** `applyUrl` points at
      account.ycombinator.com/authenticate, so these postings are read-only here and the
      application is the candidate's to make by hand.
    """
    paths = cfg.get("paths") or DEFAULT_PATHS
    out, seen, dead = [], set(), 0
    for path in paths:
        payload = _payload(_get(path))
        if payload is None:
            dead += 1
            continue
        for job in (payload.get("props") or {}).get("jobs") or []:
            if not isinstance(job, dict):
                continue
            jid = job.get("id")
            title = (job.get("title") or "").strip()
            company = (job.get("companyName") or "").strip()
            if not jid or not title or not company or jid in seen:
                continue
            seen.add(jid)
            out.append(make_posting(
                source="workatastartup",
                company=company,
                title=title,
                location=job.get("location") or "",
                url=f"{BASE}/jobs/{jid}",
                posted=None,
                age_days=None,
                salary=job.get("salary"),
                raw_id=str(jid),
                dedupe_key=f"workatastartup|{jid}",
                extra={
                    "date_missing": True,
                    # NOT `board_slug`: that key means an ATS board slug, and harvest_boards()
                    # turns it into a probe candidate. This is the YC company slug, which is a
                    # different namespace, so it must not be able to masquerade as one.
                    "company_slug": job.get("companySlug"),
                    "batch": job.get("companyBatch"),
                    "role_type": job.get("roleType"),
                    "job_type": job.get("jobType"),
                },
            ))
    sys.stderr.write(
        f"[scan] workatastartup: {len(out)} distinct postings from {len(paths) - dead} of "
        f"{len(paths)} feed pages ({dead} did not answer; no publish date exists on this "
        f"site, so all are tagged date_missing)\n")
    return out
