"""The Hermes agent engine: the older one-shot brief, plus its editorial review.

Kept for the half of the job that needs the public web -- reading a team page or a profile for
a published contact address. The default three-stage chain (`chain.py`) does the tailoring
without tools and returns in seconds; this path is slower (a couple of minutes) and is chosen
explicitly with `--engine hermes`.

A run has two passes: the researcher writes the brief, then a fresh critic sees only that brief
and is asked to delete everything that would not change what the candidate does next. A failed
review is never fatal -- the raw brief is kept, because losing research to a bad edit is the
wrong trade.

Neither pass sends, applies or writes a ledger.
"""
from __future__ import annotations

import json
import os
import shutil
import subprocess
import time

from ..config import ROOT
from . import prompts
from .posting import description_for, find_posting
from .state import (agent_path, mark_running, raw_path, review_agent_path,
                    write_json, write_result)

DEFAULT_TIMEOUT = 420          # the agent path may read several pages; direct returns in seconds
REVIEW_TIMEOUT = 240           # the critic only reads what it is handed; it does no research


def hermes_bin():
    return shutil.which("hermes") or os.path.expanduser("~/.local/bin/hermes")


def review_brief(row, brief, timeout=REVIEW_TIMEOUT, runner=subprocess.run):
    """Second pass: a critic that cuts the brief to what changes a decision.

    The candidate asked for this on 2026-10-05 -- "its a lot of info and not all of it is
    valuable" -- and the fix is an editorial pass, not a stricter researcher: a fresh agent that
    sees only the finished brief and has no research to do is much better at deleting.

    Returns (reviewed, None) or (None, reason).
    """
    path = review_agent_path(row.get("id"))
    try:
        os.unlink(path)
    except FileNotFoundError:
        pass
    prompt = prompts.build_review_prompt(row, brief)
    try:
        runner([hermes_bin(), "-z", prompt, "--in", ROOT],
               capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        return None, f"the review pass did not finish in {timeout}s"
    except Exception as exc:  # noqa: BLE001
        return None, f"the review pass could not run: {exc}"
    try:
        with open(path, encoding="utf-8") as fh:
            data = json.load(fh)
    except FileNotFoundError:
        return None, "the review pass wrote nothing"
    except json.JSONDecodeError as exc:
        return None, f"the review pass wrote invalid JSON: {exc}"
    if not isinstance(data, dict):
        return None, "the review pass did not return an object"
    return data, None


def generate(pid, timeout=DEFAULT_TIMEOUT, runner=subprocess.run):
    """Run the agent and write the brief. Returns the payload it wrote."""
    row = find_posting(pid)
    if not row:
        payload = {"status": "failed",
                   "error": f"no posting {pid} in the portal feed or the last scan"}
        write_result(pid, payload)
        return payload

    text, source = description_for(row)
    prompt = prompts.build_agent_prompt(row, text)
    try:
        os.unlink(agent_path(pid))
    except FileNotFoundError:
        pass
    mark_running(pid, description_source=source, engine="hermes")

    binary = hermes_bin()
    if not os.path.exists(binary):
        payload = {"status": "failed", "error": f"no hermes binary at {binary}"}
        write_result(pid, payload)
        return payload

    started = time.monotonic()
    tail = ""
    try:
        proc = runner([binary, "-z", prompt, "--in", ROOT],
                      capture_output=True, text=True, timeout=timeout)
        tail = ((proc.stdout or "") + "\n" + (proc.stderr or "")).strip()[-3000:]
    except subprocess.TimeoutExpired:
        payload = {"status": "failed", "error": f"the agent did not finish in {timeout}s",
                   "description_source": source,
                   "elapsed_s": round(time.monotonic() - started, 1)}
        write_result(pid, payload)
        return payload

    raw = None
    try:
        with open(agent_path(pid), encoding="utf-8") as fh:
            raw = json.load(fh)
    except FileNotFoundError:
        payload = {"status": "failed",
                   "error": "the agent finished without writing the brief file",
                   "agent_tail": tail, "description_source": source}
        write_result(pid, payload)
        return payload
    except json.JSONDecodeError as exc:
        payload = {"status": "failed", "error": f"the agent wrote invalid JSON: {exc}",
                   "agent_tail": tail, "description_source": source}
        write_result(pid, payload)
        return payload

    if not isinstance(raw, dict):
        payload = {"status": "failed", "error": "the agent's JSON was not an object",
                   "agent_tail": tail}
        write_result(pid, payload)
        return payload

    reviewed, why = review_brief(row, raw, runner=runner)
    if reviewed:
        # Keep the researcher's output for the record: the edit is reversible, the research is not.
        write_json(raw_path(pid), raw)
        payload = dict(reviewed)
        payload["reviewed"] = True
        # The critic is told to copy contacts through untouched. If it dropped them anyway the
        # verified ones win: each cost a source check and cannot be re-derived cheaply.
        if not payload.get("contacts") and raw.get("contacts"):
            payload["contacts"] = raw["contacts"]
            payload["contacts_restored"] = True
    else:
        payload = dict(raw)
        payload["reviewed"] = False
        payload["review_error"] = why
    payload["status"] = "ok"
    payload["description_source"] = source
    payload["elapsed_s"] = round(time.monotonic() - started, 1)
    payload["company"] = row.get("company")
    payload["title"] = row.get("title")
    payload["url"] = row.get("url")
    write_result(pid, payload)
    return payload
