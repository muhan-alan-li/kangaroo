"""The portal feed: the posting list the dashboard shows, with a 7-day life.

Why this exists as its own file rather than the dashboard reading `last_scan.json`:

- `last_scan.json` is one run. A posting that was live yesterday and is still live today is
  only in it if today's run happened to return it, and the per-tier cap means it often does
  not. The portal is a working list, so it accumulates: a row stays until it expires.
- The candidate's rule, 2026-10-04: a scanned posting is **discarded after 7 days**, and the
  only thing kept past that is its identity, so the same posting cannot come back as new.
  Expiry here is that rule; the identity half lives in `data/state/seen.jsonl` (the scan's dedupe
  ledger) and in `data/state/portal_expired.json`, and neither is ever pruned.
- The dashboard must not fetch anything (it is read-only and serves a request in
  milliseconds), so the enrichment -- posting text, resume keywords, outreach addresses --
  happens here, at scan time, and is cached.

`refresh()` is called at the end of every scan and by server/jobs/refresh_portal.py for a rebuild
from the last scan without a full sweep.
"""

from __future__ import annotations

import json
import os
import re
import sys
import time
from concurrent import futures
from datetime import date, datetime, timedelta, timezone

from .core import ROOT, UA, atomic_write, http_get, strip_tags
from .keywords import _profile_skills, extract_emails, extract_keywords
from .posting import apply_signature

STATE = os.path.join(ROOT, "data", "state")
FEED_PATH = os.path.join(STATE, "portal_feed.jsonl")
EXPIRED_PATH = os.path.join(STATE, "portal_expired.json")
DESC_PATH = os.path.join(STATE, "descriptions.json")
OUTREACH_PATH = os.path.join(STATE, "outreach.jsonl")

DEFAULT_WINDOW_DAYS = 7
DESC_LIMIT = 6000          # characters kept per posting: enough for the requirements block
DESC_BUDGET = 250          # postings enriched per run, so a cold cache costs minutes, not hours
FETCH_WORKERS = 8


# ------------------------------------------------------------------ file handling

def _read_json(path, default):
    try:
        with open(path, encoding="utf-8") as fh:
            return json.load(fh)
    except Exception:  # noqa: BLE001 -- a missing state file is a cold start, not an error
        return default


def read_feed():
    """{id: row} from the jsonl feed. The last row for an id wins."""
    rows = {}
    try:
        with open(FEED_PATH, encoding="utf-8") as fh:
            for line in fh:
                line = line.strip()
                if not line:
                    continue
                try:
                    row = json.loads(line)
                except json.JSONDecodeError:
                    continue
                if row.get("id"):
                    rows[row["id"]] = row
    except FileNotFoundError:
        pass
    return rows


def save_feed(rows):
    body = "\n".join(json.dumps(r, sort_keys=True) for r in rows.values())
    with atomic_write(FEED_PATH) as fh:
        fh.write(body + ("\n" if body else ""))


def load_expired():
    return _read_json(EXPIRED_PATH, {}) or {}


def save_expired(data):
    with atomic_write(EXPIRED_PATH) as fh:
        fh.write(json.dumps(data, indent=1, sort_keys=True))


# ------------------------------------------------------------------ description harvest

_WORKDAY_URL = re.compile(
    r"^https?://([a-z0-9-]+)\.(wd\d+)\.myworkdayjobs\.com/(?:[a-z]{2}-[A-Z]{2}/)?([^/?#]+)(/.*)$",
    re.I)


def _workday_parts(posting):
    """(host, tenant, site, path) for a Workday posting URL, or None."""
    hit = _WORKDAY_URL.match(posting.get("url") or "")
    if not hit:
        return None
    tenant, wd, site, path = hit.group(1), hit.group(2), hit.group(3), hit.group(4)
    return f"{tenant}.{wd}.myworkdayjobs.com", tenant, site, path.rstrip("/") or ""


def fetch_workday(posting):
    """The posting's own description. Workday's listing API does not carry it, so this is
    a second request per posting -- which is why the result is cached forever.

    The detail endpoint is a GET on the same cxs path. A POST to it answers HTTP 400 with an
    empty message, which looks like a dead board rather than a wrong verb, so do not
    "fix" this by adding a body.
    """
    parts = _workday_parts(posting)
    if not parts:
        return None
    host, tenant, site, path = parts
    raw = http_get(f"https://{host}/wday/cxs/{tenant}/{site}{path}", timeout=25, retries=1)
    if not raw:
        return None
    try:
        info = (json.loads(raw) or {}).get("jobPostingInfo") or {}
    except json.JSONDecodeError:
        return None
    return strip_tags(info.get("jobDescription") or "") or None


