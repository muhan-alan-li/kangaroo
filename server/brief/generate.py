#!/usr/bin/env python3
"""Generate the per-posting brief that the dashboard's detail view shows.

On demand only. Nothing here runs during a scan: the two things it produces -- resume-tailoring
advice and reach-out targets -- need reasoning about the posting, and the candidate asked for
that to happen when a posting is opened rather than preloaded for every row in the portal.

Two engines, one output. The default (`--engine deepseek`) runs the three-stage tailoring chain
(`server/brief/stages.py`): stage A extracts the posting's requirement contract from the posting
text alone, stage B matches each requirement against the resume with met/transferable/missing
verdicts and verbatim evidence, stage C writes the brief and resume edits from that matrix, and
`server/brief/validate.py` then drops mechanically: a `before` that is not verbatim in the resume,
and a flag for any term an edit invents that neither the resume nor the posting shows. Three
reasoning calls per brief total, no agent, no session. The contact loop (`research_contacts`)
runs concurrently with the chain, since neither needs what the other produces.

`--engine hermes` is the older one-shot agent (`hermes -z`). It exists for the half that needs
the web: reading the team page that owns the posting, a public profile, and public commit
metadata for a published address. Slower (a couple of minutes), and only worth it when contacts
matter. Neither engine ever sends, applies or writes a ledger.

    /usr/bin/python3 -m server brief <posting-id>                   # direct DeepSeek (default)
    /usr/bin/python3 -m server brief <posting-id> --engine hermes   # agent + contact research
    /usr/bin/python3 -m server brief <posting-id> --print           # show the cached brief
    /usr/bin/python3 -m server brief --list                         # what is cached
    /usr/bin/python3 -m server brief <posting-id> --prompt          # print the agent prompt

Writes `data/state/briefs/<id>.json`. A failure is written too, with the reason, so the dashboard can
say what went wrong instead of spinning forever.
"""
from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import threading
import time
from datetime import datetime, timezone

from ..config import ROOT
from ..scan.core import atomic_write
from . import llm, research, stages, validate
from .stages import MAX_POSTING_TEXT

STATE = os.path.join(ROOT, "data", "state")
BRIEFS = os.path.join(STATE, "briefs")
FEED_PATH = os.path.join(STATE, "portal_feed.jsonl")
DESC_PATH = os.path.join(STATE, "descriptions.json")
SCAN_PATH = os.path.join(STATE, "last_scan.json")
PROMPT_PATH = os.path.join(ROOT, "data", "config", "brief_prompt.md")
REVIEW_PROMPT_PATH = os.path.join(ROOT, "data", "config", "brief_review_prompt.md")
CONTACT_PROMPT_PATH = os.path.join(ROOT, "data", "config", "contact_research_system.md")
RESUME_PATH = os.path.join(ROOT, "data", "candidate", "resume-text.md")
SKILL_DIR = os.path.expanduser(
    "~/.hermes/skills/productivity/job-application-run")

DEFAULT_TIMEOUT = 420          # the agent path may read several pages; direct returns in seconds
CONTACT_ROUNDS = 8             # tool-call rounds in the contact loop before it must answer
CONTACT_TOOL_CHARS = 6000      # cap one tool result so a long page cannot blow the context
REVIEW_TIMEOUT = 240           # the critic only reads what it is handed; it does no research


def _read_json(path, default=None):
    try:
        with open(path, encoding="utf-8") as fh:
            return json.load(fh)
    except Exception:  # noqa: BLE001 -- absent state is a cold start
        return default


def _atomic_write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with atomic_write(path) as fh:
        fh.write(text)


def now_iso():
    return datetime.now(timezone.utc).isoformat(timespec="seconds")


def find_posting(pid):
    """The portal row for a posting id, falling back to the last scan's list."""
    try:
        with open(FEED_PATH, encoding="utf-8") as fh:
            for line in fh:
                line = line.strip()
                if not line:
                    continue
                try:
                    row = json.loads(line)
                except json.JSONDecodeError:
                    continue
                if str(row.get("id")) == str(pid):
                    return row
    except FileNotFoundError:
        pass
    for row in (_read_json(SCAN_PATH, {}) or {}).get("matched") or []:
        if str(row.get("id")) == str(pid):
            return row
    return None


