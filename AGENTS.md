# Kangaroo

Job search workspace for junior and new-grad software engineering roles in Vancouver, BC.
Owner: Muhan Li. Agent: Hermes.

This file is the map: what the files are, how a day runs, and the rules that always apply.
The full procedure is in the `job-application-run` skill.

The app is read-only about applications. It marks a posting applied. It never fills a form,
submits, signs in, or sends mail. The candidate applies from the posting's own link.

**This branch is `rust-app`.** It adds a native macOS app under `crates/`. Read `REPORT.md`
for its state. The Python server, the Preact client, the container and the systemd units live
under `archive/`; they are out of the build here and still run production on `main`.

## Layout

| Path | Contents |
|---|---|
| `data/candidate/` | Source of truth for every application field: `candidate_profile.yaml` (resume plus `answers_bank`), `resume-text.md`, `resume/`, `documents/`, `outreach-voice/`, `outreach-samples/` |
| `data/config/` | `sources.yaml`, `filters.yaml`, `ats_boards.yaml`, `workday_boards.yaml`, `json_boards.yaml`, `icims_boards.yaml`, `workday_tombstones.yaml`, `source_candidates.md`, and the prompt templates |
| `data/state/` | Gitignored. The Rust app keeps `kangaroo.db` (postings, the `seen` ledger, runs, descriptions, board health, briefs), `.scan.lock`, and the archived Python files: `last_scan.json`, `portal_feed.jsonl`, `descriptions.json`, `seen.jsonl`, `board_health.json`, `workday_sweep.json`, `sg_boards.json`, `applied.json`, `briefs/`, `scan.log`, `retry.log` |
| `archive/` | The Python workspace, kept for reference: `server/` (API, scan, brief, jobs, tests), `client/` (Preact dashboard), `docker/` (scan and retry image), `systemd/` (the two user timers). Nothing on this branch reads it |
| `crates/` | The Rust app: `kangaroo-core`, `kangaroo-db`, `kangaroo-protocol`, `kangaroo-scan`, `kangaroo-engine`, `kangaroo-brief`, `kangaroo-cli`, `kangaroo-server` (the menu-bar agent that owns the engine and SQLite), `kangaroo-app` (the GPUI client) |
| `assets/` | The kangaroo mark: `make-icons.py` draws it, and the two PNGs are the menu-bar glyph and the dock tile |

## How a day runs

1. **Scan, Monday to Friday at 09:00, 13:00 and 17:00.** `kangaroo-server` owns the scheduler and
   runs the scan: sweep if stale, scan five per tier, then mark reported. Each slot takes an
   exclusive `flock` on `data/state/.scan.lock`, so a manual run and a scheduled run never
   overlap. A missed slot waits for the next one.
2. **Retry and prune, by hand.** `kangaroo retry` re-checks only the Workday boards whose backoff
   has come up, then `kangaroo prune` after a clean run. Prune removes boards that are structurally
   dead, that is a Workday 404 or 422 from a wrong career-site name. Prune appends each removed key to
   `workday_tombstones.yaml`, so the next harvest does not add it again. Delete a tombstone line
   by hand to let a board back in.
3. **Apply, by the candidate.** The agent reads the posting, rules on the fit, and returns the
   answers and the gaps. It does not submit.

There is no agent in the scan loop. A scan cannot score postings or draft text. The app shows
the deterministic keyword score from `kangaroo-core`.

## Sources

Seven sources are on in `data/config/sources.yaml`: `jobbank`, `newgrad_list`, `newgrad_json`,
`ats_boards`, `startups_gallery`, `workatastartup`, and `linkedin`.

- **Age comes from the publish field, never `updated_at`.** A board touches `updated_at` at
  every rescore, so an old posting can read as fresh. Use Job Bank's posted date, Greenhouse
  `first_published`, Lever `createdAt`, or Ashby `publishedAt`. `max_age_days` is 7.
- **`newgrad_list` reads a derived file, so it cannot see a close.** The Simplify repo builds
  its `README.md` from `listings.json` and moves closed postings into collapsed blocks that
  still hold a full table. `newgrad_json` carries `active`, `is_visible` and `date_posted`, so it
  is authoritative. Keep both, and collapse the overlap once, at the merge point, by the ATS
  signature of the apply URL.
- **Never filter the new-grad feed on `category`.** The strings are inconsistent, so an
  allow-list drops roles without a message.
- **Workday is the largest source.** It lives in `workday_boards.yaml` as tenant, `wd` number
  and career site. The scan reads the first page only, because the board API is date-sorted and
  carries a real publish field. The source applies its own basic filter first: engineering
  titles, Canada or US, posted within 7 days.
- **Seven JSON vendors share one shape** in `json_boards.yaml`: Workable, SmartRecruiters,
  Rippling, BambooHR, Recruitee, Breezy and Teamtailor. Rippling and BambooHR publish no date
  anywhere, so their rows carry `age_days: None` and `date_missing: True`. Never present such a
  row as fresh. Each board leaves a health row keyed `vendor|slug`.
- **A vendor's country field beats the location text.** SmartRecruiters returns a bare ISO
  code, and those codes collide with US state abbreviations. Use the structured field.
- **Workday throttles by IP.** Keep the worker pool at 16. Retry a failed board page three
  times, and print the count of boards that still failed.
- **`startups_gallery` reveals employer boards.** Each card deep-links to the employer ATS, so
  the scan harvests the slug into `sg_boards.json`. Run `probe-ats` to add those boards to
  `ats_boards.yaml`. The probe drops a board only after two consecutive failures.
- **A level-II title is gated, not excluded.** Report one only with a strong, named fit, that is
  the two or three main requirements and the past work that meets each. Otherwise write one line
  under "Left out". Level III and L4 sit above level II. A range that includes level I counts as
  level I.
