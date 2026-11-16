# Job Hunt Rust App Plan

**Status:** Draft  
**Branch:** `rust-app`  
**Date:** 2026-11-13  
**Owner:** Muhan Li

> This document follows the principles of ASD-STE100 Simplified Technical English.
> The sentences are short. The words are simple. The document is not a certified ASD-STE100 document.

## 1. Purpose

This document gives the plan for a new macOS app. The app has a native user interface (UI)
and a Rust engine. The app replaces the current Python server and the web client for daily use.
The `main` branch stays unchanged. All work for the new app goes on the `rust-app` branch.

## 2. Scope

### 2.1 In scope

The first release has these parts:

- A native macOS app. The app uses GPUI and `gpui-component`.
- A Rust scan engine. The engine reads the job sources.
- A Rust brief engine. The engine makes fit briefs.
- SQLite for state.
- YAML for configuration.
- A one-time import from the current Python state.
- A menubar app that stays active.
- An ad-hoc-signed DMG for testers.

### 2.2 Out of scope for v1

The first release does not have these parts:

- Windows and Linux.
- The iCIMS source.
- The LinkedIn source.
- The web client.
- An HTTP API.
- The Mac App Store.
- Automatic updates.
- Notarization.
- Multiple user accounts.

## 3. Decisions

| Area | Decision |
|---|---|
| UI | Native GPUI and `gpui-component`. No web view. |
| Engine | Full Rust rewrite. No Python. |
| Configuration | YAML files in `data/config/`. |
| State | SQLite in `jobhunt.db`. |
| Transport | In-process channels. No HTTP. |
| Sources for v1 | Job Bank, Simplify JSON, Simplify HTML, ATS boards, JSON boards, Workday, startups.gallery, Work at a Startup. |
| Deferred sources | iCIMS and LinkedIn. |
| First milestone | Dashboard from imported data. Onboarding comes later. |
| Mac target | Apple Silicon (arm64). |
| Minimum macOS | The minimum that the pinned GPUI revision documents. |
| Distribution | Ad-hoc-signed DMG for testers. |
| Briefs | Bring your own key. Store the key in Keychain. Disable briefs when no key exists. |
| Branch | `rust-app`. The `main` branch stays unchanged. |

## 4. Terms

| Term | Meaning |
|---|---|
| App | The macOS application. |
| Engine | The Rust process that scans, reads state, and makes briefs. |
| UI | The GPUI windows and views. |
| Posting | One job advertisement. |
| Scan | One run of the sources. |
| Brief | The fit report for one posting. |
| Board | One employer job board. |
| Stage | One of the three lists: Open, Shortlisted, or Applied. |
| Config | A YAML file in `data/config/`. |
| State | Data in `jobhunt.db`. |
| Key | The model API key. |

## 5. Goals

The app must do these tasks:

1. Show the current postings in a native window.
2. Run a scan on a schedule.
3. Make a brief for one posting or for a batch of postings.
4. Track applied postings.
5. Show source health and board health.
6. Work without a model key.
7. Keep the current posting IDs.
8. Keep the current brief schema.

## 6. Non-goals

The first release does not add new sources. The first release does not change the brief prompts.
The first release does not support the old web client. The first release does not use Python.

## 7. Architecture

### 7.1 Process model

The app uses two processes:

- `jh-server` is the menu-bar agent. It owns the engine, the SQLite pool, the scheduler and the
  Unix socket.
- `jh-app` is the GPUI desktop window. It owns no database and no scheduler.

The two processes use a Unix socket at `data/state/jh.sock` (or `JH_SOCKET_PATH`). Each line is
one JSON object: the client writes a command, the server writes events. The server broadcasts
events to every client.

```
jh-app (GPUI)
  AppState
      ^ Event over Unix socket
      |
      v
jh-server (menu bar)
  - Database
  - Configuration
  - Scheduler
  - Scan service
  - Brief service
```

### 7.2 No HTTP

The app does not start an HTTP server. The app does not use bearer tokens. The app does not use
CORS. The menu-bar server and the desktop app are separate processes on the same machine, and
they talk over a Unix socket. Tests call the engine directly.

### 7.3 Command and event contract

