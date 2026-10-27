# Job Hunt

A local job-search workspace for junior and new-grad software engineering roles in Vancouver,
BC. It scans public job feeds and employer ATS boards, filters and ranks what it finds, serves a
small self-hosted dashboard, and generates an on-demand fit brief for any posting.

It is deliberately **read-only about applications**: it never fills a form, submits an
application, sends an email, or signs in anywhere. The candidate reads the posting, decides, and
applies from the posting's own link.

Owner: Muhan Li.

---

## What the system does

1. **Scan** — seven public sources are read in parallel: Job Bank Canada, the Simplify new-grad
   HTML and JSON feeds, ATS boards (Greenhouse, Ashby, Lever, Workday, iCIMS, and seven uniform
   JSON vendors), startups.gallery, Work at a Startup, and LinkedIn's public guest search.
2. **Filter and rank** — age, title level, location tier, and a deterministic keyword score.
   Results are capped per location tier.
3. **Publish** — the scan writes `data/state/last_scan.json` and the longer-lived
   `data/state/portal_feed.jsonl`. The dashboard reads those files on every request; the API
   never fetches anything itself.
4. **Tailor on demand** — opening a posting and pressing **Generate brief** runs a three-stage
   DeepSeek chain that extracts the posting's requirements, matches them against the resume with
   verbatim evidence, and proposes resume edits. A code-level validator drops any edit whose
   `before` is not actually in the resume.
5. **Track applied** — clicking **Apply** opens the posting in a new tab and snapshots it into
   `data/state/applied.json`. That list is a read-side record, not an application ledger.

Outreach is complete and no messages are drafted or sent by this workspace.

---

## Quick start

Run everything from the repository root.

```sh
# Requirements: Python 3.13 (3.10+ should work), PyYAML.
# For the client: Node 20+; for resume extraction: poppler-utils.
/usr/bin/python3 -c "import yaml"

# Serve the dashboard + API locally on :8787
/usr/bin/python3 -m server

# Or run one scan and print a table
/usr/bin/python3 -m server scan --per-tier 5

# Run the smoke tests
make test
```

Inside an agent session, always spell the interpreter as `/usr/bin/python3`. A bare `python3`
can resolve to a different toolchain that lacks PyYAML, which makes the scan silently find
nothing.

---

## Commands

All commands go through one entry point, `python3 -m server <command>`. With no command it
serves the dashboard.

| Command | What it does |
|---|---|
| `python3 -m server` | Serve `client/` + the API (local development, port 8787) |
| `python3 -m server --api-only --port 8788` | Serve only `/api/*`, loopback (production role) |
| `python3 -m server scan [--per-tier 5]` | Run one scan |
| `python3 -m server scan --source jobbank` | Limit a scan to one source id |
| `python3 -m server sweep` | Refresh the Workday board sweep cache |
| `python3 -m server retry` | Re-check Workday boards whose backoff has come up |
| `python3 -m server prune` | Remove structurally dead Workday boards |
| `python3 -m server refresh` | Rebuild the portal feed from the last scan |
| `python3 -m server mark-reported --all-unreported` | Mark a run's postings reported |
| `python3 -m server harvest workday\|jsonboards\|icims` | Rebuild a board-list config |
| `python3 -m server probe-ats` | Probe and rewrite `data/config/ats_boards.yaml` |
| `python3 -m server brief <id>` | Generate one posting brief |
| `python3 -m server brief <id> --engine hermes` | Use the older web-research agent |
| `python3 -m server brief <id> --deep-contacts` | Run only the web contact search |
| `python3 -m server brief --list` | List cached briefs |
| `python3 -m server table --per-tier 3` | Render the shortlist as a PNG |
| `python3 -m server feed --days 7` | Print the Simplify feed, one posting per line |
| `python3 -m server bump --bump` | Bump the client cache-buster |
| `python3 -m server resume-text` | Regenerate `data/candidate/resume-text.md` |

Useful `make` targets:

```sh
make serve        # foreground dashboard on :8787
make api          # API only, loopback, :8788
make start        # start the dashboard systemd user service
make scan         # one scan, 5 per tier
make test         # the four smoke tests
make client       # typecheck + build the client bundles
make bump         # bump the client ?v= cache-buster
make scan-docker  # one full scan in the container
make timer        # show the next scheduled scan slots
```

---

## Repository layout

