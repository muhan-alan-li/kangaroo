"""The uniform JSON vendors: Workable, SmartRecruiters, Rippling,
BambooHR, Recruitee, Breezy and Teamtailor. One shape, seven dialects."""

from __future__ import annotations

from concurrent import futures
from datetime import datetime, timezone
import json
import re
import sys
import urllib.error
import urllib.parse
import urllib.request

from .. import boardstate
from ..core import WORKDAY_ROLE_WORDS, http_get
from ..parsing import parse_age_days
from ..posting import apply_signature, make_posting
from .workday import workday_region_ok


# ------------------------------------------------------------- uniform JSON vendors
#
# Seven vendors that share one shape: a slug in a URL template that answers with JSON. All
# are public and unauthenticated. Their field names differ, so each has a small extractor;
# the fetch, the pool, the filters and the dedupe key are shared with the Workday path.
#
# Two of them publish no date at all -- Rippling's board payload has no publish field and
# BambooHR's has none either -- so their postings arrive with `age_days: None` and
# `date_missing: True` rather than being treated as fresh. There is nothing to age them by,
# so the ledger reports each once and then absorbs it.

JSON_VENDOR_URLS = {
    "workable": "https://apply.workable.com/api/v1/widget/accounts/{slug}?details=true",
    "smartrecruiters": ("https://api.smartrecruiters.com/v1/companies/{slug}"
                        "/postings?limit=100&offset={offset}"),
    "rippling": "https://ats.rippling.com/api/v1/board/{slug}/jobs",
    "bamboohr": "https://{slug}.bamboohr.com/careers/list",
    "recruitee": "https://{slug}.recruitee.com/api/offers/",
    "breezy": "https://{slug}.breezy.hr/json",
    "teamtailor": "https://{slug}.teamtailor.com/jobs.json",
}


# Where the postings live inside each response. None means the body already is the list.
JSON_VENDOR_LIST = {
    "workable": "jobs", "smartrecruiters": "content", "rippling": None,
    "bamboohr": "result", "recruitee": "offers", "breezy": None, "teamtailor": "items",
}


# Only SmartRecruiters needs paging, and its postings come back date-sorted newest first,
# so it can stop the moment it crosses the window the way Workday does. The rest hand over
# the whole board in one response and are filtered by date instead.
JSON_VENDOR_PAGED = ("smartrecruiters",)


JSON_VENDOR_PAGE_SIZE = 100


