"""Command-line entry point. It reports and never acts: it never marks a posting reported
(scan/mark_reported.py does that) and never submits anything. The no-agent scan job logs its
stdout to data/state/scan.log."""

from __future__ import annotations

from concurrent import futures
from datetime import datetime, timezone
import argparse
import fcntl
import json
import os
import sys
import time

from . import boardstate
from .core import ROOT, atomic_write, load_yaml
from .harvest import harvest_boards, unprobed_boards
from .ledger import ledger_path, load_ledger, save_ledger
from .pipeline import cap_per_tier, normalize_and_filter, sort_key, tier_counts
from .posting import collapse_requisitions
from .sources import SOURCE_FUNCS


# ------------------------------------------------------------------------- main

def main(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument("--json", action="store_true", help="machine-readable output")
    ap.add_argument("--all", action="store_true", help="ignore the reported ledger")
    ap.add_argument("--source", action="append", help="limit to a source id")
    ap.add_argument("--limit", type=int, default=0, help="cap the printed list")
    ap.add_argument("--per-tier", type=int, default=0,
                    help="cap the printed list per location tier (0 = no cap)")
    ap.add_argument("--stats", action="store_true", help="ledger summary only")
    ap.add_argument("--no-write", action="store_true", help="do not touch the ledger")
    args = ap.parse_args(argv)

    # One scan at a time, from either side of the container boundary. The entrypoints hold
    # an exclusive flock on this file around their whole run and export JOB_SCAN_LOCK_HELD
    # to say so: flock is per open file description, so this child must not re-take the lock
    # its own parent already holds. On the host there is no wrapper any more, so a manual
    # run takes the lock here and exits loudly if a container slot is mid-scan -- the two
    # never hit Workday at once. `--stats` only reads a file that is written atomically,
    # so it needs neither.
    lock_fh = None
    if not args.stats and not os.environ.get("JOB_SCAN_LOCK_HELD"):
        os.makedirs(os.path.join(ROOT, "data", "state"), exist_ok=True)
        lock_fh = open(os.path.join(ROOT, "data", "state", ".scan.lock"), "a")
        try:
            fcntl.flock(lock_fh.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError:
            lock_fh.close()
            sys.stderr.write("[scan] another scan holds data/state/.scan.lock; not starting a "
                             "second one\n")
            return 1

    filters = load_yaml("data", "config", "filters.yaml")
    sources_cfg = load_yaml("data", "config", "sources.yaml")

    ledger = load_ledger()
    if args.stats:
        reported = sum(1 for r in ledger.values() if r.get("reported"))
        print(json.dumps({
            "ledger_total": len(ledger),
            "reported": reported,
            "unreported": len(ledger) - reported,
            "ledger_path": ledger_path(),
        }, indent=2))
        return 0

    collected, per_source, per_source_secs = [], {}, {}
    runnable = []
    for cfg in sources_cfg.get("sources") or []:
        sid = cfg.get("id")
        if not cfg.get("enabled"):
            continue
        if args.source and sid not in args.source:
            continue
        func = SOURCE_FUNCS.get(cfg.get("kind"))
        if not func:
            sys.stderr.write(f"[scan] unknown source kind {cfg.get('kind')!r}\n")
            continue
        runnable.append((sid, cfg, func))

    def run_source(item):
        sid, source_cfg, func = item
        started = time.perf_counter()
        try:
            found = func(source_cfg, filters)
        except Exception as exc:  # noqa: BLE001
            sys.stderr.write(f"[scan] source {sid} raised: {exc}\n")
            found = []
        return sid, found, time.perf_counter() - started

    # Sources are independent network work, so they run concurrently: wall time becomes the
    # slowest source instead of the sum of all of them. None of them writes any state --
    # harvest_boards() does that afterwards, and the per-board health in boardstate.json is
    # only collected in memory here -- so overlapping them is safe. This is where
    # the scan's time actually goes: measured with them running in sequence, one source was
    # 285s of a 422s run and another 95s, nearly all of it spent waiting on sockets.
    boardstate.begin_run()
    phase_started = time.perf_counter()
    if len(runnable) > 1:
        with futures.ThreadPoolExecutor(max_workers=len(runnable)) as pool:
            results = list(pool.map(run_source, runnable))
    else:
        results = [run_source(item) for item in runnable]
    phase_secs = time.perf_counter() - phase_started

    # Only boards that were actually fetched leave a record, and a run that fetched none --
    # a --source run of one feed, say -- writes nothing and reports nothing. The daily run
    # is the only thing that persists per-board state.
    recorded = boardstate.end_run()
    if recorded:
        # The same trust check the separate sweep applies: a run that lost a quarter of the
        # fleet is the throttle, and its failures must not become the retry queue.
        verdict_ok, reason = boardstate.sweep_verdict(recorded)
        if verdict_ok:
            health = boardstate.finish_run(recorded, write=not args.no_write)
            sys.stderr.write(f"[scan] {boardstate.line(health)}\n")
        else:
            sys.stderr.write(f"[scan] not recording this run's board state: {reason}\n")

    for sid, found, secs in results:
        per_source[sid] = len(found)
        per_source_secs[sid] = secs
        collected.extend(found)

    # The cost of a scan is the sum of its sources, so the only way to keep that number
    # honest is to print it every run, slowest first. Guessing at which source is expensive
    # is how effort goes into the wrong one. The sum and the wall time differ by design now
    # that the sources overlap; both are worth seeing.
    if per_source_secs:
        detail = ", ".join("%s %.1fs" % (s, t) for s, t in
                           sorted(per_source_secs.items(), key=lambda kv: -kv[1]))
        sys.stderr.write("[scan] source time: %.1fs of work in %.1fs wall (%s)\n"
                         % (sum(per_source_secs.values()), phase_secs, detail))

    # What the egress guard did. Worth a line every run: a lookups-vs-cached ratio that
    # stops showing reuse, or a resolver pause, is the early sign the network is in trouble
    # again, and it is invisible in the postings.
    from . import net
    net_stats = net.stats()
    if net_stats["misses"] or net_stats["hits"]:
        sys.stderr.write("[scan] %s\n" % net.line())

    harvest_boards(collected, write=not args.no_write)

    # A feed that hands over employer boards is only half-used until the probe has run, and
    # nothing about the feed's own output reveals that. Say so every run.
    gap = unprobed_boards()
    if gap:
        shown = ", ".join(f"{b['ats']}/{b['slug']}" for b in gap[:10])
        more = "" if len(gap) <= 10 else f" (+{len(gap) - 10} more)"
        sys.stderr.write(f"[scan] {len(gap)} harvested boards are not in ats_boards.yaml "
                         f"-- run `python3 -m server probe-ats`: {shown}{more}\n")

    # collapse duplicates inside this run, then apply the filters
    kept, dropped = normalize_and_filter(collapse_requisitions(collected), filters)

    now = datetime.now(timezone.utc).isoformat(timespec="seconds")
    fresh = []
    for posting in sorted(kept, key=sort_key):
        entry = ledger.get(posting["id"])
        if entry is None:
            posting["first_seen"] = now
            posting["is_new"] = True
            ledger[posting["id"]] = {
                "id": posting["id"], "company": posting["company"],
                "title": posting["title"], "location": posting["location"],
                "url": posting["url"], "source": posting["source"],
                "first_seen": now, "reported": False,
            }
            fresh.append(posting)
        elif not entry.get("reported"):
            posting["first_seen"] = entry.get("first_seen")
            posting["is_new"] = False
            fresh.append(posting)

    if not args.no_write:
        # atomic_write, like every other state writer here: the dashboard,
        # `refresh`, `table` and the brief all read this file, and a crash mid-write would
        # leave them parsing a truncated file -- which they swallow into an empty result,
        # i.e. a quiet day the scan never had.
        payload = {"ran_at": now, "sources": per_source,
                   "source_seconds": {k: round(v, 2) for k, v in per_source_secs.items()},
                   "matched": kept,
                   "tier_counts": tier_counts(kept, filters), "dropped": dropped}
        last_scan = os.path.join(ROOT, "data", "state", "last_scan.json")
        with atomic_write(last_scan) as fh:
            json.dump(payload, fh, indent=2)
        save_ledger(ledger)
        # The portal feed is the dashboard's list: it accumulates postings, enriches them with
        # resume keywords and published addresses, and retires them after max_age_days. It is
        # refreshed after the scan has been written, and a failure here is reported and never
        # allowed to make the scan itself look failed -- the scan's output is what the portal
        # feed is built from, and it is already on disk by this point.
        try:
            from . import portal
            summary = portal.refresh(kept, filters=filters, ran_at=now)
            sys.stderr.write(portal.line(summary) + "\n")
        except Exception as exc:  # noqa: BLE001
            sys.stderr.write(f"[scan] portal refresh failed: {type(exc).__name__}: {exc}\n")

    if args.all:
        # --all ignores the reported ledger and lists every matching posting, so the
        # flag actually does what its help text and the workspace README claim.
        for posting in kept:
            entry = ledger.get(posting["id"])
            posting["first_seen"] = (entry or {}).get("first_seen")
            posting["is_new"] = entry is None
            posting["was_reported"] = bool((entry or {}).get("reported"))
        fresh = sorted(kept, key=sort_key)

    shown = fresh[:args.limit] if args.limit else fresh
    capped = 0
    if args.per_tier:
        shown, capped = cap_per_tier(shown, args.per_tier)
    counts = tier_counts(kept, filters)
    if args.json:
        print(json.dumps({
            "ran_at": now, "sources": per_source, "collected": len(collected),
            "matched_filters": len(kept), "unreported": len(fresh),
            "tier_counts": counts, "capped": capped,
            "postings": shown, "dropped": dropped,
        }, indent=2))
        return 0

    print(f"# job scan  {now}")
    print(f"# sources: " + ", ".join(f"{k}={v}" for k, v in per_source.items()))
    print(f"# collected={len(collected)} matched_filters={len(kept)} unreported={len(fresh)}")
    print("# tiers: " + "  ".join(f"{k}={v}" for k, v in counts.items() if v))
    if capped:
        print(f"# showing the top {args.per_tier} of each tier; {capped} more matched "
              f"(--per-tier 0 shows every match)")
    if dropped:
        top = sorted(dropped.items(), key=lambda kv: -kv[1])[:6]
        print("# dropped: " + ", ".join(f"{k}={v}" for k, v in top))
    print()
    if not shown:
        print("(nothing new)")
        return 0

    # One table per run, with the columns sized to the widest cell so it stays readable in a
    # terminal and in data/state/scan.log without a fixed layout to maintain by hand.
    #
    # The url and the id get a second table rather than columns in the first. They are the two
    # fields nothing else can substitute for -- mark_reported.py keys on the id and the portal
    # links on the url -- and a 90-character column would push the readable fields off screen.
    # So they stay whole, and the first table stays narrow.
    def clip(text, width):
        text = " ".join(str(text or "").split())
        return text if len(text) <= width else text[:width - 1].rstrip() + "…"

    cols = ["#", "score", "state", "age", "tier", "company", "title", "location", "kind", "flags"]
    rows = []
    notes = []
    for i, p in enumerate(shown, 1):
        age = p.get("age_days")
        flags = []
        if p.get("level_ii"):
            flags.append("LEVEL-II")
        if p.get("requires_us_work_auth"):
            flags.append("US-auth")
        if p.get("salary_annual_max"):
            pay = "$%dk-%dk" % (p["salary_annual_min"] // 1000, p["salary_annual_max"] // 1000)
            if p.get("salary_estimated"):
                pay += " ~est"
            flags.append(pay)
            if p.get("below_comp_floor"):
                flags.append("BELOW-FLOOR")
        if p.get("title_generic"):
            flags.append("generic-title")
            notes.append(f"  {i}: Job Bank lists this under the generic title "
                         f"{p['title_generic']!r}; the real title is in the table.")
        rows.append({
            "#": str(i),
            "score": str(p.get("score", "")),
            "state": "NEW" if p.get("is_new") else ("sent" if p.get("was_reported") else "held"),
            "age": f"{age}d" if age is not None else "?",
            "tier": str(p.get("tier") or "untiered"),
            "company": clip(p.get("company"), 22),
            "title": clip(p.get("title"), 44),
            "location": clip(p.get("location"), 24),
            "kind": str(p.get("location_kind") or ""),
            "flags": ",".join(flags),
        })

    width = {c: max([len(c)] + [len(r[c]) for r in rows]) for c in cols}
    head = "  ".join(c.ljust(width[c]) for c in cols[:-1]) + "  " + cols[-1]
    rule = "  ".join("-" * width[c] for c in cols[:-1]) + "  " + "-" * width[cols[-1]]
    print(head)
    print(rule)
    for r in rows:
        print("  ".join(r[c].ljust(width[c]) for c in cols[:-1]) + "  " + r["flags"])
    print()

    print("  ".join(["#".ljust(3), "id".ljust(18), "src/ats".ljust(14)]) + "  url")
    print("  ".join(["-" * 3, "-" * 18, "-" * 14]) + "  " + "-" * 40)
    for i, p in enumerate(shown, 1):
        src = str(p.get("source") or "")
        if p.get("ats"):
            src = f"{src}/{p['ats']}"
        url = p.get("url") or "(no direct link -- search the title on the company site)"
        print("  ".join([str(i).ljust(3), str(p.get("id", "")).ljust(18), src.ljust(14)])
              + "  " + url)

    if any(p.get("level_ii") for p in shown):
        print("\n  LEVEL-II rows: report one only when the fit against the candidate's own"
              " experience is strong and you can name the evidence.")
    if notes:
        print()
        for note in notes:
            print(note)
    return 0
