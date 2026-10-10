#!/usr/bin/env python3
"""Probe public ATS job boards and write the ones that answer.

The scan's `ats` source is only as good as its slug list, and a wrong slug fails
silently (404 -> no postings, no error). This script tests a candidate list, keeps
the boards that return real jobs, and writes them to config/ats_boards.yaml in the
shape scan.py reads.

    python3 -m server probe-ats                 # probe, rewrite config/ats_boards.yaml
    python3 -m server probe-ats --min-jobs 1    # keep boards with at least N open roles
    python3 -m server probe-ats --report        # probe and print, do not write

Exit code is 0 even when some boards fail: a dead slug is information, not an error.
"""
from __future__ import annotations

import argparse
import concurrent.futures as futures
import json
import os
import sys
import urllib.error
import urllib.request

from ..config import ROOT
from ..scan import net

# Same egress guard the scan uses: the probe hits api.ashbyhq.com, boards-api.greenhouse.io
# and friends once per candidate slug, which is the duplicate-lookup burst that trips the
# resolver's rate limit.
net.install()

UA = ("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) "
      "Chrome/120.0 Safari/537.36")

BOARD_URL = {
    "greenhouse": "https://boards-api.greenhouse.io/v1/boards/{slug}/jobs",
    "ashby": "https://api.ashbyhq.com/posting-api/job-board/{slug}",
    "lever": "https://api.lever.co/v0/postings/{slug}?mode=json",
    "workable": "https://apply.workable.com/api/v1/widget/accounts/{slug}?details=true",
}

