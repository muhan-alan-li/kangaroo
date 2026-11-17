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

// ---------------------------------------------------------------------------
// Menu bar status item (written directly against AppKit, on macOS only).
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
mod status_item {
    use std::cell::RefCell;

    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, NSObject};
    use objc2::{
        ClassType, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel,
    };
    use objc2_app_kit::{
        NSApplication, NSApplicationActivationPolicy, NSEventModifierFlags, NSMenu, NSMenuItem,
        NSStatusBar, NSStatusItem, NSVariableStatusItemLength,
    };
    use objc2_foundation::NSString;

    /// The message a menu click sends to the GPUI side.
    #[derive(Clone, Copy)]
    pub enum MenuAction {
        Open,
        ScanNow,
        Settings,
        Quit,
    }

    /// Instance variables carried by the target object. The sender is cloned
    /// cheaply and handed to every action method.
    struct StatusTargetIvars {
        tx: flume::Sender<MenuAction>,
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
                let _ = self.ivars().tx.send(MenuAction::Open);
            }

            #[unsafe(method(scanNow:))]
            fn scan_now(&self, _sender: *mut AnyObject) {
                let _ = self.ivars().tx.send(MenuAction::ScanNow);
            }

            #[unsafe(method(openSettings:))]
            fn open_settings(&self, _sender: *mut AnyObject) {
                let _ = self.ivars().tx.send(MenuAction::Settings);
            }

            #[unsafe(method(quit:))]
            fn quit(&self, _sender: *mut AnyObject) {
                let _ = self.ivars().tx.send(MenuAction::Quit);
            }
        }
    );

    fn make_target(tx: flume::Sender<MenuAction>, mtm: MainThreadMarker) -> Retained<StatusTarget> {
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
    pub fn install(tx: flume::Sender<MenuAction>) {
        let mtm = MainThreadMarker::new().expect("status item installs on the main thread");
        let _ = NSApplication::sharedApplication(mtm)
            .setActivationPolicy(NSApplicationActivationPolicy::Accessory);

        let target = make_target(tx, mtm);

        let menu = NSMenu::new(mtm);
        for (title, selector, key) in [
            ("Open", sel!(openDashboard:), ""),
            ("Scan now", sel!(scanNow:), "r"),
            ("Settings", sel!(openSettings:), ","),
            ("Quit", sel!(quit:), "q"),
        ] {
            let item = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm),
                    &NSString::from_str(title),
                    Some(selector),
                    &NSString::from_str(key),
                )
            };
            if !key.is_empty() {
                item.setKeyEquivalentModifierMask(NSEventModifierFlags::Command);
            }
            // SAFETY: the target object responds to every selector above and is
            // kept alive for the process lifetime in `STATUS_TARGET`.
            unsafe {
                item.setTarget(Some(target.as_super().as_super()));
            }
            menu.addItem(&item);
        }

        let status_item =
            NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
        if let Some(button) = status_item.button(mtm) {
            button.setTitle(&NSString::from_str("Kangaroo"));
        }
        status_item.setMenu(Some(&menu));

        STATUS_TARGET.with(|slot| *slot.borrow_mut() = Some(target));
        STATUS_ITEM.with(|slot| *slot.borrow_mut() = Some(status_item));
    }
}

use chrono::{DateTime, Local};
use gpui::prelude::*;
use gpui::{
    AnyWindowHandle, App, Context, Entity, KeyBinding, Render, Subscription, Window, WindowBounds,
    WindowOptions, div, px, size,
};
gpui::actions!(job_hunt, [ScanNow, OpenSettings, QuitApp]);
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::input::{Input, InputState};
use gpui_component::table::{Column, DataTable, TableDelegate, TableState};
use gpui_component::{
    ActiveTheme, Disableable as _, Theme, ThemeMode, WindowExt as _, h_flex, v_flex,
};
use gpui_platform::application;
use kangaroo_engine::{Command, DashboardSnapshot, Engine, EngineEvent, ScanMode, ScanState};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

