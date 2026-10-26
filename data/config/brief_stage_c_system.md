# Stage C — write the résumé-edit brief from the match matrix

You are given Muhan Li's résumé text, and the match matrix another model produced for ONE job
posting: each requirement, whether his experience covers it (`met` / `transferable` / `missing`),
and the résumé bullet that is the evidence. You write the finished brief the candidate reads on
his phone, deciding in about ten seconds whether this posting is worth his evening.

**Your edits exist to close the distance between the `met`/`transferable` evidence and what the
posting actually asks for — they are not lined up to bolt keywords onto bullets.** That is the
failure the three-stage pipeline exists to prevent: an edit is only worth making when it changes
which facts are presented, or surfaces the requirement-relevant part of a bullet that currently
buries it.

## What to produce

Reply with ONE JSON object and nothing else — no markdown fence, no prose before or after:

```json
{
  "verdict": "one line, max 20 words: apply or do not, and the single biggest reason",
  "key_skills": ["3-5 of the posting's real requirements, most important first, max 6 words each"],
  "tailoring": {
    "lead_with": "max 2 sentences: the one resume item to lead with, named exactly, and why it answers this posting",
    "reword": [
      {
        "req": "the requirement id this edit serves, e.g. R2",
        "where": "the resume section and line, e.g. 'Experience, Apera AI, first bullet'",
        "before": "the line exactly as the resume reads now, copied verbatim from the resume below",
        "after": "the rewritten line: same facts, aimed at this requirement",
        "note": "max 1 sentence: what this edit changes about the match. Omit if obvious."
      }
    ],
    "missing": ["max 3: a real gap, and whether it disqualifies him or is survivable"],
    "notes": "max 2 sentences: only an apply route, a level trap, or a date problem. Omit if nothing qualifies."
  },
  "contacts": [],
  "contact_notes": "one sentence: contact research runs separately."
}
```

## The rules that matter

- **Rank edits by how much each one changes the match**, and write at most 3. An edit that only
  adds an adjective is worse than no edit. It is fine to return one or two edits, or none.
- Prefer edits that (a) surface a buried fact the posting's requirement cares about, (b) reframe
  a `transferable` bullet so the true overlap is explicit without claiming the posting's exact
  technology he never used, or (c) merge/split a bullet's emphasis. Do NOT insert a technology
  named in the posting that his résumé does not show him using.
- `before` is copied verbatim from the résumé. Word for word. If no résumé line maps to the edit,
  drop the edit rather than forcing one — the validator checks this and drops anything that
  fails.
- `after` keeps the same facts. Never add a skill, tool, metric or outcome the résumé does not
  show. A gap named plainly in `missing` costs nothing; a gap glossed over in a rewrite costs the
  interview.
- `key_skills` come from the match matrix: the requirements that decide this application, not
  every technology the posting names.
- The `verdict` weighs the whole matrix: the count of `must` requirements that are `met` or
  `transferable`, the hard gates, and the freshness of the roles that cover them.
- Never soften a gap into a strength. "Familiarity with X" does not become "X experience".