The UI sends these commands:

| Command | Data | Result |
|---|---|---|
| `Scan` | `mode: quick or full` | Starts one scan. |
| `CancelScan` | none | Stops the current scan. |
| `MakeBrief` | `posting_id` | Starts one brief. |
| `MakeBriefs` | `posting_ids` | Starts a batch of briefs. |
| `Apply` | `posting_id` | Moves a posting to Applied. |
| `Unapply` | `posting_ids` | Moves postings out of Applied. |
| `SaveSettings` | settings data | Writes the settings. |
| `RefreshConfig` | none | Reads the YAML files again. |

The engine sends these events:

| Event | Data |
|---|---|
| `Dashboard` | A full dashboard snapshot. |
| `ScanStatus` | The current scan state. |
| `BriefUpdated` | One posting ID and its brief state. |
| `Error` | A message and a source. |

The engine must send a new dashboard snapshot after each write. The UI must not poll the database.

### 7.4 Menubar behavior

The app is a menubar app. The `Info.plist` file sets `LSUIElement` to `true`.
The app shows a status item in the macOS menu bar. The menu has these items:

- Open dashboard.
- Scan now.
- Settings.
- Quit.

When the user closes the window, the app stays active. The scheduler continues.
When the user selects Quit, the app stops the scheduler and exits.

### 7.5 Scheduler

The scheduler runs a scan at these local times:

- Monday to Friday at 09:00.
- Monday to Friday at 13:00.
- Monday to Friday at 17:00.

The scheduler also accepts a manual `Scan now` command.

The scheduler must not start two scans at the same time. A file lock on
`data/state/.scan.lock` prevents a second scan. The `jh-cli` tool uses the same lock.

The first scan uses a quick mode. A quick mode scans a small set of sources.
A full scan starts after the quick scan.

## 8. Configuration

Configuration stays in YAML files. The files are human-readable. The files are version-controlled.
The app reads the files at startup and on a `RefreshConfig` command.

| File | Purpose |
|---|---|
| `data/config/sources.yaml` | Source registry and network settings. |
| `data/config/filters.yaml` | Title rules, level rules, location tiers, age window, and salary floor. |
| `data/config/ats_boards.yaml` | Greenhouse, Ashby, and Lever boards. |
| `data/config/workday_boards.yaml` | Workday boards. |
| `data/config/json_boards.yaml` | Workable, SmartRecruiters, Rippling, BambooHR, Recruitee, Breezy, and Teamtailor boards. |
| `data/config/workday_tombstones.yaml` | Boards that the prune command removed. |
| `data/config/brief_stage_a_system.md` | Stage A prompt. |
| `data/config/brief_stage_b_system.md` | Stage B prompt. |
| `data/config/brief_stage_c_system.md` | Stage C prompt. |
| `data/config/source_candidates.md` | Source notes. |

The app reads candidate data from these files:

- `data/candidate/candidate_profile.yaml`
- `data/candidate/resume-text.md`
- `data/candidate/resume/*.pdf`

The app must not write secrets to a YAML file. The app stores the model key in Keychain.

The app writes YAML only for these tasks:

- Harvest a board list.
- Prune a dead board.
- Complete the onboarding wizard.

A YAML library can remove comments. The app must add a fixed header again after it writes a file.

## 9. State

### 9.1 Database

The app uses SQLite. The database file is `jobhunt.db`.
For development, the file is `data/state/jobhunt.db`.
For the packaged app, the file is in the application support directory.

The app must set these SQLite options:

```sql
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA foreign_keys = ON;
PRAGMA busy_timeout = 5000;
```

The app uses `sqlx` with the Tokio runtime. The app uses migrations in `migrations/`.

### 9.2 Schema