struct AppView {
    commands: flume::Sender<Command>,
    engine: Arc<Mutex<Option<kangaroo_engine::EngineHandle>>>,
    input: Entity<InputState>,
    table: Entity<TableState<EmptyTable>>,
    snapshot: Option<DashboardSnapshot>,
    scan_state: Option<ScanState>,
    last_error: Option<String>,
    next_slot: DateTime<Local>,
    _subscriptions: Vec<Subscription>,
}
impl AppView {
    fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        commands: flume::Sender<Command>,
        engine: Arc<Mutex<Option<kangaroo_engine::EngineHandle>>>,
    ) -> Self {
        let table = cx.new(|cx| TableState::new(EmptyTable, window, cx));
        let input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search postings (coming later)"));
        // The engine emits its first snapshot before this view exists and the
        // event channel keeps no value, so ask for a fresh one now that the
        // receiver is in place.
        let _ = commands.send(Command::Snapshot);
        Self {
            commands,
            engine,
            input,
            table,
            snapshot: None,
            scan_state: None,
            last_error: None,
            next_slot: kangaroo_engine::next_slot(Local::now()),
            _subscriptions: Vec::new(),
        }
    }
    fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.open_dialog(cx, |dialog, _, _| {
            dialog
                .title("Settings")
                .child("Settings view comes later.")
                .footer(
                    h_flex().justify_end().child(
                        Button::new("settings-close")
                            .primary()
                            .label("Close")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    ),
                )
        });
    }
    fn quit(&mut self, cx: &mut Context<Self>) {
        if let Some(engine) = self.engine.lock().ok().and_then(|mut e| e.take()) {
            let _ = engine.shutdown();
        }
        cx.quit();
    }
}
impl Render for AppView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dark = cx.theme().is_dark();
        let state = match &self.scan_state {
            None => "Starting".to_string(),
            Some(ScanState::Idle) => "Idle".to_string(),
            Some(ScanState::Running { .. }) => "Scanning".into(),
            Some(ScanState::Error(e)) => format!("Error: {e}"),
        };
        let snapshot = self.snapshot.clone();
        let path = snapshot
            .as_ref()
            .map(|s| {
                if s.has_database {
                    s.database_path.clone()
                } else {
                    format!("{} (no database)", s.database_path)
                }
            })
            .unwrap_or_else(|| "Starting…".into());
        let counts = snapshot
            .map(|s| format!("{} live postings · {} seen", s.live_postings, s.seen_count))
            .unwrap_or_else(|| "Loading database…".into());
        let last = self
            .snapshot
            .as_ref()
            .and_then(|s| s.last_scan.as_ref())
            .map(|s| {
                format!(
                    "{} · {} · {} · {}",
                    s.mode,
                    s.status,
                    s.started_at,
                    s.finished_at.as_deref().unwrap_or("in progress")
                )
            })
            .unwrap_or_else(|| "No scan has run".into());
        v_flex()
            .on_action(cx.listener(|this, _: &ScanNow, _, _| {
                let _ = this.commands.send(Command::Scan {
                    mode: ScanMode::Quick,
                });
            }))
            .on_action(
                cx.listener(|this, _: &OpenSettings, window, cx| this.open_settings(window, cx)),
            )
            .on_action(cx.listener(|this, _: &QuitApp, _, cx| this.quit(cx)))
            .size_full()
            .gap_3()
            .p_5()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .child(div().text_2xl().child("Kangaroo"))
                    .child(
                        Button::new("theme-toggle")
                            .outline()
                            .label(if dark {
                                "Switch to light"
                            } else {
                                "Switch to dark"
                            })
                            .on_click(|_, _, cx| {
                                let d = cx.theme().is_dark();
                                Theme::change(
                                    if d { ThemeMode::Light } else { ThemeMode::Dark },
                                    None,
                                    cx,
                                );
                            }),
                    ),
            )
            .child(Input::new(&self.input))
            .child(
                v_flex()
                    .gap_2()
                    .p_4()
                    .rounded_lg()
                    .bg(cx.theme().muted)
                    .child(div().text_lg().child(format!("Engine: {state}")))
                    .child(div().child(format!("Last scan: {last}")))
                    .child(div().child(format!("Database: {path}")))
                    .child(div().child(format!("Rows: {counts}")))
                    .child(div().child(format!(
                        "Next scheduled slot: {}",
                        self.next_slot.format("%a %b %-d, %H:%M")
                    )))
                    .child(div().child(format!(
                        "Last error: {}",
                        self.last_error.as_deref().unwrap_or("None")
                    ))),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("scan-now")
                            .primary()
                            .label("Scan now")
                            .on_click({
                                let tx = self.commands.clone();
                                move |_, _, _| {
                                    let _ = tx.send(Command::Scan {
                                        mode: ScanMode::Quick,
                                    });
                                }
                            }),
                    )
                    .child(
                        Button::new("cancel-scan")
                            .outline()
                            .label("Cancel")
                            .disabled(!matches!(
                                self.scan_state.as_ref(),
                                Some(ScanState::Running { .. })
                            ))
                            .on_click({
                                let tx = self.commands.clone();
                                move |_, _, _| {
                                    let _ = tx.send(Command::CancelScan);
                                }
                            }),
                    )
                    .child(
                        Button::new("settings")
                            .outline()
                            .label("Settings")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.open_settings(window, cx)),
                            ),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .rounded_lg()
                    .border_1()
                    .border_color(cx.theme().border)
                    .p_2()
                    .child(DataTable::new(&self.table).stripe(true).bordered(true)),
            )
    }
}