# Candidate list. BC/Canada first, then US majors that hire new grads at volume.
# A wrong slug costs one request and is dropped by the probe.
CANDIDATES: list[tuple[str, str, str]] = [
    # --- Vancouver / BC -----------------------------------------------------
    ("Unbounce", "greenhouse", "unbounce"),
    ("Hootsuite", "greenhouse", "hootsuite"),
    ("Clio", "greenhouse", "clio"),
    ("Visier", "greenhouse", "visier"),
    ("Dapper Labs", "greenhouse", "dapperlabs"),
    ("Copperleaf", "greenhouse", "copperleaf"),
    ("Thinkific", "greenhouse", "thinkific"),
    ("Article", "greenhouse", "article"),
    ("Later", "greenhouse", "later"),
    ("Loopio", "greenhouse", "loopio"),
    ("Miovision", "greenhouse", "miovision"),
    ("Vidyard", "greenhouse", "vidyard"),
    ("ApplyBoard", "greenhouse", "applyboard"),
    ("Ada", "greenhouse", "ada"),
    ("Dialogue", "greenhouse", "dialogue"),
    ("PointClickCare", "greenhouse", "pointclickcare"),
    ("Sanctuary AI", "greenhouse", "sanctuaryai"),
    ("Spexi", "greenhouse", "spexi"),
    ("Ideon Technologies", "greenhouse", "ideon"),
    ("Nch'kay Development", "greenhouse", "nchkay"),
    ("Absolute Software", "greenhouse", "absolute"),
    ("Sierra Wireless", "greenhouse", "sierrawireless"),
    ("AbCellera", "greenhouse", "abcellera"),
    ("Zymeworks", "greenhouse", "zymeworks"),
    ("Global Relay", "greenhouse", "globalrelay"),
    ("Traction Complete", "greenhouse", "tractioncomplete"),
    ("Coveo", "greenhouse", "coveo"),
    ("Tulip", "greenhouse", "tulip"),
    ("Jane App", "greenhouse", "jane"),
    ("Koho", "greenhouse", "koho"),
    ("Neo Financial", "greenhouse", "neofinancial"),
    ("Wealthsimple", "greenhouse", "wealthsimple"),
    ("Borrowell", "greenhouse", "borrowell"),
    ("Float", "greenhouse", "float"),
    ("Clearco", "greenhouse", "clearco"),
    ("Certn", "greenhouse", "certn"),
    ("FreshBooks", "greenhouse", "freshbooks"),
    ("Flipp", "greenhouse", "flipp"),
    ("Ritual", "greenhouse", "ritual"),
    ("Untether AI", "greenhouse", "untetherai"),
    ("Tulip.io", "lever", "tulip"),
    ("Hopper", "lever", "hopper"),
    ("1Password", "lever", "1password"),
    ("Shopify", "lever", "shopify"),
    ("Trulioo", "lever", "trulioo"),
    ("Lightspeed", "lever", "lightspeed"),
    ("Plusgrade", "lever", "plusgrade"),
    ("Nuvei", "lever", "nuvei"),
    ("Rewind", "lever", "rewind"),
    ("TealBook", "lever", "tealbook"),
    ("OSI Maritime Systems", "lever", "osimaritime"),
    ("Relay Financial", "ashby", "relay"),
    ("Quandri", "ashby", "quandri"),
    ("Vendasta", "ashby", "vendasta"),
    ("Mattermost", "ashby", "mattermost"),
    ("Tailscale", "ashby", "tailscale"),
    ("Wattpad", "ashby", "wattpad"),
    ("Anthropic", "greenhouse", "anthropic"),
    ("Amazon Web Services Canada", "greenhouse", "aws"),
    ("Samsung Research Canada", "greenhouse", "samsungresearch"),
    ("Nvidia Canada", "greenhouse", "nvidia"),
    # --- US / global, new-grad volume --------------------------------------
    ("Stripe", "greenhouse", "stripe"),
    ("Databricks", "greenhouse", "databricks"),
    ("Airbnb", "greenhouse", "airbnb"),
    ("DoorDash", "greenhouse", "doordash"),
    ("Coinbase", "greenhouse", "coinbase"),
    ("Instacart", "greenhouse", "instacart"),
    ("Robinhood", "greenhouse", "robinhood"),
    ("Brex", "greenhouse", "brex"),
    ("Plaid", "greenhouse", "plaid"),
    ("Rippling", "greenhouse", "rippling"),
    ("Snowflake", "greenhouse", "snowflake"),
    ("Figma", "greenhouse", "figma"),
    ("Notion", "greenhouse", "notion"),
    ("Airtable", "greenhouse", "airtable"),
    ("Asana", "greenhouse", "asana"),
    ("Benchling", "greenhouse", "benchling"),
    ("GitLab", "greenhouse", "gitlab"),
    ("HashiCorp", "greenhouse", "hashicorp"),
    ("Datadog", "greenhouse", "datadog"),
    ("MongoDB", "greenhouse", "mongodb"),
    ("Elastic", "greenhouse", "elastic"),
    ("Cloudflare", "greenhouse", "cloudflare"),
    ("Twilio", "greenhouse", "twilio"),
    ("HubSpot", "greenhouse", "hubspot"),
    ("Klaviyo", "greenhouse", "klaviyo"),
    ("Samsara", "greenhouse", "samsara"),
    ("Flexport", "greenhouse", "flexport"),
    ("Affirm", "greenhouse", "affirm"),
    ("Chime", "greenhouse", "chime"),
    ("Reddit", "greenhouse", "reddit"),
    ("Discord", "greenhouse", "discord"),
    ("Pinterest", "greenhouse", "pinterest"),
    ("Lyft", "greenhouse", "lyft"),
    ("Dropbox", "greenhouse", "dropbox"),
    ("Zoox", "greenhouse", "zoox"),
    ("Scale AI", "greenhouse", "scaleai"),
    ("Gusto", "greenhouse", "gusto"),
    ("Mercury", "greenhouse", "mercury"),
    ("Coda", "greenhouse", "coda"),
    ("Retool", "greenhouse", "retool"),
    ("dbt Labs", "greenhouse", "dbtlabs"),
    ("Sentry", "greenhouse", "sentry"),
    ("Astronomer", "greenhouse", "astronomer"),
    ("Weights & Biases", "greenhouse", "weightsandbiases"),
    ("Nuro", "greenhouse", "nuro"),
    ("Aurora Innovation", "greenhouse", "aurorainnovation"),
    ("Cruise", "greenhouse", "cruise"),
    ("Applied Intuition", "greenhouse", "appliedintuition"),
    ("Waymo", "greenhouse", "waymo"),
    ("Palantir", "greenhouse", "palantir"),
    ("Databricks US", "greenhouse", "databricksus"),
    ("Roblox", "greenhouse", "roblox"),
    ("Snap", "greenhouse", "snapchat"),
    ("Roku", "greenhouse", "roku"),
    ("Zillow", "greenhouse", "zillow"),
    ("Box", "greenhouse", "box"),
    ("Dropbox Inc", "greenhouse", "dropboxinc"),
    ("Verily", "greenhouse", "verily"),
    ("Tempus", "greenhouse", "tempus"),
    ("Duolingo", "greenhouse", "duolingo"),
    ("Grammarly", "greenhouse", "grammarly"),
    ("Squarespace", "greenhouse", "squarespace"),
    ("Vercel", "greenhouse", "vercel"),
    ("Snyk", "greenhouse", "snyk"),
    ("Cockroach Labs", "greenhouse", "cockroachlabs"),
    ("Confluent", "greenhouse", "confluent"),
    ("Temporal", "greenhouse", "temporal"),
    ("Sourcegraph", "greenhouse", "sourcegraph"),
    ("Grafana Labs", "greenhouse", "grafanalabs"),
    ("Postman", "greenhouse", "postman"),
    ("CircleCI", "greenhouse", "circleci"),
    ("Netlify", "greenhouse", "netlify"),
    ("Webflow", "greenhouse", "webflow"),
    ("Faire", "greenhouse", "faire"),
    ("Carta", "greenhouse", "carta"),
    ("Segment", "greenhouse", "segment"),
    ("Amplitude", "greenhouse", "amplitude"),
    ("Miro", "greenhouse", "miro"),
    ("Canva", "greenhouse", "canva"),
    ("Atlassian AU", "greenhouse", "atlassian"),
    ("Block", "greenhouse", "block"),
    ("Betterment", "greenhouse", "betterment"),
    ("Affirm Inc", "greenhouse", "affirmcareers"),
    # --- Ashby boards -------------------------------------------------------
    ("OpenAI", "ashby", "openai"),
    ("Ramp", "ashby", "ramp"),
    ("Sierra", "ashby", "sierra"),
    ("Cohere", "ashby", "cohere"),
    ("Cursor", "ashby", "cursor"),
    ("Harvey", "ashby", "harvey"),
    ("Linear", "ashby", "linear"),
    ("Notion Labs", "ashby", "notion"),
    ("Deel", "ashby", "deel"),
    ("Vanta", "ashby", "vanta"),
    ("Replit", "ashby", "replit"),
    ("Perplexity", "ashby", "perplexity"),
    ("ElevenLabs", "ashby", "elevenlabs"),
    ("Supabase", "ashby", "supabase"),
    ("Railway", "ashby", "railway"),
    ("Modal", "ashby", "modal"),
    ("Together AI", "ashby", "together"),
    ("Suno", "ashby", "suno"),
    ("Baseten", "ashby", "baseten"),
    ("Anysphere", "ashby", "anysphere"),
    ("Oscar Health", "ashby", "oscar"),
    ("Rho", "ashby", "rho"),
    ("Clerk", "ashby", "clerk"),
    ("Resend", "ashby", "resend"),
    ("WorkOS", "ashby", "workos"),
    ("Hex", "ashby", "hex"),
    ("Sourcegraph Inc", "ashby", "sourcegraph"),
    # --- Lever boards -------------------------------------------------------
    ("Netflix", "lever", "netflix"),
    ("Spotify", "lever", "spotify"),
    ("Palantir US", "lever", "palantir"),
    ("Twitch", "lever", "twitch"),
    ("Attentive", "lever", "attentive"),
    ("Nauto", "lever", "nauto"),
    ("GoFundMe", "lever", "gofundme"),
    ("Airtable Inc", "lever", "airtable"),
    ("Jobber", "lever", "jobber"),
    ("Ecobee", "lever", "ecobee"),
    ("TextNow", "lever", "textnow"),
    ("Faire Inc", "lever", "faire"),
    ("Auvik", "lever", "auvik"),
    ("Magnet Forensics", "lever", "magnetforensics"),
    ("Workleap", "lever", "workleap"),
    ("Dialogue Health", "lever", "dialogue"),
    ("Sonder", "lever", "sonder"),
    # --- Workable boards ----------------------------------------------------
    ("Tulip Retail", "workable", "tulip"),
    ("Rewind Backups", "workable", "rewind"),
]

