# Job Hunt Rust App: Implementation Plan

**Status:** Active
**Branch:** `rust-app`
**Date:** 2026-11-13
**Owner:** Muhan Li
**Orchestrator:** Hermes

This document is the working plan. `docs/rust-app-plan.md` holds the design. Where the two
disagree, this document wins, because it records what the code does today.

Sentences are short. Words are simple. The document is not a certified ASD-STE100 document.

## 1. Order of work

The candidate changed the order on 2026-11-13:

1. Build the Rust scan engine first.
2. Wrap it in a macOS menubar app.
3. Build the dashboard after that.
4. Build the brief engine last.

The reason: the scanner must become a menubar item on the Mac. The dashboard is useful only
when the scanner writes real state.

This order replaces the order in `docs/rust-app-plan.md` §14.

## 2. Decisions

| Area | Decision | Source |
|---|---|---|
| Order | Scan, menubar, dashboard, brief, distribution. | Candidate |
| Scan host in v1 | The Mac. The docker timer on `home` is retired after parity. | Candidate |
| Python dashboard | Stays live on `home` until the Rust dashboard works. | Orchestrator |
| Cutover | When a Rust scan matches a Python scan for the same day. | Orchestrator |
| Commits | One commit per slice, on a slice branch. No push. No merge to `main` without approval. | Candidate |
| Parallelism | 2 to 3 slices in separate git worktrees. One writer per worktree. | Candidate |
| Workers | opencode (`deepseek/deepseek-v4-pro`) for large slices. opencode (`opencode/iceman-free`) for small, well-scoped tasks. | Candidate |
| Platform | Apple Silicon macOS only. | Design doc |
| State | SQLite in `jobhunt.db`. YAML stays the configuration. | Design doc |
| Brief provider | DeepSeek, OpenAI-compatible. Base URL and model are settings. | Orchestrator |
| Candidate data | Copy from `home` when a slice needs it. Both data directories are gitignored. | Candidate |
| Worktrees | `.worktrees/<slice>` under the repo root. The directory is gitignored. | Orchestrator |

## 3. Corrections to the design doc

1. The `rust-app` branch already exists. Do not create it again.
2. The scan pipeline order in §10.2 is wrong. The code does:
   `collapse_requisitions` → `normalize_and_filter` → `sort_key` → ledger split → write state.
   The per-tier cap only shortens the printed list. It does not affect the dashboard.
3. Portal enrichment is separate work. It fetches descriptions (6,000 characters, 250 postings
   per run, 8 workers), extracts resume keywords, extracts and verifies email addresses, reads
   the outreach log, and expires rows after 7 days.
4. Python keeps writing state until parity. Rust scans run in shadow mode and write no state.
   Both processes take `data/state/.scan.lock`.
5. A scheduled slot has three steps, not one: sweep if stale, scan, then mark reported. A failed
   sweep is not fatal. A missed slot does not catch up.
6. `filter_order`, `max_age_days` and the comp floor are data. Read them from `filters.yaml`
   and reload them on a config refresh. Do not hard-code them.
7. Fixtures are synthetic. `data/state/` and `data/candidate/` hold personal data and are
   gitignored. Never copy them into the repository.
8. The bundled app must ship default YAML in `Resources/`. The packaged app reads Application
   Support. Development reads the repository `data/`.
9. The brief client is an OpenAI-compatible `chat/completions` call with JSON mode. The default
   model is `deepseek-flash`. The key lives in Keychain. Never log it.

## 4. Slices

Each slice is one branch and one commit. A slice is done when its acceptance test passes.
Estimates are agent-days, not calendar days.

### S1: Foundation, parity core and import

**Crates:** `jh-core`, `jh-db`, `jh-cli`

**Work:**

1. Create the Cargo workspace and `rust-toolchain.toml`.
2. Write `migrations/0001_init.sql` from the design doc §9.2.
3. Port the posting identity: `apply_signature`, `collapse_requisitions`, `make_posting`, the
   dedupe key order, and `sha1(key)[:16]`.
4. Port the filters: `title_matches`, `is_level_ii`, `location_matches`, `region_signals`,
   `_tier_lookup`, `normalize_and_filter`, `score`, `sort_key`, `cap_per_tier`, `tier_counts`.
5. Port the parsing helpers: `parse_salary`, date parsing, `strip_tags`.
6. Write the config loader for `data/config/*.yaml`.
7. Write `jh import --from DIR --db FILE`. It must be idempotent.
8. Write `fixtures/python/generate.py`. It imports the Python modules and writes JSON cases.
   Commit the output under `fixtures/python/`.

**Acceptance:**

