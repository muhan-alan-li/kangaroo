"""The seen ledger: one row per posting id, and the reported flag that keeps
the daily report from repeating itself."""

from __future__ import annotations

import json
import os

from .core import ROOT, atomic_write


# ----------------------------------------------------------------------- ledger

def ledger_path():
    return os.path.join(ROOT, "data", "state", "seen.jsonl")


def load_ledger():
    entries = {}
    path = ledger_path()
    if not os.path.exists(path):
        return entries
    with open(path, encoding="utf-8") as fh:
        for line in fh:
            line = line.strip()
            if not line:
                continue
            try:
                rec = json.loads(line)
            except json.JSONDecodeError:
                continue
            if rec.get("id"):
                entries[rec["id"]] = rec
    return entries


def save_ledger(entries):
    os.makedirs(os.path.dirname(ledger_path()), exist_ok=True)
    with atomic_write(ledger_path()) as fh:
        for rec in entries.values():
            fh.write(json.dumps(rec, sort_keys=True) + "\n")
