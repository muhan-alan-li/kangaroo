"""Where a brief lives on disk, and the only code that writes it.

A run can leave up to five files under `data/state/briefs/`:

    <id>.json          the result the dashboard reads, success or failure
    <id>.running       marker while a run is in flight; stale after BRIEF_STALE_AFTER
    <id>.raw.json      stage A + B output (default engine) or the agent's unedited brief
    <id>.agent.json    the JSON the Hermes agent is instructed to write
    <id>.review.json   the critic's edit of the agent's brief

Every write is atomic because the dashboard re-reads these files on every poll: a reader must
never catch a half-written result. This module knows nothing about how a brief is produced.
"""
from __future__ import annotations

import json
import os
from datetime import datetime, timezone

from ..config import BRIEFS
from ..fsutil import atomic_write


def now_iso():
    return datetime.now(timezone.utc).isoformat(timespec="seconds")


def read_json(path, default=None):
    """Parse one brief file. Absent or unreadable state is a cold start, not an error."""
    try:
        with open(path, encoding="utf-8") as fh:
            return json.load(fh)
    except Exception:  # noqa: BLE001
        return default


def write_json(path, payload):
    """Serialize `payload` under `path`, atomically."""
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with atomic_write(path) as fh:
        fh.write(json.dumps(payload, indent=1, ensure_ascii=False))


def final_path(pid):
    return os.path.join(BRIEFS, f"{pid}.json")


def agent_path(pid):
    return os.path.join(BRIEFS, f"{pid}.agent.json")


def review_agent_path(pid):
    return os.path.join(BRIEFS, f"{pid}.review.json")


def raw_path(pid):
    return os.path.join(BRIEFS, f"{pid}.raw.json")


def running_path(pid):
    return os.path.join(BRIEFS, f"{pid}.running")


def mark_running(pid, **fields):
    """Write the marker the dashboard polls, and make sure the briefs directory exists.

    The PID and timestamp describe *this* process. Callers add what distinguishes one kind of
    run from another (`engine`, `mode`, `description_source`).
    """
    os.makedirs(BRIEFS, exist_ok=True)
    write_json(running_path(pid), {
        "pid": os.getpid(),
        "started_at": now_iso(),
        "posting_id": str(pid),
        **fields,
    })


def clear_running(pid):
    try:
        os.unlink(running_path(pid))
    except FileNotFoundError:
        pass


def write_result(pid, payload):
    """Persist the finished (or failed) brief and clear the in-flight marker."""
    payload.setdefault("generated_at", now_iso())
    payload["posting_id"] = str(pid)
    write_json(final_path(pid), payload)
    clear_running(pid)
