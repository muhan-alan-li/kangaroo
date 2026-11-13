# S7 task: Menubar shell and engine

The candidate wants a proof of concept to look at: a menubar app that runs the engine, shows
what the engine is doing, and can be quit. The dashboard table, the sources and the briefs come
later. This slice is the shell.

Read `docs/rust-app-plan.md` §7.1, §7.3, §7.4, §7.5 and §12.1 first, then this file.

Work in this directory only. Do not modify `server/`, `client/` or `data/`.

## Crates

- `crates/jh-engine` (new library): the engine thread, the command/event channels, the database
  handle, the scheduler, and a stub scan.
- `crates/jh-app` (new binary): rename `crates/jh-spike` to it. Keep the widget patterns the
  spike proved (window, `DataTable`, input, dialog, status item, theme switch) and delete the
  500-row demo data the real window does not need.

## Command and event contract

Use the shapes in `docs/rust-app-plan.md` §7.3. Define every variant. Only `Scan`,
`CancelScan` and `RefreshConfig` need to work in this slice; the rest must be accepted and
answered with an `Error` event that says "not implemented yet", never a panic.

| Command | Data | Behaviour now |
|---|---|---|
| `Scan` | `mode: Quick \| Full` | Take the scan lock, insert a `scan_runs` row, run the stub, finish the row, emit `Dashboard` and `ScanStatus`. |
| `CancelScan` | none | Cancel the running scan task and mark the row `cancelled`. |
| `RefreshConfig` | none | Re-read `data/config/*.yaml` and emit `Dashboard`. |
| `MakeBrief`, `MakeBriefs`, `Apply`, `Unapply`, `SaveSettings` | as in the design doc | Emit `Error { kind: NotImplemented }`. |

Events: `Dashboard(snapshot)`, `ScanStatus(state)`, `BriefUpdated`, `Error(message, source)`.
The engine emits a fresh `Dashboard` after every write. The UI never reads the database.

## Engine

`Engine::spawn(db_path: PathBuf) -> anyhow::Result<EngineHandle>`:

- One `std::thread` with a Tokio runtime inside. `EngineHandle` owns the command sender, a
  cancel handle, and a join handle for shutdown.
- Owns the `sqlx` `SqlitePool` opened on `db_path`. Engine only. The UI never touches it.
- Sends events over a `flume` channel the caller keeps the receiver of.
- On start, reads and emits: live posting count, seen count, last `scan_runs` row, and the
  database path. A missing database is not an error: report "no database" in the snapshot.
- `Scan` takes an exclusive non-blocking `flock` on `data/state/.scan.lock` with `libc`.
  A second scan while one runs answers `Error { kind: Busy }` and starts nothing.
- The stub scan writes one `scan_runs` row (`mode`, `status`, `started_at`, `finished_at`,
  `matched = 0`, `stats_json = {"stub": true}`) and pauses briefly so the UI can show
  "scanning". No network, no sources.
- Scheduler: next slot is the earliest of 09:00, 13:00, 17:00 on a weekday, in local time.
  A missed slot is not replayed. A slot that fires while a scan runs is skipped. Expose
  `next_slot(now: DateTime<Local>) -> DateTime<Local>` as a pure function and unit-test it.

## App

- Window title `Job Hunt`, 900 by 633 points, resizable.
- A status panel, top to bottom: engine state (starting, idle, scanning, error), last scan
  (mode, status, started, finished), database path and row counts, next scheduled slot, and the
  last error. A "Scan now" button, a "Cancel" button disabled unless a scan runs, a "Settings"
  button that opens a dialog saying the view comes later, and a theme switch.
- A macOS status item with a menu: Open, Scan now, Settings, Quit. Copy the spike's `objc2`
  approach, including the `thread_local` retention of the status item and its target.
- Menu actions and window buttons go through the same command path.
- Closing the window leaves the app running; Quit stops the scheduler, drops the engine handle,
  joins the thread and exits.
- Keyboard: Cmd+R runs a scan, Cmd+, opens the placeholder dialog, Cmd+Q quits.

## Dependencies

Use only these, so the offline build works: `anyhow`, `chrono`, `clap`, `flume`, `libc`,
`serde`, `serde_json`, `sqlx`, `tokio`, `gpui`, `gpui-component`, `gpui_platform`, and the
`objc2` crates the spike already uses. If you need another crate, stop and report instead of
adding it.

## Build rules

- Always run cargo with `CARGO_NET_OFFLINE=true`. There is no network here.
- `.cargo/config.toml` sets `DEVELOPER_DIR` for the Metal build script; leave it alone.
- Run `cargo fmt --all`, `cargo clippy --all-targets -- -D warnings` and `cargo test --all`.
- Do not commit. The orchestrator commits outside the sandbox.

## Tests

- `next_slot`: before 09:00, between slots, after 17:00, Friday after 17:00, and Saturday.
- Command round trip: spawn the engine on a temporary SQLite file, run `Scan`, assert the
  events arrive in order (`ScanStatus` running, `Dashboard`, `ScanStatus` idle) and that
  `scan_runs` holds one row.
- Busy lock: hold the lock, send `Scan`, assert `Error { kind: Busy }`.

## Acceptance

- `cargo build -p jh-app` is green and the three checks above pass.
- `cargo run -p jh-app` opens the window and the menu bar item.
- `Scan now` writes a `scan_runs` row and the window shows scanning then idle.
- Capture `screencapture -x -o /tmp/poc.png` with the window open and confirm it shows the
  status panel.

## Report

Print what landed, the test count, the screenshot path, and anything in the design doc that
cannot be built as written.
