"""The seen ledger: one row per posting id, and the reported flag that keeps
the daily report from repeating itself."""

from __future__ import annotations

import json
import os

from .core import ROOT


# ----------------------------------------------------------------------- ledger

def ledger_path():
    return os.path.join(ROOT, "state", "seen.jsonl")


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
    # The temp name carries the pid: two scans can run at once (two cron jobs that were
    # both overdue fire within a second of each other), and a shared temp path lets one
    # process rename the file while the other is still writing into it.
    tmp = f"{ledger_path()}.{os.getpid()}.tmp"
    with open(tmp, "w", encoding="utf-8") as fh:
        for rec in entries.values():
            fh.write(json.dumps(rec, sort_keys=True) + "\n")
    os.replace(tmp, ledger_path())
