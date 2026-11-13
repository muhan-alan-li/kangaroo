//! GPUI spike: prove that the six dashboard widgets build and run on
//! `aarch64-apple-darwin` at the pinned revisions.
//!
//! The six deliverables:
//!   1. a window
//!   2. a `gpui-component` `DataTable` with 500 rows
//!   3. a text input with a placeholder and a change callback
//!   4. a modal dialog that opens and closes
//!   5. a macOS menu bar status item (Open, Scan now, Settings, Quit)
//!   6. a light/dark theme switch that repaints the window

use gpui::prelude::*;
use gpui::{
    AnyWindowHandle, App, Context, Entity, Render, SharedString, Subscription, Window,
    WindowBounds, WindowOptions, div, px, size,
};
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::table::{Column, DataTable, TableDelegate, TableState};
use gpui_component::{ActiveTheme, Theme, ThemeMode, WindowExt as _, h_flex, v_flex};
use gpui_kit_assets::Assets;
use gpui_platform::application;

// ---------------------------------------------------------------------------
// Menu bar status item (written directly against AppKit, on macOS only).
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
mod status_item {
    use std::cell::RefCell;

    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, NSObject};
    use objc2::{define_class, msg_send, sel, ClassType, DefinedClass, MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{
        NSMenu, NSMenuItem, NSStatusBar, NSStatusItem, NSVariableStatusItemLength,
    };
    use objc2_foundation::NSString;

    /// The message a menu click sends to the GPUI side.
    #[derive(Clone, Copy)]
    pub enum MenuAction {
        Open,
        ScanNow,
        Settings,
    }

    impl MenuAction {
        pub fn from_str(s: &str) -> Option<Self> {
            match s {
                "open" => Some(MenuAction::Open),
                "scan" => Some(MenuAction::ScanNow),
                "settings" => Some(MenuAction::Settings),
                _ => None,
            }
        }
    }

    /// Instance variables carried by the target object. The sender is cloned
    /// cheaply and handed to every action method.
    struct StatusTargetIvars {
        tx: flume::Sender<&'static str>,
    }

    // The Objective-C target that AppKit calls when a menu item is chosen.
    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "JHStatusTarget"]
        #[ivars = StatusTargetIvars]
        struct StatusTarget;

        impl StatusTarget {
            #[unsafe(method(openDashboard:))]
            fn open_dashboard(&self, _sender: *mut AnyObject) {
                let _ = self.ivars().tx.send("open");
            }

            #[unsafe(method(scanNow:))]
            fn scan_now(&self, _sender: *mut AnyObject) {
                let _ = self.ivars().tx.send("scan");
            }

            #[unsafe(method(openSettings:))]
            fn open_settings(&self, _sender: *mut AnyObject) {
                let _ = self.ivars().tx.send("settings");
            }

            #[unsafe(method(quit:))]
            fn quit(&self, _sender: *mut AnyObject) {
                // The spike's Quit simply leaves the process. A real app would
                // stop its scheduler and let GPUI shut down first.
                std::process::exit(0);
            }
        }
    );

    fn make_target(tx: flume::Sender<&'static str>, mtm: MainThreadMarker) -> Retained<StatusTarget> {
        let this = StatusTarget::alloc(mtm).set_ivars(StatusTargetIvars { tx });
        unsafe { msg_send![super(this), init] }
    }

    // Keep the status item and its target alive for the process lifetime.
    // `NSMenuItem`'s target is a weak property and `NSStatusBar` does not
    // retain the status item, so both must outlive every menu interaction.
    thread_local! {
        static STATUS_ITEM: RefCell<Option<Retained<NSStatusItem>>> = const { RefCell::new(None) };
        static STATUS_TARGET: RefCell<Option<Retained<StatusTarget>>> = const { RefCell::new(None) };
    }

    /// Install the status item. Must run on the main thread.
    pub fn install(tx: flume::Sender<&'static str>) {
        let mtm = MainThreadMarker::new().expect("status item installs on the main thread");

        let target = make_target(tx, mtm);

        let menu = NSMenu::new(mtm);
        for (title, selector) in [
            ("Open", sel!(openDashboard:)),
            ("Scan now", sel!(scanNow:)),
            ("Settings", sel!(openSettings:)),
            ("Quit", sel!(quit:)),
        ] {
            let item = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm),
                    &NSString::from_str(title),
                    Some(selector),
                    &NSString::from_str(""),
                )
            };
            // SAFETY: the target object responds to every selector above and is
            // kept alive for the process lifetime in `STATUS_TARGET`.
            unsafe {
                item.setTarget(Some(target.as_super().as_super()));
            }
            menu.addItem(&item);
        }

        let status_item = NSStatusBar::systemStatusBar()
            .statusItemWithLength(NSVariableStatusItemLength);
        if let Some(button) = status_item.button(mtm) {
            button.setTitle(&NSString::from_str("Job Hunt"));
        }
        status_item.setMenu(Some(&menu));

        STATUS_TARGET.with(|slot| *slot.borrow_mut() = Some(target));
        STATUS_ITEM.with(|slot| *slot.borrow_mut() = Some(status_item));
    }
}

// ---------------------------------------------------------------------------
// Table delegate: a four-column, 500-row posting table.
// ---------------------------------------------------------------------------

struct JobTable {
    columns: Vec<Column>,
}

