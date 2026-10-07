"""Records the employer boards a feed reveals, for server/jobs/probe_ats.py to pick up."""

from __future__ import annotations

from datetime import datetime, timezone
import json
import os

from .core import ROOT, load_yaml


def harvest_boards(collected, write=True):
    """Record the ATS boards that a feed revealed, for the board probe to pick up.

    server/jobs/probe_ats.py reads state/sg_boards.json and treats every slug in it as a probe
    candidate, so a company that surfaced once gets its whole board scanned later instead
    of only the posting that happened to appear. The file accumulates across runs, so a
    company that drops out of the feed keeps its entry.
    """
    path = os.path.join(ROOT, "state", "sg_boards.json")
    try:
        with open(path, encoding="utf-8") as fh:
            store = json.load(fh)
    except (OSError, json.JSONDecodeError):
        store = {}
    boards = {f"{b.get('ats')}|{b.get('slug')}": b for b in store.get("boards") or []}
    now = datetime.now(timezone.utc).isoformat(timespec="seconds")
    for posting in collected:
        slug, ats = posting.get("board_slug"), posting.get("ats")
        if not (slug and ats):
            continue
        row = boards.setdefault(f"{ats}|{slug}", {
            "name": posting.get("company"), "ats": ats, "slug": slug,
            "first_seen": now, "postings_seen": 0,
        })
        row["last_seen"] = now
        row["postings_seen"] = int(row.get("postings_seen") or 0) + 1
    if write:
        # Atomic, and the temp name carries the pid: two scans can run at once.
        tmp = f"{path}.{os.getpid()}.tmp"
        with open(tmp, "w", encoding="utf-8") as fh:
            json.dump({"updated_at": now,
                       "boards": sorted(boards.values(), key=lambda b: (b["ats"], b["slug"]))},
                      fh, indent=1)
        os.replace(tmp, path)
    return len(boards)


# The kinds server/jobs/probe_ats.py can actually read; mirrors its BOARD_URL. A harvested board of
# any other kind is out of the probe's reach, so reporting it would be noise.
PROBE_KINDS = ("greenhouse", "ashby", "lever", "workable")


def unprobed_boards():
    """Harvested boards that are still absent from config/ats_boards.yaml.

    A feed that deep-links into employer boards is only half-used until the probe has run,
    and the gap is invisible from the outside: the feed still yields its own handful of
    postings per run, so nothing looks wrong. It has recurred twice -- 42 boards holding
    1,597 roles on 2026-09-30, another 14 on 2026-10-03 -- so the scan reports it rather
    than waiting for someone to notice.
    """
    path = os.path.join(ROOT, "state", "sg_boards.json")
    try:
        with open(path, encoding="utf-8") as fh:
            store = json.load(fh)
    except (OSError, json.JSONDecodeError):
        return []
    try:
        cfg = load_yaml("config", "ats_boards.yaml") or {}
    except Exception:
        return []
    have = set()
    for value in cfg.values():
        if not isinstance(value, list):
            continue
        for entry in value:
            if isinstance(entry, dict) and entry.get("ats") and entry.get("slug"):
                have.add((entry["ats"], entry["slug"]))
    out = []
    for board in store.get("boards") or []:
        ats, slug = board.get("ats"), board.get("slug")
        if not (ats and slug) or ats not in PROBE_KINDS:
            continue
        if (ats, slug) not in have:
            out.append(board)
    return out