# SmartRecruiters hands back a bare two-letter country code, and that code collides with the
# US state abbreviations the text filter looks for: "Ludwigshafen, RP, de" reads as Delaware,
# "Barcelona, CT, es" reads as Connecticut. Measured on a 60-board sample, 581 overseas
# postings slipped through that way. Where a vendor gives a structured country, that field is
# authoritative and the text heuristic is skipped entirely -- the map below is only for making
# the location string readable in the portal.
ISO_COUNTRIES = {
    "us": "United States", "ca": "Canada", "gb": "United Kingdom", "ie": "Ireland",
    "au": "Australia", "nz": "New Zealand", "sg": "Singapore", "in": "India",
    "pk": "Pakistan", "bd": "Bangladesh", "lk": "Sri Lanka", "np": "Nepal",
    "cn": "China", "hk": "Hong Kong", "tw": "Taiwan", "jp": "Japan", "kr": "South Korea",
    "vn": "Vietnam", "th": "Thailand", "id": "Indonesia", "my": "Malaysia",
    "ph": "Philippines", "kh": "Cambodia", "mm": "Myanmar", "mn": "Mongolia",
    "fr": "France", "de": "Germany", "at": "Austria", "ch": "Switzerland",
    "nl": "Netherlands", "be": "Belgium", "lu": "Luxembourg", "es": "Spain",
    "pt": "Portugal", "it": "Italy", "gr": "Greece", "mt": "Malta", "gi": "Gibraltar",
    "se": "Sweden", "no": "Norway", "dk": "Denmark", "fi": "Finland", "is": "Iceland",
    "pl": "Poland", "cz": "Czechia", "sk": "Slovakia", "hu": "Hungary", "ro": "Romania",
    "bg": "Bulgaria", "hr": "Croatia", "si": "Slovenia", "rs": "Serbia", "ba": "Bosnia",
    "mk": "North Macedonia", "al": "Albania", "ee": "Estonia", "lv": "Latvia",
    "lt": "Lithuania", "md": "Moldova", "ua": "Ukraine", "ru": "Russia", "tr": "Turkey",
    "il": "Israel", "ae": "United Arab Emirates", "sa": "Saudi Arabia", "qa": "Qatar",
    "kw": "Kuwait", "bh": "Bahrain", "om": "Oman", "jo": "Jordan", "lb": "Lebanon",
    "iq": "Iraq", "ir": "Iran", "eg": "Egypt", "ma": "Morocco", "tn": "Tunisia",
    "dz": "Algeria", "ly": "Libya", "za": "South Africa", "ng": "Nigeria", "ke": "Kenya",
    "gh": "Ghana", "tz": "Tanzania", "ug": "Uganda", "zm": "Zambia", "zw": "Zimbabwe",
    "mz": "Mozambique", "ao": "Angola", "cm": "Cameroon", "sn": "Senegal",
    "mu": "Mauritius", "mg": "Madagascar", "bw": "Botswana", "na": "Namibia",
    "rw": "Rwanda", "et": "Ethiopia", "sd": "Sudan", "so": "Somalia",
    "mx": "Mexico", "br": "Brazil", "ar": "Argentina", "cl": "Chile", "co": "Colombia",
    "pe": "Peru", "ec": "Ecuador", "bo": "Bolivia", "py": "Paraguay", "uy": "Uruguay",
    "ve": "Venezuela", "cr": "Costa Rica", "pa": "Panama", "gt": "Guatemala",
    "hn": "Honduras", "sv": "El Salvador", "ni": "Nicaragua", "cu": "Cuba",
    "do": "Dominican Republic", "jm": "Jamaica", "tt": "Trinidad and Tobago",
    "bs": "Bahamas", "bb": "Barbados", "bm": "Bermuda",
    "kz": "Kazakhstan", "uz": "Uzbekistan", "az": "Azerbaijan", "ge": "Georgia",
    "am": "Armenia", "cy": "Cyprus", "fj": "Fiji", "pg": "Papua New Guinea",
    "nc": "New Caledonia", "pf": "French Polynesia", "pm": "Saint Pierre",
    # US territories: their codes are not "us", but the roles are US-eligible.
    "pr": "United States", "gu": "United States", "vi": "United States",
}


# Country codes this search will accept from a vendor's own structured field.
ELIGIBLE_COUNTRIES = ("us", "ca", "pr", "gu", "vi")


def _country_display(code):
    return ISO_COUNTRIES.get(str(code or "").strip().lower(), str(code or "").strip())


def _row_region_ok(row):
    """Canada/US only, trusting a vendor's stated country over the location text.

    The text heuristic exists for sources that give nothing but a location string, and it
    has to keep guessing at those. Where a vendor states the country, that field is
    authoritative and the guess is skipped -- SmartRecruiters' codes collide head-on with US
    state abbreviations ("de" is Delaware, "CT" is Connecticut), which is how 581 overseas
    postings survived a single 60-board sample.
    """
    code = (row.get("country_code") or "").strip().lower()
    if code:
        return code in ELIGIBLE_COUNTRIES
    return workday_region_ok(row.get("location") or "")


def _as_date(value):
    """These vendors spell dates six ways; every one of them starts YYYY-MM-DD."""
    hit = re.match(r"(\d{4})-(\d{2})-(\d{2})", str(value or "").strip())
    if not hit:
        return None
    try:
        return datetime(int(hit.group(1)), int(hit.group(2)), int(hit.group(3)),
                        tzinfo=timezone.utc).date()
    except ValueError:
        return None


def _loc_name(value):
    """A location arrives as a string, as {name: ...}, or as a list of either."""
    if not value:
        return ""
    if isinstance(value, str):
        return value.strip()
    if isinstance(value, dict):
        return str(value.get("name") or value.get("label") or "").strip()
    if isinstance(value, list):
        return ", ".join(x for x in (_loc_name(v) for v in value) if x)
    return str(value).strip()


def _join(*parts):
    return ", ".join(str(p).strip() for p in parts if p and str(p).strip())


def _url_id(url, pattern):
    """The id the vendor puts in its own job links -- the one the feed links agree with."""
    hit = re.search(pattern, url or "")
    return hit.group(1) if hit else None