def description_for(row):
    """The posting's text, from the cache, the row, or the board API.

    Reuses scan.portal's fetchers rather than growing a second set, so the two never disagree
    about how to read a Greenhouse or Workday posting. The text is written back to the shared
    cache so the next open of this posting is instant; that file is written atomically on both
    sides, so a concurrent scan can only mean last-writer-wins on identical data.
    """
    pid = row.get("id")
    cache = _read_json(DESC_PATH, {}) or {}
    if cache.get(pid):
        return cache[pid], "cache"
    if row.get("description"):
        return row["description"], "row"
    # Pre-fix rows: the portal kept `has_text: true` but dropped the source's text, so the
    # feed row and the cache are both empty while the last scan still carries it.
    for scan_row in (_read_json(SCAN_PATH, {}) or {}).get("matched") or []:
        if str(scan_row.get("id")) == str(pid) and scan_row.get("description"):
            return scan_row["description"], "scan"
    try:
        from ..scan import portal
    except Exception:  # noqa: BLE001
        return None, "none"
    if not portal._description_fetcher(row):
        return None, "unfetchable"
    text = portal._safe_description(row)
    if not text:
        return None, "failed"
    cache[pid] = text
    _atomic_write(DESC_PATH, json.dumps(cache, indent=1))
    return text, "fetched"


def build_prompt(row, text):
    with open(PROMPT_PATH, encoding="utf-8") as fh:
        template = fh.read()
    keys = ", ".join(row.get("keywords") or []) or "(none extracted)"
    have = ", ".join(row.get("skills_have") or []) or "(none)"
    gap = ", ".join(row.get("skills_gap") or []) or "(none)"
    body = text or ("(not supplied -- fetch the url above and read it before writing the brief)")
    return (template
            .replace("{{ID}}", str(row.get("id")))
            .replace("{{COMPANY}}", str(row.get("company") or ""))
            .replace("{{TITLE}}", str(row.get("title") or ""))
            .replace("{{LOCATION}}", str(row.get("location") or ""))
            .replace("{{TIER}}", str(row.get("tier") or ""))
            .replace("{{SCORE}}", str(row.get("score") or ""))
            .replace("{{URL}}", str(row.get("url") or ""))
            .replace("{{KEYWORDS}}", keys)
            .replace("{{SKILLS_HAVE}}", have)
            .replace("{{SKILLS_GAP}}", gap)
            .replace("{{ROOT}}", ROOT)
            .replace("{{SKILL_DIR}}", SKILL_DIR)
            .replace("{{OUTPUT_PATH}}", agent_path(row.get("id")))
            .replace("{{POSTING_TEXT}}", body[:MAX_POSTING_TEXT]))


def agent_path(pid):
    return os.path.join(BRIEFS, f"{pid}.agent.json")


def final_path(pid):
    return os.path.join(BRIEFS, f"{pid}.json")


def running_path(pid):
    return os.path.join(BRIEFS, f"{pid}.running")


def review_agent_path(pid):
    return os.path.join(BRIEFS, f"{pid}.review.json")


def raw_path(pid):
    return os.path.join(BRIEFS, f"{pid}.raw.json")


def build_review_prompt(row, brief):
    """The critic's prompt: the brief to edit, and nothing else to research."""
    with open(REVIEW_PROMPT_PATH, encoding="utf-8") as fh:
        template = fh.read()
    return (template
            .replace("{{OUTPUT_PATH}}", review_agent_path(row.get("id")))
            .replace("{{BRIEF_JSON}}", json.dumps(brief, indent=1, ensure_ascii=False))
            .replace("{{COMPANY}}", str(row.get("company") or ""))
            .replace("{{TITLE}}", str(row.get("title") or ""))
            .replace("{{LOCATION}}", str(row.get("location") or ""))
            .replace("{{TIER}}", str(row.get("tier") or "")))


def review_brief(row, brief, timeout=REVIEW_TIMEOUT, runner=subprocess.run):
    """Second pass: a critic that cuts the brief to what changes a decision.

    The candidate asked for this on 2026-10-05 -- "its a lot of info and not all of it is
    valuable" -- and the fix is an editorial pass, not a stricter researcher: a fresh agent that
    sees only the finished brief and has no research to do is much better at deleting.

    Returns (reviewed, None) or (None, reason). A failed review is never fatal: the caller keeps
    the raw brief, because losing the research to a bad edit is the wrong trade.
    """
    path = review_agent_path(row.get("id"))
    try:
        os.unlink(path)
    except FileNotFoundError:
        pass
    prompt = build_review_prompt(row, brief)
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


def hermes_bin():
    return shutil.which("hermes") or os.path.expanduser("~/.local/bin/hermes")


