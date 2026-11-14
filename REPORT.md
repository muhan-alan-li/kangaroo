# Job Hunt Rust App

**Branch:** `rust-app` · **Date:** 2026-11-13 · **State:** shell works, sources and dashboard to come

A native macOS app that replaces the Python server and the web dashboard. The engine is Rust,
the UI is GPUI with `gpui-component`, and the state is SQLite. The YAML files in `data/config/`
stay the configuration.

`main` is untouched and still runs production on `home`. Nothing here is pushed.

The design is in `docs/rust-app-plan.md`. Where the design and this file disagree, this file
wins, because it describes what runs.

## Crates

| Crate | Duty | Tests |
|---|---|---|
| `jh-core` | Posting id and dedupe key, apply signatures, filters, score and order, parsing, config | 7 |
| `jh-db` | SQLite pool, migrations, import | — |
| `jh-cli` | `jh import --from DIR --db FILE`, idempotent | — |
| `jh-scan` | Egress guard: cache, single-flight, token bucket, breaker, sockets | 17 |
| `jh-engine` | Engine thread, channels, scheduler, scan lock, stub scan, snapshot | 10 |
| `jh-app` | Window, status panel, controls, status item, theme | — |

34 tests pass. `cargo fmt --check` and `cargo clippy -- -D warnings` are clean.

The app starts, creates and migrates its database, shows engine and scan state, schedules
09:00 / 13:00 / 17:00 on weekdays, takes the scan lock, and runs a stub scan that writes one
`scan_runs` row. It has no sources yet, so a scan finds nothing.

## Run it

```sh
make run        # build and launch
make check      # format check, clippy, every test
make import     # load fixtures/import_state into data/state/jobhunt.db
make db         # show the contents
make reset-db   # delete the database
```

The app reads and writes `data/state/jobhunt.db` in the current directory. Set `JH_DB_PATH` to
move it. The raw cargo forms work too: `cargo run -p jh-app`, `cargo test --all`.

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

The macOS status item has no GPUI form, so it uses AppKit through `objc2`. AppKit retains
neither the item nor its target, so both live in `thread_local` storage.

## Architecture

One process, two threads. GPUI owns the main thread. The engine owns a Tokio runtime on its own
thread and the SQLite pool. The UI never touches the database.

```
UI (main) ──Command──► Engine (Tokio) ──► SQLite
UI (main) ◄──Event──── Engine (Tokio)
```

Commands: `Scan`, `CancelScan`, `Snapshot`, `RefreshConfig`, `MakeBrief`, `MakeBriefs`,
`Apply`, `Unapply`, `SaveSettings`. Events: `Dashboard`, `ScanStatus`, `BriefUpdated`, `Error`.

Four commands work today. The rest answer `Error { kind: NotImplemented }`.

`flume` keeps no value for a late reader. The view therefore sends `Snapshot` after it
subscribes; without that, the first snapshot is lost and the panel stays on `Engine: Starting`.

A slot runs sweep, scan, then mark reported. The scheduler and the lock exist. The three steps
arrive with the sources.

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

## Decisions

| Area | Decision |
|---|---|
| Order | Scan engine, menu bar shell, dashboard, briefs, distribution |
| Scan host | The Mac. The docker timer on `home` retires after parity |
| Production | The Python dashboard stays live until the Rust scan matches it |
| Commits | Local only. No push, and no merge to `main`, without approval |
| State | SQLite `data/state/jobhunt.db`. YAML stays the config |
| Briefs | DeepSeek-compatible chat, key in Keychain, off when absent |
| Platform | Apple Silicon macOS only |

## Known issues

| Symptom | Cause | Status |
|---|---|---|
| A finished scan reads `in progress` | The panel reads `finished_at` badly | Open, fix with the dashboard |
| A capture shows the first frame after a change | macOS defers drawing while the window is occluded | Not a defect. Activate the app first |
| `Scan now` ignores synthetic keys | The app needs Accessibility permission | Not a defect. Click it |
| A scan finds nothing | There are no sources yet | Expected until S4 |

## Roadmap

| Slice | Work | Done when |
|---|---|---|
| S4 | `simplify_json`, `jobbank`, `ats_boards` behind one trait | Each parser matches its saved response, and a live run writes postings |
| S5 | The seven JSON vendors, Workday plus sweep, `startups_gallery`, `workatastartup` | The cache and board health work, and the retry schedule matches Python |
| S6 | Pipeline, ledger, portal, health, retry, prune, probe, harvest | A Rust scan and a Python scan give the same ids and fields |
| S8 | Dashboard: stages, table, drawer, health, settings | Both tables show the same rows |
| S9 | Brief engine, bundle, signing, CI | A brief uses the current schema, and a tester can install the app |

Pipeline order: collapse requisitions, filter and tier, sort, split the ledger, write state. The
per-tier cap shortens a printed list only. It does not touch the dashboard.

`startups.gallery` now dates its cards with `<time datetime>`, so the Python source records no
publish date for them. The Rust parser must read that attribute. The Python source stays
unchanged on this branch.

## Where things are

| Path | Contents |
|---|---|
| `Makefile` | `run`, `check`, `import`, `db`, `reset-db`, and the Python targets |
| `crates/` | The six crates |
| `migrations/` | `0001_init.sql` |
| `fixtures/python/` | Parity cases and their generator |
| `fixtures/http/` | One saved response per source family, with provenance in its README |
| `fixtures/import_state/` | A state directory for the import command |
| `docs/rust-app-plan.md` | The design: schema, sources, UI, packaging, risks |
