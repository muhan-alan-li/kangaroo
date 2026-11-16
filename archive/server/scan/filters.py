"""The title, level and location filters, and the location tiers that order
the report."""

from __future__ import annotations

import re


# ------------------------------------------------------------------- filtering

def _phrase_regex(phrase):
    escaped = re.escape(phrase.strip())
    return re.compile(rf"(?<![a-z0-9]){escaped}(?![a-z0-9])", re.IGNORECASE)


def title_matches(title, filters):
    for phrase in filters.get("titles_exclude") or []:
        if _phrase_regex(phrase).search(title):
            return False, f"excluded:{phrase}"
    for phrase in filters.get("titles_include") or []:
        if _phrase_regex(phrase).search(title):
            return True, f"matched:{phrase}"
    # Broadening rule: a generic role word plus a tech word also counts, so
    # "Cloud Developer" or "Systems Programmer" aren't missed by the phrase list.
    role = next((w for w in filters.get("titles_role_words") or []
                 if _phrase_regex(w).search(title)), None)
    tech = next((w for w in filters.get("titles_tech_words") or []
                 if _phrase_regex(w).search(title)), None)
    if role and tech:
        return True, f"role+tech:{role}+{tech}"
    return False, "no-title-match"


def is_level_ii(title, filters):
    """True when the title states a level-II band.

    "Software Engineer II", "SDE II", "Developer II", "Software Engineer (L3)". The
    candidate has had little success with these, so they are held to a higher bar:
    `level_ii.min_score` here, and a strong, specific fit before the portal row is pursued.
    """
    cfg = filters.get("level_ii") or {}
    if not cfg.get("enabled"):
        return False
    rng = cfg.get("level_i_range_pattern")
    if rng and re.search(rng, title):
        return False
    for phrase in cfg.get("patterns") or []:
        if _phrase_regex(phrase).search(title):
            return True
    return False


US_STATES = [
    "alabama", "alaska", "arizona", "arkansas", "california", "colorado",
    "connecticut", "delaware", "florida", "georgia", "hawaii", "idaho",
    "illinois", "indiana", "iowa", "kansas", "kentucky", "louisiana", "maine",
    "maryland", "massachusetts", "michigan", "minnesota", "mississippi",
    "missouri", "montana", "nebraska", "nevada", "new hampshire", "new jersey",
    "new mexico", "new york", "north carolina", "north dakota", "ohio",
    "oklahoma", "oregon", "pennsylvania", "rhode island", "south carolina",
    "south dakota", "tennessee", "texas", "utah", "vermont", "virginia",
    "washington", "west virginia", "wisconsin", "wyoming",
]


US_ABBRS = [
    "AL", "AK", "AZ", "AR", "CA", "CO", "CT", "DE", "FL", "GA", "HI", "ID",
    "IL", "IN", "IA", "KS", "KY", "LA", "ME", "MD", "MA", "MI", "MN", "MS",
    "MO", "MT", "NE", "NV", "NH", "NJ", "NM", "NY", "NC", "ND", "OH", "OK",
    "OR", "PA", "RI", "SC", "SD", "TN", "TX", "UT", "VT", "VA", "WA", "WV",
    "WI", "WY", "DC",
]


CA_PROV_ABBRS = ["BC", "AB", "SK", "MB", "ON", "QC", "NB", "NS", "PE", "NL",
                 "YT", "NT", "NU"]


_US_ABBR_RE = re.compile(r"(?<![A-Za-z])(" + "|".join(US_ABBRS) + r")(?![A-Za-z])")


_CA_ABBR_RE = re.compile(r"(?<![A-Za-z])(" + "|".join(CA_PROV_ABBRS) + r")(?![A-Za-z])")


def region_signals(location):
    """(mentions_canada, mentions_usa) for a location string.

    Abbreviations are matched case-sensitively and as whole tokens, because "in",
    "or" and "me" are English words and "CAN" must not read as California's "CA".
    """
    low = location.lower()
    ca = ("canada" in low
          or "british columbia" in low
          or bool(_CA_ABBR_RE.search(location))
          or bool(re.search(r"(?<![A-Za-z])CAN(?![A-Za-z])", location)))
    us = (any(state in low for state in US_STATES)
          or bool(_US_ABBR_RE.search(location))
          # bare "US": uppercase-only, or clearly a country token after a separator,
          # so the English word "us" in prose can't trigger it.
          or bool(re.search(r"(?<![A-Za-z])U\.?S\.?(?![A-Za-z])", location))
          or bool(re.search(r"(?:,|\-|\()\s*us\b", low))
          or any(s in low for s in ("united states", "usa", "u.s.")))
    return ca, us


def _tier_lookup(filters):
    """{tier name: (rank, country)} from filters.yaml's location_tiers, in order."""
    out = {}
    for i, tier in enumerate(filters.get("location_tiers") or []):
        if tier.get("name"):
            out[tier["name"]] = (i, (tier.get("country") or "").lower(), tier.get("label"))
    return out


def location_matches(location, filters):
    """Return (ok, kind, tier_name).

    Tiers come from filters.yaml, in the user's own priority order: Metro Vancouver,
    then Canada-remote, then Toronto, then Seattle/Bellevue, then
    the two remote catch-alls. A posting outside every tier is dropped.

    kind: local | remote_ca | remote_us | remote_unspecified
    """
    loc = (location or "").strip()
    if not loc:
        return False, "unknown", None
    low = loc.lower()
    ca, us = region_signals(loc)
    wrong_country = us and not ca

    # 1. A city from one of the ranked tiers. US tiers take any location without a
    #    Canadian signal; Canadian tiers reject US-only locations, so "Vancouver, WA"
    #    lands nowhere and "London, England" cannot pass as Toronto.
    for tier in filters.get("location_tiers") or []:
        name = tier.get("name")
        country = (tier.get("country") or "").lower()
        for city in tier.get("cities") or []:
            if city in low:
                if country == "usa":
                    if not ca:
                        return True, "local", name
                elif not wrong_country:
                    return True, "local", name
        for city in tier.get("ambiguous_cities") or []:
            if city in low and ca:
                return True, "local", name

    # 2. The legacy Metro list, if a config still carries one and no tier matched.
    #    Ambiguous names still need the Canada signal, or "Surrey, UK" walks in.
    conf = filters.get("locations") or {}
    if not wrong_country:
        for city in conf.get("metro") or []:
            if city in low:
                return True, "local", None
    if ca:
        for city in conf.get("metro_ambiguous") or []:
            if city in low:
                return True, "local", None

    # 3. Remote.
    remote_cfg = filters.get("remote") or {}
    if remote_cfg.get("enabled", True) and re.search(r"\b(remote|anywhere|wfh)\b", low):
        countries = [c.lower() for c in (remote_cfg.get("accept_countries") or [])]
        if ca or any(re.search(rf"(?<![a-z]){re.escape(c)}(?![a-z])", low) for c in countries):
            return True, "remote_ca", remote_cfg.get("tier_canada")
        if wrong_country:
            if remote_cfg.get("accept_us", False):
                return True, "remote_us", remote_cfg.get("tier_us")
            return False, "remote-wrong-country", None
        if remote_cfg.get("accept_unspecified", True):
            return True, "remote_unspecified", remote_cfg.get("tier_unspecified")
        return False, "remote-unspecified-rejected", None

    return False, "location-rejected", None