def write_result(pid, payload):
    payload.setdefault("generated_at", now_iso())
    payload["posting_id"] = str(pid)
    _atomic_write(final_path(pid), json.dumps(payload, indent=1, ensure_ascii=False))
    try:
        os.unlink(running_path(pid))
    except FileNotFoundError:
        pass


def direct_messages(row, text):
    """The chain's prompts, printed by `--prompt` for review. Not a call path any more."""
    resume = stages.read_prompt(RESUME_PATH) or "(resume text is unavailable)"
    return [
        {"role": "system", "content": stages.read_prompt(stages.STAGE_A_PROMPT),
         "user": "# The posting\n\n"
                 f"- title: {row.get('title')}\n- company: {row.get('company')}\n"
                 f"- location: {row.get('location')}\n\n## Posting text\n\n```\n"
                 + stages.cap_text(text) + "\n```"},
        {"role": "system", "content": stages.read_prompt(stages.STAGE_B_PROMPT),
         "user": "# Resume\n\n" + resume + "\n\n# Requirements: stage A's output goes here"},
        {"role": "system", "content": stages.read_prompt(stages.STAGE_C_PROMPT),
         "user": "# Resume\n\n" + resume + "\n\n# Match matrix: stage B's output goes here"},
    ]


# The tools the contact loop may call. They are the copied research primitives -- no agent, no
# key -- and the model is told to fetch a page before citing an address from it. `browser_fetch`
# is a local Chromium for the pages a plain GET cannot read.
CONTACT_TOOLS = [
    {"type": "function", "function": {
        "name": "web_search",
        "description": "Search the public web. Returns titles, URLs and snippets. Use it to find "
                       "a company's team page, an engineer's public profile, or a hiring post.",
        "parameters": {"type": "object", "properties": {
            "query": {"type": "string", "description": "the search query"}},
            "required": ["query"]}}},
    {"type": "function", "function": {
        "name": "web_fetch",
        "description": "Fetch one public URL and return its visible text plus any email addresses "
                       "published in its raw HTML, including Cloudflare-obfuscated ones. Read the "
                       "page with this before citing an address.",
        "parameters": {"type": "object", "properties": {
            "url": {"type": "string"}}, "required": ["url"]}}},
    {"type": "function", "function": {
        "name": "browser_fetch",
        "description": "Fetch one public URL in a real headless browser and return the rendered "
                       "text plus any email addresses in the page. Use this instead of web_fetch "
                       "when the page renders client-side or web_fetch came back nearly empty. "
                       "Slower, so reach for it only after web_fetch.",
        "parameters": {"type": "object", "properties": {
            "url": {"type": "string"}}, "required": ["url"]}}},
    {"type": "function", "function": {
        "name": "github_commit_emails",
        "description": "Recent commit authors (name, work email, commit URL) for a public GitHub "
                       "repo such as 'stripe/stripe-python'. Commit metadata carries work "
                       "addresses the company published.",
        "parameters": {"type": "object", "properties": {
            "repo": {"type": "string", "description": "owner/name"}},
            "required": ["repo"]}}},
]


def _emails_from(value):
    """Every address mentioned anywhere in a tool result, for the verbatim check."""
    found = []
    if isinstance(value, dict):
        for item in value.values():
            found += _emails_from(item)
    elif isinstance(value, list):
        for item in value:
            found += _emails_from(item)
    elif isinstance(value, str):
        found += research.emails_in(value)
    return found


def _tool_result(name, args):
    """Run one tool call. A failure is a result, not an exception, so the loop continues."""
    try:
        if name == "web_search":
            return research.search(str(args.get("query") or ""), limit=6)
        if name == "web_fetch":
            return research.fetch(str(args.get("url") or ""))
        if name == "browser_fetch":
            return research.browser_fetch(str(args.get("url") or ""))
        if name == "github_commit_emails":
            return research.github_commit_emails(str(args.get("repo") or ""))
        return {"error": f"unknown tool {name!r}"}
    except Exception as exc:  # noqa: BLE001 -- the model decides what to do about it
        return {"error": str(exc)}


def contact_messages(row, text):
    system = stages.read_prompt(CONTACT_PROMPT_PATH)
    user = (
        "# The posting\n\n"
        f"- company: {row.get('company')}\n"
        f"- title: {row.get('title')}\n"
        f"- location: {row.get('location')}\n"
        f"- url: {row.get('url')}\n\n"
        "## Posting text\n\n```\n"
        + (text or "(no posting text available)")[:MAX_POSTING_TEXT]
        + "\n```\n\n"
        "Find a published work address to email about this posting."
    )
    return [{"role": "system", "content": system}, {"role": "user", "content": user}]