COUNT_KEYS = {"greenhouse": ("jobs",), "ashby": ("jobs",), "lever": None}


def harvested_candidates() -> list[tuple[str, str, str]]:
    """Board slugs that a feed revealed, for the probe to test.

    scan.py writes state/sg_boards.json while it reads startups.gallery, and the file
    accumulates: a company that surfaced once keeps its entry even after it drops out of
    the feed. Only the ATS kinds this probe can read are returned, and a company already
    in CANDIDATES is skipped so the hand-written list stays authoritative.
    """
    path = os.path.join(ROOT, "state", "sg_boards.json")
    try:
        with open(path, encoding="utf-8") as fh:
            store = json.load(fh)
    except (OSError, json.JSONDecodeError):
        return []
    known = {(ats, slug) for _, ats, slug in CANDIDATES}
    out = []
    for row in store.get("boards") or []:
        ats, slug = row.get("ats"), row.get("slug")
        if ats not in BOARD_URL or not slug or (ats, slug) in known:
            continue
        out.append((row.get("name") or slug, ats, slug))
    return out


def probe(entry: tuple[str, str, str], timeout: int = 20):
    name, ats, slug = entry
    url = BOARD_URL[ats].format(slug=slug)
    try:
        req = urllib.request.Request(url, headers={"User-Agent": UA, "Accept": "application/json"})
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            body = resp.read().decode("utf-8", "replace")
            status = resp.status
    except urllib.error.HTTPError as exc:
        return {"name": name, "ats": ats, "slug": slug, "status": exc.code, "jobs": 0}
    except Exception as exc:  # noqa: BLE001
        return {"name": name, "ats": ats, "slug": slug, "status": type(exc).__name__, "jobs": 0}
    try:
        data = json.loads(body)
    except json.JSONDecodeError:
        return {"name": name, "ats": ats, "slug": slug, "status": status, "jobs": 0}
    if isinstance(data, list):
        jobs = data
    else:
        jobs = data.get("jobs") or data.get("results") or []
    return {"name": name, "ats": ats, "slug": slug, "status": status, "jobs": len(jobs)}


