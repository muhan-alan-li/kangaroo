"""Mechanical factuality checks on the brief's `tailoring.reword` edits.

The single-call generator held these rules entirely in the prompt, and a prompt failure read as
a valid brief on the page. The chain makes the rules checkable, in code, before anything is
shown:

- `before` must be in the resume. A `before` line that is not verbatim present in the resume text
  is either a paraphrase or an invention, and both are disqualifying: the edit is dropped and the
  reason recorded.
- `after` must not invent. Every notable term it names (an acronym, a capitalized product word, a
  token with a digit) must already appear in the resume or in the posting text. A new technology
  in `after` that exists nowhere else is flagged, never silently kept.

Nothing here is heuristic about the *quality* of a rewrite — only about whether it could be true.
"""
from __future__ import annotations

import re

# Terms a rewrite could use to fake experience the evidence does not show. A token counts as
# "notable" when it is distinctive: an all-caps acronym, a capitalized product-suspect word, or
# anything carrying a version digit. Plain lowercase prose is never flagged.
# Kept deliberately narrow: this is a tripwire for invention, not a spellchecker.
_TOKEN = re.compile(r"[A-Za-z][A-Za-z0-9.+#]*")


def _normalize(text):
    """One comparable form: zero-width chars out, whitespace collapsed, lowercased, unicode dash
    forms folded together — the resume's PDF text carries invisible separators."""
    text = (text or "").replace("\u200b", "").replace("\ufeff", "")
    text = text.replace("\u2013", "-").replace("\u2014", "-")
    return re.sub(r"\s+", " ", text).strip().lower()


def _loose(text):
    """A whitespace/punctuation-normalized form for the line-wrap fallback match."""
    return re.sub(r"\s+", " ", re.sub(r"[^\w\s+#/.-]", " ", _normalize(text))).strip()


def before_in_resume(before, resume_text):
    """True when `before` appears verbatim (modulo whitespace and dash style) in the resume."""
    if not before:
        return False
    needle = _normalize(before)
    if not needle:
        return False
    hay = _normalize(resume_text)
    if needle in hay:
        return True
    # The bullet may be line-wrapped around the PDF's column layout; also try the token
    # sequence with the punctuation that varies between renders (bullets, ellipses) stripped.
    return _loose(needle) in _loose(hay)


def _notable(content):
    """The distinctive terms in a block of text, order preserved, deduped case-insensitively."""
    out, seen = [], set()
    for token in _TOKEN.findall(content or ""):
        low = token.lower().strip(".#")
        if len(low) < 2 or low in seen:
            continue
        # An ordinary word capitalized only because it starts the line is not a product name.
        first_word = len(out) == 0
        if token.isupper() or any(c.isdigit() for c in token) or (token[:1].isupper() and not first_word):
            seen.add(low)
            out.append(token)
    return out


def invented_terms(after, resume_text, posting_text):
    """Terms `after` introduces that appear neither in the resume nor in the posting text."""
    corpus = _normalize(resume_text) + "\n" + _normalize(posting_text)
    return [t for t in _notable(after) if _normalize(t) not in corpus]


def check_rewords(rewords, resume_text, posting_text):
    """Split `reword` into the ones that may be shown and the reasons the rest were dropped.

    Returns (kept, dropped, flags):
      kept    -- the surviving edits, each annotated with "validated": True
      dropped -- [{where, reason}] for every edit removed
      flags   -- ["term invented: X", ...] for vandalism-adjacent wording that was kept anyway
                 (the term might be a legitimate restatement of something in the posting)
    """
    kept, dropped, flags = [], [], []
    for item in rewords or []:
        before = str(item.get("before") or "")
        after = str(item.get("after") or "")
        where = str(item.get("where") or "").strip()
        if not after and not before:
            continue                                    # advice-only legacy row: leave it alone
        if not before_in_resume(before, resume_text):
            dropped.append({"where": where, "reason": "before is not in the resume"})
            continue
        bad = invented_terms(after, resume_text, posting_text)
        if bad:
            flags.append(f"{where or 'reword'}: after names {', '.join(bad)} (not in resume or posting)")
        kept.append({**item, "validated": True})
    return kept, dropped, flags