def research_contacts(row, text, timeout=DEFAULT_TIMEOUT):
    """The copied contact loop: search/fetch/browser/commit tools until the model can name one.

    Returns `{contacts, contact_notes}`. Only addresses that appeared verbatim in a tool result
    are kept, so a hallucinated address never reaches the brief -- the skill's "re-verify every
    address yourself" rule, enforced in code instead of trusted to the model.
    """
    messages = contact_messages(row, text)
    seen_emails = set()
    for _ in range(CONTACT_ROUNDS):
        message = llm.chat_message(messages, tools=CONTACT_TOOLS, timeout=timeout)
        messages.append({key: message.get(key) for key in ("role", "content", "tool_calls")
                         if message.get(key) is not None})
        calls = message.get("tool_calls") or []
        if not calls:
            break
        for call in calls:
            function = call.get("function") or {}
            try:
                args = json.loads(function.get("arguments") or "{}")
            except json.JSONDecodeError:
                args = {}
            if not isinstance(args, dict):
                args = {}
            result = _tool_result(function.get("name") or "", args)
            seen_emails.update(email.lower() for email in _emails_from(result))
            messages.append({"role": "tool", "tool_call_id": call.get("id") or "",
                             "content": json.dumps(result, ensure_ascii=False)[:CONTACT_TOOL_CHARS]})
    messages.append({"role": "user", "content":
                     "Stop searching and reply with the final JSON object only."})
    try:
        final = llm.chat_json(messages, timeout=timeout)
    except llm.LLMError as exc:
        return {"contacts": [], "contact_notes": f"contact research failed: {exc}"}
    contacts = []
    for contact in final.get("contacts") or []:
        if not isinstance(contact, dict):
            continue
        email = str(contact.get("email") or "").strip()
        if email and email.lower() in seen_emails:
            contacts.append(contact)
    notes = str(final.get("contact_notes") or "").strip()
    if not contacts and not notes:
        notes = "No published address was found in the sources searched."
    return {"contacts": contacts, "contact_notes": notes}


def generate_direct(pid, timeout=DEFAULT_TIMEOUT, contacts=True):
    """Generate the brief with the three-stage tailoring chain, contacts running alongside.

    Stage A extracts the posting's requirement contract (posting text only, resume withheld so
    it cannot bias what the posting asks for). Stage B matches every requirement against the
    resume: met / transferable / missing, with the evidence bullet copied verbatim. Stage C
    writes the finished brief from that matrix. `server/brief/validate.py` then drops any edit
    whose `before` is not verbatim in the resume, and flags a term an edit invents that neither
    the resume nor the posting shows; validity is enforced in code, not trusted to a prompt.

    Three chain calls plus, concurrently, the contact tool loop (`research_contacts`): finding an
    address needs only the row and the posting text, never the chain's outputs, so it starts with
    stage A and its wall time hides inside the chain's. Only an address that appeared verbatim in
    a tool result is kept.
    """
    row = find_posting(pid)
    if not row:
        payload = {"status": "failed", "error": f"no posting {pid} in the portal feed or the last scan"}
        write_result(pid, payload)
        return payload

    text, source = description_for(row)
    os.makedirs(BRIEFS, exist_ok=True)
    _atomic_write(running_path(pid), json.dumps(
        {"pid": os.getpid(), "started_at": now_iso(), "posting_id": str(pid),
         "description_source": source, "engine": "deepseek"}, indent=1))

    started = time.monotonic()
    resume = stages.read_prompt(RESUME_PATH) or "(resume text is unavailable)"

    # Contacts do not wait for the chain. Stage C's prompt returns an empty contact list on
    # purpose -- the model must not invent an address -- so this loop is the only path that can
    # populate them, and it needs nothing the chain produces.
    found = {"contacts": [], "contact_notes": "contact research failed"}
    contact_thread = None
    if contacts:
        def _run_contacts():
            try:
                found.update(research_contacts(row, text, timeout=timeout))
            except Exception as exc:  # noqa: BLE001
                found["contacts"] = []
                found["contact_notes"] = f"contact research failed: {exc}"
        contact_thread = threading.Thread(target=_run_contacts)
        contact_thread.start()

    try:
        requirements = stages.stage_requirements(row, text)
        matches = stages.stage_matches(row, text, requirements, resume)
        written = stages.stage_brief(row, text, requirements, matches, resume)
        _atomic_write(raw_path(pid), json.dumps(
            {"stage_a": requirements, "stage_b": matches}, indent=1, ensure_ascii=False))
    except Exception as exc:  # noqa: BLE001 -- every failure is the page's to report
        if contact_thread:
            contact_thread.join(timeout=timeout)
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

    if contact_thread:
        contact_thread.join(timeout=timeout)
        payload["contacts"] = found["contacts"]
        payload["contact_notes"] = found["contact_notes"]
        payload["contacts_source"] = "direct tools"

    payload["status"] = "ok"
    payload["description_source"] = source
    payload["elapsed_s"] = round(time.monotonic() - started, 1)
    payload["company"] = row.get("company")
    payload["title"] = row.get("title")
    payload["url"] = row.get("url")
    write_result(pid, payload)
    return payload


