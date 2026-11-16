"""The second-level contact search: the Hermes agent reads the public web.

The default brief no longer researches contacts. This is the on-demand flow for the cases where
a contact matters: one agent run over team pages, public profiles and public commit metadata,
merged into the existing brief without touching the resume-tailoring advice.

It writes the same in-flight marker as every other brief run, so the dashboard polls one flag
for both. It never sends an email, applies anywhere, or writes to LinkedIn.
"""
from __future__ import annotations

import os
import subprocess
import time

from ..config import ROOT
from . import prompts
from .agent import DEFAULT_TIMEOUT, hermes_bin
from .posting import description_for, find_posting
from .state import (agent_path, final_path, mark_running, now_iso, read_json,
                    write_result)


def generate(pid, timeout=DEFAULT_TIMEOUT, runner=subprocess.run):
    """Run the deep contact search and merge the result into the cached brief."""
    row = find_posting(pid)
    if not row:
        payload = {"status": "failed",
                   "error": f"no posting {pid} in the portal feed or the last scan"}
        write_result(pid, payload)
        return payload

    text, source = description_for(row)
    try:
        os.unlink(agent_path(pid))
    except FileNotFoundError:
        pass
    mark_running(pid, description_source=source, mode="deep-contacts")

    started = time.monotonic()
    binary = hermes_bin()
    if not os.path.exists(binary):
        data = read_json(final_path(pid), {}) or {}
        data.setdefault("status", "ok")
        data["contact_notes"] = f"Deep contact search unavailable: no hermes binary at {binary}."
        write_result(pid, data)
        return data
    try:
        runner([binary, "-z", prompts.build_contact_prompt(row, text), "--in", ROOT],
               capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        data = read_json(final_path(pid), {}) or {}
        data.setdefault("status", "ok")
        data["contact_notes"] = f"Deep contact search did not finish in {timeout}s."
        write_result(pid, data)
        return data

    raw = read_json(agent_path(pid), {}) or {}
    contacts = [c for c in (raw.get("contacts") or []) if isinstance(c, dict)]
    notes = str(raw.get("contact_notes") or "").strip()
    data = read_json(final_path(pid), {}) or {}
    data["contacts"] = contacts
    if notes:
        data["contact_notes"] = notes
    elif not contacts and not data.get("contact_notes"):
        data["contact_notes"] = "The deep search found no published address."
    data["contacts_source"] = "hermes agent (deep)"
    data["deep_contacts_at"] = now_iso()
    data.setdefault("status", "ok")
    data["elapsed_s"] = round(time.monotonic() - started, 1)
    data["company"] = row.get("company")
    data["title"] = row.get("title")
    data["url"] = row.get("url")
    write_result(pid, data)
    return data
