# Kangaroo Rust App

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
| `kangaroo-core` | Posting id and dedupe key, apply signatures, filters, score and order, parsing, keywords, config | 11 |
| `kangaroo-db` | SQLite pool, migrations, import | — |
| `kangaroo-protocol` | The command, event and dashboard types shared by both processes | — |
| `kangaroo-scan` | Egress guard, the source trait, and one module per feed | 33 |
| `kangaroo-engine` | Engine thread, scheduler, scan lock, scan pipeline, ledger, board health, brief/apply commands, snapshot, one-posting detail | 13 |
| `kangaroo-brief` | Three-stage brief chain, validator, SQLite brief rows | 5 |
| `kangaroo-cli` | `import`, `scan`, `sweep`, `retry`, `prune`, `probe-ats`, `harvest` | — |
| `kangaroo-server` | Menu-bar agent: engine owner, SQLite owner, Unix-socket server | — |
| `kangaroo-app` | GPUI desktop client: stages, table, tick column, detail panel, scan controls, appearance setting | 19 |

82 tests pass. `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` are clean.

## Run it

```sh
make run        # build both binaries and launch the desktop app
make run-server # run the menu-bar server in the foreground
make check      # format check, clippy, every test
make import     # load fixtures/import_state into data/state/kangaroo.db
make db         # show the contents
make reset-db   # delete the database
```

The server reads and writes `data/state/kangaroo.db` in the current directory. Set
`KANGAROO_DB_PATH` to move it and `KANGAROO_SOCKET_PATH` to move the Unix socket. The app starts a
sibling `kangaroo-server` when none answers, then talks to it over that socket.

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

The macOS status item has no GPUI form, so `kangaroo-server` writes it directly against AppKit
through `objc2`. AppKit retains neither the item nor its target, so both live in `thread_local`
storage. The item shows the kangaroo mark, not a word: `assets/kangaroo-menubar.png` as a template
image, which the menu bar tints for the light and dark themes.

## The mark

`assets/make-icons.py` draws both images from one set of shapes, and the two binaries embed the
results at build time. Re-run it after a shape change.

| File | Use |
|---|---|
| `assets/kangaroo-menubar.png` | The status item: 40 pixels, black on transparent, a template image at 22 points |
| `assets/kangaroo-app.png` | The dock tile: 1024 pixels, dark kangaroo on a warm gradient, on the macOS icon grid |

The dock draws the image into a fixed slot, so the tile size sets the size on screen. The tile is
856 of the 1024 pixels, which matches the other icons in the dock pixel for pixel. A full-bleed
tile reads as oversized next to them.
The status item asks for 22 points and falls back to `ScaleProportionallyDown`, so the glyph
fills the button and never clips.

The binaries are plain executables and not an `.app` bundle, so the dock tile comes from
`NSApplication::setApplicationIconImage` in `kangaroo-app` (`src/brand.rs`). It is not a
`CFBundleIconFile`.

## Architecture

Two processes. `kangaroo-server` is a menu-bar agent: it owns the engine, the SQLite pool and the
scheduler. `kangaroo-app` is a GPUI window and never touches the database.

```
kangaroo-app (GPUI) ──Command──► Unix socket ──► kangaroo-server (Tokio engine) ──► SQLite
kangaroo-app (GPUI) ◄──Event──── Unix socket ◄── kangaroo-server (Tokio engine)
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

One page in three parts. The title bar carries the wordmark, the current section, the scan state
and the scan button; the app draws it, and AppKit keeps only the traffic lights, at the left.
The navigation column groups the three pipeline stages under Pipeline and the two system pages
under System, with the count at the end of each stage row. The content area starts with a page
title and one line of summary, then the table card or the health and settings cards.

The table draws date, location, company and title, cut down by the search field and the location
filter, and the date reads as a day: Today, Yesterday, 17 Nov. A row click opens the detail panel
on the right; `Esc` closes it. The title column takes the width the other
columns leave, so the table fills its card at any window size, and the fixed columns tighten
when the drawer takes 400 points.

The page size follows the window height: the app divides the space left by the chrome by the
40-point row height and shows that many rows, so the table never ends in blank space. Pagination
appears only when the stage has more rows than the window holds.

In the Open stage a column of ticks marks rows for a batch brief. `Brief` sends one `MakeBriefs`
for the ticked rows and clears the marks. A tick is drawn by the app, not by the table: the
table's own selection is one row, which is the row the panel shows. A finished brief moves the
posting to Shortlisted, so the mark goes with it.

The drawer puts the posting's title and its chips at the top, the facts and the posting text in
a scrolling body, and the three actions in a footer that stays in view: `Open`, `Brief` and
`Mark applied`. A brief sits above the facts: the verdict first, then the skills it names, the
proposed resume edits, and the gaps.

The Health page shows the scan state, the board counts, the database path and the posting
counts. The Settings page shows the model-key state and the shortcuts the window answers to.
Both are read-only until the real forms exist.

`--settings` and `--health` open the window on that page, and `--open-first` opens the drawer on
the top posting once the first snapshot arrives. The three exist for screenshots and manual
checks.

### The palette, in two tints

The window is Slate: `#F7F7F7` page, `#FFFFFF` card, `#E4E4E4` hairline, `#171717` text,
`#6B6B6B` muted text, and `#3F7D73` teal for actions, with `#B4793A` amber as the second voice.
`src/theme.rs` holds the same six colours twice, once for a light page and once for a dark one, and
derives every other token by mixing two of them.

