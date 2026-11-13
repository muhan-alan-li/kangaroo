# S2 task: GPUI spike

This slice is time-boxed. Stop after two agent-days of work and report, even if it is not
finished.

Read `docs/rust-app-plan.md` §13 first, then `docs/rust-app-implementation-plan.md` §4.

Work in this directory only.

## Goal

Answer one question with working code: can the app be built on GPUI and `gpui-component` at
pinned revisions, on this Mac, with the widgets the dashboard needs?

## Deliverables

A crate at `crates/jh-spike` that builds and runs on `aarch64-apple-darwin`:

1. A window.
2. A table with 500 rows that scrolls smoothly.
3. A text input with a working placeholder and a change callback.
4. A modal dialog that opens and closes.
5. A macOS menu bar status item with a menu: Open, Scan now, Settings, Quit.
6. A light/dark theme switch that repaints the window.

Pin both crates by revision in `Cargo.toml`. A git dependency is fine.

Add `docs/spike-report.md` with:

- The exact pinned revisions and their `Cargo.lock` hashes.
- The minimum macOS version the pin documents.
- Whether `gpui-component` compiles. If it does not, say which widget forced the fallback and
  use `gpui`'s `uniform_list` for the table.
- The time each item took, and what fought back.
- A recommendation: keep GPUI, or change the UI stack.

## Rules

- One commit: `feat: Add the GPUI spike`.
- Do not modify the design doc. Add the report as a new file.
- Do not add the dashboard, the engine, or SQLite. This slice is only the six widgets.
- If a crate fails to build after two hours of work, write the failure into the report and stop.
  Do not switch UI toolkits alone.

## Acceptance

`cargo build -p jh-spike` succeeds, and `cargo run -p jh-spike` opens the window with the six
items. Capture a screenshot with `screencapture -x -o /tmp/spike.png` while the window is open,
and mention in the report whether the screenshot shows the table and the menu item.