# Greenhouse's own board URL is the clean case, but plenty of employers deep-link a Stripe-style
# `.../jobs/search?gh_jid=123` from their website. The numeric id is Greenhouse's, and the board
# slug is the company's own host label, so the description is still one request away.
_GH_JID = re.compile(r"[?&]gh_jid=(\d+)", re.I)
# Host labels that are the company's job site rather than the board slug. "app" and "careers"
# resolve to nothing at the board API and only produce noise in the log.
_GH_SLUG_STOP = {"app", "careers", "jobs", "job", "boards", "board", "apply", "hire",
                 "hiring", "work", "www", "gh", "greenhouse", "recruiting"}
# A slug that 404s once 404s for the run: a host label like "pinterestcareers" is the company's
# job site, not its board slug, and several postings share it. Remembering the miss keeps three
# postings from producing three wasted requests and three identical log lines.
_GH_SLUG_OK = {}


def fetch_greenhouse(posting):
    vendor, slug, job_id, _key = apply_signature(posting.get("url") or "")
    if vendor != "greenhouse":
        hit = _GH_JID.search(posting.get("url") or "")
        if not hit:
            return None
        host = re.sub(r"^https?://", "", posting.get("url") or "").split("/")[0].lower()
        slug = host.split(":")[0].removeprefix("www.").split(".")[0]
        job_id = hit.group(1)
        if not slug or not job_id or slug in _GH_SLUG_STOP:
            return None
    if _GH_SLUG_OK.get(slug) is False:
        return None
    raw = http_get(f"https://boards-api.greenhouse.io/v1/boards/{slug}/jobs/{job_id}?content=true",
                   timeout=25, retries=1)
    if not raw:
        _GH_SLUG_OK[slug] = False
        return None
    _GH_SLUG_OK[slug] = True
    try:
        return strip_tags(json.loads(raw).get("content") or "") or None
    except json.JSONDecodeError:
        return None


def fetch_lever(posting):
    vendor, slug, job_id, _key = apply_signature(posting.get("url") or "")
    if vendor != "lever":
        return None
    raw = http_get(f"https://api.lever.co/v0/postings/{slug}/{job_id}?mode=json",
                   timeout=25, retries=1)
    if not raw:
        return None
    try:
        data = json.loads(raw) or {}
    except json.JSONDecodeError:
        return None
    return strip_tags(data.get("descriptionPlain") or data.get("description") or "") or None


def fetch_ashby(posting):
    """Ashby has no per-job endpoint, so the board is fetched once per run and its jobs are
    read out of it. The whole board arrives in one request, which is why this is worth doing
    even for a single posting."""
    vendor, slug, job_id, _key = apply_signature(posting.get("url") or "")
    if vendor != "ashby":
        return None
    raw = http_get(f"https://api.ashbyhq.com/posting-api/job-board/{slug}", timeout=30, retries=1)
    if not raw:
        return None
    try:
        jobs = json.loads(raw).get("jobs") or []
    except json.JSONDecodeError:
        return None
    for job in jobs:
        if str(job.get("id")) == str(job_id):
            return strip_tags(job.get("descriptionPlain") or job.get("descriptionHtml") or "") or None
    return None


def _description_fetcher(posting):
    ats = posting.get("ats")
    if ats == "workday":
        return fetch_workday
    return {"greenhouse": fetch_greenhouse, "lever": fetch_lever, "ashby": fetch_ashby}.get(ats)


def harvest_descriptions(postings, cache, budget=DESC_BUDGET):
    """Fill `cache` with posting text for the ones that have a fetchable source.

    Returns (fetched, failed, skipped). A posting whose text is already cached, or which is
    already carrying one from its source, is skipped without a request. Source-supplied
    text (Job Bank detail pages, the Simplify feed) is seeded into the cache first, so it
    survives the run instead of evaporating with the transient posting dict -- without
    this the portal row keeps `has_text: true` but no text, and the brief later finds
    nothing to read.
    """
    for posting in postings:
        pid = posting.get("id")
        text = posting.get("description")
        if pid and text and not cache.get(pid):
            cache[pid] = re.sub(r"\s+", " ", text).strip()[:DESC_LIMIT]
    todo = []
    for posting in postings:
        pid = posting.get("id")
        if not pid or cache.get(pid):
            continue
        if posting.get("description"):
            continue
        if _description_fetcher(posting):
            todo.append(posting)
    skipped = len(todo) - min(len(todo), budget)
    todo = todo[:budget]
    if not todo:
        return 0, 0, skipped

    fetched = failed = 0
    with futures.ThreadPoolExecutor(max_workers=FETCH_WORKERS) as pool:
        for posting, text in zip(todo, pool.map(_safe_description, todo)):
            if text:
                cache[posting["id"]] = text[:DESC_LIMIT]
                fetched += 1
            else:
                failed += 1
    return fetched, failed, skipped