struct EmptyTable;
impl TableDelegate for EmptyTable {
    fn columns_count(&self, _: &App) -> usize {
        4
    }
    fn rows_count(&self, _: &App) -> usize {
        0
    }
    fn column(&self, index: usize, _: &App) -> Column {
        let (name, width) = [
            ("Date", 120.),
            ("Location", 180.),
            ("Company", 220.),
            ("Title", 340.),
        ][index];
        Column::new(name.to_lowercase(), name).width(px(width))
    }
    fn render_td(
        &mut self,
        _: usize,
        _: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        div()
    }
}

fn main() {
    let db_path = std::env::var_os("JH_DB_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("data/state/kangaroo.db"));
    let engine = match Engine::spawn(db_path) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("engine startup failed: {e:#}");
            return;
        }
    };
    let commands = engine.commands.clone();
    let events = engine.events.clone();
    let engine = Arc::new(Mutex::new(Some(engine)));
    application().run(move |cx| {
        gpui_component::init(cx);
        cx.bind_keys([
            KeyBinding::new("cmd-r", ScanNow, None),
            KeyBinding::new("cmd-,", OpenSettings, None),
            KeyBinding::new("cmd-q", QuitApp, None),
        ]);
        #[cfg(target_os = "macos")]
        let menu_rx = {
            let (tx, rx) = flume::unbounded::<status_item::MenuAction>();
            status_item::install(tx);
            rx
        };
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(900.), px(633.)), cx)),
            ..Default::default()
        };
        let app_commands = commands.clone();
        let app_engine = engine.clone();
        let view_handle = Arc::new(Mutex::new(None::<Entity<AppView>>));
        let created_view = view_handle.clone();
        let handle = cx
            .open_window(options, move |window, cx| {
                window.set_window_title("Kangaroo");
                window.on_window_should_close(cx, |window, _| {
                    window.minimize_window();
                    false
                });
                let view = cx.new(|cx| AppView::new(window, cx, app_commands, app_engine));
                if let Ok(mut slot) = created_view.lock() {
                    *slot = Some(view.clone());
                }
                view
            })
            .expect("failed to open Kangaroo window");
        let window: AnyWindowHandle = handle.into();
        let event_view = view_handle.clone();
        cx.spawn(move |cx: &mut gpui::AsyncApp| {
            let cx = cx.clone();
            async move {
                while let Ok(event) = events.recv_async().await {
                    let target = event_view.lock().ok().and_then(|v| v.clone());
                    if let Some(target) = target {
                        cx.update(|cx| {
                            target.update(cx, |view, cx| {
                                match event {
                                    EngineEvent::Dashboard(d) => {
                                        view.scan_state = Some(d.scan_state.clone());
                                        view.snapshot = Some(d);
                                    }
                                    EngineEvent::ScanStatus(s) => view.scan_state = Some(s),
                                    EngineEvent::Error { message, .. } => {
                                        view.last_error = Some(message)
                                    }
                                    EngineEvent::BriefUpdated { .. } => {}
                                    EngineEvent::PostingDetail(_) => {}
                                }
                                cx.notify();
                            });
                        });
                    }
                }
            }
        })
        .detach();
        #[cfg(target_os = "macos")]
        {
            let target_window = window;
            let commands = commands.clone();
            let target_view = view_handle.clone();
            let shutdown_engine = engine.clone();
            cx.spawn(move |cx: &mut gpui::AsyncApp| {
                let cx = cx.clone();
                async move {
                    while let Ok(action) = menu_rx.recv_async().await {
                        let commands = commands.clone();
                        let target_view = target_view.clone();
                        let shutdown_engine = shutdown_engine.clone();
                        cx.update(|cx| match action {
                            status_item::MenuAction::ScanNow => {
                                let _ = commands.send(Command::Scan {
                                    mode: ScanMode::Quick,
                                });
                            }
                            status_item::MenuAction::Open => {
                                let _ = target_window
                                    .update(cx, |_, window, _| window.activate_window());
                            }
                            status_item::MenuAction::Settings => {
                                let _ = target_window.update(cx, |_, window, cx| {
                                    window.activate_window();
                                    if let Some(view) =
                                        target_view.lock().ok().and_then(|v| v.clone())
                                    {
                                        view.update(cx, |view, cx| view.open_settings(window, cx));
                                    }
                                });
                            }
                            status_item::MenuAction::Quit => {
                                if let Some(engine) =
                                    shutdown_engine.lock().ok().and_then(|mut e| e.take())
                                {
                                    let _ = engine.shutdown();
                                }
                                cx.quit();
                            }
                        });
                    }
                }
            })
            .detach();
        }
    });
}
