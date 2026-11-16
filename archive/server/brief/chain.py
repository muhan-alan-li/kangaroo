"""The default brief engine: the three-stage tailoring chain, end to end.

`stages.py` owns the three prompts and their single model calls; this module owns the run around
them: locate the posting, get its text, write the in-flight marker, run A -> B -> C, enforce the
edits in `validate.py`, and persist the result. It is the only path that produces a brief with
no contacts.

Three reasoning calls per brief, no agent, no session. Finding a contact is deliberately a
separate, on-demand flow (`contacts.py`), because it is the only half that needs the web.
"""
from __future__ import annotations

import time

from . import stages, validate
from .posting import RESUME_PATH, description_for, find_posting
from .state import mark_running, raw_path, write_json, write_result


def generate(pid):
    """Generate one brief with the three-stage chain. Returns the payload it wrote.

    Stage A extracts the posting's requirement contract from the posting text alone (the
    resume is withheld so it cannot bias what the posting asks for). Stage B matches every
    requirement against the resume: met / transferable / missing, with the evidence bullet
    copied verbatim. Stage C writes the finished brief from that matrix.

    `validate.check_rewords` then drops any edit whose `before` is not verbatim in the resume
    and flags an edit that invents a term found in neither the resume nor the posting. Validity
    is enforced in code, not trusted to a prompt.
    """
    row = find_posting(pid)
    if not row:
        payload = {"status": "failed",
                   "error": f"no posting {pid} in the portal feed or the last scan"}
        write_result(pid, payload)
        return payload

    text, source = description_for(row)
    mark_running(pid, description_source=source, engine="deepseek")

    started = time.monotonic()
    resume = stages.read_prompt(RESUME_PATH) or "(resume text is unavailable)"

    try:
        requirements = stages.stage_requirements(row, text)
        matches = stages.stage_matches(row, text, requirements, resume)
        written = stages.stage_brief(row, text, requirements, matches, resume)
        write_json(raw_path(pid), {"stage_a": requirements, "stage_b": matches})
    except Exception as exc:  # noqa: BLE001 -- every failure is the page's to report
        payload = {"status": "failed", "error": str(exc), "description_source": source,
                   "stage": "tailoring chain", "elapsed_s": round(time.monotonic() - started, 1)}
        write_result(pid, payload)
        return payload

    # Validate before anything is shown. Edit quality stays with stage C; truth is enforced here.
    tailoring = written.get("tailoring") if isinstance(written.get("tailoring"), dict) else {}
    kept, dropped, flags = validate.check_rewords(tailoring.get("reword"), resume, text)
    tailoring["reword"] = kept

    payload = dict(written)
    payload["tailoring"] = tailoring
    if dropped:
        payload["reword_dropped"] = dropped
    if flags:
        payload["reword_flags"] = flags

    payload["status"] = "ok"
    payload["description_source"] = source
    payload["elapsed_s"] = round(time.monotonic() - started, 1)
    payload["company"] = row.get("company")
    payload["title"] = row.get("title")
    payload["url"] = row.get("url")
    write_result(pid, payload)
    return payload