```sql
CREATE TABLE settings (
  key TEXT PRIMARY KEY,
  value_json TEXT NOT NULL,
  updated_at TEXT NOT NULL
) STRICT;

CREATE TABLE postings (
  id TEXT PRIMARY KEY,
  source TEXT NOT NULL,
  ats TEXT,
  raw_id TEXT,
  company TEXT NOT NULL,
  title TEXT NOT NULL,
  location TEXT,
  url TEXT,
  salary_raw TEXT,
  salary_annual_min INTEGER,
  salary_annual_max INTEGER,
  salary_estimated INTEGER NOT NULL DEFAULT 0,
  below_comp_floor INTEGER NOT NULL DEFAULT 0,
  posted TEXT,
  posted_derived INTEGER NOT NULL DEFAULT 0,
  age_days INTEGER,
  scanned_at TEXT,
  tier TEXT,
  score INTEGER,
  level_ii INTEGER NOT NULL DEFAULT 0,
  requires_us_work_auth INTEGER NOT NULL DEFAULT 0,
  keywords_json TEXT NOT NULL DEFAULT '[]',
  skills_have_json TEXT NOT NULL DEFAULT '[]',
  skills_gap_json TEXT NOT NULL DEFAULT '[]',
  emails_json TEXT NOT NULL DEFAULT '[]',
  verified_emails_json TEXT NOT NULL DEFAULT '[]',
  has_text INTEGER NOT NULL DEFAULT 0,
  first_seen_at TEXT NOT NULL,
  last_seen_at TEXT NOT NULL,
  expires_on TEXT,
  retired_at TEXT
) STRICT;

CREATE INDEX postings_live ON postings (retired_at, tier, posted);
CREATE INDEX postings_company ON postings (company);

CREATE TABLE descriptions (
  posting_id TEXT PRIMARY KEY REFERENCES postings (id) ON DELETE CASCADE,
  text TEXT NOT NULL,
  source TEXT,
  fetched_at TEXT
) STRICT;

CREATE TABLE seen (
  id TEXT PRIMARY KEY,
  reported INTEGER NOT NULL DEFAULT 0,
  first_seen_at TEXT NOT NULL,
  last_seen_at TEXT NOT NULL
) STRICT;

CREATE INDEX seen_reported ON seen (reported);

CREATE TABLE scan_runs (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  mode TEXT NOT NULL,
  status TEXT NOT NULL,
  started_at TEXT NOT NULL,
  finished_at TEXT,
  matched INTEGER,
  error TEXT,
  stats_json TEXT NOT NULL DEFAULT '{}'
) STRICT;

CREATE TABLE source_runs (
  scan_run_id INTEGER NOT NULL REFERENCES scan_runs (id) ON DELETE CASCADE,
  source TEXT NOT NULL,
  found INTEGER NOT NULL DEFAULT 0,
  kept INTEGER NOT NULL DEFAULT 0,
  seconds REAL NOT NULL DEFAULT 0,
  error TEXT,
  PRIMARY KEY (scan_run_id, source)
) STRICT;

CREATE TABLE briefs (
  posting_id TEXT PRIMARY KEY REFERENCES postings (id) ON DELETE CASCADE,
  status TEXT NOT NULL,
  engine TEXT,
  generated_at TEXT,
  started_at TEXT,
  payload_json TEXT NOT NULL,
  error TEXT
) STRICT;

CREATE TABLE brief_artifacts (
  posting_id TEXT NOT NULL REFERENCES briefs (posting_id) ON DELETE CASCADE,
  kind TEXT NOT NULL,
  payload_json TEXT NOT NULL,
  created_at TEXT NOT NULL,
  PRIMARY KEY (posting_id, kind)
) STRICT;

CREATE TABLE applied (
  posting_id TEXT PRIMARY KEY,
  applied_at TEXT NOT NULL,
  snapshot_json TEXT NOT NULL
) STRICT;

CREATE TABLE board_health (
  key TEXT PRIMARY KEY,
  vendor TEXT,
  slug TEXT,
  tenant TEXT,
  wd TEXT,
  site TEXT,
  status TEXT,
  consecutive_failures INTEGER NOT NULL DEFAULT 0,
  last_ok_at TEXT,
  last_checked_at TEXT,
  next_check_at TEXT,
  last_error TEXT,
  error_kind TEXT,
  updated_at TEXT NOT NULL
) STRICT;

CREATE INDEX board_health_retry ON board_health (status, next_check_at);

CREATE TABLE probe_failures (
  key TEXT PRIMARY KEY,
  streak INTEGER NOT NULL DEFAULT 0,
  last_error TEXT,
  updated_at TEXT NOT NULL
) STRICT;

CREATE TABLE harvested_boards (
  key TEXT PRIMARY KEY,
  source TEXT,
  payload_json TEXT NOT NULL,
  first_seen_at TEXT NOT NULL,
  last_seen_at TEXT NOT NULL
) STRICT;

CREATE TABLE cache (
  key TEXT PRIMARY KEY,
  value_json TEXT NOT NULL,
  updated_at TEXT NOT NULL
) STRICT;
```

