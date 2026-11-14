# Job Hunt Rust App

**Branch:** `rust-app`
**Date:** 2026-11-13
**Status:** engine and menubar shell working; sources, dashboard and briefs still to come

This branch is a native macOS app that replaces the Python server and the web dashboard.
The engine is Rust. The UI is GPUI with `gpui-component`. State is SQLite. Configuration
stays in the YAML files under `data/config/`.

The `main` branch is untouched and still runs the production job search on `home`. Nothing
here is pushed and nothing here changes production.

The design is in `docs/rust-app-plan.md`. Where the design and this report disagree, the code
and this report win, because both describe what actually runs.

## 1. What exists today

| Crate | Role | Tests |
|---|---|---|
| `jh-core` | Posting identity and dedupe key, ATS apply signatures, title/location filters, scoring and ordering, salary and date parsing, config loader | 7 parity tests against Python |
| `jh-db` | SQLite pool, migrations, import | — |
| `jh-cli` | `jh import --from DIR --db FILE`, idempotent | — |
| `jh-scan` | The egress guard: resolver cache, negative cache, single-flight, token bucket, breaker, stale-on-error, socket cap, DNS no-retry | 17 parity tests |
| `jh-engine` | Engine thread, command/event channels, scheduler, scan lock, stub scan, dashboard snapshot | 10 tests |
| `jh-app` | Window, status panel, scan controls, status item, theme switch, keyboard | — |

34 tests pass. `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` are clean.

What the app does now: it starts, creates and migrates its database, shows engine and scan
state, schedules 09:00 / 13:00 / 17:00 on weekdays, takes an exclusive scan lock, and runs a
stub scan that writes one `scan_runs` row. It has no sources yet, so a scan finds nothing.

## 2. Build and run

```sh
make help          # every target
make run           # build and launch the app
make check         # format check, clippy, every test
```

The app reads and writes `data/state/jobhunt.db` in the current directory. Set `JH_DB_PATH` to
point somewhere else. To see the panel with data in it:

```sh
make import        # load fixtures/import_state into the dev database
make db            # show what it holds
make run
```

`make reset-db` deletes the dev database, so the next start creates an empty one. The raw cargo
equivalents work too: `cargo run -p jh-app`, `cargo test --all`, `cargo clippy --all-targets --
-D warnings`.

## 3. Machine facts

- **Rust 1.98.1**, pinned in `rust-toolchain.toml`. Required, not preferred: `gpui-pre` uses
  `Result::map_or_default`, which is unstable in 1.97.
- **Full Xcode, not Command Line Tools.** GPUI compiles Metal shaders in a build script and
  `/usr/bin/metal` ships only in Xcode.app. `.cargo/config.toml` sets `DEVELOPER_DIR` and does
  not override an existing value.
- **Minimum macOS 12.0**, read from the pinned GPUI source, not from prose. Newer paths are
  optional and fall back.
- Workers run cargo with `CARGO_NET_OFFLINE=true`; fetch dependencies once with `cargo fetch`.

## 4. Pins

| Crate | Package | Version | Revision |
|---|---|---|---|
| `gpui` | `gpui-pre` | 0.3.12 | zed `d2a4fb6ded7c470be25d1541281da3882495b65b` |
| `gpui_platform` | `gpui-pre-platform` | 0.3.12 | same snapshot |
| `gpui_component` | `gpui-component` | 0.9.1 | `longbridge/gpui-kit` `39ab849` |

`gpui-component` compiles against the pinned `gpui`; no `uniform_list` fallback is needed.
The macOS status item has no GPUI equivalent, so it is written against AppKit through `objc2`,
with the status item and its action target held in `thread_local` storage because AppKit does
not retain either.

## 5. Architecture

One process, two threads. GPUI owns the main thread; the engine owns a Tokio runtime on its own
thread. They talk over two `flume` channels. The engine owns the SQLite pool; the UI never
touches the database.

```
UI (main thread)  --Command-->  Engine (Tokio thread)  --> SQLite
UI (main thread)  <--Event----  Engine (Tokio thread)
```

Commands: `Scan`, `CancelScan`, `Snapshot`, `RefreshConfig`, `MakeBrief`, `MakeBriefs`,
`Apply`, `Unapply`, `SaveSettings`. Events: `Dashboard`, `ScanStatus`, `BriefUpdated`, `Error`.
Only `Scan`, `CancelScan`, `Snapshot` and `RefreshConfig` are implemented; the rest answer
`Error { kind: NotImplemented }` rather than panic.