def json_vendor_row(vendor, job, slug):
    """Normalise one vendor record into title / location / posted / url / raw_id."""
    if vendor == "workable":
        return {
            "title": job.get("title"),
            "location": _join(job.get("city"), job.get("state"), job.get("country")),
            "posted": _as_date(job.get("published_on")),
            "url": job.get("url") or job.get("application_url"),
            "raw_id": job.get("shortcode") or job.get("code"),
        }
    if vendor == "smartrecruiters":
        where = job.get("location") or {}
        jid = job.get("id")
        code = where.get("country") or ""
        return {
            "title": job.get("name"),
            # Expanded for the portal row, carried raw so the region filter can use it directly
            # instead of pattern-matching a two-letter code against US state abbreviations.
            "location": _join(where.get("city"), where.get("region"), _country_display(code)),
            "country_code": code,
            "posted": _as_date(job.get("releasedDate")),
            # The API returns no link of its own. This is the shape of the site's URL, and
            # it carries the same id the feed's links do, so the two paths agree on the key.
            "url": f"https://jobs.smartrecruiters.com/{slug}/{jid}" if jid else None,
            "raw_id": str(jid or job.get("uuid") or ""),
        }
    if vendor == "rippling":
        return {
            "title": job.get("name"),
            "location": _loc_name(job.get("workLocation")),
            "posted": None,
            "url": job.get("url"),
            "raw_id": job.get("uuid"),
        }
    if vendor == "bamboohr":
        where = job.get("location") or {}
        jid = job.get("id")
        return {
            "title": job.get("jobOpeningName"),
            "location": (_join(where.get("city"), where.get("state"))
                         or _loc_name(job.get("atsLocation"))),
            "posted": None,
            "url": f"https://{slug}.bamboohr.com/careers/{jid}/" if jid else None,
            "raw_id": str(jid or ""),
        }
    if vendor == "recruitee":
        return {
            "title": job.get("title"),
            "location": _join(job.get("city"), job.get("state_name"), job.get("country")),
            "posted": _as_date(job.get("published_at")),
            # Keyed on the offer slug, because that is what the feed's links carry.
            "url": job.get("careers_url") or (
                f"https://{slug}.recruitee.com/o/{job.get('slug')}" if job.get("slug") else None),
            "raw_id": job.get("slug") or str(job.get("id") or ""),
        }
    if vendor == "breezy":
        # Links carry the bare id, while `friendly_id` is "{id}-{slug}", so the bare id is
        # the only field that can match what the feed links say.
        return {
            "title": job.get("name"),
            "location": _loc_name(job.get("location")) or _loc_name(job.get("locations")),
            "posted": _as_date(job.get("published_date")),
            "url": job.get("url"),
            "raw_id": job.get("id"),
        }
    if vendor == "teamtailor":
        where = (job.get("_jobposting") or {}).get("jobLocation") or {}
        # schema.org allows jobLocation to be an array of places, so this arrives as a list
        # on some boards and a bare object on others.
        if isinstance(where, list):
            where = where[0] if where and isinstance(where[0], dict) else {}
        address = where.get("address") or {}
        return {
            "title": job.get("title"),
            "location": _join(address.get("addressLocality"), address.get("addressRegion"),
                              _country_display(address.get("addressCountry"))),
            "posted": _as_date(job.get("date_published")),
            # The JSON Feed's own id is a UUID; the number in its link is what the site uses.
            "url": job.get("url"),
            "raw_id": _url_id(job.get("url"), r"/jobs/(\d+)"),
        }
    return {}


