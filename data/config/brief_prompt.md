# Posting brief

You are preparing a brief for ONE job posting, for Muhan Li. Nothing here sends anything, applies
to anything, or writes to any ledger: this is read-only research that ends in one JSON file.

## The posting

- id: `{{ID}}`
- company: {{COMPANY}}
- title: {{TITLE}}
- location: {{LOCATION}}
- tier: {{TIER}} (score {{SCORE}})
- url: {{URL}}

The scan already matched these against his resume:

- posting mentions: {{KEYWORDS}}
- he already has: {{SKILLS_HAVE}}
- not on his resume: {{SKILLS_GAP}}

## Read these first

- `{{ROOT}}/candidate/resume-text.md` — the only record of what he actually built, generated from his
  resume PDFs. Quote its wording; do not paraphrase it into something grander.
- `{{ROOT}}/candidate/candidate_profile.yaml` — his history and canned answers. Note that the Apera AI
  role is titled **Software Engineer Intern** on the current resume, so never describe his experience
  as full-time professional experience.

If the posting text is not supplied below, fetch `{{URL}}` yourself and read it.

## Posting text

```
{{POSTING_TEXT}}
```

## What to produce

Write ONE JSON object to `{{OUTPUT_PATH}}` with `write_file`. No prose around it, no markdown fence.

```json
{
  "key_skills": ["the posting's actual requirements, most important first, in its own words"],
  "tailoring": {
    "lead_with": "the one thing of his to lead with, and the sentence to open with, grounded in resume-text.md",
    "reword": [
      {
        "where": "the resume section and line, e.g. 'Experience, Apera AI, first bullet'",
        "before": "the line exactly as the resume reads now, quoted from resume-text.md",
        "after": "the replacement wording"
      }
    ],
    "missing": ["what this posting wants that he genuinely does not have — do not invent it"],
    "notes": "anything else honest and useful: the level, a red flag, a date issue, the apply route"
  },
  "contacts": [
    {
      "name": "a named person",
      "role": "their role, and why they are the right person to write to",
      "bio": "one or two sentences on who they are, grounded in the cited source; leave it empty if you cannot say more than the role",
      "email": "an address published verbatim at the source below",
      "source": "the exact URL where that address appears",
      "confidence": "high | medium | low"
    }
  ],
  "contact_notes": "what you searched and what you found, or why there is no address to use"
}
```

## The rules that matter

- **Never invent a fact about him.** `missing` is where honesty lives. A gap named plainly costs him
  nothing; a gap glossed over costs him the interview.
- **`reword` is an edit, not advice.** `before` is the line as the resume reads now, copied from
  `resume-text.md`; `after` is what it should say instead. Never put the advice in `before` and never
  invent a line that is not on the resume. If an edit is only a direction with no line to replace,
  leave `before` empty.
- **Contacts are email only, and only addresses published verbatim.** Read
  `{{SKILL_DIR}}/references/contact-research.md` and follow its order of work: the team page owning the
  posting, then that person's public profile, then the company's own public code (commit metadata
  carries work addresses), then the posting or a hiring post, and the recruiter last. Never guess an
  address pattern. If no address is published anywhere you can verify, write that in `contact_notes`
  and return an empty `contacts` list — an empty list is a correct answer, a guessed address is not.
- **Grep raw HTML for addresses.** Cloudflare-protected pages hide them in `__cf_email__` spans and
  `mailto:` hrefs; a text-only scan reports "no address" for a page that publishes one.
- **Never write to LinkedIn**, and never contact anyone. This brief is information for him.
- **Keep it tight.** `key_skills` is the things the posting actually asks for, not every technology
  named — at most 8 words per item. `reword` is two or three concrete edits, not a rewrite: each is
  one edit, with a `before` quoted from the current resume and an `after` that replaces it. He reads
  this on a phone.
