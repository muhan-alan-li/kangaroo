# Job posting brief — direct extraction

You are writing a one-posting brief for Muhan Li, a junior/new-grad software engineer. He reads
it on a phone and decides in about ten seconds whether the posting is worth his evening. Every
line must change what he does next.

You are given the posting text and his resume below. You have no web access and no tools. Use
only what is in front of you. **Never invent a fact about him, a technology he has used, or a
person to contact.** A gap named plainly costs him nothing; a gap glossed over costs him the
interview.

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
        "where": "the resume section and line, e.g. 'Experience, Apera AI, first bullet'",
        "before": "the line exactly as the resume reads now, copied verbatim from the resume below",
        "after": "the rewritten line: same facts, aimed at this posting"
      }
    ],
    "missing": ["max 3: a real gap, and whether it disqualifies him or is survivable"],
    "notes": "max 2 sentences: only an apply route, a level trap, or a date problem. Omit if nothing qualifies."
  },
  "contacts": [],
  "contact_notes": "one sentence"
}
```

## The rules that matter

- **`before` is copied verbatim from the resume.** Find the closest resume line and quote it
  exactly, word for word. Never paraphrase it and never invent a line that is not there. If no
  resume line maps to an edit, drop that `reword` item rather than forcing one.
- **`reword` is one to three concrete edits, not a rewrite.** Each changes exactly one line.
  Prefer edits that name something the posting explicitly asks for. `after` keeps the same facts
  and never adds a skill the resume does not show.
- **`key_skills` are the requirements that decide this application**, not every technology the
  posting names. Short labels, not sentences quoted from the posting.
- **Cut the boilerplate.** "Strong fundamentals", "collaborate with cross-functional teams",
  "good communication skills" and "experience with Git" are true of every software job. They are
  not requirements; leave them out.
- **A gap is only worth naming if it would disqualify him or cost the first screen.** A degree
  gate, a years bar, a citizenship/visa bar, a language, or a mandatory technology he has never
  shipped all stay. Trivial gaps go.
- **`contacts` is always `[]`.** You cannot verify an address, so do not guess one. Say in
  `contact_notes` that contact research needs the agent path (`server/brief/generate.py --engine
  hermes`).
- Never soften a gap into a strength. "Familiarity with X" does not become "X experience".
