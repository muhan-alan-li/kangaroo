# Job Hunt Rust App

**Branch:** `rust-app` · **Date:** 2026-11-13 · **State:** backend ported, dashboard in

A native macOS app that replaces the Python server and the web dashboard. The engine is Rust,
the UI is GPUI with `gpui-component`, and the state is SQLite. The YAML files in `data/config/`
stay the configuration.

`main` is untouched and still runs production on `home`.

The design is in `docs/rust-app-plan.md`. Where the design and this file disagree, this file
wins, because it describes what runs.

## Crates

| Crate | Duty | Tests |
|---|---|---|
| `jh-core` | Posting id and dedupe key, apply signatures, filters, score and order, parsing, keywords, config | 11 |
| `jh-db` | SQLite pool, migrations, import | — |
| `jh-protocol` | The command, event and dashboard types shared by both processes | — |
| `jh-scan` | Egress guard, the source trait, and one module per feed | 33 |
| `jh-engine` | Engine thread, scheduler, scan lock, scan pipeline, ledger, board health, brief/apply commands, snapshot, one-posting detail | 13 |
| `jh-brief` | Three-stage brief chain, validator, SQLite brief rows | 5 |
| `jh-cli` | `import`, `scan`, `sweep`, `retry`, `prune`, `probe-ats`, `harvest` | — |
| `jh-server` | Menu-bar agent: engine owner, SQLite owner, Unix-socket server | — |
| `jh-app` | GPUI desktop client: stages, table, tick column, detail panel, scan controls, theme | 10 |

72 tests pass. `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` are clean.

## Run it

```sh
make run        # build both binaries and launch the desktop app
make run-server # run the menu-bar server in the foreground
make check      # format check, clippy, every test
make import     # load fixtures/import_state into data/state/jobhunt.db
make db         # show the contents
make reset-db   # delete the database
```

The server reads and writes `data/state/jobhunt.db` in the current directory. Set `JH_DB_PATH` to
move it and `JH_SOCKET_PATH` to move the Unix socket. The app starts a sibling `jh-server` when
none answers, then talks to it over that socket.

## Machine facts

- **Rust 1.98.1** is required, not preferred. `gpui-pre` uses `Result::map_or_default`.
- **Use full Xcode.** GPUI compiles Metal shaders, and `/usr/bin/metal` ships only in Xcode.
  `.cargo/config.toml` sets `DEVELOPER_DIR` and keeps an existing value.
- **macOS 12.0 or later.** Read from the pinned GPUI source.
- Worker cargo runs use `CARGO_NET_OFFLINE=true`. Fetch once with `cargo fetch`.

## Pins

| Crate | Package | Version | Revision |
|---|---|---|---|
| `gpui` | `gpui-pre` | 0.3.12 | zed `d2a4fb6` |
| `gpui_platform` | `gpui-pre-platform` | 0.3.12 | same snapshot |
| `gpui_component` | `gpui-component` | 0.9.1 | `gpui-kit` `39ab849` |
| `gpui_kit_assets` | `gpui-kit-assets` | 0.9.1 | same revision; the icon bundle |

The macOS status item has no GPUI form, so `jh-server` writes it directly against AppKit through
`objc2`. AppKit retains neither the item nor its target, so both live in `thread_local` storage.

## Architecture

Two processes. `jh-server` is a menu-bar agent: it owns the engine, the SQLite pool and the
scheduler. `jh-app` is a GPUI window and never touches the database.

```
jh-app (GPUI) ──Command──► Unix socket ──► jh-server (Tokio engine) ──► SQLite
jh-app (GPUI) ◄──Event──── Unix socket ◄── jh-server (Tokio engine)
```

One JSON object per line in both directions. The server broadcasts every event to every
connected client, so a second window sees the same state. A client asks for `Snapshot` after it
connects; the event stream keeps no retained value.

All commands work: `Scan`, `CancelScan`, `Snapshot`, `RefreshConfig`, `MakeBrief`, `MakeBriefs`,
`PostingDetail`, `Apply`, `Unapply`, `SaveSettings`. Events: `Dashboard`, `PostingDetail`,
`ScanStatus`, `BriefUpdated`, `Error`.

`PostingDetail` answers with one posting: the table's columns, the cached text, the salary and
keyword fields, and the brief's stage-C payload. The dashboard lists stay small, so the drawer
asks for the rest when a row is clicked.

## The window

One page. The tab bar picks the stage: Open, Shortlisted, Applied, each with its count. The
table draws date, location, company, title and fit, cut down by the search field. A row click
opens the detail panel on the right; `Esc` closes it.

In the Open stage a column of ticks marks rows for a batch brief. `Generate briefs` sends one
`MakeBriefs` for the ticked rows and clears the marks. A tick is drawn by the app, not by the
table: the table's own selection is one row, which is the row the panel shows. A finished brief
moves the posting to Shortlisted, so the mark goes with it.

The panel shows the verdict, the key skills, the proposed resume edits, the gaps, the facts and
the posting text. It holds the actions that belong to one posting: `Generate brief`,
`Mark applied` and `Open posting`.