- **Location tiers set the order:** vancouver, seattle, toronto, sf_bay_area, remote_canada,
  remote (US or anywhere), rest_of_canada. A Canada posting that matches no metro and is not
  remote lands in rest_of_canada; anything else unmatched is dropped. Keep a US posting and flag
  it. `--per-tier N` caps each tier in the printed list only. The scheduled scan uses 5.

## Commands

Do not run the archived Python workspace from this branch. `make` targets build the Rust app.

`kangaroo-server` is a menu-bar agent. It owns the engine, `data/state/kangaroo.db` and the
scheduler. Its status item shows the kangaroo mark, not a word.
`kangaroo-app` is the GPUI window; it talks to the server over `data/state/kangaroo.sock` and
never opens the database. The app starts a sibling server when none answers. The backend port has
every source except iCIMS, the scan pipeline, board health, sweep, retry, prune, probe, harvest and
the brief engine. Read `REPORT.md` for the state.

| Command | Does |
|---|---|
| `make run-server` | Run the menu-bar server in the foreground |
| `make run` | Build both binaries and launch the desktop app |
| `target/debug/kangaroo scan --db FILE [--source ID] [--json]` | Run one scan |
| `target/debug/kangaroo sweep --db FILE [--limit N] [--dry-run]` | Sweep Workday into the cache |
| `target/debug/kangaroo retry --db FILE [--force] [--dry-run]` | Re-check the boards that failed |
| `target/debug/kangaroo prune --db FILE [--dry-run]` | Remove structurally dead Workday boards |
| `target/debug/kangaroo probe-ats --db FILE [--report]` | Probe ATS slugs, rewrite `ats_boards.yaml` |
| `target/debug/kangaroo harvest workday\|jsonboards --db FILE [--report]` | Rebuild a board list from the feed |
| `target/debug/kangaroo import --from DIR --db FILE` | Import a Python state directory |

`--source` takes an id from `sources.yaml`: `jobbank`, `newgrad_list`, `newgrad_json`,
`ats_boards`, `startups_gallery`, `workatastartup`, `linkedin`, `remotive`. A wrong id matches
nothing and exits clean, which looks like a quiet day. `--source ats` is wrong and
`--source ats_boards` is right.

## Dashboard

`kangaroo-app` is the desktop window. It holds no state and opens no database. Each window asks
`kangaroo-server` for a `Snapshot` after it connects, then applies the events that follow, so two
windows agree. The server broadcasts every event to every client.

The window is a pipeline in three stages, in the navigation column, grouped under Pipeline with
the count at the end of each row. A posting with a brief leaves Open for Shortlisted. An applied
posting moves to Applied. No row is in two tabs. A failed brief stays in Open, so the next press
retries it.

The title bar is drawn by the app: the wordmark, the current section, the scan state and the
scan button. AppKit keeps only the traffic lights, placed at the left of that bar.

The table draws date, location, company and title. A row click opens the detail panel on the
right, and `Esc` closes it. The panel asks for `PostingDetail`, which carries the cached
text, the salary and keyword fields, and the brief. In the Open tab a column of ticks marks rows
for one batch brief.

The page size follows the window height, not a fixed count: the app divides the space left by
the chrome by the row height, so the table never ends in blank space.

The window is Slate: `#F7F7F7` page, `#FFFFFF` card, `#E4E4E4` hairline, `#171717` text,
`#6B6B6B` muted text, `#3F7D73` teal for actions, `#B4793A` amber as the second voice.
`crates/kangaroo-app/src/theme.rs` holds those six colours twice — once for the light page, once
for the dark — and mixes every other token from two of them. The dark tint lifts the teal and
makes the accent's label near-black, because both would otherwise lose contrast.

The tint is chosen in Settings under Appearance. The choice is saved through `SaveSettings`, and
the dashboard snapshot carries the stored settings back, so the next launch opens in the same
tint; `--tint light|dark` overrides it for one run.

The primary accent marks only what the reader can press and where they are: the primary buttons,
the active navigation row's mark, icon and count, a ticked checkbox, a selected row's border, an
empty state's icon, and focus. Headings take the text colour. The second accent is spent on the
Shortlist row and the brief it holds. Status colours carry meaning, so each tint states them
instead of deriving them.

## Rules

- Never submit an application without explicit approval for that batch.
- Never invent a fact about the candidate. A missing answer becomes a question, marked UNKNOWN.
- Never answer a voluntary self-identification survey unless the candidate asks. Record his
  words exactly when he does.
- Never defeat a bot check. Hand the CAPTCHA or the code to the candidate.
- Never type a password. Account creation and sign-in belong to the candidate.
- Never automate LinkedIn or Indeed as the candidate. The scan may read the public guest list.
- For US and out-of-town applications use the confirmed values: work authorization Yes,
  TN-eligible, no sponsorship needed, willing to relocate Yes. Outside Metro Vancouver use
  `located_in_listed_city_other`.
- Outreach is complete. The candidate said: "i've completed outreach and any drafts will remain
  unsent". Write no draft and send nothing unless he asks for one message.
- Outreach is email only. Use only an address that the posting, the employer board, or a public
  source gives verbatim. Never guess a pattern. Keep the volume cap.
- Log every outbound message in `data/state/outreach.jsonl`.
- Look for new sources, and adopt each one for the long term. Adoption takes four steps: a
  module in `crates/kangaroo-scan/src/sources/`, a row in `sources::build`, a block in
  `sources.yaml`, and a row in `source_candidates.md`. If the source reveals employer boards, run
  `probe-ats` too. A feed that hands over boards is half-used until that probe runs.
