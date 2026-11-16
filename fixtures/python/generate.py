#!/usr/bin/env python3
"""Generate deterministic parity cases from the Python implementation."""
import json
import sys
from datetime import date
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))
from archive.server.scan.filters import title_matches, is_level_ii, location_matches
from archive.server.scan.parsing import parse_salary
from archive.server.scan.pipeline import cap_per_tier, normalize_and_filter, score, tier_counts
from archive.server.scan.posting import apply_signature, collapse_requisitions, make_posting

OUT = ROOT / "fixtures/python"
FILTERS = __import__("yaml").safe_load((ROOT / "data/config/filters.yaml").read_text())

def dump(name, data):
    (OUT / name).write_text(json.dumps(data, ensure_ascii=False, indent=2, sort_keys=True) + "\n")

def main():
    identity = []
    for kwargs in [
        dict(source="ats", company="Acme", title="Software Engineer", location="Vancouver, BC", url=None, raw_id="123"),
        dict(source="feed", company="  Café ", title=" Junior Dev ", location="Montréal", url=None, dedupe_key="ats|é-42"),
        dict(source="Feed", company=" Acme ", title=" SWE ", location=" Toronto ", url=None),
        dict(source="feed", company="  ACME  ", title="SWE", location="Toronto", url=None, raw_id="  XyZ "),
    ]:
        p = make_posting(**kwargs)
        identity.append({"input": kwargs, "expected": p})
    dump("posting_ids.json", identity)

    urls = [
        "https://boards.greenhouse.io/acme/jobs/12345?gh_src=x",
        "https://jobs.ashbyhq.com/acme/01234567-89ab-cdef-0123-456789abcdef",
        "https://jobs.lever.co/acme/01234567-89ab-cdef-0123-456789abcdef",
        "https://acme.wd5.myworkdayjobs.com/en-US/External/job/Canada/Software-Engineer_JR123",
        "https://wd5.myworkdaysite.com/en-US/recruiting/acme/External/job/Canada/Software-Engineer_JR123",
        "https://apply.workable.com/acme/j/ABCDEF123456/",
        "https://jobs.smartrecruiters.com/Acme/123456789-engineer",
        "https://ats.rippling.com/acme/jobs/01234567-89ab-cdef-0123-456789abcdef",
        "https://acme.bamboohr.com/careers/52",
        "https://acme.recruitee.com/o/software-engineer",
        "https://acme.breezy.hr/p/software-engineer",
        "https://acme.teamtailor.com/jobs/1234567",
        "https://acme.icims.com/jobs/12345/software-engineer",
        "https://example.com/jobs/123", "https://jobs.lever.co/acme/short", "http://notgreenhouse.io/acme/jobs/12",
        "https://acme.wd5.myworkdayjobs.com/External/job/Software-Engineer_JR45",
        "https://apply.workable.com/acme/j/ABC12/", "", "https://acme.bamboohr.com/careers/not-an-id",
    ]
    dump("apply_signatures.json", [{"url": u, "expected": apply_signature(u)} for u in urls])

    titles = ["Software Engineer", "Senior Software Engineer", "Cloud Developer", "Systems Programmer", "Data Engineer", "Software Engineer II", "Software Engineer I-II", "SDE II"]
    locations = ["Vancouver, BC, Canada", "Remote - Canada", "Toronto, ON", "Seattle, WA", "Remote - United States", "Remote", "Vancouver, WA", "London, England", "", "Surrey, UK", "Anywhere in Canada", "San Francisco, CA", "SF Bay Area", "Calgary, AB", "Montréal, QC", "Greater Vancouver, BC", "Greater Seattle Area", "Remote - US"]
    dump("filters.json", {
        "titles": [{"title": t, "expected": title_matches(t, FILTERS)} for t in titles],
        "levels": [{"title": t, "expected": is_level_ii(t, FILTERS)} for t in titles],
        "locations": [{"location": l, "expected": location_matches(l, FILTERS)} for l in locations],
    })

    rows = [
        make_posting("ats", "Acme", "Junior Software Engineer", "Vancouver, BC, Canada", "https://jobs.lever.co/acme/0123456789abcdef", date(2026, 11, 13), 0, "$90,000 annually", "lever", "a"),
        make_posting("jobbank", "Local Co", "Software Developer", "Toronto, ON", "https://example.org/2", date(2026, 11, 12), 1, "$35 hourly", None, "b"),
        make_posting("ats", "Far Co", "Senior Software Engineer", "Austin, TX", None, date(2026, 11, 10), 3, None, None, "c"),
        make_posting("ats", "Bay Co", "Software Engineer", "San Francisco, CA", "https://jobs.lever.co/bay/0123456789abcdef", date(2026, 11, 11), 2, None, "lever", "d"),
        make_posting("jobbank", "Prairie Co", "Software Developer", "Calgary, AB", None, date(2026, 11, 9), 4, None, None, "e"),
    ]
    kept, dropped = normalize_and_filter(rows, FILTERS)
    ordered = sorted(kept, key=__import__("archive.server.scan.pipeline", fromlist=["sort_key"]).sort_key)
    capped, cap_dropped = cap_per_tier(ordered, 1)
    dump("pipeline.json", {"postings": rows, "expected": [{"id": p["id"], "score": p["score"], "tier": p["tier"], "tier_rank": p["tier_rank"]} for p in ordered], "kept_ids": [p["id"] for p in ordered], "dropped": dropped, "cap_ids": [p["id"] for p in capped], "cap_dropped": cap_dropped, "tier_counts": tier_counts(ordered, FILTERS)})

    salaries = [None, "", "$50.00 to $70.00 hourly", "$130,000 to $150,000 annually", "$60,000", "$8,000", "$25 per hour", "Competitive salary"]
    dump("salary.json", [{"input": x, "expected": parse_salary(x)} for x in salaries])

    pairs = [
        (make_posting("newgrad_list", "Acme", "Software Engineer", "Vancouver", "https://boards.greenhouse.io/acme/jobs/123", raw_id="one"), make_posting("ats", "Acme", "Software Engineer", "Vancouver", "https://boards.greenhouse.io/acme/jobs/123", raw_id="two")),
        (make_posting("linkedin", "X", "Engineer", "Remote", "https://jobs.lever.co/x/0123456789abcdef", raw_id="3"), make_posting("newgrad_json", "X", "Engineer", "Remote", "https://jobs.lever.co/x/0123456789abcdef", raw_id="4")),
        (make_posting("jobbank", "Same", "Dev", "Toronto", None, raw_id="5"), make_posting("jobbank", "Same", "Dev", "Toronto", None, raw_id="6")),
    ]
    cases = []
    for group in pairs:
        cases.append({"postings": list(group), "keep_ids": [p["id"] for p in collapse_requisitions(list(group))]})
    dump("collapse.json", cases)

if __name__ == "__main__":
    main()
