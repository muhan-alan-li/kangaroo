# S1 task: Foundation, parity core, import

Read these first, in this order:

1. `docs/rust-app-implementation-plan.md` (this file's parent).
2. `docs/rust-app-plan.md` §7, §8, §9, §10.2, §20.
3. `server/scan/posting.py`, `server/scan/filters.py`, `server/scan/pipeline.py`,
   `server/scan/parsing.py`, `server/config.py`.

Work in this directory only. Do not touch `server/`, `client/` or `data/`, except to read them.

## Environment

- `cargo` 1.97.1 is installed. `rustc` 1.97.1. Target `aarch64-apple-darwin`.
- `python3` is 3.12 with PyYAML. Use it only to generate fixtures.
- No network calls in this slice. Sources come later.

## Deliverables

### 1. Workspace

```
Cargo.toml                 workspace: crates/jh-core, crates/jh-db, crates/jh-cli
rust-toolchain.toml        channel = "1.97.1", components = rustfmt, clippy
crates/jh-core/            pure logic, no I/O except config file reads
crates/jh-db/              sqlx sqlite pool, migrations, row types
crates/jh-cli/             clap binary `jh`
migrations/0001_init.sql   from docs/rust-app-plan.md §9.2
```

Use `sqlx` with the `sqlite`, `runtime-tokio`, `macros` features and offline-friendly
`query_as`. Use `serde`, `serde_yaml`, `sha1`, `regex`, `chrono`, `anyhow`, `clap`.

Add `migrations/0002_additions.sql` only if §9.2 lacks a column this slice needs. Say so
in the report.

### 2. Posting identity, in `jh-core::posting`

Port `server/scan/posting.py` exactly:

- `apply_signature(url) -> Option<(vendor, slug, job_id, key)>` with all fourteen patterns and
  their templates. Copy the regexes byte for byte, including the case-insensitive flags and the
  optional Workday middle segment.
- `collapse_requisitions(postings)` with `SOURCE_PREFERENCE`.
- `make_posting(...)` with the key order: `dedupe_key`, else `source|raw_id`, else
  lowercase `company|title|location`. The id is `sha1(key)[:16]`.

### 3. Filters and pipeline, in `jh-core::filters` and `jh-core::pipeline`

Port `title_matches`, `is_level_ii`, `region_signals`, `_tier_lookup`, `location_matches`,
`normalize_and_filter`, `score`, `sort_key`, `cap_per_tier`, `tier_counts`. Also
`parse_salary` and the date helpers from `parsing.py`.

Read every list from `filters.yaml`. Never hard-code a title phrase, a tier name or the
comp floor.

### 4. Config loader, in `jh-core::config`

Typed structs for `sources.yaml` and `filters.yaml`. Keep unknown keys working: the Python
files carry comment-documented extras and some keys are read by only one source.

### 5. Import, in `jh-db` and `jh-cli`

`jh import --from DIR --db FILE`:

- Reads the state files listed in design doc §9.3.
- Writes one transaction per table family.
- Is idempotent. A second run changes no row.
- Skips a missing file without failing.
- Prints a per-table row count.

`jh import --to-sqlite-from-fixture DIR` is not needed. Keep one command.

### 6. Fixtures and tests

Write `fixtures/python/generate.py`. It imports the Python modules from `server/` and writes:

| File | Content |
|---|---|
| `posting_ids.json` | Cases for the three key branches, plus unicode, extra whitespace and mixed case. |
| `apply_signatures.json` | Every vendor pattern, plus six URLs that must not match. |
| `filters.json` | `title_matches`, `is_level_ii`, `location_matches` cases with expected values. |
| `pipeline.json` | Postings plus expected `score`, `tier`, `tier_rank` and the kept id list. |
| `salary.json` | `parse_salary` inputs and outputs. |
| `collapse.json` | Posting lists and the ids to keep. |

Use a fixed, hand-written case list. No randomness. No network.

Commit the generated JSON. Rust tests read the JSON and compare. A fixture mismatch is a
failed test, never a skipped one.

## Rules

- One commit at the end: `feat: Port posting identity, filters and import`.
- `cargo fmt` before the commit. `cargo clippy -- -D warnings` must be clean.
- Do not add a source, a network client, or the GPUI dependency. That is S3 and S4.
- Do not rename or reformat Python files.
- If a Python behaviour is unclear, read the Python code again. Do not guess.

## Acceptance

1. `cargo test` is green, including every parity test.
2. `jh import` on a fixture state directory runs twice with an identical database file hash.
3. `cargo clippy -- -D warnings` is clean.

## Report

When you finish, print a short report:

- What landed, per crate.
- Any Python behaviour you could not port exactly, and why.
- Any column you had to add to the schema.
- The test count and the commands you ran.
