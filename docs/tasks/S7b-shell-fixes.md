# S7b task: Fix the two startup bugs in the shell

The menubar shell builds and the window renders, but its status panel never leaves the values it
paints on the first frame. Two defects cause that. Fix both, and prove the fix with tests.

Work in this directory only. Do not modify `server/` or `client/`.

## Defect 1: the engine does not create a missing database

`crates/jh-engine/src/lib.rs`, `run_engine` opens the pool only `if db_path.exists()`. When the
file is absent the pool stays `None` forever, so the app never creates `data/state/jobhunt.db`,
never migrates it, and reports no rows.

Fix: open (and migrate) the database whenever the engine starts, creating the parent directory
and the file as needed. A failure to open must arrive as an `Error` event with source
`"database"`, not silence. `jh_db::connect` and `jh_db::migrate` already do the work; the
creation path exists elsewhere in the file and can be shared.

## Defect 2: the first Dashboard event is lost

The engine emits one `Dashboard` right after it starts. The view subscribes to the event channel
after that, and `flume` is a queue with no retained value, so the first snapshot is dropped. The
window then shows `Engine: Starting`, `Database: Starting…`, `Rows: Loading database…` until
some later event happens to arrive.

Fix, in this order of preference:

1. Add a `Snapshot` command that makes the engine emit a fresh `Dashboard`, and send it from the
   view's `new` right after it subscribes. Keep `RefreshConfig` for a config reload.
2. If that cannot be made to work cleanly, make the engine hold the latest snapshot and re-send
   it on a `Subscribe` command.

Either way, the window must show the true state within one event round trip of opening, with no
scan required.

## Also fix while you are there

- The status panel must show `idle` rather than `starting` once the first snapshot has arrived.
- `Scan now` must write one `scan_runs` row and return the panel to idle. Verify that with a test,
  not by eye.

## Tests

Add tests that fail on the current code and pass after the fix:

1. Spawn the engine against a path whose parent does not exist. Assert the file appears, the
   migrations ran, and a `Dashboard` arrives after a `Snapshot` command.
2. Spawn the engine, send `Snapshot` immediately, and assert the first event received is a
   `Dashboard` that reports the idle state, not `starting`.
3. `Scan` on a fresh database writes exactly one `scan_runs` row with status `ok`.

## Rules

- No new dependencies. No network: always run cargo with `CARGO_NET_OFFLINE=true`.
- Leave `.cargo/config.toml` alone; it sets `DEVELOPER_DIR`.
- Do not commit and do not push. The orchestrator commits.
- Do not try to screenshot; there is no screen in this session. Another process verifies the
  window.
- Run `cargo fmt --all`, `cargo clippy --all-targets -- -D warnings`, and `cargo test --all`
  before you finish. All three must pass.
- Kill any `jh-app` process you start.

## Report

State which fix you chose for defect 2 and why, the three test names, and the final test count.
