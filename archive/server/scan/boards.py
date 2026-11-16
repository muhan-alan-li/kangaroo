"""Board-list loading both scheduled jobs share.

The sweep (`server/jobs/scan/sweep.py`) and the retry (`server/jobs/retry/retry.py`)
read the same two files -- the Workday board list and the `ats_boards` source block
in sources.yaml -- and the probe (`server/jobs/probe_ats.py`) walks the same
`boards_*` shape. One copy here, so a schema change cannot fix one job and
silently leave the other on the old walk.
"""
from __future__ import annotations

from .core import load_yaml


def iter_board_rows(data):
    """Every dict row under a `boards_*` key, in file order."""
    if isinstance(data, dict):
        for key, rows in data.items():
            if key.startswith("boards_"):
                for row in rows or []:
                    if isinstance(row, dict):
                        yield row
    elif isinstance(data, list):
        for row in data:
            if isinstance(row, dict):
                yield row


def load_workday_boards(path=("data", "config", "workday_boards.yaml")):
    """Every Workday board in the configured list, in file order."""
    return [b for b in iter_board_rows(load_yaml(*path))
            if b.get("tenant") and b.get("site")]


def find_ats_source(path=("data", "config", "sources.yaml")):
    """The `ats_boards` source block in sources.yaml, or {}."""
    cfg = load_yaml(*path)
    for source in cfg.get("sources") or []:
        if source.get("id") == "ats_boards":
            return source
    return {}


def workday_opts():
    """The `workday:` options under the `ats_boards` source, or {}."""
    return find_ats_source().get("workday") or {}


def ats_slugs(data):
    """{"ats/slug"} for every row that carries both."""
    return {f"{r['ats']}/{r['slug']}" for r in iter_board_rows(data)
            if r.get("ats") and r.get("slug")}