def generate(pid, engine="deepseek", timeout=DEFAULT_TIMEOUT, runner=subprocess.run,
             contacts=True):
    """Dispatch to the direct DeepSeek path or the Hermes agent path."""
    if engine == "hermes":
        return generate_agent(pid, timeout=timeout, runner=runner)
    return generate_direct(pid, timeout=timeout, contacts=contacts)


def generate_agent(pid, timeout=DEFAULT_TIMEOUT, runner=subprocess.run):
    """Run the agent and write the brief. Returns the payload it wrote."""
    row = find_posting(pid)
    if not row:
        payload = {"status": "failed", "error": f"no posting {pid} in the portal feed or the last scan"}
        write_result(pid, payload)
        return payload

    text, source = description_for(row)
    prompt = build_prompt(row, text)
    os.makedirs(BRIEFS, exist_ok=True)
    try:
        os.unlink(agent_path(pid))
    except FileNotFoundError:
        pass
    _atomic_write(running_path(pid), json.dumps(
        {"pid": os.getpid(), "started_at": now_iso(), "posting_id": str(pid),
         "description_source": source}, indent=1))

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
                   "description_source": source, "elapsed_s": round(time.monotonic() - started, 1)}
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

    # Second pass. A critic sees only the finished brief, has nothing to research, and is asked
    # to delete everything that would not change what the candidate does next -- the fix for
    # "its a lot of info and not all of it is valuable".
    reviewed, why = review_brief(row, raw, runner=runner)
    if reviewed:
        # Keep the researcher's output for the record: the edit is reversible, the research is not.
        _atomic_write(raw_path(pid), json.dumps(raw, indent=1, ensure_ascii=False))
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


def build_contact_agent_prompt(row, text):
    """The Hermes agent prompt for the deep contact search: the contact rules, the posting, and
    an instruction to write the result JSON to the agent path."""
    rules = stages.read_prompt(CONTACT_PROMPT_PATH)
    body = (text or "(not supplied -- fetch the url above and read it before writing)")[:MAX_POSTING_TEXT]
    return (rules
            + "\n\n## The posting\n\n"
            + f"- company: {row.get('company')}\n"
            + f"- title: {row.get('title')}\n"
            + f"- location: {row.get('location')}\n"
            + f"- url: {row.get('url')}\n\n"
            + "## Posting text\n\n```\n" + body + "\n```\n\n"
            + "## Output\n\nDo the research with your tools, then write ONE JSON object (the shape "
            + "shown above) to exactly this path, creating no other file:\n\n"
            + agent_path(row.get("id")) + "\n\n"
            + "Write valid JSON only -- no markdown fence, no prose. Do not send any email, do "
            + "not apply anywhere, and do not write to LinkedIn.")


