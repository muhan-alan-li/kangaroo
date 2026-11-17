# Kangaroo

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
make run        # build the menu-bar server and the app, then launch the app
make run-server # run the menu-bar server in the foreground
make check      # format check, clippy, every test
make import     # load the fixture state into data/state/kangaroo.db
make db         # show what the database holds
```

The menu-bar server owns the engine and the database. The desktop app connects to it over a
Unix socket at `data/state/kangaroo.sock`; it never opens SQLite. The app starts a sibling server
when none is listening.

The window is one page. The tab bar picks the stage (Open, Shortlisted, Applied), the table
lists the postings in it, and a row click opens the detail panel: the brief, the facts, and the
cached posting text. In Open, tick the rows you want and press `Generate briefs`.

### The archived Python workspace

`archive/` holds the Python server, the Preact web client, the scan container and the two user
timers. They are out of the build on this branch. They still run production on `main`. To use
them, check out `main`, or move the folders back to the root.

## Commands

`make help` lists every target.

| Command | Does |
|---|---|
| `make run` | Build both binaries and launch the desktop app |
| `make run-server` | Run the menu-bar server in the foreground |
| `make check` | Format check, clippy, every test |
| `make import` | Load `fixtures/import_state` into the dev database |
| `make db` | Show what the dev database holds |
| `make reset-db` | Delete the dev database |

The CLI does the same work without the app:

| Command | Does |
|---|---|
| `kangaroo scan --db FILE [--source ID] [--json]` | Run one scan |
| `kangaroo sweep --db FILE [--limit N] [--dry-run]` | Sweep Workday into the cache |
| `kangaroo retry --db FILE [--force] [--dry-run]` | Re-check the boards that failed |
| `kangaroo prune --db FILE [--dry-run]` | Remove dead Workday boards |
| `kangaroo probe-ats --db FILE [--report]` | Probe ATS slugs, rewrite `ats_boards.yaml` |
| `kangaroo harvest workday\|jsonboards --db FILE [--report]` | Rebuild a board list from the feed |
| `kangaroo import --from DIR --db FILE` | Import a Python state directory |

`kangaroo` is `target/debug/kangaroo`.

## Layout

```
crates/          Rust app: kangaroo-core, kangaroo-db, kangaroo-protocol, kangaroo-scan,
                 kangaroo-engine, kangaroo-brief, kangaroo-cli, kangaroo-server, kangaroo-app
migrations/      The SQLite schema, embedded by kangaroo-db at startup
fixtures/        Saved source responses, the Python parity cases and the import state
docs/            The design, in rust-app-plan.md
data/config/     Sources, filters, board lists, prompt templates
data/candidate/  Personal inputs, gitignored; the source of truth for answers
data/state/      The database, the scan lock and logs, gitignored
archive/         The Python server, web client, container and timers, kept for reference
```

`AGENTS.md` holds the full list of files in each directory.

## How data flows

A scan reads `data/config/` and the network, then writes SQLite:

```
sources ──► collapse requisitions ──► filter and tier ──► score and sort ──► postings
                                                                              │
                                       seen ledger, scan_runs, source_runs ◄──┤
                                       descriptions, board health ◄──────────┘
```

Sources do network work only. Filtering, ranking and storage live in shared modules, so every
source behaves the same way.

The desktop app reads nothing itself. It asks `kangaroo-server` for a `Snapshot` over the Unix
socket, then applies the events that follow.

## The brief engine

`kangaroo-brief` runs a three-stage chain and validates each edit before it saves. A brief lives in
SQLite, next to the posting. The key is read from `DEEPSEEK_API_KEY`, or from the pool that
Hermes keeps in `~/.hermes/auth.json`. The key is never printed.

`REPORT.md` describes the chain, the validator and the cache.

## Configuration and state

`data/config/` holds `sources.yaml`, `filters.yaml`, the board lists, the Workday tombstones,
`source_candidates.md` and the prompt templates. `AGENTS.md` describes each file.

`data/state/kangaroo.db` holds the postings, the `seen` ledger, the run rows, descriptions, board
health and briefs. `data/state/.scan.lock` keeps two scans apart. `data/state/` is generated.
`data/candidate/` is personal. Neither is committed.

## Scheduled jobs

`kangaroo-server` runs the scan at 09:00, 13:00 and 17:00 on weekdays, under an exclusive `flock` on
`data/state/.scan.lock`. A missed slot waits for the next one. The app shows the next slot and
lets you run one by hand.

## Tests

```sh
make test     # every crate
make check    # format check, clippy, every test
```

The parity cases in `fixtures/python` pin the Rust port to the Python behaviour it replaced.

## Gotchas

- **Age.** Always use the publish field. Rippling, BambooHR and Work at a Startup publish no
  date, so their rows carry `date_missing` and sort last.
- **Workday.** It throttles by IP. Keep the worker pool at 16.
- **Brief.** The command answer arrives before the brief exists. Wait for `BriefUpdated`.
- **Toolchain.** Rust 1.98.1 and full Xcode. The Command Line Tools alone cannot build GPUI.

## Related

- `AGENTS.md`: the operational map, the schedule and the rules.
- `REPORT.md`: the Rust app, its state and its plan.
- `data/config/source_candidates.md`: every source considered, with the evidence.