```
client/                  Dashboard web client (Preact + TypeScript, built by Vite)
  src/                   postings.tsx, health.tsx, detail.tsx, detailParts.tsx,
                         api.ts, lib.ts, types.ts
  dist/                  Built entry bundles (committed; the server box has no Node)
  index.html             Postings table, with Open / Applied switch
  health.html            Scan cost and Workday board health
  styles.css             Handwritten "Lilac" theme, untouched by the build

server/                  The server package: one entry point, split by role
  cli.py                 Command table; imports each job lazily
  config.py              Shared paths and constants
  fsutil.py              Atomic-write helper used by every state writer
  api/                   Dashboard HTTP service (reads state, starts brief jobs)
  scan/                  Scan engine, one module per source under sources/
  brief/                 On-demand brief engine
  jobs/                  Scheduled and maintenance commands
  tests/                 Four smoke tests

data/
  candidate/             Candidate inputs (gitignored, personal)
    candidate_profile.yaml   Parsed resume + answers_bank (source of truth)
    resume-text.md           Text of every PDF, generated
    resume/                  Resume PDFs and tailored variants
    documents/               Transcript and similar uploads
    outreach-voice/, outreach-samples/
  config/                Sources, filters, board lists, prompt templates
  state/                 Scan output, portal feed, briefs, applied list, logs

docker/                  Scan/retry image and its two entry points
systemd/                 The two user timers and services
AGENTS.md                The operational map and standing rules
Makefile                 Common commands
```

---

## How data flows

### Scan path

```
data/config/sources.yaml
        │
        ▼
server/scan/sources/*  ──►  filters ──► pipeline (score, tier, cap, sort)
        │                                   │
        │                                   ├──► data/state/last_scan.json
        │                                   ├──► data/state/seen.jsonl (dedupe ledger)
        │                                   ▼
        └── board harvest ──► data/state/sg_boards.json
                              server/jobs/probe_ats.py ──► data/config/ats_boards.yaml

last_scan.json ──► server/scan/portal.py ──► data/state/portal_feed.jsonl
                                             data/state/descriptions.json
                                             data/state/portal_expired.json
```

`server/scan/engine.py` orchestrates a scan. Sources do network work only; filtering, ranking
and persistence happen in shared modules so every source behaves the same way.

### Dashboard path

```
browser ──► tailscale serve ──► server/api/app.py ──► data/state/*
                                                    (read on every request)
```

- `GET /api/dashboard` — everything the postings page shows.
- `GET /api/posting/<id>` — one posting, its cached description, and its brief state.
- `POST /api/brief/<id>` — start a brief generation.
- `POST /api/deep-contacts/<id>` — start the deep contact search.
- `POST /api/applied/<id>` — snapshot a posting into the applied list.
- `POST /api/applied/remove` — remove a selection, one write for the batch.

The API process never fetches a page and never calls a model. It reads files and launches
detached generator processes.

### Brief path

```
POST /api/brief/<id>
        │
        ▼
python3 -m server brief <id>        (detached)
        │
        ├── default: server/brief/chain.py
        │     stage A: requirements  (posting text only)
        │     stage B: evidence match (resume + requirements)
        │     stage C: brief + edits  (resume + matrix + posting)
        │             │
        │             ▼
        │     server/brief/validate.py   drop edits whose `before` is not verbatim,
        │                                 flag terms that appear nowhere
        │
        └── --engine hermes: server/brief/agent.py
              researcher prompt ──► critic prompt (delete-only review)
              ──► optional contacts, merged by server/brief/contacts.py

output: data/state/briefs/<id>.json        (the result; also written on failure)
        data/state/briefs/<id>.raw.json    (stage A/B or the agent's unedited brief)
        data/state/briefs/<id>.running     (present while a run is in flight)
```

The client polls `GET /api/posting/<id>` every 7 seconds until `running` clears. A marker older
than 15 minutes is reported as stale, so a crashed run cannot spin forever.

---

## The brief engine

The default engine is the three-stage chain in `server/brief/`:

| Module | Responsibility |
|---|---|
| `stages.py` | The three stage prompts and their single DeepSeek calls |
| `chain.py` | Runs A → B → C, then applies the validator and persists the result |
| `validate.py` | Mechanical factuality checks on proposed resume edits |
| `llm.py` | Minimal OpenAI-compatible chat client (DeepSeek) |
| `agent.py` | The older one-shot Hermes agent and its delete-only review pass |
| `contacts.py` | On-demand deep contact search, merged into an existing brief |
| `prompts.py` | Prompt assembly; all wording lives in `data/config/*.md` |
| `posting.py` | Finding the portal row and the posting text |
| `state.py` | The brief files under `data/state/briefs/` and their atomic writes |
| `generate.py` | CLI, dispatch, and `--list` / `--print` / `--prompt` |

