"""LinkedIn's public guest search. Read only, never signed in; applying there
stays manual."""

from __future__ import annotations

from datetime import datetime
import re
import time
import urllib.error
import urllib.parse
import urllib.request

from ..core import http_get, strip_tags
from ..parsing import parse_age_days
from ..posting import make_posting


def src_linkedin(cfg, filters):
    """LinkedIn's public guest job search, unauthenticated.

    Read-only by design: this uses the same endpoint the logged-out jobs page does,
    so no LinkedIn account is involved and no application is ever submitted. The
    policy file still marks LinkedIn manual_only for *applying* -- this source only
    finds postings, it never fills a form.
    """
    out = []
    base = cfg.get("url", "https://www.linkedin.com/jobs-guest/jobs/api/seeMoreJobPostings/search")
    for location in cfg.get("locations") or []:
        for keywords in cfg.get("keywords") or []:
            query = urllib.parse.urlencode({
                "keywords": keywords, "location": location, "start": 0, "f_TPR": cfg.get("recency", ""),
            })
            page = http_get(f"{base}?{query}", timeout=30, retries=1)
            if not page:
                continue
            for card in re.split(r"<li\b", page)[1:]:
                urn = re.search(r'data-entity-urn="urn:li:jobPosting:(\d+)"', card)
                title = re.search(r'class="base-search-card__title[^"]*">\s*(.*?)\s*</h3>', card, re.S)
                company = re.search(r'class="base-search-card__subtitle[^"]*">.*?>\s*(.*?)\s*</a>', card, re.S)
                where = re.search(r'class="job-search-card__location">\s*(.*?)\s*</span>', card, re.S)
                when = re.search(r'class="job-search-card__listdate[^"]*"\s+datetime="([\d-]+)"', card)
                if not (urn and title):
                    continue
                posted = None
                if when:
                    try:
                        posted = datetime.fromisoformat(when.group(1)).date()
                    except ValueError:
                        posted = None
                out.append(make_posting(
                    source="linkedin",
                    company=strip_tags(company.group(1)) if company else "Unknown",
                    title=strip_tags(title.group(1)),
                    location=strip_tags(where.group(1)) if where else location,
                    url=f"https://www.linkedin.com/jobs/view/{urn.group(1)}/",
                    posted=posted, age_days=parse_age_days(None, posted),
                    raw_id=urn.group(1),
                ))
            time.sleep(1.2)          # be a polite guest; this endpoint is not an API
    return out
