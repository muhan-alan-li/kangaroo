"""The `ats_boards` source: the probed Greenhouse/Ashby/Lever slugs, plus the
three batches above, which run concurrently against different hosts."""

from __future__ import annotations

from concurrent import futures
from datetime import datetime, timezone
import json
import os
import sys

from .. import boardstate
from ..core import ROOT, http_get, load_yaml
from ..endpoints import BOARD_URL
from ..parsing import parse_age_days
from ..posting import make_posting
from .icims import fetch_icims_boards
from .jsonboards import JSON_VENDOR_URLS, fetch_json_boards
from .workday import fetch_workday_boards


def src_ats(cfg, filters):
    entries = list(cfg.get("companies") or [])
    files = list(cfg.get("companies_files") or [])
    if cfg.get("companies_file"):
        files.insert(0, cfg["companies_file"])
    for companies_file in files:
        path = os.path.join(ROOT, companies_file)
        if not os.path.exists(path):
            sys.stderr.write(f"[scan] ats companies_file missing: {path}\n")
            continue
        boards = load_yaml(*companies_file.split("/")) or {}
        for key, rows in (boards or {}).items():
            if not key.startswith("boards_"):
                continue
            default_ats = key[len("boards_"):]
            for row in rows or []:
                if not isinstance(row, dict):
                    continue
                entry = {
                    "name": row.get("name"),
                    "ats": row.get("ats") or default_ats,
                    "slug": row.get("slug"),
                }
                # Workday needs three fields, not one slug: tenant, wd number and career
                # site. Carry whatever the row declares so the branch can rebuild the API
                # path, and give the entry a slug so the board ledger stays uniform.
                for extra in ("tenant", "wd", "site", "urls"):
                    if row.get(extra) is not None:
                        entry[extra] = row[extra]
                if not entry["slug"]:
                    bits = [str(row[b]) for b in ("tenant", "wd", "site") if row.get(b)]
                    if bits:
                        entry["slug"] = "|".join(bits)
                entries.append(entry)

    # Workday runs as one parallel batch instead of inside the loop below. The board list
    # is over a thousand boards, and answering them sequentially is minutes of waiting.
    workday_entries = [e for e in entries
                       if e.get("ats") == "workday" and e.get("tenant") and e.get("site")]
    entries = [e for e in entries if e.get("ats") != "workday"]

    # The JSON vendors run as one batch too, for the same reason: over five hundred boards
    # answered one at a time is minutes of nothing but waiting.
    json_entries = [e for e in entries
                    if e.get("ats") in JSON_VENDOR_URLS and e.get("slug")]
    entries = [e for e in entries if e.get("ats") not in JSON_VENDOR_URLS]

    # iCIMS goes through its own subprocess, not the loop below: it needs a browser TLS
    # fingerprint, which means curl_cffi, which means a different interpreter.
    icims_entries = [e for e in entries if e.get("ats") == "icims" and e.get("slug")]
    entries = [e for e in entries if e.get("ats") != "icims"]

    out = []
    for entry in entries:
        ats, slug = entry.get("ats"), entry.get("slug")
        name = entry.get("name") or slug
        if not (ats and slug):
            continue
        if ats == "greenhouse":
            url = BOARD_URL["greenhouse"].format(slug=slug)
            data = http_get(url)
            if not data:
                continue
            try:
                jobs = json.loads(data).get("jobs", [])
            except json.JSONDecodeError:
                continue
            for job in jobs:
                # `first_published` is the real posting date. `updated_at` moves every
                # time the board touches the record, which makes a June req read as
                # "0d". Keep updated_at only as the fallback.
                raw_date = job.get("first_published") or job.get("updated_at")
                posted = None
                if raw_date:
                    try:
                        posted = datetime.fromisoformat(
                            raw_date.replace("Z", "+00:00")).date()
                    except ValueError:
                        posted = None
                out.append(make_posting(
                    source="ats", company=name, title=job.get("title", ""),
                    location=(job.get("location") or {}).get("name", ""),
                    url=job.get("absolute_url"), posted=posted,
                    age_days=parse_age_days(None, posted), ats="greenhouse",
                    raw_id=str(job.get("id")),
                ))
        elif ats == "lever":
            data = http_get(BOARD_URL["lever"].format(slug=slug))
            if not data:
                continue
            try:
                jobs = json.loads(data)
            except json.JSONDecodeError:
                continue
            for job in jobs:
                cats = job.get("categories") or {}
                # Lever states the real posting time in `createdAt` (epoch milliseconds).
                posted = None
                if job.get("createdAt"):
                    try:
                        posted = datetime.fromtimestamp(
                            int(job["createdAt"]) / 1000, timezone.utc).date()
                    except (ValueError, TypeError, OSError):
                        posted = None
                out.append(make_posting(
                    source="ats", company=name, title=job.get("text", ""),
                    location=cats.get("location", ""), url=job.get("hostedUrl"),
                    posted=posted, age_days=parse_age_days(None, posted),
                    ats="lever", raw_id=str(job.get("id")),
                ))
        elif ats == "ashby":
            data = http_get(BOARD_URL["ashby"].format(slug=slug))
            if not data:
                continue
            try:
                jobs = json.loads(data).get("jobs", [])
            except json.JSONDecodeError:
                continue
            for job in jobs:
                if job.get("isListed") is False:
                    continue
                # Ashby publishes `publishedAt` (ISO 8601). `updatedAt` moves.
                posted = None
                raw_date = job.get("publishedAt") or job.get("updatedAt")
                if raw_date:
                    try:
                        posted = datetime.fromisoformat(
                            raw_date.replace("Z", "+00:00")).date()
                    except ValueError:
                        posted = None
                out.append(make_posting(
                    source="ats", company=name, title=job.get("title", ""),
                    location=job.get("location", ""), url=job.get("jobUrl"),
                    posted=posted, age_days=parse_age_days(None, posted),
                    ats="ashby", raw_id=str(job.get("id")),
                ))
    # The three batches are independent and each is almost entirely socket waiting, so they
    # run together. Measured in sequence they were the whole scan: 285s of a 288s run, which
    # means the scan's runtime was their sum. Overlapped, it is the slowest one alone. They
    # talk to different hosts -- Workday, the JSON vendors, iCIMS -- so there is no shared
    # rate limit to trip, though both failure counts are still reported either way.
    # Workday is the slowest batch by an order of magnitude -- about two minutes of a
    # three-minute scan, for 1,328 boards. server/jobs/scan/sweep.py sweeps that fleet on its own
    # beat and leaves the postings in data/state/workday_sweep.json; when that file is fresh it is
    # read instead and the sweep is skipped, which drops the daily run's floor onto iCIMS.
    # A missing or stale cache falls back to sweeping here, so a night the separate pass did
    # not run costs time and never coverage.
    workday_opts = cfg.get("workday") or {}
    cache_path = workday_opts.get("cache") or boardstate.SWEEP_PATH
    if cache_path and not os.path.isabs(cache_path):
        cache_path = os.path.join(ROOT, cache_path)
    cached, cache_age = boardstate.read_sweep(
        workday_opts.get("max_cache_age_hours", 30), path=cache_path)
    prefetched = []
    if workday_entries and cached is not None:
        prefetched = cached
        sys.stderr.write(
            f"[scan] workday: {len(cached)} postings from the sweep cache "
            f"({cache_age:.1f}h old; {len(workday_entries)} boards not re-read)\n")
        workday_entries = []      # nothing to fetch, so nothing for the pool to do

    batches = []
    if workday_entries:
        batches.append(lambda: fetch_workday_boards(workday_entries, cfg))
    if json_entries:
        batches.append(lambda: fetch_json_boards(json_entries, cfg))
    if icims_entries:
        batches.append(lambda: fetch_icims_boards(icims_entries, cfg))
    if len(batches) > 1:
        with futures.ThreadPoolExecutor(max_workers=len(batches)) as pool:
            for found in pool.map(lambda run: run(), batches):
                out.extend(found)
    else:
        for run in batches:
            out.extend(run())
    out.extend(prefetched)
    return out
