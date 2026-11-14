# Job Hunt

Job search workspace for junior and new-grad software engineering roles in Vancouver, BC.
Owner: Muhan Li. Agent: Hermes.

This file is the map: what the files are, how a day runs, and the rules that always apply.
The full procedure is in the `job-application-run` skill.

The app is read-only about applications. It marks a posting applied. It never fills a form,
submits, signs in, or sends mail. The candidate applies from the posting's own link.

**This branch is `rust-app`.** It adds a native macOS app under `crates/`. Read `REPORT.md`
for its state. The Python workspace still runs production.

## Layout

| Path | Contents |
|---|---|
| `data/candidate/` | Source of truth for every application field: `candidate_profile.yaml` (resume plus `answers_bank`), `resume-text.md`, `resume/`, `documents/`, `outreach-voice/`, `outreach-samples/` |
| `data/config/` | `sources.yaml`, `filters.yaml`, `ats_boards.yaml`, `workday_boards.yaml`, `json_boards.yaml`, `icims_boards.yaml`, `workday_tombstones.yaml`, `source_candidates.md`, and the prompt templates |
| `data/state/` | Scan output and portal data, gitignored: `last_scan.json`, `portal_feed.jsonl`, `descriptions.json`, `seen.jsonl`, `board_health.json`, `workday_sweep.json`, `sg_boards.json`, `applied.json`, `briefs/`, `scan.log`, `retry.log` |
| `server/` | `api/` (HTTP), `scan/` (engine, one module per source), `brief/` (brief engine), `jobs/` (timers and utilities), `tests/` |
| `client/` | Preact + TypeScript dashboard. `dist/` is committed, because the server box has no Node |
| `docker/` | Scan and retry image, with `entrypoint.sh` and `entrypoint-retry.sh` |
| `systemd/` | The two user timers |
| `crates/` | The Rust app: `jh-core`, `jh-db`, `jh-cli`, `jh-scan`, `jh-engine`, `jh-app` |

## How a day runs

1. **Scan, Monday to Friday at 09:00, 13:00 and 17:00.** `job-scan.timer` starts
   `job-scan.service`. The service runs `docker compose -f docker/compose.yaml run --rm scan`.
   The container runs `entrypoint.sh`: sweep if stale, scan five per tier, then mark reported.
   It mounts `server/`, `data/config/` and `data/candidate/` read-only and `data/state/`
   read-write. It uses public DNS, so the burst of board lookups never hits the home resolver
   limit. A missed slot waits for the next one.
2. **Retry, Sunday at 22:00.** `job-retry.timer` starts `job-retry.service`, which runs
   `entrypoint-retry.sh`: retry, then prune after a clean run. Retry re-checks only the Workday
   boards whose backoff has come up. Prune removes boards that are structurally dead, that is a
   Workday 404 or 422 from a wrong career-site name. Prune appends each removed key to
   `workday_tombstones.yaml`, so the next harvest does not add it again. Delete a tombstone line
   by hand to let a board back in.
3. **Apply, by the candidate.** The agent reads the posting, rules on the fit, and returns the
   answers and the gaps. It does not submit.

Both timers take an exclusive `flock` on `data/state/.scan.lock`, so a manual run and a
scheduled run never overlap, and the two jobs never hit Workday at once. Output goes to
`data/state/scan.log` and `data/state/retry.log`.

There is no agent in the scan loop. A scan cannot score postings or draft text. The dashboard
shows the deterministic keyword score from `server/scan/engine.py`.

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
- **Location tiers set the order:** vancouver, remote_canada, toronto, seattle, remote_us,
  remote_unspecified. No match means the posting is dropped. Keep a US posting and flag it.
  `--per-tier N` caps each tier in the printed list only. The timer uses 5.

## Commands

Run these with `/usr/bin/python3`, not a bare `python3`. Inside an agent session `python3`
resolves to a build with no PyYAML, and the scan finds nothing.

| Command | Does |
|---|---|
| `-m server` | Serve `client/` and the API on :8787 |
| `-m server --api-only --port 8788` | Serve `/api` only, loopback (the production role) |
| `-m server scan [--per-tier 5]` | Run one scan |
| `-m server scan --source jobbank` | Limit the scan to one source |
| `-m server sweep`, `retry`, `prune` | Maintain the Workday cache and board list |
| `-m server refresh` | Rebuild the portal feed from the last scan |
| `-m server probe-ats` | Test ATS slugs, rewrite `ats_boards.yaml` |
| `-m server brief <id> [--deep-contacts]` | Generate one brief |
| `-m server feed --days 7` | Print the Simplify feed |

`--source` takes an id from `sources.yaml`. A wrong id matches nothing and exits clean, which
looks like a quiet day. `--source ats` is wrong and `--source ats_boards` is right.

## Dashboard

`tailscale serve` hosts `client/` at `https://home.taild62434.ts.net:8443/` and proxies `/api`
to the Python server on loopback `:8788`. The API is never on the LAN or the tailnet. Every
request re-reads `data/state/`, so the page is current when a scan ends.

Port 443 is not available: Pi-hole v6 binds it. Do not change Pi-hole.

```sh
sudo tailscale serve reset
sudo tailscale serve --bg --yes --https=8443 /home/muhan-li/job-hunt/client
sudo tailscale serve --bg --yes --https=8443 --set-path=/api http://127.0.0.1:8788/api
```

The page is a pipeline in three stages, and the tab bar selects the stage. A posting with a
brief leaves Open for Shortlisted. An applied posting moves to Applied. No row is in two tabs.
A failed brief stays in Open, so the next press retries it.

The API answers these routes:

| Route | Does |
|---|---|
| `GET /api/dashboard` | The three lists, with `expires_on` per row |
| `GET /api/posting/<id>` | One posting, its text, and its brief state |
| `POST /api/brief/<id>`, `/api/brief/batch` | Start one brief, or a selection |
| `POST /api/deep-contacts/<id>` | Start the contact search |
| `POST /api/applied/<id>`, `/api/applied/remove` | Mark applied, or remove a selection |

A brief runs in a detached process. The API answers before the brief exists, and the page
polls. A batch route skips an id that already has a brief, so a select-all cannot spend a model
call on a row the reader has already read.

The scan retires a posting `max_age_days` after its publish date. The page reports that date as
`expires_on`. Workday publishes a relative age only, so those dates are accurate to about one
day. Rippling and BambooHR publish no date, so their rows sort last.

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
  module in `scan/sources/`, a row in `SOURCE_FUNCS`, a block in `sources.yaml`, and a row in
  `source_candidates.md`. If the source reveals employer boards, run `probe-ats` too. A feed
  that hands over boards is half-used until that probe runs.