The API key is read from `DEEPSEEK_API_KEY`, falling back to the pool Hermes already keeps at
`~/.hermes/auth.json`. The key is never printed.

The default brief carries no contacts. Contact research is a separate, on-demand action
(`--deep-contacts`) because it is the only half that needs the web.

Prompt templates in `data/config/`:

- `brief_stage_a_system.md`, `brief_stage_b_system.md`, `brief_stage_c_system.md` — the chain.
- `brief_prompt.md`, `brief_review_prompt.md` — the Hermes agent and its critic.
- `contact_research_system.md` — the deep contact search.

---

## Configuration and state

### `data/config/`

| File | Purpose |
|---|---|
| `sources.yaml` | The source registry: enabled sources, filters, per-source options |
| `filters.yaml` | Title/level rules, compensation floor, age window, location tiers |
| `ats_boards.yaml` | Probed Greenhouse / Ashby / Lever boards |
| `workday_boards.yaml` | Workday tenant + site triples |
| `json_boards.yaml` | Workable, SmartRecruiters, Rippling, BambooHR, Recruitee, Breezy, Teamtailor |
| `icims_boards.yaml` | iCIMS customer hosts |
| `workday_tombstones.yaml` | Structurally dead boards that harvest must not re-add |
| `source_candidates.md` | The register of every source considered and why it is on or off |
| `*_system.md`, `brief_*.md` | Model prompt templates |

### `data/state/`

| File | Purpose |
|---|---|
| `last_scan.json` | The most recent scan's result |
| `portal_feed.jsonl` | The dashboard's live posting list (7-day life) |
| `descriptions.json` | Cached posting text |
| `seen.jsonl` | Dedupe ledger and reported flags |
| `board_health.json` | Per-board health and retry schedule |
| `workday_sweep.json` | Workday sweep cache read by the daily scan |
| `sg_boards.json` | Boards harvested from startups.gallery |
| `applied.json` | The applied list shown by the dashboard |
| `briefs/` | Generated briefs and their artifacts |
| `outreach.jsonl` | Historical outreach log |
| `scan.log`, `retry.log` | Output of the two no-agent jobs |

`data/state/` is generated and gitignored. `data/candidate/` is personal and gitignored:
provide it before generating a brief, and regenerate `resume-text.md` whenever the resume PDF
changes.

---

## Scheduled jobs

Two systemd **user** timers run containerized jobs:

| Timer | Schedule | Service | What runs |
|---|---|---|---|
| `job-scan.timer` | Mon–Fri 09:00, 13:00, 17:00 | `job-scan.service` | `docker compose run --rm scan` |
| `job-retry.timer` | Sunday 22:00 | `job-retry.service` | `docker compose run --rm retry` |

The scan container runs `docker/entrypoint.sh`: sweep-if-stale, `server scan --per-tier 5`, then
`mark-reported --all-unreported`, all under one `flock` on `data/state/.scan.lock`. The retry
container runs `docker/entrypoint-retry.sh`: `server retry`, then `server prune` after a clean
run. Both use public DNS directly so the burst of board lookups never touches the home resolver's
rate limit.

The unit files live in `systemd/` and are symlinked into `~/.config/systemd/user/`. A missed slot
waits for the next one (`Persistent=false`); resuming never stampedes several scans at once.

Manual runs:

```sh
systemctl --user start job-scan.service
systemctl --user start job-retry.service
make scan-docker
```

Logs: `data/state/scan.log` and `data/state/retry.log`.

---

## Dashboard deployment

The production page is hosted by `tailscale serve` at
`https://home.taild62434.ts.net:8443/`, which serves `client/` and reverse-proxies `/api` to the
Python server on loopback `:8788`.

```sh
sudo tailscale serve reset
sudo tailscale serve --bg --yes --https=8443 /home/muhan-li/job-hunt/client
sudo tailscale serve --bg --yes --https=8443 --set-path=/api http://127.0.0.1:8788/api
```

The dashboard itself runs as a user service:

```sh
systemctl --user status  job-dashboard.service
systemctl --user restart job-dashboard.service
journalctl --user -u job-dashboard.service -n 50
```

Port 443 is unavailable to `tailscale serve` because Pi-hole binds it; the page therefore lives
on 8443. Leave Pi-hole alone.

After changing anything under `client/src/`, `client/styles.css`, or the two HTML shells:

```sh
cd client && npm run build && cd ..
/usr/bin/python3 -m server bump --bump
```

`client/dist/` is committed so the server box needs no Node.

---

## Development notes

### Tests

```sh
make test
```

or individually:

```sh
/usr/bin/python3 server/tests/test_boardstate.py   # board health, backoff, sweep cache
/usr/bin/python3 server/tests/test_dashboard.py    # routes, MIME types, path check, brief helpers
/usr/bin/python3 server/tests/test_net.py          # DNS cache, pacing, resolver pause, sockets
/usr/bin/python3 server/tests/test_retry.py        # probe-result contract, retry unpack
```

### Adding a scan source

A new source is not adopted until all four exist:

1. A module in `server/scan/sources/`.
2. A row in `SOURCE_FUNCS` in `server/scan/sources/__init__.py`.
3. A block in `data/config/sources.yaml` (`enabled: true` only when it works).
4. A row in `data/config/source_candidates.md` recording the evidence.

If the source reveals employer boards, run `server/jobs/probe_ats.py` so those boards enter
`data/config/ats_boards.yaml`. A feed that hands over boards is half-used until that probe runs.

### Adding a command

Add a lazy entry to `COMMANDS` in `server/cli.py`, pointing at a `main(argv)` in `server/jobs/`
(or the relevant engine). Keep the job module thin: it should parse arguments and call shared
code, not re-implement scanning or state I/O.

### Separation of concerns

The project follows a few rules so the pieces stay independent:

- **One entry point.** Everything runs through `python3 -m server <command>`; there are no
  standalone scripts in a `bin/` directory.
- **The API only reads state and launches jobs.** It never fetches a page and never calls a
  model. Network and model work happen in `scan/` and `brief/`.
- **The scan owns fetching, filtering, ranking and persistence.** Sources return rows; shared
  modules decide what survives and how it is stored.
- **The brief engine is split by stage.** Prompts (`stages.py`), orchestration (`chain.py`),
  factual checks (`validate.py`), the older agent (`agent.py`), contact search (`contacts.py`),
  input lookup (`posting.py`), and file I/O (`state.py`) do not know each other's internals.
- **Paths live in `server/config.py`.** Modules import constants instead of re-deriving paths.
- **State writes are atomic.** Every writer uses `server/fsutil.py:atomic_write`, so a reader
  never sees a half-written file.
- **`data/candidate/` is the source of truth.** Never invent a fact about the candidate; a
  missing answer is a question, not a guess.
- **Applications are manual.** The workspace may mark a posting applied, but it never submits.

---

## Gotchas

- **Interpreter:** use `/usr/bin/python3`, not a bare `python3` inside an agent session.
- **`--source` ids:** use `jobbank`, `newgrad_list`, `newgrad_json`, `ats_boards`,
  `startups_gallery`, `workatastartup`, `linkedin`. A wrong id exits clean with an empty report,
  which looks like a quiet day.
- **Age:** always comes from the publish field, never `updated_at`. Workday and the JSON vendors
  publish only relative ages or no date at all; those rows are marked `date_missing` and sort
  last.
- **Workday throttles by IP.** Keep its worker pool at 16; widening it trades boards for seconds.
- **New-grad HTML hides closes.** `newgrad_json` is authoritative; `newgrad_list` remains a
  fallback.
- **iCIMS needs browser impersonation.** It runs under `.venv-icims` with `curl_cffi`; without
  that venv the source is skipped rather than failing the scan.
- **The brief launcher is a separate process.** `POST /api/brief/<id>` answers immediately; the
  page polls. Do not expect a brief in the POST response.

---

## Related documentation

- `AGENTS.md` — the operational map, daily workflow, and standing rules for the agent.
- `data/config/source_candidates.md` — every source considered, with evidence.
- `data/config/sources.yaml` — the comments there explain each source's quirks and limits.
- `docker/Dockerfile`, `docker/compose.yaml`, `docker/entrypoint*.sh` — how the scheduled jobs
  run and why they bypass the local resolver.