HEADER = """# Public ATS job boards, verified by server/jobs/probe_ats.py.
#
# Every entry answered its board API with at least one open posting when it was last
# probed. A wrong slug fails silently, so re-run the probe when a board stops
# returning postings:
#
#     python3 -m server probe-ats
#
# Companies are grouped by ATS. Add new slugs to the CANDIDATES list in the probe
# script, not by hand here, so the verification stays honest.
"""


def main(argv=None) -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--min-jobs", type=int, default=1)
    ap.add_argument("--report", action="store_true", help="print only, do not write")
    ap.add_argument("--workers", type=int, default=10)
    args = ap.parse_args(argv)

    candidates = list(CANDIDATES)
    harvested = harvested_candidates()
    candidates += harvested

    with futures.ThreadPoolExecutor(max_workers=args.workers) as pool:
        results = list(pool.map(probe, candidates))

    live = [r for r in results if r["status"] == 200 and r["jobs"] >= args.min_jobs]
    dead = [r for r in results if r not in live]
    live.sort(key=lambda r: (r["ats"], -r["jobs"], r["name"].lower()))

    print(f"[probe] {len(candidates)} candidates ({len(harvested)} harvested from feeds), "
          f"{len(live)} live boards, {len(dead)} dead")
    for r in dead:
        print(f"  dead: {r['ats']:10} {r['slug']:20} status={r['status']}")

    by_ats: dict[str, list[dict]] = {}
    for r in live:
        by_ats.setdefault(r["ats"], []).append(r)

    lines = [HEADER]
    for ats in sorted(by_ats):
        lines.append(f"\n# --- {ats} ({len(by_ats[ats])} live boards) ---")
        lines.append(f"boards_{ats}:")
        for r in by_ats[ats]:
            lines.append(f"  - {{name: {json.dumps(r['name'])}, ats: {ats}, slug: {r['slug']}}}")
    text = "\n".join(lines) + "\n"

    if args.report:
        print(text[:1200])
        return 0
    out = os.path.join(ROOT, "config", "ats_boards.yaml")
    with open(out, "w", encoding="utf-8") as fh:
        fh.write(text)
    print(f"[probe] wrote {out} ({len(live)} boards, {len(text)} bytes)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