The board lists stay in YAML. The `board_health` table holds only health and retry data.

### 9.3 Import mapping

The command `jh import --from ./data --db ./jobhunt.db` must be idempotent.

| Source | Destination |
|---|---|
| `portal_feed.jsonl` | `postings` |
| `last_scan.json` | `scan_runs` and `source_runs`; missing matched rows become retired `postings` |
| `descriptions.json` | `descriptions` |
| `seen.jsonl` | `seen` |
| `portal_expired.json` | `postings.retired_at` and `postings.expires_on` |
| `board_health.json` | `board_health` |
| `workday_sweep.json` | `cache['workday_sweep']` |
| `sg_boards.json` | `harvested_boards` |
| `ats_probe_failures.json` | `probe_failures` |
| `applied.json` | `applied` |
| `briefs/*.json` | `briefs` |
| `briefs/*.raw.json` | `brief_artifacts` with kind `raw` |
| `briefs/*.agent.json` | `brief_artifacts` with kind `agent` |
| `briefs/*.review.json` | `brief_artifacts` with kind `review` |

The config YAML files and the candidate files are not imported. The app reads them directly.

### 9.4 Data rules

- A posting ID must be `sha1(key)[:16]`.
- The key order must match `server/scan/posting.py`.
- The board health key must match the Python key format.
- A brief must use the current JSON schema.
- The app must write state in a transaction.
- The app must use ISO 8601 dates.
- The app must use UTF-8.

The key order is:

1. `dedupe_key` when the source gives one.
2. `source|raw_id` when the source gives a raw ID.
3. `company|title|location`, in lowercase, when the source gives no ID.

## 10. Engine design

### 10.1 Network guard

The engine must use one HTTP client. The client must have a custom DNS resolver.
The resolver must have:

- A TTL cache.
- A negative cache.
- A token bucket.
- A circuit breaker.
- Stale-on-error behavior.

The engine must also limit the number of open sockets. The default limit is 32.
The limit comes from `sources.yaml`.

The engine must use these rules for a DNS error:

- Do not retry a DNS error immediately.
- Stop requests for a short time after a series of temporary DNS errors.

### 10.2 Scan pipeline

A scan must do these steps in order:

1. Load the configuration.
2. Load the ledger.
3. Start the sources in parallel.
4. Collect the postings.
5. Normalize each posting.
6. Filter each posting.
7. Score each posting.
8. Assign a location tier.
9. Apply the per-tier cap.
10. Sort the postings.
11. Collapse duplicate requisitions.
12. Write the scan run.
13. Write the postings.
14. Update the ledger.
15. Update the portal feed.
16. Write the board health.
17. Write the cache.

A source must implement one trait:

```rust
#[async_trait]
trait Source {
    fn id(&self) -> &'static str;
    async fn fetch(&self, context: &ScanContext) -> Result<Vec<Posting>>;
}
```

### 10.3 Board state

The app must keep these values for each board:

- Status: `ok`, `transient`, `structural`, or `broken`.
- Consecutive failures.
- Last success time.
- Last check time.
- Next check time.
- Last error.
- Error kind.

The retry schedule must match the Python schedule:

- Transient: 1 day, 3 days, 14 days.
- Structural: 7 days, 30 days.
- Broken: 30 days.

### 10.4 Brief engine

The brief engine must use the three-stage chain:

1. Stage A: extract the requirements.
2. Stage B: match the requirements to the resume.
3. Stage C: write the brief and the resume edits.

The validator must drop an edit when the `before` text is not in the resume.
The validator must flag a term that is not in the resume.

The app must not use the Hermes agent. The app must not run the deep contact search.
The contact tab shows email addresses from the scan.