impl JobTable {
    fn new() -> Self {
        Self {
            columns: vec![
                Column::new("date", "Date").width(px(120.)),
                Column::new("location", "Location").width(px(180.)),
                Column::new("company", "Company").width(px(220.)),
                Column::new("title", "Title").width(px(340.)),
            ],
        }
    }
}

impl TableDelegate for JobTable {
    fn columns_count(&self, _: &App) -> usize {
        self.columns.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        500
    }

    fn column(&self, col_ix: usize, _: &App) -> Column {
        self.columns[col_ix].clone()
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let text: String = match col_ix {
            0 => format!("2026-11-{:02}", (row_ix % 28) + 1),
            1 => match row_ix % 3 {
                0 => "Vancouver, BC".to_string(),
                1 => "Remote (Canada)".to_string(),
                _ => "Toronto, ON".to_string(),
            },
            2 => format!("Company {}", row_ix + 1),
            3 => format!("Software Engineer {}", row_ix + 1),
            _ => String::new(),
        };
        div().child(text)
    }
}

// ---------------------------------------------------------------------------
// The window's root view.
// ---------------------------------------------------------------------------

struct AppView {
    table: Entity<TableState<JobTable>>,
    input: Entity<InputState>,
    echo: SharedString,
    _subscriptions: Vec<Subscription>,
}

impl AppView {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let table = cx.new(|cx| TableState::new(JobTable::new(), window, cx));
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Type a filter…"));

        let subscriptions = vec![cx.subscribe_in(&input, window, {
            let input = input.clone();
            move |this, _, ev: &InputEvent, _window, cx| {
                if matches!(ev, InputEvent::Change) {
                    this.echo = input.read(cx).value();
                    cx.notify();
                }
            }
        })];

        Self {
            table,
            input,
            echo: SharedString::default(),
            _subscriptions: subscriptions,
        }
    }
}

impl Render for AppView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dark = cx.theme().is_dark();

        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                h_flex()
                    .p_4()
                    .gap_3()
                    .items_center()
                    .child("GPUI spike")
                    .child(
                        div()
                            .text_color(cx.theme().muted_foreground)
                            .child(self.echo.clone()),
                    )
                    .child(
                        div().flex_1().child(
                            Button::new("open-dialog")
                                .primary()
                                .label("Open dialog")
                                .on_click(|_, window, cx| {
                                    window.open_dialog(cx, |dialog, _, _| {
                                        dialog
                                            .title("Settings")
                                            .child(
                                                "This is a modal dialog. Press Esc, click the \
                                                 backdrop, or use the button below to close it.",
                                            )
                                            .footer(
                                                h_flex()
                                                    .justify_end()
                                                    .gap_2()
                                                    .child(
                                                        Button::new("dialog-close")
                                                            .primary()
                                                            .label("Close")
                                                            .on_click(|_, window, cx| {
                                                                window.close_dialog(cx);
                                                            }),
                                                    ),
                                            )
                                    });
                                }),
                        ),
                    )
                    .child(
                        Button::new("theme-toggle")
                            .outline()
                            .label(if dark { "Switch to light" } else { "Switch to dark" })
                            .on_click(|_, _, cx| {
                                let dark = cx.theme().is_dark();
                                Theme::change(
                                    if dark { ThemeMode::Light } else { ThemeMode::Dark },
                                    None,
                                    cx,
                                );
                            }),
                    ),
            )
            .child(div().px_4().pb_2().child(Input::new(&self.input)))
            .child(
                div()
                    .h(px(440.))
                    .w_full()
                    .px_4()
                    .pb_4()
                    .child(DataTable::new(&self.table).stripe(true).bordered(true)),
            )
    }
}

// ---------------------------------------------------------------------------
// Entry point.
// ---------------------------------------------------------------------------

fn main() {
    application()
        .with_assets(Assets)
        .run(|cx| {
            // Initialize gpui-component (and gpui-base) before any component.
            gpui_component::init(cx);

            // Menu bar status item -> GPUI, via a channel. The status item is
            // created here so the platform/run loop is already up.
            #[cfg(target_os = "macos")]
            let menu_rx = {
                let (tx, rx) = flume::unbounded::<&'static str>();
                status_item::install(tx);
                rx
            };

            let options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(900.), px(600.)), cx)),
                ..Default::default()
            };

            let handle = cx
                .open_window(options, |window, cx| {
                    window.set_window_title("GPUI spike");
                    cx.new(|cx| {
                        let view = cx.new(|cx| AppView::new(window, cx));
                        gpui_base::Root::new(view, window, cx)
                    })
                })
                .expect("failed to open window");

            // Route menu clicks back onto the main thread and act on them.
            #[cfg(target_os = "macos")]
            {
                let any: AnyWindowHandle = handle.into();
                cx.spawn(move |cx: &mut gpui::AsyncApp| {
                    let cx = cx.clone();
                    async move {
                        while let Ok(menu_action) = menu_rx.recv_async().await {
                            cx.update(|cx| {
                                let action = status_item::MenuAction::from_str(menu_action);
                                match action {
                                    Some(status_item::MenuAction::Open) => {
                                        println!("[spike] menu: Open");
                                        let _ = any.update(cx, |_, window, _| window.activate_window());
                                    }
                                    Some(status_item::MenuAction::ScanNow) => {
                                        println!("[spike] menu: Scan now");
                                    }
                                    Some(status_item::MenuAction::Settings) => {
                                        println!("[spike] menu: Settings");
                                    }
                                    _ => {}
                                }
                            });
                        }
                    }
                })
                .detach();
            }
        });
}