| | Light | Dark |
|---|---|---|
| Page | `#F7F7F7` | `#141414` |
| Card | `#FFFFFF` | `#1C1C1C` |
| Hairline | `#E4E4E4` | `#2E2E2E` |
| Text | `#171717` | `#EDEDED` |
| Muted text | `#6B6B6B` | `#9A9A9A` |
| Accent | `#3F7D73` | `#4F9A8E` |
| Second accent | `#B4793A` | `#C98F4E` |
| On the accent | `#FFFFFF` | `#0B1412` |

The dark tint is not the light one inverted. The teal is lifted, because the light value loses
contrast on near-black; the accent's label turns near-black, because white on the lifted teal
would not read; and every mix weight gains a third, because a hairline that reads on white
disappears on black. A test holds both tints to the same floors: body text at AAA, muted text and
the accent's label at AA.

The same derivation gives both: the table header is the card pulled 35% toward the hairline, the
row a reader picked is the card pulled 12% toward the accent, the sidebar is the page pulled 35%
toward the hairline. Two places needed a rule of their own, because they are surfaces rather than
tints: an unticked checkbox takes the muted text colour on a dark ground, and the active
navigation row takes the accent tint there instead of the white card it uses in the light.

#### Where each accent goes

The first accent marks what the reader can press and where they are, and nothing else:

- The primary buttons: `Scan` in the title bar, `Brief` in the table header.
- The active navigation row: a two-point mark at its left edge, its icon, and its count.
- A ticked row's checkbox, and the border of the row the table has selected.
- An empty state's icon, at 55% opacity.
- Focus rings, the text caret and links.

Headings take the text colour, not the accent, so the window does not read as a brand exercise.
The second accent has two jobs and no more: the Shortlist row in the navigation, and the brief it
holds — the word `BRIEF` and the skills it names. The four status colours carry meaning rather
than taste, so each tint states them rather than deriving them.

The tint is chosen in Settings, under Appearance: two buttons, the current one filled. The choice
is written to the `settings` table through `SaveSettings`, and the dashboard snapshot carries the
stored settings back, so the next launch opens in the same tint. `--tint light|dark` overrides the
stored value for one run, which keeps screenshots and manual checks predictable.

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
`com.muhanli.kangaroo.deepseek`, or `~/.hermes/auth.json`, in that order. With no key the brief
buttons are gated and a run records `status: failed` rather than calling the model.

## Decisions

| Area | Decision |
|---|---|
| Processes | Menu-bar server owns the engine; the GPUI app is a socket client |
| Scan host | The Mac. The docker timer on `home` retires after parity |
| Production | The Python dashboard stays live until the Rust scan matches it |
| Commits | Local only. No push, and no merge to `main`, without approval |
| State | SQLite `data/state/kangaroo.db`. YAML stays the config |
| Briefs | DeepSeek-compatible chat, key in Keychain, off when absent |
| Platform | Apple Silicon macOS only |

## Known issues

| Symptom | Cause | Status |
|---|---|---|
| A capture shows the first frame after a change | macOS defers drawing while the window is occluded | Not a defect. Activate the app first |
| `Scan now` ignores synthetic keys | The app needs Accessibility permission | Not a defect. Click it |
| A full scan hits ~2,000 boards | That is the real Workday and JSON-vendor fleet | Expected; the sweep cache removes Workday from the daily path |
| `kangaroo-server` has no login item yet | Packaging is not started | Open, S9 |

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