def _safe_description(posting):
    fetcher = _description_fetcher(posting)
    try:
        text = fetcher(posting) if fetcher else None
    except Exception:  # noqa: BLE001
        return None
    if not text:
        return None
    return re.sub(r"\s+", " ", text).strip()[:DESC_LIMIT]


# ------------------------------------------------------------------ outreach addresses

def outreach_index():
    """{lowercased company: [addresses]} from messages the workspace has already sent.

    These are addresses that came from a published source and were verified when they were
    used, so they are worth showing next to a posting from the same employer. Historical
    record, not a suggestion: nothing is sent from here.
    """
    found = {}
    try:
        with open(OUTREACH_PATH, encoding="utf-8") as fh:
            for line in fh:
                line = line.strip()
                if not line:
                    continue
                try:
                    row = json.loads(line)
                except json.JSONDecodeError:
                    continue
                company = (row.get("company") or "").strip().lower()
                addr = (row.get("contact_email") or "").strip()
                if company and addr and row.get("status") == "sent":
                    found.setdefault(company, [])
                    if addr not in found[company]:
                        found[company].append(addr)
    except FileNotFoundError:
        pass
    return found


# ------------------------------------------------------------------ expiry

def _posted_date(row):
    if row.get("posted"):
        return str(row["posted"])[:10]
    if row.get("posted_derived"):
        return str(row["posted_derived"])[:10]
    return None


def expires_on(row, window_days, ran_at):
    """(date string the row dies on, reason) or (None, reason it is undated)."""
    posted = _posted_date(row)
    if posted:
        try:
            base = date.fromisoformat(posted)
            return (base + timedelta(days=window_days)).isoformat(), "posted"
        except ValueError:
            pass
    first = (row.get("first_seen") or ran_at or "")[:10]
    if first:
        try:
            base = date.fromisoformat(first)
            return (base + timedelta(days=window_days)).isoformat(), "first_seen"
        except ValueError:
            pass
    return None, "undated"


def _is_expired(row, today, window_days):
    end, reason = expires_on(row, window_days, row.get("scanned_at"))
    if end:
        try:
            return date.fromisoformat(end) <= today, reason, end
        except ValueError:
            return False, reason, end
    # Undated (Rippling and BambooHR publish no date anywhere). Counted from the day it was
    # first seen, which is the only clock available, and never presented as fresh.
    return False, reason, None


# ------------------------------------------------------------------ the refresh

def build_row(posting, ran_at, claimed_skills, outreach, existing=None, description=None,
              window_days=DEFAULT_WINDOW_DAYS):
    text = description or posting.get("description") or ""
    keywords, have, gap = extract_keywords(
        f"{posting.get('title') or ''} {posting.get('company') or ''} {text}", claimed_skills)
    emails = extract_emails(text)
    verified = [a for a in outreach.get((posting.get("company") or "").strip().lower(), [])
                if a not in emails]
    row = {
        "id": posting.get("id"),
        "company": posting.get("company"),
        "title": posting.get("title"),
        "location": posting.get("location"),
        "url": posting.get("url"),
        "source": posting.get("source"),
        "ats": posting.get("ats"),
        "tier": posting.get("tier"),
        "tier_rank": posting.get("tier_rank"),
        "score": posting.get("score"),
        "age_days": posting.get("age_days"),
        "posted": posting.get("posted"),
        "level_ii": bool(posting.get("level_ii")),
        "requires_us_work_auth": bool(posting.get("requires_us_work_auth")),
        "below_comp_floor": bool(posting.get("below_comp_floor")),
        "salary_annual_min": posting.get("salary_annual_min"),
        "salary_annual_max": posting.get("salary_annual_max"),
        "first_seen": (existing or {}).get("first_seen") or posting.get("first_seen") or ran_at,
        "scanned_at": ran_at,
        "keywords": keywords,
        "skills_have": have,
        "skills_gap": gap,
        "emails": emails,
        "verified_emails": verified,
        "has_text": bool(text),
    }
    if not row["posted"] and row["age_days"] is not None:
        try:
            row["posted_derived"] = (datetime.fromisoformat(ran_at)
                                     - timedelta(days=int(row["age_days"]))).date().isoformat()
        except (ValueError, TypeError):
            pass
    end, reason = expires_on(row, window_days, ran_at)
    row["expires_on"] = end
    row["date_basis"] = reason
    row["expires_in_days"] = None
    if end:
        try:
            row["expires_in_days"] = (date.fromisoformat(end) - date.fromisoformat(ran_at[:10])).days
        except (ValueError, TypeError):
            pass
    return row