The app registers `gpui_kit_assets::Assets`. `gpui_component` names an icon but ships no source
for it, so without an asset source the tick and the close button draw nothing.

The scheduler runs 09:00 / 13:00 / 17:00 on weekdays. A slot runs the scan under an exclusive
`flock` on `data/state/.scan.lock`. The scan writes postings, the `seen` ledger, `scan_runs`,
`source_runs`, descriptions and board health. A posting expires after `max_age_days` and does
not return.

## Sources

| Source | Kind | Status |
|---|---|---|
| Simplify JSON | `simplify_json` | Ported |
| Simplify README | `simplify` | Ported |
| Job Bank | `jobbank` | Ported, including detail pages |
| Greenhouse, Ashby, Lever | `ats` | Ported |
| Workable, SmartRecruiters, Rippling, BambooHR, Recruitee, Breezy, Teamtailor | `ats` | Ported |
| Workday | `ats` | Ported, including the sweep cache |
| startups.gallery | `startups_gallery` | Ported; reads `<time datetime>` and harvests boards |
| Work at a Startup | `workatastartup` | Ported; `date_missing` |
| LinkedIn guest search | `linkedin` | Ported, read-only |
| Remotive | `remotive` | Ported, disabled by default |
| iCIMS | `ats` | Not ported; needs browser TLS impersonation |

Every parser has a test against `fixtures/http/`. Three of those bodies are 4 MiB prefixes cut
mid-token; the Rust parser salvages the complete array elements.

## Parity rules

Break one of these and postings re-report or applications orphan.

- **Id.** `sha1(key)[:16]`, lowercase hex.
- **Key order.** `dedupe_key`, else `source|raw_id`, else lowercase `company|title|location`.
- **Signature.** Copy the fourteen patterns from `server/scan/posting.py`. Greenhouse, Ashby and
  Lever key on `ats|{id}`. The JSON vendors and iCIMS put the board in the key, because their
  ids are scoped to one board.
- **Order.** Tier rank, then score high to low, then age, undated last.
- **Health key.** Workday is `tenant|wd|site`. A JSON vendor is `vendor|slug`.
- **Dates.** ISO 8601, UTC, seconds.

`fixtures/python/` holds cases that `generate.py` builds from the Python modules. Regenerate
them when Python changes. A behaviour change must fail a Rust test; never edit a fixture to
make it pass.

## Briefs

The brief is the three-stage chain: requirements, evidence match, finished brief. `validate`
drops a `before` that is not verbatim in the resume and flags a notable term that appears in
neither the resume nor the posting. The key is read from `DEEPSEEK_API_KEY`, the Keychain item
`com.muhanli.jobhunt.deepseek`, or `~/.hermes/auth.json`, in that order. With no key the brief
buttons are gated and a run records `status: failed` rather than calling the model.

## Decisions

| Area | Decision |
|---|---|
| Processes | Menu-bar server owns the engine; the GPUI app is a socket client |
| Scan host | The Mac. The docker timer on `home` retires after parity |
| Production | The Python dashboard stays live until the Rust scan matches it |
| Commits | Local only. No push, and no merge to `main`, without approval |
| State | SQLite `data/state/jobhunt.db`. YAML stays the config |
| Briefs | DeepSeek-compatible chat, key in Keychain, off when absent |
| Platform | Apple Silicon macOS only |

## Known issues

| Symptom | Cause | Status |
|---|---|---|
| A capture shows the first frame after a change | macOS defers drawing while the window is occluded | Not a defect. Activate the app first |
| `Scan now` ignores synthetic keys | The app needs Accessibility permission | Not a defect. Click it |
| A full scan hits ~2,000 boards | That is the real Workday and JSON-vendor fleet | Expected; the sweep cache removes Workday from the daily path |
| `jh-server` has no login item yet | Packaging is not started | Open, S9 |

## Roadmap

| Slice | Work | Done when |
|---|---|---|
| S4 | Sources behind one trait | Done: every parser matches its saved response, and live runs write postings |
| S5 | JSON vendors, Workday plus sweep, gallery, Work at a Startup | Done: cache, board health and retry schedule work |
| S6 | Pipeline, ledger, health, retry, prune, probe, harvest | Done in Rust; a same-day id/field diff against Python is still worth running |
| S8 | Dashboard: stages, table, drawer, health, settings | Stages, table, ticks and drawer are in; health and settings are not |
| S9 | Brief engine, bundle, signing, CI | Brief engine is in; bundle, signing and CI are not |

Pipeline order: collapse requisitions, filter and tier, sort, split the ledger, write state. The
per-tier cap shortens a printed list only. It does not touch the dashboard.

## Where things are

| Path | Contents |
|---|---|
| `Makefile` | `run`, `run-server`, `check`, `import`, `db`, `reset-db`, and the Python targets |
| `crates/` | The nine crates |
| `migrations/` | `0001_init.sql`, `0002_board_cursor.sql` |
| `fixtures/python/` | Parity cases and their generator |
| `fixtures/http/` | One saved response per source family, with provenance in its README |
| `fixtures/import_state/` | A state directory for the import command |
| `docs/rust-app-plan.md` | The design: schema, sources, UI, packaging, risks |
