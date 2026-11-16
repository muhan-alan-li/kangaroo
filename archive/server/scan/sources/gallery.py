"""startups.gallery: about 20 of the newest postings per run, each one a deep
link into the employer's own ATS."""

from __future__ import annotations

import html
import re

from ..core import http_get, strip_tags
from ..parsing import parse_age_days, parse_date
from ..posting import apply_signature, make_posting


def src_startups_gallery(cfg, filters):
    """startups.gallery: early-stage startups, newest postings first.

    The site is a Framer page, so it has no API and no pagination. The HTML carries about
    20 server-rendered cards, newest first, and each one deep-links into the employer's
    own ATS. A run therefore sees today's slice only; the daily drip is the point, and
    the ledger absorbs the repeats.

    Each card also names the board it came from, and every posting carries that slug in
    `board_slug`. main() harvests those into data/state/sg_boards.json, which server/jobs/probe_ats.py
    reads: a company seen here once then gets its whole board probed, instead of only the
    one posting that surfaced today.

    The card text runs "Title | Company · City, Remote, City · Posted on Sep 28, 2026",
    so the title comes from the h2 and the rest is split on the middle dot. The tail of
    the page (footer links) is cut at the card's closing anchor.
    """
    page = http_get(cfg.get("url", "https://startups.gallery/jobs"), timeout=60, retries=1)
    if not page:
        return []

    out = []
    for block in re.split(r'(?=<a class="framer-zpdfqp)', page)[1:]:
        if not block.startswith('<a class="framer-zpdfqp'):
            continue
        block = block.split("</a>", 1)[0]
        anchor = re.match(r'<a[^>]*href="([^"]+)"', block)
        if not anchor:
            continue
        apply_url = html.unescape(anchor.group(1))
        vendor, slug, job_id, board_key = apply_signature(apply_url)
        if not vendor:
            continue                      # nav, sponsor slot, "see all jobs"
        head = re.search(r"<h2[^>]*>(.*?)</h2>", block, re.S)
        if not head:
            continue
        title = strip_tags(head.group(1))
        fields = [f.strip() for f in strip_tags(block[head.end():]).split("\u00b7") if f.strip()]
        company = fields[0] if fields else ""
        posted, where = None, []
        for field in fields[1:]:
            stamp = re.match(r"Posted on (.+)$", field)
            if stamp:
                posted = parse_date(stamp.group(1))
                continue
            where.append(field)
        if not (title and company):
            continue
        out.append(make_posting(
            source="startups_gallery", company=company, title=title,
            location=", ".join(where), url=apply_url, posted=posted,
            age_days=parse_age_days(None, posted), ats=vendor,
            raw_id=job_id, dedupe_key=board_key,
            extra={"board_slug": slug},
        ))
    return out
