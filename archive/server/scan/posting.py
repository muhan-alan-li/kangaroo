"""Posting identity: the id a posting gets, the ATS signature of an apply
URL, and the one-requisition-one-posting collapse."""

from __future__ import annotations

import hashlib
import re


# The employer's own board, read off an apply link. Only the three boards the `ats`
# source can read are useful here: anything else is reported as a plain posting.
# (vendor, URL pattern, dedupe-key template)
#
# The template exists because ids are not equally trustworthy across vendors. A Greenhouse
# id is global, so `ats|{id}` is safe, and it is what every entry used before this template
# existed -- those keys must not change, or every already-reported posting would be reported
# again. A BambooHR id is a small integer scoped to one company ("52", "448") and a Workday
# requisition is scoped to one career site, so those carry the board in the key: without it
# two companies' unrelated postings hash to the same id and one is lost silently.
APPLY_SIGNATURES = (
    ("greenhouse", re.compile(
        r"^https?://(?:job-boards|boards)\.greenhouse\.io/([^/\"'?#]+)/jobs/(\d+)", re.I),
     "ats|{id}"),
    ("ashby", re.compile(
        r"^https?://jobs\.ashbyhq\.com/([^/\"'?#]+)/([0-9a-fA-F-]{16,})", re.I),
     "ats|{id}"),
    ("lever", re.compile(
        r"^https?://jobs\.lever\.co/([^/\"'?#]+)/([0-9a-fA-F-]{16,})", re.I),
     "ats|{id}"),
    # Workday carries no numeric job id in the link: the requisition lives in the last
    # path segment ("...-New-College-Grad-2026_JR2026576"). That segment is identical
    # whether it arrives from the board API's `externalPath` or from a feed's own link,
    # which is what lets a Workday posting found twice report once. The middle is optional,
    # because most links are /job/{location}/{title} but some carry the title alone -- an
    # earlier version required two segments and silently stopped matching those.
    ("workday", re.compile(
        r"^https?://[a-z0-9-]+\.wd\d+\.myworkdayjobs\.com/(?:[a-z]{2}-[A-Z]{2}/)?"
        r"([^/\"'?#]+)/job/(?:.+?/)?([^/\"'?#]+)(?:[?#].*)?$", re.I),
     "ats|{id}"),
    ("workday", re.compile(
        r"^https?://wd\d+\.myworkdaysite\.com/(?:[a-z]{2}-[A-Z]{2}/)?recruiting/"
        r"[^/\"'?#]+/([^/\"'?#]+)/job/(?:.+?/)?([^/\"'?#]+)(?:[?#].*)?$", re.I),
     "ats|{id}"),
    # The JSON vendors. Each id is unique only inside its own board, so the board is part of
    # the key. Group 2 is deliberately the id that appears in the vendor's public job URL,
    # not whatever id the API happens to return alongside it -- Breezy returns a
    # `friendly_id` of "{id}-{slug}" while its links carry the bare id, and Teamtailor's own
    # feed id is a UUID while its links carry a number, so using the API field would mean the
    # two paths never agree.
    ("workable", re.compile(
        r"^https?://apply\.workable\.com/([^/\"'?#]+)/j/([0-9A-Fa-f]{6,})", re.I),
     "ats|workable:{slug}:{id}"),
    ("smartrecruiters", re.compile(
        r"^https?://jobs\.smartrecruiters\.com/([^/\"'?#]+)/([0-9]+)", re.I),
     "ats|smartrecruiters:{slug}:{id}"),
    ("rippling", re.compile(
        r"^https?://ats\.rippling\.com/([^/\"'?#]+)/jobs/([0-9a-fA-F-]{16,})", re.I),
     "ats|rippling:{slug}:{id}"),
    ("bamboohr", re.compile(
        r"^https?://([a-z0-9-]+)\.bamboohr\.com/careers/(\d+)", re.I),
     "ats|bamboohr:{slug}:{id}"),
    ("recruitee", re.compile(
        r"^https?://([a-z0-9-]+)\.recruitee\.com/o/([^/\"'?#]+)", re.I),
     "ats|recruitee:{slug}:{id}"),
    ("breezy", re.compile(
        r"^https?://([a-z0-9-]+)\.breezy\.hr/p/([^/\"'?#]+)", re.I),
     "ats|breezy:{slug}:{id}"),
    ("teamtailor", re.compile(
        r"^https?://([a-z0-9-]+)\.teamtailor\.com/jobs/(\d+)", re.I),
     "ats|teamtailor:{slug}:{id}"),
    # iCIMS identifies a posting by host + numeric id. The host is the board, so it is part
    # of the key for the same reason BambooHR's company is.
    ("icims", re.compile(
        r"^https?://([a-z0-9-]+)\.icims\.com/jobs/(\d+)", re.I),
     "ats|icims:{slug}:{id}"),
)


