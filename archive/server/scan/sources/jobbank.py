"""Job Bank Canada. Official, local, and the only Canadian source that
carries a real posted date."""

from __future__ import annotations

from concurrent import futures
import html
import re
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

from ..core import http_get, strip_tags
from ..filters import location_matches
from ..parsing import parse_age_days, parse_date
from ..posting import make_posting


# --------------------------------------------------------------------- sources

def _jobbank_detail(jid):
    """Detail-page data for one Job Bank posting.

    One fetch covers everything the posting's text needs:

      title        the employer's real title, after the partner label.
      description  the employer's own posting text. Job Bank hides it in
                   `<span class="hidden" property="description">`, HTML-escaped a
                   second time. Reading only the title made every partner
                   posting reach the portal as a "stub with no requirements" --
                   which is what happened to Presto on 2026-09-30 while the page
                   was carrying the full role description.
      deadline     `<p property="validThrough">`, the advertised-until date.
      apply_url    `<a id="externalJobLink">`, the partner's own apply route,
                   which is the only way to apply (Job Bank is a republisher).
    """
    page = http_get(f"https://www.jobbank.gc.ca/jobsearch/jobposting/{jid}",
                    timeout=25, retries=1)
    detail = {"title": None, "description": None, "deadline": None, "apply_url": None}
    if not page:
        return detail

    match = re.search(r'<span class="source-title-inner">(.*?)</span>\s*</p>', page, re.S)
    if match:
        inner = re.sub(r'<span class="orig-title-label">.*?</span>', " ", match.group(1), flags=re.S)
        detail["title"] = strip_tags(inner) or None

    match = re.search(r'property="description">(.*?)</span>', page, re.S)
    if match:
        # the page escapes the employer's HTML a second time, so unescape first:
        # stripping tags before that leaves literal "<p>" in the stored description. The
        # stored description is also cut mid-tag, hence the trailing fragment strip.
        text = strip_tags(html.unescape(match.group(1)))
        detail["description"] = re.sub(r"<[^>]*$", "", text).strip()[:6000] or None

    match = re.search(r'property="validThrough">\s*([0-9]{4}-[0-9]{2}-[0-9]{2})', page)
    if match:
        detail["deadline"] = match.group(1)

    tag = re.search(r'<a[^>]*id="externalJobLink"[^>]*>', page)
    if tag:
        href = re.search(r'href="([^"]+)"', tag.group(0))
        if href:
            detail["apply_url"] = html.unescape(href.group(1))

    return detail


def _jobbank_real_title(jid):
    """Real title only, for callers that want nothing else."""
    return _jobbank_detail(jid)["title"]


def _jobbank_extra(listing):
    """The detail-page fields that ride along with a Job Bank posting.

    `description` is what lets a reader judge a partner posting on its real
    requirements instead of calling it a stub; `apply_url` is the only route that
    can actually submit an application, because Job Bank republishes.
    """
    detail = listing.get("detail") or {}
    extra = {
        "title_generic": listing.get("title_generic"),
        "description": detail.get("description"),
        "deadline": detail.get("deadline"),
        "apply_url": detail.get("apply_url"),
    }
    return {k: v for k, v in extra.items() if v} or None


def src_jobbank(cfg, filters, max_detail_fetches=60):
    """Collect Job Bank postings.

    Two passes: the search pages give id/title/company/date/salary/location, then
    every partner listing that survives the location filter gets one detail fetch so
    the title filters run against the employer's real title rather than Job Bank's
    generic one.
    """
    listings = {}
    location = cfg.get("location", "Vancouver, BC")
    queries = cfg.get("search_strings") or ["software"]

    for query in queries:
        url = ("https://www.jobbank.gc.ca/jobsearch/jobsearch"
               f"?searchstring={urllib.parse.quote(query)}"
               f"&locationstring={urllib.parse.quote(location)}&sort=M")
        page = http_get(url)
        if not page:
            continue
        for chunk in page.split("<article")[1:]:
            chunk = chunk.split("</article>")[0]
            jid = re.search(r'id="article-(\d+)"', chunk)
            title = re.search(r'<span class="noctitle">\s*(.*?)\s*</span>', chunk, re.S)
            if not (jid and title):
                continue
            key = jid.group(1)
            if key in listings:
                continue
            business = re.search(r'<li class="business">\s*(.*?)\s*</li>', chunk, re.S)
            date = re.search(r'<li class="date">\s*(.*?)\s*</li>', chunk, re.S)
            salary = re.search(r'<li class="salary">\s*(.*?)\s*</li>', chunk, re.S)
            loc_li = re.search(r'<li class="location">\s*(.*?)\s*</li>', chunk, re.S)
            posted = parse_date(strip_tags(date.group(1)) if date else "")
            loc_text = re.sub(r"Location", "", strip_tags(loc_li.group(1)) if loc_li else "").strip()
            listings[key] = {
                "jid": key,
                "title": strip_tags(title.group(1)),
                "company": strip_tags(business.group(1)) if business else "Unknown",
                "location": loc_text or location,
                "posted": posted,
                "salary": strip_tags(salary.group(1)).replace("Salary", "").strip() if salary else None,
                # a job-source icon means this is a republished partner listing
                "partner": bool(re.search(r'class="job-source', chunk)),
            }
        time.sleep(0.6)

    # Second pass: resolve real titles for partner listings that could still match.
    # These run concurrently. Serially, with the politeness delay, they were 95s of the run
    # -- a quarter of the whole scan -- for 60 requests, nearly all of it waiting on a
    # socket. The pool stays small and a short delay stays in: Job Bank is a government site
    # and this is not an API, so the point is to stop burning wall time on idle waiting, not
    # to raise the request rate.
    targets = []
    for listing in listings.values():
        if not listing["partner"]:
            continue
        if len(targets) >= max_detail_fetches:
            break
        ok, _, _ = location_matches(listing["location"], filters)
        if ok:
            targets.append(listing)

    def resolve(listing):
        time.sleep(0.2)
        return listing, _jobbank_detail(listing["jid"])

    fetched = 0
    if targets:
        with futures.ThreadPoolExecutor(max_workers=min(5, len(targets))) as pool:
            for listing, detail in pool.map(resolve, targets):
                fetched += 1
                if detail["title"]:
                    listing["title_generic"] = listing["title"]
                    listing["title"] = detail["title"]
                listing["detail"] = detail
    sys.stderr.write(f"[scan] jobbank: {len(listings)} unique, "
                     f"{fetched} detail fetches for real titles\n")

    return [make_posting(
        source="jobbank",
        company=listing["company"],
        title=listing["title"],
        location=listing["location"],
        url=f"https://www.jobbank.gc.ca/jobsearch/jobposting/{listing['jid']}",
        posted=listing["posted"],
        age_days=parse_age_days(None, listing["posted"]),
        salary=listing["salary"],
        raw_id=listing["jid"],
        extra=_jobbank_extra(listing),
    ) for listing in listings.values()]
