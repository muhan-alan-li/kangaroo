"""The SimplifyJobs new-grad feed, in both its forms: the JSON the repo
generates (authoritative) and the README derived from it (fallback)."""

from __future__ import annotations

from datetime import datetime, timezone
import json
import re
import sys

from ..core import first_href, http_get, strip_tags
from ..parsing import parse_age_days
from ..posting import apply_signature, make_posting


def src_simplify(cfg, filters):
    page = http_get(cfg.get("url", ""), timeout=45)
    if not page:
        return []
    out = []
    for row in re.findall(r"<tr>(.*?)</tr>", page, re.S):
        cells = re.findall(r"<td[^>]*>(.*?)</td>", row, re.S)
        if len(cells) < 4:
            continue
        company = strip_tags(cells[0])
        title = strip_tags(cells[1])
        location = strip_tags(cells[2])
        apply_url = first_href(cells[3])
        age_text = strip_tags(cells[4]) if len(cells) > 4 else ""
        if not (company and title):
            continue
        out.append(make_posting(
            source="newgrad_list",
            company=re.sub(r"^[^\w]+", "", company),
            title=title,
            location=location,
            url=apply_url,
            age_days=parse_age_days(age_text),
            raw_id=f"{company}|{title}|{location}",
            extra={"age_text": age_text},
        ))
    return out


def src_simplify_json(cfg, filters):
    """SimplifyJobs/New-Grad-Positions, read from the JSON the repo generates itself.

    The repo's README.md is *derived* from .github/scripts/listings.json, and the
    derivation loses one thing that matters: closed postings are demoted into collapsed
    <details> blocks that still contain a full <table>, so an HTML scrape cannot tell a
    live row from a dead one -- src_simplify reads every row and trusts max_age_days to
    sort it out, which it cannot, because a posting closed today is still young. The JSON
    carries `active` and `is_visible` as flags, plus the stable `id`, the exact
    `date_posted` epoch and the employer's own ATS URL.

    Kept alongside src_simplify rather than replacing it. Both are cheap, the overlap
    collapses on the dedupe key below, and the HTML view keeps working if the repo ever
    moves the JSON.
    """
    url = cfg.get("url")
    if not url:
        return []
    window = int(cfg.get("window_days") or 30)
    excluded = {s.strip().lower() for s in (cfg.get("exclude_sponsorship") or [])}
    raw = http_get(url, timeout=90, retries=1)      # ~13 MB; one retry, not two
    if not raw:
        return []
    try:
        records = json.loads(raw)
    except (ValueError, TypeError):
        sys.stderr.write("[scan] simplify_json: response was not JSON\n")
        return []
    if isinstance(records, dict):
        records = records.get("listings") or []

    out, closed, old, nosponsor = [], 0, 0, 0
    for rec in records:
        if not isinstance(rec, dict):
            continue
        if not (rec.get("active") and rec.get("is_visible")):
            closed += 1
            continue
        sponsorship = (rec.get("sponsorship") or "").strip()
        if sponsorship.lower() in excluded:
            nosponsor += 1
            continue
        # The feed holds the full history, so bound the work before building postings.
        # The global max_age_days filter still runs afterwards and is the authority.
        posted = None
        stamp = rec.get("date_posted")
        if isinstance(stamp, (int, float)) and stamp > 0:
            posted = datetime.fromtimestamp(stamp, timezone.utc).date()
        age_days = parse_age_days(None, posted)
        if age_days is not None and age_days > window:
            old += 1
            continue
        company = (rec.get("company_name") or "").strip()
        title = (rec.get("title") or "").strip()
        if not (company and title):
            continue
        apply_url = (rec.get("url") or "").strip() or None
        # Where the link is a board deep link, key on the ATS job id so a posting the ats
        # source also found is reported once. Otherwise the feed's own UUID is the key.
        _vendor, _slug, job_id, board_key = apply_signature(apply_url)
        out.append(make_posting(
            source=cfg.get("id") or "newgrad_json",
            company=company,
            title=title,
            location=" | ".join(x for x in (rec.get("locations") or []) if x),
            url=apply_url,
            posted=posted,
            age_days=age_days,
            raw_id=str(rec.get("id") or "") or None,
            dedupe_key=board_key,
            extra={
                "sponsorship": sponsorship or None,
                "degrees": rec.get("degrees") or [],
                "company_url": rec.get("company_url"),
                "feed": "listings.json",
            },
        ))
    sys.stderr.write(
        "[scan] simplify_json: %d recent (skipped %d closed, %d past %dd, %d no-sponsorship)\n"
        % (len(out), closed, old, window, nosponsor))
    return out