def refresh(postings, filters=None, ran_at=None, no_write=False, desc_budget=DESC_BUDGET):
    """Fold this run's postings into the feed, enrich them, and expire the old ones.

    Returns a summary dict for the scan log. Writes nothing when no_write is set.
    """
    filters = filters or {}
    window_days = int(filters.get("max_age_days") or DEFAULT_WINDOW_DAYS)
    ran_at = ran_at or datetime.now(timezone.utc).isoformat(timespec="seconds")
    today = date.fromisoformat(ran_at[:10])

    feed = read_feed()
    expired = load_expired()
    cache = _read_json(DESC_PATH, {}) or {}
    outreach = outreach_index()

    # `claimed_skills` is read once: it is the profile, and it does not change mid-run.
    claimed = _profile_skills()

    harvest_targets = [p for p in postings if p.get("id")]
    # Live rows from earlier runs that still have no text get another chance: a fetch that failed
    # on the day the row was added is usually a throttle or a transient 404, not an answer. They
    # queue behind this run's postings, so the budget always serves what is new first.
    queued = {p["id"] for p in harvest_targets}
    for pid, row in feed.items():
        if pid not in queued and not cache.get(pid) and not row.get("has_text"):
            harvest_targets.append(row)

    fetched, failed, skipped = (0, 0, 0)
    if harvest_targets:
        fetched, failed, skipped = harvest_descriptions(harvest_targets, cache, budget=desc_budget)

    added = updated = 0
    for posting in postings:
        pid = posting.get("id")
        if not pid:
            continue
        existing = feed.get(pid)
        was_expired = expired.get(pid)
        # The dedupe half of the rule: an id that already expired does not come back, even
        # if a source hands it over again. It would only come back as a *new* posting, which
        # is a different id.
        if was_expired and not existing:
            continue
        row = build_row(posting, ran_at, claimed, outreach, existing=existing,
                        description=cache.get(pid) or posting.get("description"),
                        window_days=window_days)
        if row["expires_in_days"] is not None and row["expires_in_days"] <= 0:
            continue
        if existing is None:
            added += 1
        else:
            updated += 1
        feed[pid] = row

    # Rows carried from earlier runs, including ones today's scan did not return. They are
    # rebuilt only for expiry, because their enrichment is already on them.
    retired = 0
    for pid in list(feed):
        if pid in expired:
            del feed[pid]
            continue
        row = feed[pid]
        gone, reason, end = _is_expired(row, today, window_days)
        if gone:
            expired[pid] = {"expired_at": ran_at, "reason": reason, "expires_on": end,
                            "company": row.get("company"), "title": row.get("title")}
            del feed[pid]
            retired += 1

    summary = {
        "live": len(feed),
        "added": added,
        "updated": updated,
        "expired_total": len(expired),
        "retired_now": retired,
        "with_email": sum(1 for r in feed.values() if r.get("emails") or r.get("verified_emails")),
        "with_keywords": sum(1 for r in feed.values() if r.get("keywords")),
        "with_text": sum(1 for r in feed.values() if r.get("has_text")),
        "descriptions_fetched": fetched,
        "descriptions_failed": failed,
        "descriptions_deferred": skipped,
        "window_days": window_days,
        "ran_at": ran_at,
    }

    if not no_write:
        ordered = sorted(feed.values(), key=lambda r: (r.get("tier_rank") if r.get("tier_rank")
                                                       is not None else 99,
                                                       -(r.get("score") or 0)))
        save_feed({r["id"]: r for r in ordered})
        save_expired(expired)
        with atomic_write(DESC_PATH) as fh:
            fh.write(json.dumps(cache, indent=1))
    return summary


def line(summary):
    """One log line, for the scan's stderr."""
    return (f"[scan] portal: {summary['live']} live (+{summary['added']} new, "
            f"{summary['retired_now']} expired), {summary['with_keywords']} with keywords, "
            f"{summary['with_email']} with an address, "
            f"{summary['descriptions_fetched']} descriptions fetched "
            f"({summary['descriptions_deferred']} deferred)")
