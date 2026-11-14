# Job Hunt

A local job-search workspace for junior and new-grad software engineering roles in Vancouver,
BC. It scans public feeds and employer ATS boards, filters and ranks the results, serves a small
dashboard, and writes a fit brief for any posting on demand.

It is read-only about applications. It never fills a form, submits, signs in, or sends mail.

Owner: Muhan Li. The agent's map and standing rules are in `AGENTS.md`. The Rust app's state is
in `REPORT.md`.

## Quick start

### The Rust app (this branch)

Needs Rust 1.98.1 and full Xcode. GPUI compiles Metal shaders, and the Command Line Tools do
not include the `metal` binary.

```sh
make run        # build and launch the menu bar app
make check      # format check, clippy, every test
make import     # load the fixture state into data/state/jobhunt.db
make db         # show what the database holds
```

### The Python workspace (production)

Needs Python 3.13 with PyYAML, Node 20 or later for the client, and poppler-utils to read the
resume PDFs.

```sh
/usr/bin/python3 -m server            # dashboard + API on :8787
/usr/bin/python3 -m server scan --per-tier 5
make test-py                          # the four smoke tests
```

Always write `/usr/bin/python3`. Inside an agent session a bare `python3` can resolve to a
toolchain with no PyYAML, and the scan then finds nothing.

## Commands

One entry point, `python3 -m server <command>`. With no command it serves the dashboard.

| Command | Does |
|---|---|
| `-m server` | Serve `client/` and the API on :8787 |
| `-m server --api-only --port 8788` | Serve `/api` only, loopback (production) |
| `-m server scan [--per-tier 5] [--source ID]` | Run one scan |
| `-m server sweep` / `retry` / `prune` | Maintain the Workday cache and board list |
| `-m server refresh` | Rebuild the portal feed from the last scan |
| `-m server harvest workday\|jsonboards\|icims` | Rebuild a board-list config |
| `-m server probe-ats` | Probe and rewrite `ats_boards.yaml` |
| `-m server brief <id>` | Generate one brief (`--deep-contacts` for contacts) |
| `-m server brief --list` | List cached briefs |
| `-m server table --per-tier 3` | Render the shortlist as a PNG |
| `-m server feed --days 7` | Print the Simplify feed |
| `-m server bump --bump` | Bump the client cache-buster |
| `-m server resume-text` | Regenerate `resume-text.md` |

`make help` lists every target.

## Layout

```
crates/        Rust app: jh-core, jh-db, jh-cli, jh-scan, jh-engine, jh-app
server/        api/ (HTTP), scan/ (engine), brief/ (brief engine), jobs/, tests/
client/        Preact + TypeScript dashboard; dist/ is committed
data/config/   Sources, filters, board lists, prompt templates
data/candidate/  Personal inputs, gitignored; the source of truth for answers
data/state/    Scan output and portal data, gitignored
docker/        The scheduled scan and retry image
systemd/       The two user timers
```

`AGENTS.md` holds the full list of files in each directory.

## How data flows

A scan reads `data/config/` and the network, then writes state:

```
sources ──► collapse requisitions ──► filter and tier ──► score and sort ──► ledger split
                                                                              │
                            last_scan.json ◄──────────────────────────────────┤
                            seen.jsonl      ◄─────────────────────────────────┤
                            portal_feed.jsonl (via portal.py) ◄───────────────┘
```

Sources do network work only. Filtering, ranking and storage live in shared modules, so every
source behaves the same way.

The dashboard reads state and nothing else:

```
browser ──► tailscale serve ──► api/app.py ──► data/state/*  (read on each request)
```

The API never fetches a page and never calls a model. It reads files and starts detached jobs.

## The brief engine

The default engine is a three-stage chain. The key is read from `DEEPSEEK_API_KEY`, or from the
pool that Hermes keeps in `~/.hermes/auth.json`. The key is never printed.

| Module | Duty |
|---|---|
| `stages.py` | The three prompts and their model calls |
| `chain.py` | Run stage A, B, C, then validate and save |
| `validate.py` | Drop an edit whose `before` is not verbatim in the resume |
| `agent.py` | The older web-research agent and its review |
| `contacts.py` | The on-demand contact search |
| `state.py` | The brief files and their atomic writes |

A brief writes `data/state/briefs/<id>.json` on success and on failure. A `.running` marker
shows a run in flight. The page polls until the marker clears, and reports a marker older than
15 minutes as stale.

The default brief carries no contacts. Contact research needs the web, so it is a separate
action.

## Configuration and state

`data/config/` holds `sources.yaml`, `filters.yaml`, the five board lists, the Workday
tombstones, `source_candidates.md` and the prompt templates. `AGENTS.md` describes each file.

`data/state/` holds `last_scan.json`, `portal_feed.jsonl`, `descriptions.json`, `seen.jsonl`,
`board_health.json`, `workday_sweep.json`, `sg_boards.json`, `applied.json`, `briefs/`, and the
two job logs.

`data/state/` is generated. `data/candidate/` is personal. Neither is committed. Regenerate
`resume-text.md` whenever the resume PDF changes.

## Scheduled jobs

Two systemd user timers run containerized jobs: `job-scan.timer` on weekdays at 09:00, 13:00 and
17:00, and `job-retry.timer` on Sunday at 22:00. Both bypass the home resolver, because a burst
of board lookups would hit its rate limit. Both share the scan lock.

Run one by hand:

```sh
make scan-docker
systemctl --user start job-scan.service
```

Logs: `data/state/scan.log` and `data/state/retry.log`.

## Deployment

`tailscale serve` hosts `client/` on 8443 and proxies `/api` to the Python server on loopback
`:8788`. Port 443 belongs to Pi-hole, so the page lives on 8443. The setup commands are in
`AGENTS.md`.

After a change under `client/`:

```sh
cd client && npm run build && cd ..
/usr/bin/python3 -m server bump --bump
```

`client/dist/` is committed, so the server box needs no Node.

## Tests

```sh
make test-py
```

| File | Covers |
|---|---|
| `test_boardstate.py` | Board health, backoff, retry queue, sweep cache, expiry |
| `test_dashboard.py` | Routes, MIME types, path check, brief helpers |
| `test_net.py` | DNS cache, pacing, resolver pause, socket cap |
| `test_retry.py` | Probe contract, retry unpack |

## Gotchas

- **Interpreter.** Use `/usr/bin/python3`.
- **Source ids.** A wrong id exits clean with an empty report, which looks like a quiet day.
- **Age.** Always use the publish field. Rippling, BambooHR and Work at a Startup publish no
  date, so their rows carry `date_missing` and sort last.
- **Workday.** It throttles by IP. Keep the pool at 16.
- **Brief.** The POST answer arrives before the brief exists. Poll, do not wait.
- **iCIMS.** It needs browser impersonation through `.venv-icims` with `curl_cffi`. Without
  that venv the source is skipped, and the scan still finishes.

## Related

- `AGENTS.md`: the operational map, the schedule and the rules.
- `REPORT.md`: the Rust app, its state and its plan.
- `data/config/source_candidates.md`: every source considered, with the evidence.
