"""The three-stage tailoring chain: extract requirements, match evidence, write the brief.

Replaces the old single mega-prompt, which asked one model to extract the requirements, find the
matching experience and rewrite the resume in one pass. Splitting it does two things: each stage
has a narrow, focused system prompt, and the matching stage's verdicts are written down as
explicit JSON in between, so a rewrite is anchored to a named requirement and a named resume
bullet instead of being bolted on top of a keyword-stuffed first impression.

    A  stage_requirements  -- posting text only; the resume is withheld so it cannot bias what
                             the posting asks for. Output: requirement groups with ids.
    B  stage_matches       -- resume first, then A's groups. One verdict per requirement:
                             met / transferable / missing, with the verbatim evidence bullet.
    C  stage_brief         -- resume and the matrix. Writes the finished brief object, which
                             keeps the shape the dashboard already renders.

Every call goes through `llm.chat_json`: same DeepSeek endpoint, retries, and fence-tolerant
parsing as the single call it replaced. The user messages put the stable content (system prompt,
resume) before the per-posting text so DeepSeek's automatic prefix caching gets its best shot.

Each stage is one chain link; nothing is cached here (the chain runs once per posting, by
design), and no stage sees more than it needs.
"""
from __future__ import annotations

import json
import os

from ..config import ROOT
from . import llm

MAX_POSTING_TEXT = 12000       # shared bound with generate.py's old cap; keep them in step
STAGE_A_PROMPT = os.path.join(ROOT, "data", "config", "brief_stage_a_system.md")
STAGE_B_PROMPT = os.path.join(ROOT, "data", "config", "brief_stage_b_system.md")
STAGE_C_PROMPT = os.path.join(ROOT, "data", "config", "brief_stage_c_system.md")


def read_prompt(path):
    try:
        with open(path, encoding="utf-8") as fh:
            return fh.read()
    except OSError:
        return ""


def cap_text(text):
    return (text or "(no posting text was available; say so in notes rather than guessing)")[:MAX_POSTING_TEXT]


def stage_requirements(row, text):
    """Stage A: the posting's requirement contract, from the posting text alone."""
    system = read_prompt(STAGE_A_PROMPT)
    user = (
        "# The posting\n\n"
        f"- title: {row.get('title')}\n"
        f"- company: {row.get('company')}\n"
        f"- location: {row.get('location')}\n\n"
        "## Posting text\n\n```\n" + cap_text(text) + "\n```"
    )
    return llm.chat_json([{"role": "system", "content": system},
                          {"role": "user", "content": user}])


def stage_matches(row, text, requirements, resume_text):
    """Stage B: one `met|transferable|missing` verdict per requirement, evidence verbatim.

    The resume comes first in the user message -- it is stable across postings -- and the
    requirement contract second, so the model reads the evidence before the claims it must test.
    """
    system = read_prompt(STAGE_B_PROMPT)
    user = (
        "# Muhan's resume (the only record of what he actually did)\n\n" + (resume_text or "(unavailable)") +
        "\n\n# The requirement contract extracted from this posting\n\n```json\n"
        + json.dumps(requirements, indent=1, ensure_ascii=False)
        + "\n```\n\n"
        "For every requirement group in that contract, produce one match entry. Do not skip any."
    )
    return llm.chat_json([{"role": "system", "content": system},
                          {"role": "user", "content": user}])


def stage_brief(row, text, requirements, matches, resume_text):
    """Stage C: the finished brief object, written against the match matrix."""
    system = read_prompt(STAGE_C_PROMPT)
    user = (
        "# Muhan's resume (the only record of what he actually did)\n\n" + (resume_text or "(unavailable)") +
        "\n\n# The match matrix\n\n```json\n"
        + json.dumps(matches, indent=1, ensure_ascii=False)
        + "\n```\n\n"
        "# The posting, for the verdict's context\n\n"
        f"- title: {row.get('title')}\n"
        f"- company: {row.get('company')}\n"
        f"- location: {row.get('location')}\n"
        f"- url: {row.get('url')}\n\n"
        "## Posting text\n\n```\n" + cap_text(text) + "\n```\n\n"
        "Write the brief. Remember: edits ranked by how much they change the match, at most 3, "
        "`before` verbatim from the resume, and no skill invented."
    )
    return llm.chat_json([{"role": "system", "content": system},
                          {"role": "user", "content": user}])