def deep_contacts(pid, timeout=DEFAULT_TIMEOUT, runner=subprocess.run):
    """Second-level contact search: the Hermes agent reads the web, not the direct tool loop.

    The default brief runs the copied contact loop (`research_contacts`). This runs the older
    agent path for the cases that loop misses, and merges what it finds into the existing brief
    without touching the resume-tailoring advice. Writes the same running marker, so the client
    polls one flag for both kinds of run.
    """
    row = find_posting(pid)
    if not row:
        payload = {"status": "failed", "error": f"no posting {pid} in the portal feed or the last scan"}
        write_result(pid, payload)
        return payload
    text, source = description_for(row)
    os.makedirs(BRIEFS, exist_ok=True)
    try:
        os.unlink(agent_path(pid))
    except FileNotFoundError:
        pass
    _atomic_write(running_path(pid), json.dumps(
        {"pid": os.getpid(), "started_at": now_iso(), "posting_id": str(pid),
         "mode": "deep-contacts", "description_source": source}, indent=1))
    started = time.monotonic()
    binary = hermes_bin()
    if not os.path.exists(binary):
        data = _read_json(final_path(pid), {}) or {}
        data.setdefault("status", "ok")
        data["contact_notes"] = f"Deep contact search unavailable: no hermes binary at {binary}."
        write_result(pid, data)
        return data
    try:
        runner([binary, "-z", build_contact_agent_prompt(row, text), "--in", ROOT],
               capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        data = _read_json(final_path(pid), {}) or {}
        data.setdefault("status", "ok")
        data["contact_notes"] = f"Deep contact search did not finish in {timeout}s."
        write_result(pid, data)
        return data
    raw = _read_json(agent_path(pid), {}) or {}
    contacts = [c for c in (raw.get("contacts") or []) if isinstance(c, dict)]
    notes = str(raw.get("contact_notes") or "").strip()
    data = _read_json(final_path(pid), {}) or {}
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


def main(argv=None):
    ap = argparse.ArgumentParser(description="Generate one posting brief on demand.")
    ap.add_argument("posting_id", nargs="?")
    ap.add_argument("--timeout", type=int, default=DEFAULT_TIMEOUT)
    ap.add_argument("--print", dest="show", action="store_true",
                    help="print the cached brief instead of generating")
    ap.add_argument("--prompt", action="store_true",
                    help="print the prompt that would be sent, and generate nothing")
    ap.add_argument("--list", action="store_true", help="list the cached briefs")
    ap.add_argument("--engine", choices=("deepseek", "hermes"), default="deepseek",
                    help="deepseek: direct API calls plus the contact tool loop (default); "
                         "hermes: the older agent path")
    ap.add_argument("--no-contacts", action="store_true",
                    help="direct engine: skip the contact-research tool loop")
    ap.add_argument("--deep-contacts", action="store_true",
                    help="run only the Hermes agent contact search and merge it into the brief")
    args = ap.parse_args(argv)

    if args.list:
        if not os.path.isdir(BRIEFS):
            print("no briefs yet")
            return 0
        for name in sorted(os.listdir(BRIEFS)):
            if not name.endswith(".json") or name.endswith(".agent.json"):
                continue
            data = _read_json(os.path.join(BRIEFS, name), {}) or {}
            print("%-18s %-9s %-34s %s" % (data.get("posting_id") or name[:-5],
                                           data.get("status"), (data.get("company") or "")[:34],
                                           data.get("generated_at") or ""))
        return 0

    if not args.posting_id:
        ap.error("a posting id is required unless --list is used")

    if args.show:
        data = _read_json(final_path(args.posting_id))
        if not data:
            print("no cached brief for", args.posting_id)
            return 1
        print(json.dumps(data, indent=1, ensure_ascii=False))
        return 0

    if args.prompt:
        row = find_posting(args.posting_id)
        if not row:
            print("no such posting:", args.posting_id)
            return 1
        text, source = description_for(row)
        if args.engine == "deepseek":
            print(f"# description: {source}\n# engine: deepseek (three-stage chain)\n")
            names = ("stage A: requirements", "stage B: matches", "stage C: brief")
            for name, message in zip(names, direct_messages(row, text)):
                print(f"===== {name}: system =====\n{message['content']}\n")
                if message.get("user"):
                    print(f"===== {name}: user =====\n{message['user']}\n")
        else:
            print(f"# description: {source}\n# engine: hermes\n")
            print(build_prompt(row, text))
        return 0

    if args.deep_contacts:
        payload = deep_contacts(args.posting_id, timeout=args.timeout)
        print(json.dumps({k: payload.get(k) for k in
                          ("status", "company", "title", "deep_contacts_at", "elapsed_s", "error")},
                         indent=1))
        print("contacts   :", len(payload.get("contacts") or []))
        return 0 if payload.get("status") == "ok" else 1

    payload = generate(args.posting_id, engine=args.engine, timeout=args.timeout,
                       contacts=not args.no_contacts)
    print(json.dumps({k: payload.get(k) for k in
                      ("status", "company", "title", "description_source", "elapsed_s", "error")},
                     indent=1))
    if payload.get("status") == "ok":
        print("\nkey_skills :", ", ".join(payload.get("key_skills") or []) or "(none)")
        print("contacts   :", len(payload.get("contacts") or []))
        print("brief      :", final_path(args.posting_id))
        return 0
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
