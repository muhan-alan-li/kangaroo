#!/usr/bin/env python3
"""Generate the per-posting brief that the dashboard's detail view shows.

On demand only. Nothing here runs during a scan: the two things it produces -- resume-tailoring
advice and, when asked, reach-out targets -- need reasoning about the posting, and the candidate
asked for that to happen when a posting is opened rather than preloaded for every row.

The package is split by what each part has to touch:

    stages.py    the three prompts and their single model calls (A requirements, B matches,
                 C brief)
    chain.py     the default engine: runs A -> B -> C and enforces validate.py's edit rules
    validate.py  mechanical factuality: a `before` must be in the resume; an `after` may not
                 invent a term found nowhere
    agent.py     the older one-shot Hermes agent and its editorial review
    contacts.py  the on-demand deep contact search, merged into an existing brief
    prompts.py   prompt assembly; all wording lives in data/config/*.md
    posting.py   finding the portal row and its text
    state.py     where the four brief files live and how they are written
    llm.py       the minimal DeepSeek chat client

The default engine is the chain (three reasoning calls, no agent, no session). `--engine hermes`
selects the older agent for work that needs the web. `--deep-contacts` runs only the agent's
contact search. Neither engine ever sends, applies or writes a ledger.

    /usr/bin/python3 -m server brief <posting-id>                   # direct DeepSeek (default)
    /usr/bin/python3 -m server brief <posting-id> --engine hermes   # agent + contact research
    /usr/bin/python3 -m server brief <posting-id> --print           # show the cached brief
    /usr/bin/python3 -m server brief --list                         # what is cached
    /usr/bin/python3 -m server brief <posting-id> --prompt          # print the prompts

Writes `data/state/briefs/<id>.json`. A failure is written too, with the reason, so the dashboard
can say what went wrong instead of spinning forever.
"""
from __future__ import annotations

import argparse
import json
import os
import subprocess

from ..config import BRIEFS
from . import agent, chain, contacts, prompts
from .agent import DEFAULT_TIMEOUT
from .posting import description_for, find_posting
from .state import final_path, read_json

# Generated alongside the result; `--list` shows only the results.
_ARTIFACT_SUFFIXES = (".agent.json", ".raw.json", ".review.json")


def generate(pid, engine="deepseek", timeout=DEFAULT_TIMEOUT, runner=subprocess.run):
    """Dispatch to the default chain or the Hermes agent path."""
    if engine == "hermes":
        return agent.generate(pid, timeout=timeout, runner=runner)
    return chain.generate(pid)


def _list_briefs():
    if not os.path.isdir(BRIEFS):
        print("no briefs yet")
        return 0
    for name in sorted(os.listdir(BRIEFS)):
        if not name.endswith(".json") or name.endswith(_ARTIFACT_SUFFIXES):
            continue
        data = read_json(os.path.join(BRIEFS, name), {}) or {}
        print("%-18s %-9s %-34s %s" % (data.get("posting_id") or name[:-5],
                                       data.get("status"), (data.get("company") or "")[:34],
                                       data.get("generated_at") or ""))
    return 0


def _print_prompts(pid, engine):
    row = find_posting(pid)
    if not row:
        print("no such posting:", pid)
        return 1
    text, source = description_for(row)
    if engine == "deepseek":
        print(f"# description: {source}\n# engine: deepseek (three-stage chain)\n")
        names = ("stage A: requirements", "stage B: matches", "stage C: brief")
        for name, message in zip(names, prompts.direct_messages(row, text)):
            print(f"===== {name}: system =====\n{message['content']}\n")
            if message.get("user"):
                print(f"===== {name}: user =====\n{message['user']}\n")
    else:
        print(f"# description: {source}\n# engine: hermes\n")
        print(prompts.build_agent_prompt(row, text))
    return 0


def main(argv=None):
    ap = argparse.ArgumentParser(description="Generate one posting brief on demand.")
    ap.add_argument("posting_id", nargs="?")
    ap.add_argument("--timeout", type=int, default=DEFAULT_TIMEOUT)
    ap.add_argument("--print", dest="show", action="store_true",
                    help="print the cached brief instead of generating")
    ap.add_argument("--prompt", action="store_true",
                    help="print the prompts that would be sent, and generate nothing")
    ap.add_argument("--list", action="store_true", help="list the cached briefs")
    ap.add_argument("--engine", choices=("deepseek", "hermes"), default="deepseek",
                    help="deepseek: direct API calls (default); "
                         "hermes: the older agent path")
    ap.add_argument("--deep-contacts", action="store_true",
                    help="run only the Hermes agent contact search and merge it into the brief")
    args = ap.parse_args(argv)

    if args.list:
        return _list_briefs()

    if not args.posting_id:
        ap.error("a posting id is required unless --list is used")

    if args.show:
        data = read_json(final_path(args.posting_id))
        if not data:
            print("no cached brief for", args.posting_id)
            return 1
        print(json.dumps(data, indent=1, ensure_ascii=False))
        return 0

    if args.prompt:
        return _print_prompts(args.posting_id, args.engine)

    if args.deep_contacts:
        payload = contacts.generate(args.posting_id, timeout=args.timeout)
        print(json.dumps({k: payload.get(k) for k in
                          ("status", "company", "title", "deep_contacts_at", "elapsed_s", "error")},
                         indent=1))
        print("contacts   :", len(payload.get("contacts") or []))
        return 0 if payload.get("status") == "ok" else 1

    payload = generate(args.posting_id, engine=args.engine, timeout=args.timeout)
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