def _json_vendor_board(board, window, timeout, role_re, max_pages):
    """Newest postings from one JSON-vendor board, with the same three basic filters."""
    vendor, slug = board["ats"], board["slug"]
    rows, dropped, undated = [], 0, 0
    pages = max_pages if vendor in JSON_VENDOR_PAGED else 1
    offset = 0
    for _ in range(max(1, pages)):
        url = JSON_VENDOR_URLS[vendor].format(
            slug=urllib.parse.quote(slug, safe=""), offset=offset)
        text = http_get(url, timeout=timeout, retries=1)
        if text is None:
            return rows, False, dropped, undated
        try:
            data = json.loads(text)
        except ValueError:
            return rows, False, dropped, undated
        key = JSON_VENDOR_LIST[vendor]
        page = data if key is None else (data.get(key) or [])
        if not isinstance(page, list) or not page:
            return rows, True, dropped, undated
        for job in page:
            if not isinstance(job, dict):
                continue
            row = json_vendor_row(vendor, job, slug)
            if not row.get("title"):
                continue
            age = parse_age_days(None, row.get("posted"))
            if age is not None and age > window:
                if vendor in JSON_VENDOR_PAGED:
                    # Date-sorted newest first, so the rest of the board is older still.
                    return rows, True, dropped, undated
                continue
            if role_re is not None and not role_re.search(row["title"]):
                dropped += 1
                continue
            if not _row_region_ok(row):
                dropped += 1
                continue
            if age is None:
                undated += 1
            rows.append((row, age))
        if len(page) < JSON_VENDOR_PAGE_SIZE:
            break
        offset += JSON_VENDOR_PAGE_SIZE
    return rows, True, dropped, undated


def fetch_json_boards(boards, cfg):
    """Read every board of every JSON vendor in one parallel batch.

    One request per board for six of the seven: these endpoints hand back the whole board
    at once, so there is nothing to page. The three basic filters run here rather than
    downstream, so a board's overseas and non-engineering reqs never become postings.
    """
    opts = cfg.get("jsonboards") or {}
    workers = max(1, int(opts.get("workers") or 12))
    window = int(opts.get("window_days") or 7)
    timeout = int(opts.get("timeout") or 25)
    max_pages = int(opts.get("max_pages") or 2)
    words = tuple(opts.get("title_words") or WORKDAY_ROLE_WORDS)
    role_re = re.compile(r"\b(" + "|".join(re.escape(w) for w in words) + r")\b", re.I)

    out, dead, filtered, undated = [], 0, 0, 0
    kept = {}
    with futures.ThreadPoolExecutor(max_workers=workers) as pool:
        pending = {pool.submit(_json_vendor_board, b, window, timeout, role_re,
                               max_pages): b for b in boards}
        for future in futures.as_completed(pending):
            board = pending[future]
            err = None
            try:
                rows, ok, skipped, no_date = future.result()
            except Exception as exc:  # noqa: BLE001 -- a dead board must not kill the scan
                rows, ok, skipped, no_date = [], False, 0, 0
                err = f"{type(exc).__name__}: {exc}"
            filtered += skipped
            undated += no_date
            # The 513 JSON-vendor boards get the same health row the Workday fleet has, so a
            # board that stops answering shows up on the dashboard instead of being a stderr
            # line nobody sees. The key is the posting's own `vendor|slug`, two segments, so
            # it can never collide with a Workday `tenant|wd|site` row; retry and prune look
            # keys up in their own config lists and simply skip these.
            boardstate.note(f"{board['ats']}|{board['slug']}", ok,
                            hits=len(rows), err=err, name=board.get("name"))
            if not ok:
                dead += 1
                continue
            vendor, slug = board["ats"], board["slug"]
            company = (board.get("name") or slug).strip()
            for row, age in rows:
                # Build the key from the same function the feed sources use, so one posting
                # found twice lands on one id instead of being reported twice.
                _v, _s, job_id, dedupe_key = apply_signature(row.get("url") or "")
                if not dedupe_key:
                    job_id = row.get("raw_id")
                    dedupe_key = (f"ats|{vendor}:{slug.lower()}:{job_id}" if job_id else None)
                extra = {"board": f"{vendor}|{slug}"}
                if age is None:
                    extra["date_missing"] = True
                out.append(make_posting(
                    source="ats", company=company, title=row["title"],
                    location=row.get("location") or "", url=row.get("url"),
                    posted=row.get("posted"), age_days=age, ats=vendor,
                    raw_id=row.get("raw_id"), dedupe_key=dedupe_key, extra=extra,
                ))
                kept[vendor] = kept.get(vendor, 0) + 1
    if dead:
        sys.stderr.write(f"[scan] jsonboards: {dead} of {len(boards)} boards did not answer\n")
    breakdown = ", ".join(f"{v} {n}" for v, n in sorted(kept.items()))
    sys.stderr.write(
        f"[scan] jsonboards: {len(out)} recent engineering postings in Canada/US "
        f"({filtered} dropped by the role/region filter, {undated} undated)"
        + (f" [{breakdown}]" if breakdown else "") + "\n")
    return out