The engine emits a fresh `Dashboard` after every write. `flume` retains no value, so the view
sends `Snapshot` once it has subscribed; without that, the first snapshot is lost and the panel
sits on `Engine: Starting`.

A scheduled slot runs sweep → scan → mark-reported in the Python system. The Rust engine has
the scheduler and the lock; the three steps land with the sources.

## 6. Parity contract

These must not drift, or postings re-report and applications orphan:

- Posting id: `sha1(key)[:16]`, lowercase hex.
- Key order: `dedupe_key`, else `source|raw_id`, else lowercase `company|title|location`.
- `apply_signature`: the fourteen patterns and their templates, copied from
  `server/scan/posting.py`. Greenhouse, Ashby and Lever key on `ats|{id}`; the JSON vendors and
  iCIMS put the board in the key, because their ids are scoped to one board.
- Sort: tier rank, then descending score, then age, undated last.
- Board health keys: Workday is `tenant|wd|site`; a JSON vendor is `vendor|slug`.
- Dates: ISO 8601, UTC, seconds.

`fixtures/python/` holds cases generated from the Python modules by
`fixtures/python/generate.py`. Regenerate with `python3 fixtures/python/generate.py`. A change
in Python behaviour must show up as a failing Rust test, never as a silently edited fixture.

## 7. Decisions

| Area | Decision |
|---|---|
| Order of work | Scan engine, then menubar shell, then dashboard, then briefs, then distribution |
| Scan host in v1 | The Mac; the docker timer on `home` retires after parity |
| Production | `main` and the Python dashboard stay live until the Rust scan matches it |
| Commits | Local only; no push and no merge to `main` without the owner's approval |
| State | SQLite `data/state/jobhunt.db`; YAML stays the configuration |
| Briefs | DeepSeek-compatible chat completions, key in Keychain, disabled when absent |
| Platform | Apple Silicon macOS only |

## 8. Known issues

| Symptom | Cause | Status |
|---|---|---|
| A finished scan still reads `in progress`. | The panel keys that text off `finished_at` badly. | Open, fix with the dashboard |
| A window capture of the app shows the first frame after a state change. | macOS defers drawing while the window is occluded. | Not a defect; activate the app before judging a repaint |
| `Scan now` cannot be driven by synthetic key events. | The app needs Accessibility permission first. | Not a defect; click it |
| A scan finds nothing. | There are no sources yet. | Expected until S4 |

## 9. Roadmap

| Slice | Work | Done when |
|---|---|---|
| S4 | `simplify_json`, `jobbank`, `ats_boards` behind one `Source` trait | Each parser matches its saved response in `fixtures/http/`; a live run writes postings |
| S5 | `jsonboards` (seven vendors), `workday` plus sweep, `startups_gallery`, `workatastartup` | The Workday cache and per-board health work; the retry schedule matches Python |
| S6 | Pipeline, ledger, portal enrichment, board health, retry, prune, probe, harvest | A Rust scan and a Python scan on the same day give the same ids and fields |
| S8 | Dashboard: three stages, table, drawer, health, settings | The Rust table and the web table show the same rows |
| S9 | Brief engine, then bundle, signing, DMG, CI | A brief uses the current schema; a tester can install and run it |

Pipeline order, for reference: collapse requisitions → filter and tier → sort → ledger split →
write state. The per-tier cap only shortens a printed list; it does not touch the dashboard.

`startups.gallery` now dates its cards with `<time datetime>` instead of `Posted on …`, so the
Python source records no publish date for them. The Rust parser must read the attribute. The
Python source is not changed on this branch.

## 10. Where things are

| Path | Contents |
|---|---|
| `Makefile` | Local targets: `run`, `check`, `import`, `db`, `reset-db`, and the Python ones |
| `crates/` | The six crates |
| `migrations/` | `0001_init.sql` |
| `fixtures/python/` | Parity cases and their generator |
| `fixtures/http/` | One saved response per source family, with provenance in its README |
| `fixtures/import_state/` | A state directory for the import command and the app |
| `docs/rust-app-plan.md` | The design: schema, sources, UI, packaging, risks |