### 10.5 Model key

The app stores the model key in Keychain. The app keeps a copy of the key in memory.
The app must not write the key to the database. The app must not write the key to a log.

When no key exists:

- The app must show the scan and the dashboard.
- The app must disable the brief buttons.
- The app must show a message that tells the user to add a key.

## 11. Sources

| Order | Source | Difficulty | Notes |
|---|---|---|---|
| 1 | `simplify_json` | Low | JSON feed. It has an active flag and exact dates. |
| 2 | `jobbank` | Medium | HTML search and detail pages. It has salary data. |
| 3 | `ats` | Low | Greenhouse, Ashby, and Lever JSON APIs. |
| 4 | `jsonboards` | Medium | Seven vendors. Some boards have no date. |
| 5 | `workday` | High | It throttles by IP. It has relative dates and a page-two cursor. |
| 6 | `startups_gallery` | Low | Framer HTML. It reveals employer boards. |
| 7 | `workatastartup` | Medium | Inertia JSON. It needs a browser-shaped `Accept` header. |

The first release must not include iCIMS or LinkedIn.
iCIMS needs browser TLS impersonation. LinkedIn has terms-of-service risk.

## 12. UI

### 12.1 Views

The app has these views:

- **Dashboard.** Three stages: Open, Shortlisted, Applied.
- **Detail drawer.** Summary, Resume, and Contact tabs.
- **Health.** Source time, source counts, and board health.
- **Settings.** Model key, sources, schedule, and data directory.
- **Onboarding.** Resume import and profile form. This view comes later.

### 12.2 Table

The table must have four columns:

- Date.
- Location.
- Company.
- Title.

The table must have two filter levels:

1. The stage. The tab bar controls the stage.
2. The location tier and the search text.

The table must support these page sizes:

- Fit to window.
- 10 rows.
- 25 rows.
- 50 rows.

The app must measure the free space for the fit page size. The app must calculate the row count
from the table height.

### 12.3 Theme

The app must use the current Lilac theme. The app must support a dark mode and a light mode.
The app must store the theme in the `settings` table.

The app must use a sans font for text and a monospace font for dates and code.
The first build can use system fonts. A later build can bundle Outfit and Cascadia Mono.

### 12.4 Keyboard

The app must support these shortcuts:

- `Cmd+R`: refresh.
- `Cmd+F`: focus the search field.
- `Cmd+,`: open settings.
- `Esc`: close the drawer.
- `Cmd+Q`: quit.

### 12.5 Empty and error states

The app must show a clear message for these states:

- No scan has run.
- The database is empty.
- No model key exists.
- A scan is running.
- A brief failed.
- A source failed.

## 13. GPUI spike

The team must complete a spike before the UI work starts. The spike has a time limit of 5 days.
The spike must show these items:

1. A window with the pinned GPUI revision.
2. A `gpui-component` table with 500 rows.
3. A text input.
4. A modal dialog.
5. A menu bar status item.
6. A light/dark theme switch.

If `gpui-component` does not compile with the pinned GPUI revision, the team must use
`uniform_list` for the table. The team can use `gpui-component` for the other widgets.

The team must record the pinned revisions in `Cargo.toml` and in this document.

## 14. Milestones

### M0: Foundation and UI spike

**Estimate:** 1 to 2 weeks.

Work:

1. Create the `rust-app` branch.
2. Create the workspace and the crates.
3. Write `migrations/0001_init.sql`.
4. Write the import command.
5. Write the configuration loader.
6. Write the dashboard query.
7. Complete the GPUI spike.
8. Show the Open table from the database.
9. Add unit tests for the table logic.
10. Write `docs/rust-app-plan.md`.

**Acceptance:** The app opens. The Open table shows the imported postings.
The filters, sorting, and paging work. The detail drawer shows the cached description.
The menu bar item works.

### M1: Dashboard parity

**Estimate:** 2 to 3 weeks.

Work:

1. Add the Shortlisted and Applied stages.
2. Add the health view.
3. Add the settings view.
4. Add Keychain support.
5. Add the scan status view.
6. Add a minimal scan with `simplify_json` and `jobbank`.
7. Connect `Scan now`.

