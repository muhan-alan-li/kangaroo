"""Scoring, tiering, capping and ordering, plus the filter pass that turns
raw source rows into the posted list."""

from __future__ import annotations

from .filters import _tier_lookup, is_level_ii, location_matches, title_matches
from .parsing import parse_salary


def score(posting, filters):
    """Cheap deterministic pre-score. The agent does the real fit scoring."""
    s = 0
    title = posting["title"].lower()
    for phrase in ("new grad", "graduate", "entry level", "junior", "jr.", "i ", " 1"):
        if phrase in title:
            s += 25
            break
    if posting.get("ats"):
        s += 15          # real, applyable posting on a known ATS
    if posting.get("source") == "jobbank":
        s += 8          # Canadian and explicitly local
    kind = posting.get("location_kind")
    if kind == "local":
        s += 20
    elif kind == "local_bc":
        s += 8
    elif kind == "remote_ca":
        s += 10
    elif kind == "remote_unspecified":
        s += 4
    age = posting.get("age_days")
    if age is not None:
        if age <= 3:
            s += 20
        elif age <= 7:
            s += 12
        elif age <= 14:
            s += 6
        elif age > 30:
            s -= 10
    if posting.get("salary"):
        s += 5
    if posting.get("below_comp_floor"):
        s -= 6      # flagged and sorted down, never dropped
    if posting.get("url"):
        s += 5
    return s


def tier_counts(kept, filters):
    """{tier name: count} in tier order, for the portal header."""
    order = [t.get("name") for t in filters.get("location_tiers") or []]
    counts = {name: 0 for name in order if name}
    counts["untiered"] = 0
    for posting in kept:
        key = posting.get("tier") or "untiered"
        counts[key] = counts.get(key, 0) + 1
    return counts


def cap_per_tier(postings, cap):
    """Keep the best `cap` postings per tier; return (kept, dropped_count).

    A Bay Area board sweep can return hundreds of matches in one run, which buries
    the Vancouver roles the search actually exists for. The cap is per tier, so a
    busy tier can never crowd out a quiet one.
    """
    if not cap or cap <= 0:
        return postings, 0
    seen, kept = {}, []
    for posting in postings:
        key = posting.get("tier") or "untiered"
        seen[key] = seen.get(key, 0) + 1
        if seen[key] <= cap:
            kept.append(posting)
    return kept, len(postings) - len(kept)


def sort_key(posting):
    """Tier first, then the deterministic score, then freshness.

    The tier is the user's own priority order (Vancouver, Canada-remote, Toronto,
    Seattle, Bay Area, then the remote catch-alls), so a tier-1 posting always prints
    above a tier-3 one no matter how the scores fall.
    """
    return (posting.get("tier_rank", 99), -posting.get("score", 0),
            posting.get("age_days") if posting.get("age_days") is not None else 999)


def normalize_and_filter(postings, filters):
    kept, dropped = [], {}
    max_age = filters.get("max_age_days")
    comp_floor = filters.get("comp_floor_cad")
    excluded_companies = [c.lower() for c in (filters.get("company_exclude") or [])]
    tiers = _tier_lookup(filters)
    for posting in postings:
        if not posting.get("title"):
            continue
        if any(c in posting["company"].lower() for c in excluded_companies):
            dropped["company-excluded"] = dropped.get("company-excluded", 0) + 1
            continue
        ok, why = title_matches(posting["title"], filters)
        if not ok:
            dropped[why] = dropped.get(why, 0) + 1
            continue
        ok, kind, tier = location_matches(posting["location"], filters)
        if not ok:
            dropped[kind] = dropped.get(kind, 0) + 1
            continue
        posting["location_kind"] = kind
        rank, country, label = tiers.get(tier, (len(tiers), "", None))
        posting["tier"] = tier
        posting["tier_rank"] = rank
        posting["tier_label"] = label
        if country == "usa":
            # A US posting means the application asks about US work authorization.
            # Flagged, never dropped: the candidate decides.
            posting["requires_us_work_auth"] = True
        age = posting.get("age_days")
        if max_age and age is not None and age > max_age:
            dropped["too-old"] = dropped.get("too-old", 0) + 1
            continue
        # Annualise the salary so it can be compared against the floor. Job Bank states
        # a rate; the other sources usually state nothing.
        parsed_salary = parse_salary(posting.get("salary"))
        if parsed_salary:
            lo, hi, estimated = parsed_salary
            posting["salary_annual_min"] = round(lo)
            posting["salary_annual_max"] = round(hi)
            posting["salary_estimated"] = estimated
            if comp_floor and hi < comp_floor:
                # Flagged, never hidden -- the user asked to see these.
                posting["below_comp_floor"] = True
        posting["score"] = score(posting, filters)
        if is_level_ii(posting["title"], filters):
            posting["level_ii"] = True
            # Level II has not converted for this candidate. Keep one only when the
            # deterministic score is strong; the portal tags it so the reader can apply the
            # real test against the candidate's experience.
            floor = (filters.get("level_ii") or {}).get("min_score") or 0
            if posting["score"] < floor:
                dropped["level-ii-weak-fit"] = dropped.get("level-ii-weak-fit", 0) + 1
                continue
        kept.append(posting)
    return kept, dropped