def apply_signature(url):
    """(vendor, slug, job_id, dedupe_key) for a deep link into an employer's ATS.

    One place to turn a URL into a key, so the board fetchers and the feed sources cannot
    drift apart on what counts as the same posting.
    """
    for vendor, pattern, template in APPLY_SIGNATURES:
        hit = pattern.match(url or "")
        if not hit:
            continue
        slug, job_id = hit.group(1), hit.group(2)
        key = template.format(vendor=vendor, slug=(slug or "").lower(), id=job_id)
        return vendor, slug, job_id, key
    return None, None, None, None


# Which copy of one requisition to keep when two sources deliver it. The `ats` source reads
# the employer's own board, so its link is the cleanest; the machine-readable feed is next;
# the HTML feed's link carries tracking parameters; a LinkedIn link (`linkedin` is last) is
# login-walled, so it is the copy a human cannot apply from and an agent cannot fill.
SOURCE_PREFERENCE = {"ats": 0, "newgrad_json": 1, "startups_gallery": 1, "newgrad_list": 2,
                     "remotive": 2, "jobbank": 2, "linkedin": 3}


def collapse_requisitions(postings):
    """One posting per requisition, even when several sources deliver the same one.

    The id-only collapse it replaces folds copies that already agree on a key, and a source
    that does not know the employer's requisition id gives its rows their own key. Both
    Simplify sources are on by design: the JSON view keys on the employer's ATS job id, the
    README view keys on company|title|location. On 2026-10-02 that put the same Expedia
    requisition (R-109478) into one report twice, under two ids, and the report had to say
    so. The ATS signature of the apply URL is the merge point, because it depends on no
    source's own id scheme -- a source can be added or re-keyed without re-opening this.

    Collapsing happens inside one run only. The ledger keeps every id it has ever seen and
    its keys are never rewritten, so a copy that was already reported is not resurrected as
    new: it is dropped here instead of re-reported.
    """
    kept, by_requisition = {}, {}
    for posting in postings:
        key = apply_signature(posting.get("url") or "")[3]
        if not key:
            kept.setdefault(posting["id"], posting)
            continue
        prior = by_requisition.get(key)
        if prior is None:
            by_requisition[key] = posting
            kept.setdefault(posting["id"], posting)
            continue
        this_rank = SOURCE_PREFERENCE.get(posting.get("source"), 5)
        prior_rank = SOURCE_PREFERENCE.get(prior.get("source"), 5)
        if this_rank < prior_rank:
            kept.pop(prior["id"], None)
            kept[posting["id"]] = posting
            by_requisition[key] = posting
    return list(kept.values())


# ---------------------------------------------------------------------- posting

def make_posting(source, company, title, location, url, posted=None,
                 age_days=None, salary=None, ats=None, raw_id=None, extra=None,
                 dedupe_key=None):
    """Build a posting with a stable dedupe id.

    The id prefers the source's own identifier. Deriving it from company+title+location
    instead collapses two distinct postings into one whenever a source lists two roles
    with the same title at the same employer and city -- which Job Bank does routinely,
    because it gives republished partner postings generic titles. One of the two would
    then never be reported.

    `dedupe_key` overrides that choice. A source that only carries a deep link into the
    employer's own ATS passes the same key the ATS source builds (`ats|<job id>`), so one
    posting found twice is reported once. Two sources routinely overlap: startups.gallery
    links straight into boards that the ats source already scans.
    """
    if dedupe_key:
        key = dedupe_key
    elif raw_id:
        key = f"{source}|{raw_id}"
    else:
        key = f"{company}|{title}|{location}".lower()
    posting = {
        "id": hashlib.sha1(key.encode("utf-8")).hexdigest()[:16],
        "source": source,
        "company": (company or "").strip(),
        "title": (title or "").strip(),
        "location": (location or "").strip(),
        "url": url,
        "salary": salary,
        "ats": ats,
        "posted": posted.isoformat() if posted else None,
        "age_days": age_days,
        "raw_id": raw_id,
    }
    if extra:
        posting.update(extra)
    return posting
