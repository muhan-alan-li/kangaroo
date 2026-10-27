"""Prompt assembly, kept apart from the code that runs a prompt.

Every template lives in `data/config/`, so wording is edited there rather than in Python. This
module only fills one in and says which output path the agent must write to. There are four
shapes: the Hermes agent brief, its editorial review, the deep-contact search, and the printed
form of the three-stage chain (`--prompt`, for reviewing the chain without spending a call).
"""
from __future__ import annotations

import json
import os

from ..config import ROOT
from . import stages
from .posting import RESUME_PATH
from .state import agent_path, review_agent_path

PROMPT_PATH = os.path.join(ROOT, "data", "config", "brief_prompt.md")
REVIEW_PROMPT_PATH = os.path.join(ROOT, "data", "config", "brief_review_prompt.md")
CONTACT_PROMPT_PATH = os.path.join(ROOT, "data", "config", "contact_research_system.md")
SKILL_DIR = os.path.expanduser("~/.hermes/skills/productivity/job-application-run")


def _template(path):
    with open(path, encoding="utf-8") as fh:
        return fh.read()


def build_agent_prompt(row, text):
    """The Hermes agent prompt: posting facts, filters and the output path."""
    keys = ", ".join(row.get("keywords") or []) or "(none extracted)"
    have = ", ".join(row.get("skills_have") or []) or "(none)"
    gap = ", ".join(row.get("skills_gap") or []) or "(none)"
    body = text or "(not supplied -- fetch the url above and read it before writing the brief)"
    return (_template(PROMPT_PATH)
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
            .replace("{{POSTING_TEXT}}", body[:stages.MAX_POSTING_TEXT]))


def build_review_prompt(row, brief):
    """The critic's prompt: the brief to edit, and nothing else to research."""
    return (_template(REVIEW_PROMPT_PATH)
            .replace("{{OUTPUT_PATH}}", review_agent_path(row.get("id")))
            .replace("{{BRIEF_JSON}}", json.dumps(brief, indent=1, ensure_ascii=False))
            .replace("{{COMPANY}}", str(row.get("company") or ""))
            .replace("{{TITLE}}", str(row.get("title") or ""))
            .replace("{{LOCATION}}", str(row.get("location") or ""))
            .replace("{{TIER}}", str(row.get("tier") or "")))


def build_contact_prompt(row, text):
    """The Hermes agent prompt for the deep contact search.

    The contact rules come first, then the posting, then the one output path. The prompt also
    repeats the no-send rule: this is research, never an outbound message.
    """
    rules = _template(CONTACT_PROMPT_PATH)
    body = (text or "(not supplied -- fetch the url above and read it before writing)")[
        :stages.MAX_POSTING_TEXT]
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


def direct_messages(row, text):
    """The chain's three prompts as message lists, for `--prompt` inspection only.

    The stage B and C user messages carry placeholders instead of the previous stage's output,
    because printing the prompts must not spend a model call.
    """
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