- `cargo test` is green. Every ported function matches its Python fixture case.
- `jh import` runs twice on the same directory. The second run changes no row.
- `cargo clippy -- -D warnings` is clean.

### S2: GPUI spike, time-boxed

**Crates:** `jh-app`

**Work:** window, 500-row table, text input, modal dialog, menu bar status item, light/dark
theme switch. Record the pinned `gpui` and `gpui-component` revisions in `Cargo.toml`.

**Acceptance:** the spike window opens and the six items work. A short report says whether
`gpui-component` compiles with the pinned `gpui`, and which macOS version the pin documents.
If `gpui-component` does not compile, use `uniform_list` for the table and report that.

### S3: Network guard

**Crates:** `jh-scan`

**Work:** one HTTP client with a custom resolver. TTL cache, negative cache, token bucket,
circuit breaker, stale-on-error. A global socket cap from `sources.yaml`. Never retry a DNS error
immediately.

**Acceptance:** unit tests for cache hits, negative caching, pacing, breaker open and close, and
the socket cap. A live smoke test fetches one URL from each host family.

### S4: Sources, first group

**Crates:** `jh-scan`

**Work:** `simplify_json`, `jobbank`, `ats_boards`. One `Source` trait. Per-source fixtures from
saved responses.

**Acceptance:** each parser returns the expected postings from its fixture. A live run of the
three sources writes a scan result.

### S5: Sources, second group

**Crates:** `jh-scan`

**Work:** `jsonboards` (seven vendors), `workday` (board list, first page only, retry, health),
`startups_gallery` (plus board harvest), `workatastartup` (browser-shaped `Accept` header).

**Acceptance:** the Workday sweep writes a cache and per-board health. The scan reads the cache
when it is fresh. The retry schedule matches the Python schedule: transient 1/3/14 days,
structural 7/30 days, broken 30 days.

### S6: Scan pipeline, ledger and portal

**Crates:** `jh-scan`, `jh-engine`

**Work:** the pipeline in the corrected order. The ledger with its `reported` flag. The portal
feed with enrichment and 7-day expiry. Board health, sweep, retry, prune, probe and harvest.
The scan lock. The three-step scheduled slot.

**Acceptance:** a Rust scan and a Python scan on the same day produce the same posting IDs and
the same field values for every ported source, except for documented differences.

### S7: Menubar app and scheduler

**Crates:** `jh-app`, `jh-engine`

**Work:** one process, two threads. `flume` commands and events. The menu bar item with Open,
Scan now, Settings and Quit. The scheduler at 09:00, 13:00 and 17:00 on weekdays. The status
window. No dashboard yet.

**Acceptance:** the app scans on schedule with no window open. `Scan now` works. Quit stops the
scheduler.

### S8: Dashboard

**Crates:** `jh-app`

**Work:** the three stages, the table (filter, sort, page, fit), the drawer with Summary,
Résumé and Contact tabs, the health view, the settings view. Port the pure functions in
`client/src/table.tsx` and test them.

**Acceptance:** the Rust table and the web table show the same rows for the same state.

### S9: Brief engine, then distribution

**Crates:** `jh-brief`, `jh-app`

**Work:** the model client with Keychain, the three stages, the validator, the chain, batch
briefs, key gating. Then the app bundle, ad-hoc signing, the DMG and macOS CI.

**Acceptance:** a brief uses the current JSON schema. The validator drops an edit whose
`before` text is not in the resume.

## 5. Parity contract

- Posting id: `sha1(key)[:16]`, lowercase hex.
- Key order: `dedupe_key`, else `source|raw_id`, else lowercase `company|title|location`.
- Board health key: the Python format. Workday is `tenant|wd|site`. A JSON vendor is `vendor|slug`.
- `apply_signature` patterns and templates: copy `server/scan/posting.py` exactly.
- Sort: tier rank, then descending score, then age, with no age last.
- Dates: ISO 8601, UTC, seconds precision.

## 6. Orchestration rules

- One writer per worktree. Never two agents in the same directory.
- The orchestrator reviews every slice before it lands on `rust-app`.
- A slice branch is merged with `git merge --no-ff`. No squash, so the slice stays visible.
- Tests run in the integration worktree before the merge.
- No push and no merge to `main` without the candidate's approval.
- A blocked worker reports and stops. It does not change the slice scope alone.

## 7. Open items

| Item | Owner | Blocks |
|---|---|---|
| Copy candidate data from `home`. SSH is blocked by tailnet policy. | Candidate | Keyword parity, brief engine |
| Confirm the GPUI pin and the minimum macOS version. | S2 | Packaging |
| Decide the description-fetch budget for the Rust portal. | Orchestrator | S6 |
| Decide when the docker timer on `home` stops. | Candidate | S6 cutover |