**Acceptance:** The user can do daily work without the current web dashboard.

### M2: Scan engine

**Estimate:** 4 to 6 weeks.

Work:

1. Add the network guard.
2. Add the pipeline.
3. Add the ledger and the portal feed.
4. Add the board state.
5. Add the sources in the order in Section 11.
6. Add harvest, sweep, retry, prune, and probe.
7. Add golden tests.

**Acceptance:** A Rust scan and a Python scan produce the same IDs and the same postings
for the ported sources on the same day.

### M3: Brief engine

**Estimate:** 2 to 3 weeks.

Work:

1. Add the model client.
2. Add the three stages.
3. Add the validator.
4. Add the chain.
5. Add batch briefs.
6. Add key gating.

**Acceptance:** A brief uses the same schema as the current brief.
The validator drops an edit when the `before` text is not in the resume.

### M4: Onboarding and distribution

**Estimate:** 3 to 4 weeks.

Work:

1. Add the PDF import.
2. Add the profile form.
3. Make the filters and the sources from the profile.
4. Add the scheduler and the login item.
5. Build the app bundle.
6. Ad-hoc sign the app.
7. Build the DMG.
8. Add macOS CI.

**Acceptance:** A new user can install the app, complete onboarding, run a scan,
read a posting, and make a brief.

### M5: Polish

**Estimate:** 2 to 4 weeks.

Work:

1. Add accessibility.
2. Add a diagnostics view.
3. Add import of the current data.
4. Test on the minimum macOS version.
5. Remove the old code from the branch.
6. Update the documentation.

## 15. Packaging

### 15.1 Target

- `aarch64-apple-darwin`.
- Apple Silicon only.

### 15.2 Minimum macOS

The app must use the minimum macOS version that the pinned GPUI revision documents.
The team must read the pinned revision. The team must test on that version.

### 15.3 Bundle

```
JobHunt.app/
  Contents/
    Info.plist
    MacOS/JobHunt
    Resources/
      AppIcon.icns
      fonts/
```

The `Info.plist` must set these keys:

- `CFBundleIdentifier`: `com.muhanli.jobhunt`
- `CFBundleName`: `Job Hunt`
- `CFBundleShortVersionString`
- `CFBundleVersion`
- `LSUIElement`: `true`
- `LSMinimumSystemVersion`
- `NSHighResolutionCapable`: `true`

### 15.4 Signing for testers

The first build uses an ad-hoc signature. The build script must sign the app from the inside out.
Testers must right-click the app and select Open on the first start.

The build script must make a DMG with `hdiutil`.

### 15.5 Login item

The first build uses a LaunchAgent. The file is
`~/Library/LaunchAgents/com.muhanli.jobhunt.plist`.
The agent must set `RunAtLoad` to `true` and `KeepAlive` to `false`.

A later build can use `SMAppService`.

### 15.6 Later signing

A later build must use a Developer ID certificate. The build must use the hardened runtime.
The build must notarize the app with `notarytool`. The build must staple the ticket.

## 16. Testing

### 16.1 Unit tests

The tests must cover these parts:

- The configuration loader.
- The posting ID function.
- The filter and score functions.
- The table filter, sort, and page functions.
- The date parsers.
- The brief validator.

### 16.2 Golden tests

The team must make a snapshot of the current Python state. The snapshot goes in `fixtures/python/`.
The snapshot has these files:

- `portal_feed.jsonl`
- `last_scan.json`
- `descriptions.json`
- `seen.jsonl`
- `board_health.json`
- `applied.json`
- `briefs/`

The import test must load the snapshot. The test must compare the database rows with the source values.

### 16.3 Source tests

Each source must have a saved page or a saved JSON response.
The test must run the parser on the saved data.
The test must compare the postings with the expected postings.

### 16.4 Database tests

The tests must cover these cases:

- The import is idempotent.
- A transaction rolls back on an error.
- A retired posting stays in the database.
- An applied posting survives a feed retirement.
- A brief stays after the posting retires.

### 16.5 UI tests

The tests must cover these cases:

- The table shows the correct rows for each stage.
- The filters work.
- The pager works.
- The drawer opens and closes.
- The brief button is disabled when no key exists.

