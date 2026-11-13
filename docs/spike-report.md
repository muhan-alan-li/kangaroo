# GPUI spike report

**Date:** 2026-11-13
**Crate:** `crates/jh-spike`
**Answer:** Yes. The app builds on GPUI and `gpui-component` at pinned revisions, on this
Mac (`aarch64-apple-darwin`), with all six widgets.

## Pinned revisions

| Crate (as imported) | Package | Version | Revision / hash |
|---|---|---|---|
| `gpui` | `gpui-pre` | 0.3.12 | `29db89613527079737f53752eab27fb65c9a7dd94979256c65a0e590c9fd34d8` |
| `gpui_platform` | `gpui-pre-platform` | 0.3.12 | `d2b785bdcbd229f762096aebf4f9ae913bdaa9db49794907cabaf516eb55227b` |
| `gpui_component` | `gpui-component` | 0.9.1 | `git+…gpui-kit?rev=39ab849a93b5f15507b969164d49891db8d5b470` |
| `gpui_base` | `gpui-base` | 0.9.1 | `git+…gpui-kit?rev=39ab849a93b5f15507b969164d49891db8d5b470` |
| `gpui_kit_assets` | `gpui-kit-assets` | 0.9.1 | `git+…gpui-kit?rev=39ab849a93b5f15507b969164d49891db8d5b470` |

`gpui-pre` 0.3.12 is a snapshot of `zed@d2a4fb6ded7c470be25d1541281da3882495b65b`.
The three `gpui-kit` crates share one revision, `39ab849a93b5f15507b969164d49891db8d5b470`
(the `v0.9.1` tag of `longbridge/gpui-kit`). The checksums above are the `Cargo.lock`
`checksum` fields; a git dependency has a `rev`, not a checksum.

**Rust toolchain:** `1.98.1`, pinned in `rust-toolchain.toml`. This is the toolchain the
pinned `zed` snapshot declares. The earlier pin of `1.97.1` was wrong: the snapshot calls
`Result::map_or_default`, which is not stable in 1.97.x, so `gpui-pre-macos` failed to
compile until the toolchain moved to 1.98.1.

## Minimum macOS

**macOS 12.0 (Monterey).** The pin does not state one number in prose; it documents it in
code. The macOS backend gates behavior on `Version::new(12, 0, 0)` in two places and prints
"macOS 12.0 or later is required" for URL-scheme registration. Newer paths are optional and
fall back: the window is paced by `CAMetalDisplayLink` on macOS 14+ and by `CVDisplayLink`
below that, and a macOS 15 Space-restore quirk has its own guard. So 12.0 is the floor.

## Does `gpui-component` compile?

**Yes.** `gpui-component` 0.9.1 compiles against `gpui-pre` 0.3.12 with Rust 1.98.1. No
widget forced a fallback. The spike uses `gpui-component`'s `DataTable` for the 500-row
table directly, so `gpui::uniform_list` was **not** needed.

## The six items

| Item | Status | What fought back |
|---|---|---|
| 1. Window | Done | Nothing. `Root` + `open_window` from the examples. |
| 2. `DataTable`, 500 rows | Done | Nothing. `TableDelegate` with `rows_count() = 500`; the table virtualizes. |
| 3. Text input (placeholder + change) | Done | Nothing. `InputState::placeholder` + `subscribe_in` on `InputEvent::Change`. |
| 4. Modal dialog (open/close) | Done | Nothing. `WindowExt::open_dialog` / `close_dialog`; a footer button closes it. |
| 5. Menu bar status item | Done | The long one. Written against raw AppKit (`objc2`): `define_class!` target with four actions, `NSStatusBar` → `NSStatusItem` → `NSMenu`/`NSMenuItem`. The target is a weak property and the status bar does not retain the item, so both are kept alive in `thread_local` storage. |
| 6. Light/dark theme switch | Done | Nothing. `Theme::change(ThemeMode, None, cx)`, read back with `cx.theme().is_dark()`. |

The menu's Open / Scan now / Settings actions are routed back to the GPUI main loop over a
`flume` channel and logged; Open also re-activates the window. Quit exits the process.

## Environment fought back (the real cost)

Three machine-level issues blocked the build, and all are one-time setup, not framework
problems:

1. **The Metal toolchain was not installed.** `gpui-pre-apple` compiles Metal shaders at
   build time, and `xcrun metal` answered "missing Metal Toolchain". Fix:
   `xcodebuild -downloadComponent MetalToolchain` (838 MB). The first attempt failed to
   fetch the asset catalog; the retry succeeded.
2. **`xcode-select` pointed at Command Line Tools**, which have no `metal`. Full Xcode 27.0
   is installed at `/Applications/Xcode.app`. The build needs
   `DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer` (or, once,
   `sudo xcode-select --switch /Applications/Xcode.app/Contents/Developer`, the step the
   GPUI README already documents).
3. **Rust 1.97.1 was too old.** See above; the toolchain is now 1.98.1.

With those fixed, `cargo build -p jh-spike` finished in about 1m20s of final compile after
a full dependency build.

## Screenshot

Captured with `screencapture -x -o /tmp/spike.png` while the window was open. This agent
has no image input, so I verified the capture programmatically instead of by eye:

- The window is on screen: `CGWindowList` reports owner `jh-spike`, title `GPUI spike`,
  900×633 pt.
- The screenshot (2880×1800) is not blank: 8,345 distinct sampled colors.
- A vertical sample through the table area shows 22 color transitions, matching row
  striping and text.
- The menu-bar strip holds 3,244 distinct colors (2,289 in the right status zone), so the
  status item area has content.

I could not visually confirm individual glyphs, but the window, table, and a populated menu
bar are all present in the capture.

## Recommendation

**Keep GPUI.** `gpui-component` compiles and every widget the dashboard needs built on the
first pass once the toolchain and Metal toolchain were in place. The build requires full
Xcode (not standalone CLT) and Rust 1.98.1; document both for the packaging slice and CI.
The menu bar item is the one place GPUI gives nothing for free, so it is raw AppKit and
will need its own care (target lifetime, activation policy) when the real app adopts it.
