# Rust vs Python pipeline validation

**Date:** 2026-11-16 · **Branch:** `rust-app` · **Scope:** does the Rust scan match the Python
scan it replaces, and does it miss postings the Python one finds.

## Method

Ran both pipelines live against the same `data/config`, back-to-back, and diffed the
posting-id sets and fields.

- Python (in a `main` worktree, read-only): `python3 -m server scan --json --no-write --source …`
- Rust: `jh scan --db <fresh>.db --source …`, then read the SQLite `postings` table.

The six feed sources ran in full. The board fleets ran on a trimmed sample to stay off
Workday's IP throttle: 245 Greenhouse/Ashby/Lever boards (full), 43 JSON-vendor boards,
25 Workday boards. iCIMS is skipped by both in this environment (no `.venv-icims`).

## Result: parity holds — Rust does not miss postings

| Scope | Python | Rust | Missed by Rust |
|---|---|---|---|
| Feed sources — collected | 4039 | 4039 (per-source identical) | — |
| Feed sources — matched | 115 | 116 | 0 |
| Greenhouse/Ashby/Lever + JSON vendors — matched | 46 | 46 | 0 |
| Workday (25-board sample) | 0 (HTTP 429) | 226–267 | n/a (see below) |

Per-source collected counts were identical across all six feed sources
(`jobbank` 100, `newgrad_list` 2570, `newgrad_json` 918, `linkedin` 150,
`startups_gallery` 20, `workatastartup` 281), so the fetchers and parsers agree on both
volume and content. The 62 non-GPUI tests pass, including the per-source parser fixtures and
the Python-generated parity cases.

## Divergences found

1. **Simplify HTML leading-emoji id mismatch (real, reproducible bug).** One posting — Amazon
   "Software Development Engineer Early Career — Amazon Dedicated Cloud" — got different ids:
   Python `b3ed3eb47e36c4a7`, Rust `55983ad8a01c5909`. The Simplify README marks that row with a
   `🔥`, so the company cell is `"🔥 Amazon"`. Python builds the dedupe key from the
   **un-stripped** company (`raw_id = f"{company}|{title}|{location}"` in
   `archive/server/scan/sources/simplify.py:39`), while Rust strips the emoji *before* building
   the key (`crates/jh-scan/src/sources/simplify.rs:219-220`). Same posting, different ledger id,
   so an already-reported posting re-reports. It only surfaces for postings whose apply URL is
   not a recognised ATS (otherwise `collapse_requisitions` prefers the JSON copy); that is why
   exactly one posting diverged in the recent window.

2. **One LinkedIn posting in Rust but not Python (live-feed drift, not a code bug).** "Canonical —
   Linux Software Engineer …" appeared only in Rust's draw. Python's own filters would keep it
   (`title_matches → matched:software engineer`, location Toronto, not level-II), and both draws
   collected 150 LinkedIn postings, so the guest search returned a slightly different set in the
   ~1 minute between runs. The small dropped-bucket deltas (`excluded:senior` −1,
   `excluded:technician` +1, `level-ii-weak-fit` +2) reconcile to the same drift, not to filter
   logic, which the `fixtures/python/filters.json` cases already pin.

3. **Workday: Python got 429, Rust succeeded.** The same 25 boards returned 0 postings from
   Python (all `HTTP 429 Too Many Requests` via `urllib`) and 226–267 from Rust (via
   `reqwest`/rustls). Both send the same URL, body and headers; the difference is the HTTP/TLS
   stack. This is the reverse of a miss — Rust's Workday fetch is more resilient here — but it
   is a single observation and should be confirmed across a couple more runs before relying on
   it.

4. **iCIMS is the one real coverage gap (known, documented).** The Rust scan skips all 247
   iCIMS boards (`icims: 247 boards skipped`); the Python pipeline reads them through a
   `curl_cffi` browser-impersonation subprocess. In production Python this is ~476 postings
   within 7 days per sweep (per `data/config/sources.yaml`), before the location/title filters.
   Until iCIMS is ported, Rust misses every one of those.

## Consistent behaviour worth noting

Postings exactly at the age boundary (`age_days == max_age_days == 7`) are counted in both
pipelines' `matched`, and both expire them on that boundary day — Python's portal drops them at
`expires_in_days <= 0`, Rust writes a retired row at `expires <= today`. The only difference is
cosmetic: Rust's retired row keeps only the minimal columns, so `ats`/`tier`/`score` read null
in SQLite for those rows. No action needed.

## Improvements (theorised)

- **Fix the Simplify HTML id bug** so Rust matches Python byte-for-byte: build `raw_id` from the
  pre-strip company (or canonicalise on both sides and re-key the ledger deliberately). Add the
  🔥-company case to `fixtures/python` and a Rust test so a regression fails.
- **Port iCIMS** — it is the only source where Rust currently loses postings Python finds. The
  blocker is TLS-fingerprint impersonation, the same thing `server/scan/icims_fetch.py` already
  solves with `curl_cffi`; a Rust equivalent (or shelling out to the same venv) closes the last
  miss.
- **Make the differential repeatable.** Today this is a manual same-day run. A small script that
  runs both pipelines on the same sources and diffs ids/fields, run on demand or in CI, would
  catch exactly the kind of id drift in finding (1) the fixtures miss.
- **Re-measure the Workday stack difference.** If `reqwest`/rustls genuinely avoids the 429 that
  `urllib` hits, document it and consider whether the Python side (or any other urllib path)
  should move onto the same stack, and whether the Rust worker pool can widen safely as a result.
- **Keep the source registry in one place.** The two registries (`server/scan/sources/__init__.py`
  vs `crates/jh-scan/src/sources/mod.rs`) and the per-source modules are hand-kept mirrors. Any
  future source added to one side without the other will be a silent miss until another diff like
  this one is run.