### 16.6 CI

The CI runs on `macos-14`. The CI must do these steps:

1. Install the Rust toolchain.
2. Build the workspace.
3. Run the tests.
4. Build the app bundle.
5. Ad-hoc sign the app.
6. Build the DMG.

The CI must upload the DMG as an artifact.

## 17. Risks

| Risk | Impact | Mitigation |
|---|---|---|
| GPUI and `gpui-component` are pre-1.0. | The API can change. | Pin both revisions. Run the spike first. Keep the table code separate. |
| A source changes its HTML or JSON. | A source finds no postings. | Add fixtures. Compare with the Python output. Show source health. |
| The Workday source throttles the IP. | Boards do not answer. | Keep the worker limit at 16. Use the retry schedule. Show the failure count. |
| SQLite writes block the UI. | The app stops. | Use WAL. Use a pool. Keep the UI on the main thread. Send snapshots. |
| The posting ID changes. | Applied and brief rows orphan. | Port the ID function exactly. Add a test with known IDs. |
| A YAML writer removes comments. | The config loses documentation. | Write a fixed header. Keep the generated lists short. |
| The model key leaks. | The user pays for unwanted calls. | Store the key in Keychain. Keep it in memory. Do not log it. |
| The app cannot be notarized. | Testers must use right-click Open. | Start with ad-hoc signing. Add notarization later. |
| The team cannot build on Linux. | No local macOS build. | Use a Mac or GitHub Actions `macos-14`. |

## 18. Deferred items

These items come after the first release:

- Onboarding wizard.
- iCIMS source.
- LinkedIn source.
- Automatic updates.
- Notarization.
- Intel support.
- Contact research.
- Web client.
- Mobile app.

## 19. First actions

The team must do these actions first:

1. Create the `rust-app` branch from `main`.
2. Create the workspace.
3. Add `rust-toolchain.toml`.
4. Add `migrations/0001_init.sql`.
5. Add the `jh-db` crate.
6. Add the `jh-cli import` command.
7. Add the `jh-core` configuration loader.
8. Add the `jh-core` dashboard query.
9. Run the GPUI spike.
10. Add the `jh-app` Open table.
11. Add the tests.
12. Commit the first slice.

## 20. Reference map

| Python file | Rust crate | Notes |
|---|---|---|
| `server/config.py` | `jh-core` | Paths and constants. |
| `server/scan/core.py` | `jh-scan` | HTTP and YAML helpers. |
| `server/scan/net.py` | `jh-scan` | DNS guard. |
| `server/scan/posting.py` | `jh-core` | Posting IDs and collapse. |
| `server/scan/filters.py` | `jh-core` | Location and title filters. |
| `server/scan/pipeline.py` | `jh-scan` | Score and sort. |
| `server/scan/ledger.py` | `jh-scan` | Ledger. |
| `server/scan/portal.py` | `jh-scan` | Portal feed. |
| `server/scan/boardstate.py` | `jh-scan` | Board health. |
| `server/scan/engine.py` | `jh-engine` | Scan orchestration. |
| `server/scan/sources/*.py` | `jh-scan/sources` | One module for each source. |
| `server/scan/keywords.py` | `jh-core` | Keyword extraction. |
| `server/brief/llm.py` | `jh-brief` | Model client. |
| `server/brief/stages.py` | `jh-brief` | Three stages. |
| `server/brief/validate.py` | `jh-brief` | Edit checks. |
| `server/brief/chain.py` | `jh-brief` | Chain. |
| `server/api/store.py` | `jh-core` | Dashboard query. |
| `server/api/triage.py` | `jh-core` | Applied list. |
| `server/api/briefs.py` | `jh-core` | Brief cache. |
| `server/jobs/harvest/*.py` | `jh-scan/harvest` | Board harvest. |
| `server/jobs/retry/*.py` | `jh-scan/retry` | Retry and prune. |
| `server/jobs/probe_ats.py` | `jh-scan/probe` | ATS probe. |
| `server/jobs/scan/sweep.py` | `jh-scan/sweep` | Workday sweep. |
| `server/tests/*.py` | `jh-*` tests | Smoke tests. |
