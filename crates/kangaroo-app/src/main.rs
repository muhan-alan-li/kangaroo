//! The desktop app: a GPUI window that talks to the menu-bar server over a
//! Unix socket. It owns no engine and no database.
//!
//! One page: three stage tabs, a table of the postings in the current stage, and
//! a detail panel for the row under the cursor. The Open stage also marks rows
//! for a batch brief, because a brief is the only step that costs a model call.

mod client;
mod detail;
mod table;
mod theme;
mod widgets;

use chrono::{DateTime, Local};
use detail::BriefView;
use gpui::prelude::*;
use gpui::{
    AnyElement, AnyWindowHandle, App, Context, Entity, KeyBinding, Render, SharedString,
    Subscription, Window, WindowBounds, WindowOptions, div, px, size,
};
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::kbd::Kbd;
use gpui_component::pagination::Pagination;
use gpui_component::searchable_list::SearchableVec;
use gpui_component::select::{Select, SelectEvent, SelectState};
use gpui_component::table::{ColumnSort, DataTable, TableEvent, TableState};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, Size, h_flex, v_flex,
};
use gpui_platform::application;
use kangaroo_protocol::{
    Command, DashboardSnapshot, EngineEvent, PostingDetail, PostingRow, ScanMode, ScanState,
    Settings,
};
use serde_json::json;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use table::{PostingTable, RowEvent, selection_note, sort_rows};
use theme::Tint;

gpui::actions!(
    kangaroo,
    [ScanNow, OpenSettings, QuitApp, CloseDetail, FocusSearch,]
);

/// The height of one table row. The page size follows from this and the window
/// height, so the table never leaves blank space under its last row.
const ROW_HEIGHT: f32 = 40.;

/// The navigation column width.
const SIDEBAR_WIDTH: f32 = 232.;

/// The title bar height. The traffic lights sit inside it at the left.
const TITLE_BAR_HEIGHT: f32 = 38.;

/// The chrome a postings page carries above and below its rows: title bar, page
/// header, the card's toolbar and column headers, the pagination footer, and the
/// window padding. Subtracted from the window height to size one page.
const PAGE_CHROME: f32 = 240.;

/// The detail drawer width, and the gap between it and the table. The width is
/// the width of a readable column, not a share of the window.
const DETAIL_WIDTH: f32 = 400.;
const DETAIL_GAP: f32 = 12.;

/// The three stages of the pipeline. A posting with a brief is Shortlisted; an
/// applied posting is Applied. No posting is in two stages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Open,
    Shortlisted,
    Applied,
}

impl Stage {
    fn empty(self) -> &'static str {
        match self {
            Stage::Open => "No postings to review. Run a scan.",
            Stage::Shortlisted => "No briefs yet. Tick rows in Postings, then generate one.",
            Stage::Applied => "No application marked yet.",
        }
    }
}

/// The sidebar sections: the settings and health views, plus the three
/// pipeline stages, renamed for the navigation (Open is "Postings").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    Settings,
    Health,
    Postings,
    Shortlist,
    Applied,
}

impl Section {
    fn id(self) -> &'static str {
        match self {
            Section::Settings => "nav-settings",
            Section::Health => "nav-health",
            Section::Postings => "nav-postings",
            Section::Shortlist => "nav-shortlist",
            Section::Applied => "nav-applied",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Section::Settings => "Settings",
            Section::Health => "Health",
            Section::Postings => "Postings",
            Section::Shortlist => "Shortlist",
            Section::Applied => "Applied",
        }
    }

    /// The pipeline stage this section shows, or `None` for Settings/Health.
    fn stage(self) -> Option<Stage> {
        match self {
            Section::Postings => Some(Stage::Open),
            Section::Shortlist => Some(Stage::Shortlisted),
            Section::Applied => Some(Stage::Applied),
            Section::Settings | Section::Health => None,
        }
    }
}

pub struct AppView {
    commands: flume::Sender<Command>,
    search: Entity<InputState>,
    location: Entity<SelectState<SearchableVec<String>>>,
    table: Entity<TableState<PostingTable>>,
    /// The window this view lives in, kept so the async event loop can update
    /// the location filter's options, which needs a `Window`.
    window: AnyWindowHandle,
    section: Section,
    stage: Stage,
    /// The search text, mirrored so the row filter can run without reading the
    /// input's buffer on every frame.
    query: String,
    /// The location the table is filtered to, `None` for every location.
    location_filter: Option<String>,
    /// The Date column's sort direction. The delegate mirrors it; the view is
    /// the source of truth, so it can re-apply it after a new snapshot.
    sort: ColumnSort,
    /// The current page of the paginated table, 1-based.
    table_page: usize,
    /// How many rows fit the window. Recomputed on every render from the window
    /// height, so a resize never leaves blank space under the last row.
    page_size: usize,
    /// The width the table draws into. Recomputed on every render, so the title
    /// column always takes the leftover width.
    table_width: f32,
    snapshot: Option<DashboardSnapshot>,
    scan_state: Option<ScanState>,
    last_error: Option<String>,
    next_slot: DateTime<Local>,
    /// Rows ticked for a batch brief. Only meaningful in the Open stage.
    selected: HashSet<String>,
    /// The posting the panel shows. The detail itself arrives from the server.
    detail_id: Option<String>,
    detail: Option<PostingDetail>,
    note: Option<String>,
    /// Set by `--open-first`: open the top posting once a snapshot arrives.
    open_first: bool,
    /// The tint the window wears. Saved in the app's settings, so the next launch
    /// starts in the same one.
    tint: Tint,
    /// True when `--tint` chose the tint, so a stored value must not override it.
    tint_from_flag: bool,
    /// The settings as the server has them, so a save writes back every key
    /// rather than only the one this window changed.
    settings: serde_json::Value,
    _subscriptions: Vec<Subscription>,
}

impl AppView {
    fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        commands: flume::Sender<Command>,
        open_first: bool,
        tint: Tint,
        tint_from_flag: bool,
    ) -> Self {
        let (rows_tx, rows_rx) = flume::unbounded();
        let table = cx.new(|cx| TableState::new(PostingTable::new(rows_tx), window, cx));
        let search = cx
            .new(|cx| InputState::new(window, cx).placeholder("Filter company, title or location"));
        let location = cx
            .new(|cx| SelectState::new(SearchableVec::new(Vec::<String>::new()), None, window, cx));
        // The server keeps no retained event, so ask for a fresh snapshot now
        // that this view's receiver is in place.
        let _ = commands.send(Command::Snapshot);
        let mut view = Self {
            commands,
            search,
            location,
            table,
            window: window.window_handle(),
            section: Section::Postings,
            stage: Stage::Open,
            query: String::new(),
            location_filter: None,
            sort: ColumnSort::Descending,
            table_page: 1,
            page_size: 12,
            table_width: 900.,
            snapshot: None,
            scan_state: None,
            last_error: None,
            next_slot: kangaroo_engine_next_slot(),
            selected: HashSet::new(),
            detail_id: None,
            detail: None,
            note: None,
            open_first,
            tint,
            tint_from_flag,
            settings: serde_json::Value::Null,
            _subscriptions: Vec::new(),
        };

        let search = view.search.clone();
        view._subscriptions
            .push(cx.subscribe(&search, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.query = this.search.read(cx).value().to_string();
                    this.sync_table(cx);
                }
            }));

        let location = view.location.clone();
        view._subscriptions.push(cx.subscribe(
            &location,
            |this, _, event: &SelectEvent<SearchableVec<String>>, cx| {
                let SelectEvent::Confirm(value) = event;
                this.location_filter = value.clone();
                this.sync_table(cx);
            },
        ));

        let table = view.table.clone();
        view._subscriptions.push(cx.subscribe_in(
            &table,
            window,
            |this, _, event: &TableEvent, _window, cx| {
                if let TableEvent::SelectRow(row_ix) = event {
                    this.open_row(*row_ix, cx);
                }
            },
        ));

        // A tick in a row sends on this channel rather than through an entity
        // handle, because the cell renders inside the table's own update.
        cx.spawn(async move |this, cx| {
            while let Ok(event) = rows_rx.recv_async().await {
                let _ = this.update(cx, |view, cx| match event {
                    RowEvent::Toggle(id) => view.toggle(id, cx),
                    RowEvent::Sort(sort) => view.set_sort(sort, cx),
                });
            }
        })
        .detach();

        view
    }

    fn rows_for(&self, stage: Stage) -> Vec<PostingRow> {
        self.snapshot
            .as_ref()
            .map(|snapshot| match stage {
                Stage::Open => snapshot.open.clone(),
                Stage::Shortlisted => snapshot.shortlisted.clone(),
                Stage::Applied => snapshot.applied.clone(),
            })
            .unwrap_or_default()
    }

    /// The rows the table draws: the stage's rows, cut down by the location
    /// filter and the search text, then ordered by the Date column's sort.
    fn visible_rows(&self) -> Vec<PostingRow> {
        let query = self.query.trim().to_lowercase();
        let mut rows = self.rows_for(self.stage);
        if let Some(location) = &self.location_filter {
            rows.retain(|row| row.location.as_deref() == Some(location.as_str()));
        }
        if !query.is_empty() {
            rows.retain(|row| {
                row.company.to_lowercase().contains(&query)
                    || row.title.to_lowercase().contains(&query)
                    || row
                        .location
                        .as_deref()
                        .is_some_and(|text| text.to_lowercase().contains(&query))
            });
        }
        sort_rows(rows, self.sort)
    }

    /// The number of pages the current filtered set spans.
    fn total_pages(&self) -> usize {
        self.visible_rows().len().div_ceil(self.page_size.max(1))
    }

    /// The slice of [`Self::visible_rows`] the current page shows.
    fn page_rows(&self) -> Vec<PostingRow> {
        let page = self.table_page.clamp(1, self.total_pages().max(1));
        let page_size = self.page_size.max(1);
        self.visible_rows()
            .into_iter()
            .skip((page - 1) * page_size)
            .take(page_size)
            .collect()
    }

    /// How many rows fit the window. The chrome and the row height come from
    /// the style, so this is a plain division, and the result drives the page
    /// slice on the next frame.
    fn page_size_for(window: &Window) -> usize {
        let height = f32::from(window.bounds().size.height);
        (((height - PAGE_CHROME) / ROW_HEIGHT).floor() as usize).clamp(4, 40)
    }

    /// The width the table draws into: the window, less the navigation column,
    /// the page padding, and the detail drawer when it is open.
    fn table_width_for(&self, window: &Window) -> f32 {
        let window_width = f32::from(window.bounds().size.width);
        let drawer = if self.detail.is_some() {
            DETAIL_WIDTH + DETAIL_GAP
        } else {
            0.
        };
        (window_width - SIDEBAR_WIDTH - 32. - drawer).max(320.)
    }

    /// Re-cut the page and the columns after the window changed size. This runs
    /// inside `render`, so it must not notify the view: only the table's caches
    /// are stale, and one table refresh is enough.
    fn sync_layout(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.resize_page(Self::page_size_for(window), cx);
        let width = self.table_width_for(window);
        if (width - self.table_width).abs() > 0.5 {
            self.table_width = width;
            self.table.update(cx, |table, cx| {
                table.delegate_mut().table_width = width;
                table.refresh(cx);
                cx.notify();
            });
        }
    }

    /// Re-cut the page after the window height changed. This runs inside
    /// `render`, so it must not notify the view: only the table's row cache is
    /// stale, and one table refresh is enough.
    fn resize_page(&mut self, page_size: usize, cx: &mut Context<Self>) {
        if page_size == self.page_size {
            return;
        }
        self.page_size = page_size;
        self.table_page = self.table_page.clamp(1, self.total_pages().max(1));
        let rows = self.page_rows();
        self.table.update(cx, |table, cx| {
            table.delegate_mut().rows = rows;
            table.refresh(cx);
            cx.notify();
        });
    }

    fn sync_table(&mut self, cx: &mut Context<Self>) {
        // A tick follows its row: once a brief moves the posting out of Open,
        // the posting is no longer something a batch can brief.
        let open = self.rows_for(Stage::Open);
        self.selected
            .retain(|id| open.iter().any(|row| &row.id == id));
        // A filter or a new snapshot can shrink the set below the current page.
        self.table_page = self.table_page.clamp(1, self.total_pages().max(1));
        let rows = self.page_rows();
        let empty: SharedString = if self.query.trim().is_empty() {
            self.stage.empty().into()
        } else {
            format!("No posting matches “{}”.", self.query.trim()).into()
        };
        let stage = self.stage;
        let selected = self.selected.clone();
        let sort = self.sort;
        self.table.update(cx, |table, cx| {
            let delegate = table.delegate_mut();
            delegate.rows = rows;
            delegate.stage = stage;
            delegate.selected = selected;
            delegate.sort = sort;
            delegate.empty = empty;
            table.refresh(cx);
            // The row cache lives in this entity, so it must repaint itself;
            // notifying the view alone leaves the last frame on screen.
            cx.notify();
        });
        cx.notify();
    }

    fn set_section(&mut self, section: Section, cx: &mut Context<Self>) {
        if self.section == section {
            return;
        }
        self.section = section;
        if let Some(stage) = section.stage() {
            self.stage = stage;
            self.table_page = 1;
            self.sync_table(cx);
        }
        cx.notify();
    }

    fn set_table_page(&mut self, page: usize, cx: &mut Context<Self>) {
        let page = page.clamp(1, self.total_pages().max(1));
        if self.table_page == page {
            return;
        }
        self.table_page = page;
        self.sync_table(cx);
    }

    fn set_sort(&mut self, sort: ColumnSort, cx: &mut Context<Self>) {
        if self.sort == sort {
            return;
        }
        self.sort = sort;
        self.sync_table(cx);
    }

    /// The distinct non-empty locations across every stage, in sorted order.
    fn locations(&self) -> Vec<String> {
        let Some(snapshot) = &self.snapshot else {
            return Vec::new();
        };
        let mut set = std::collections::BTreeSet::new();
        for row in snapshot
            .open
            .iter()
            .chain(&snapshot.shortlisted)
            .chain(&snapshot.applied)
        {
            if let Some(location) = row
                .location
                .as_deref()
                .map(str::trim)
                .filter(|text| !text.is_empty())
            {
                set.insert(location.to_owned());
            }
        }
        set.into_iter().collect()
    }

    /// Rebuild the location filter's options from the current postings, and
    /// drop a filter value that is no longer on offer.
    fn refresh_location_options(&mut self, cx: &mut Context<Self>) {
        let locations = self.locations();
        let window = self.window;
        let location = self.location.clone();
        let _ = window.update(cx, |_, window, cx| {
            location.update(cx, |state, cx| {
                state.set_items(SearchableVec::new(locations), window, cx);
            });
        });
        self.location_filter = self.location.read(cx).selected_value().cloned();
    }

    fn toggle(&mut self, id: String, cx: &mut Context<Self>) {
        if !self.selected.remove(&id) {
            self.selected.insert(id);
        }
        self.refresh_marks(cx);
    }

    /// Redraw the checkboxes without touching the rows, so a tick does not
    /// scroll the table back to the top.
    fn refresh_marks(&mut self, cx: &mut Context<Self>) {
        let selected = self.selected.clone();
        self.table.update(cx, |table, cx| {
            table.delegate_mut().selected = selected;
            cx.notify();
        });
        cx.notify();
    }

    fn select_all(&mut self, cx: &mut Context<Self>) {
        for row in self.visible_rows() {
            self.selected.insert(row.id);
        }
        self.refresh_marks(cx);
    }

    fn clear_selection(&mut self, cx: &mut Context<Self>) {
        self.selected.clear();
        self.refresh_marks(cx);
    }

    /// The ticked ids that are still in the Open list, in the order the server
    /// ranked them. A stale id from a stage change is not sent.
    fn selected_open_ids(&self) -> Vec<String> {
        self.rows_for(Stage::Open)
            .into_iter()
            .filter(|row| self.selected.contains(&row.id))
            .map(|row| row.id)
            .collect()
    }

    fn briefs_for_selection(&mut self, cx: &mut Context<Self>) {
        let ids = self.selected_open_ids();
        if ids.is_empty() {
            return;
        }
        let count = ids.len();
        let _ = self.commands.send(Command::MakeBriefs { posting_ids: ids });
        self.selected.clear();
        self.note = Some(format!(
            "Generating {count} brief{}…",
            if count == 1 { "" } else { "s" }
        ));
        self.refresh_marks(cx);
    }

    fn open_row(&mut self, row_ix: usize, cx: &mut Context<Self>) {
        let id = self
            .table
            .read(cx)
            .delegate()
            .rows
            .get(row_ix)
            .map(|row| row.id.clone());
        if let Some(id) = id {
            self.open_detail(id, cx);
        }
    }

    fn open_detail(&mut self, id: String, cx: &mut Context<Self>) {
        if self.detail_id.as_deref() != Some(id.as_str()) {
            self.detail = None;
        }
        self.detail_id = Some(id.clone());
        let _ = self
            .commands
            .send(Command::PostingDetail { posting_id: id });
        cx.notify();
    }

    fn close_detail(&mut self, cx: &mut Context<Self>) {
        if self.detail_id.take().is_some() {
            self.detail = None;
            cx.notify();
        }
    }

    /// Ask again for the open posting, after an action the server answered with
    /// a new dashboard.
    fn reload_detail(&self) {
        if let Some(id) = &self.detail_id {
            let _ = self.commands.send(Command::PostingDetail {
                posting_id: id.clone(),
            });
        }
    }

    fn make_brief(&mut self, posting_id: String) {
        let _ = self.commands.send(Command::MakeBrief { posting_id });
        self.note = Some("Generating one brief…".into());
    }

    fn apply(&mut self, posting_id: String) {
        let _ = self.commands.send(Command::Apply { posting_id });
    }

    fn unapply(&mut self, posting_id: String) {
        let _ = self.commands.send(Command::Unapply {
            posting_ids: vec![posting_id],
        });
    }

    /// The window's title bar: the wordmark, the current section and the scan
    /// controls. `WindowControlArea::Drag` gives the drag and the double-click
    /// zoom to AppKit, so neither needs a handler here.
    fn render_title_bar(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let scanning = matches!(self.scan_state.as_ref(), Some(ScanState::Running { .. }));
        let left = h_flex()
            .items_center()
            .gap_3()
            .pl_1()
            .child(
                div()
                    .text_size(px(15.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .font_family("Optima")
                    .text_color(cx.theme().primary)
                    .child("Kangaroo"),
            )
            .child(
                div()
                    .w(px(1.))
                    .h(px(14.))
                    .bg(cx.theme().border)
                    .flex_shrink_0(),
            )
            .child(
                div()
                    .text_sm()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(cx.theme().secondary)
                    .child(self.section.title()),
            );
        let right = h_flex()
            .items_center()
            .gap_2()
            .pr_2()
            .child(self.scan_chip(cx))
            .when(scanning, |el| {
                el.child(
                    Button::new("cancel-scan")
                        .ghost()
                        .small()
                        .label("Cancel")
                        .on_click({
                            let tx = self.commands.clone();
                            move |_, _, _| {
                                let _ = tx.send(Command::CancelScan);
                            }
                        }),
                )
            })
            .child(
                Button::new("scan-now")
                    .primary()
                    .small()
                    .icon(Icon::new(IconName::RefreshCw).with_size(px(13.)))
                    .label("Scan")
                    .on_click({
                        let tx = self.commands.clone();
                        move |_, _, _| {
                            let _ = tx.send(Command::Scan {
                                mode: ScanMode::Quick,
                            });
                        }
                    }),
            )
            .children(Kbd::binding_for_action(&ScanNow, None, window).map(|kbd| {
                div()
                    .text_color(cx.theme().muted_foreground)
                    .child(kbd.appearance(false))
            }));
        div()
            .id("title-bar")
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .h(px(TITLE_BAR_HEIGHT))
            .w_full()
            .flex_shrink_0()
            .pl(px(84.))
            .pr_3()
            .bg(crate::theme::sidebar(self.tint))
            .border_b_1()
            .border_color(cx.theme().border)
            .window_control_area(gpui::WindowControlArea::Drag)
            .child(left)
            .child(right)
            .into_any_element()
    }

    /// Take the tint the settings hold, once, unless `--tint` chose one. A
    /// client that starts fresh reports the stored value on its first snapshot.
    fn apply_stored_tint(&mut self, settings: &serde_json::Value, cx: &mut Context<Self>) {
        if self.tint_from_flag {
            return;
        }
        let Some(stored) = settings
            .get("tint")
            .and_then(serde_json::Value::as_str)
            .and_then(Tint::from_arg)
        else {
            return;
        };
        self.tint_from_flag = true;
        if stored != self.tint {
            self.tint = stored;
            crate::theme::apply(cx, stored);
            self.sync_table(cx);
            cx.notify();
        }
    }

    /// Choose the tint and write it to the settings, so the next launch starts
    /// here.
    fn set_tint(&mut self, tint: Tint, cx: &mut Context<Self>) {
        if self.tint == tint {
            return;
        }
        self.tint = tint;
        self.tint_from_flag = true;
        crate::theme::apply(cx, tint);
        let mut values = if self.settings.is_object() {
            self.settings.clone()
        } else {
            json!({})
        };
        values["tint"] = json!(tint.name().to_lowercase());
        self.settings = values.clone();
        let _ = self.commands.send(Command::SaveSettings {
            settings: Settings { values },
        });
        self.sync_table(cx);
        cx.notify();
    }

    /// The scan state as one quiet line: a status dot and a few words. The
    /// button beside it is the action, so the chip never needs to be a button.
    fn scan_chip(&self, cx: &App) -> AnyElement {
        let (dot, text) = match &self.scan_state {
            Some(ScanState::Running { mode }) => (
                cx.theme().info,
                match mode {
                    ScanMode::Quick => "Scanning · quick",
                    ScanMode::Full => "Scanning · full",
                }
                .to_string(),
            ),
            Some(ScanState::Error(_)) => (cx.theme().danger, "Scan failed".to_string()),
            _ => {
                let last = self.snapshot.as_ref().and_then(|s| s.last_scan.as_ref());
                match last {
                    Some(scan) => (
                        cx.theme().success,
                        format!("Scanned {}", short_stamp(scan.finished_at.as_deref())),
                    ),
                    None => (cx.theme().muted_foreground, "No scan yet".to_string()),
                }
            }
        };
        h_flex()
            .items_center()
            .gap_1p5()
            .px_2()
            .py_1()
            .rounded_full()
            .bg(cx.theme().background.opacity(0.7))
            .child(div().size(px(6.)).rounded_full().bg(dot).flex_shrink_0())
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(text),
            )
            .into_any_element()
    }

    /// The page title, a one-line summary, and whatever actions the page owns.
    fn render_page_header(
        &self,
        title: &str,
        subtitle: String,
        actions: Option<AnyElement>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        h_flex()
            .items_end()
            .justify_between()
            .gap_4()
            .pt_1()
            .child(
                v_flex()
                    .gap_0p5()
                    .child(
                        div()
                            .text_size(px(20.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(cx.theme().secondary)
                            .child(title.to_owned()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(subtitle),
                    ),
            )
            .children(actions)
            .into_any_element()
    }

    /// The left navigation: the three pipeline stages under one label, then the
    /// two system pages under another. Counts sit at the end of the row, so the
    /// labels stay in one column.
    fn render_sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        let open = self.rows_for(Stage::Open).len();
        let shortlisted = self.rows_for(Stage::Shortlisted).len();
        let applied = self.rows_for(Stage::Applied).len();
        v_flex()
            .w(px(SIDEBAR_WIDTH))
            .h_full()
            .flex_shrink_0()
            .bg(crate::theme::sidebar(self.tint))
            .border_r_1()
            .border_color(cx.theme().border)
            .px_2()
            .py_2()
            .child(self.nav_label("Pipeline", cx))
            .child(self.nav_item(Section::Postings, IconName::Inbox, Some(open), cx))
            .child(self.nav_item(Section::Shortlist, IconName::Star, Some(shortlisted), cx))
            .child(self.nav_item(Section::Applied, IconName::CircleCheck, Some(applied), cx))
            .child(self.nav_label("System", cx))
            .child(self.nav_item(Section::Health, IconName::Network, None, cx))
            .child(self.nav_item(Section::Settings, IconName::Settings, None, cx))
            .child(div().flex_1())
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .child(
                        Icon::new(IconName::Calendar)
                            .with_size(px(12.))
                            .text_color(cx.theme().muted_foreground),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("Next scan · {}", self.next_slot.format("%a %H:%M"))),
                    ),
            )
            .into_any_element()
    }

    /// The small caps heading that groups navigation rows.
    fn nav_label(&self, title: &str, _cx: &App) -> AnyElement {
        div()
            .px_2()
            .pt_2()
            .pb_1()
            .text_size(px(10.))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(crate::theme::section_ink(self.tint))
            .child(title.to_uppercase())
            .into_any_element()
    }

    /// One navigation row: icon, label, optional count. The active row lifts
    /// off the sidebar as a white card, which is how a macOS sidebar reads.
    fn nav_item(
        &self,
        section: Section,
        icon: IconName,
        count: Option<usize>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let active = self.section == section;
        let theme = cx.theme();
        div()
            .id(section.id())
            .h(px(30.))
            .px_2()
            .mb_0p5()
            .rounded(px(7.))
            .border_1()
            .border_color(if active { theme.border } else { theme.sidebar })
            .bg(match (active, crate::theme::active(cx)) {
                // On the light page the active row lifts to white; on the dark
                // one white is not available, so it takes the accent tint.
                (true, crate::theme::Tint::Light) => theme.background,
                (true, crate::theme::Tint::Dark) => theme.list_active,
                _ => theme.sidebar,
            })
            .relative()
            // The accent's second job in the chrome: a two-point mark at the
            // edge says where the reader is, in the place they look for it.
            .when(active, |el| {
                el.child(
                    div()
                        .absolute()
                        .left_0()
                        .top(px(6.))
                        .bottom(px(6.))
                        .w(px(2.))
                        .rounded_full()
                        .bg(theme.primary),
                )
            })
            .cursor_pointer()
            .when(!active, |el| {
                el.hover(|el| el.bg(crate::theme::hover(self.tint)))
            })
            .child(
                h_flex()
                    .h_full()
                    .items_center()
                    .gap_2()
                    .child(Icon::new(icon).with_size(px(15.)).text_color(if active {
                        theme.primary
                    } else {
                        theme.muted_foreground
                    }))
                    .child(
                        div()
                            .flex_1()
                            .truncate()
                            .text_size(px(13.))
                            .when(active, |el| {
                                el.font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(theme.secondary)
                            })
                            .when(!active, |el| el.text_color(theme.foreground))
                            .child(section.title()),
                    )
                    .children(count.map(|count| {
                        div()
                            .text_size(px(11.))
                            .text_color(if active {
                                theme.primary
                            } else {
                                theme.muted_foreground
                            })
                            .child(count.to_string())
                    })),
            )
            .on_click(cx.listener(move |this, _, _, cx| this.set_section(section, cx)))
            .into_any_element()
    }

    /// The status and the batch-brief controls for the table card's header. The
    /// status sits next to the buttons that change it, so a reader never looks
    /// in two places for one state.
    fn render_batch_controls(&self, cx: &mut Context<Self>) -> (AnyElement, AnyElement) {
        let rows = self.visible_rows().len();
        let selected = self.selected_open_ids().len();
        let enabled = self
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.briefs_enabled)
            .unwrap_or(false);
        let open = self.stage == Stage::Open;
        let status_text = if let Some(note) = &self.note {
            note.clone()
        } else if !enabled {
            "No model key".into()
        } else if open {
            selection_note(selected, rows).to_string()
        } else {
            format!("{rows} postings")
        };
        let status = div()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(status_text)
            .into_any_element();
        let buttons = if open {
            h_flex()
                .items_center()
                .gap_1()
                .child(
                    Button::new("select-all")
                        .ghost()
                        .xsmall()
                        .label("Select all")
                        .on_click(cx.listener(|this, _, _, cx| this.select_all(cx))),
                )
                .child(
                    Button::new("clear-selection")
                        .ghost()
                        .xsmall()
                        .label("Clear")
                        .disabled(selected == 0)
                        .on_click(cx.listener(|this, _, _, cx| this.clear_selection(cx))),
                )
                .child(
                    Button::new("briefs-for-selection")
                        .primary()
                        .small()
                        .icon(Icon::new(IconName::Bot).with_size(px(13.)))
                        .label(match selected {
                            0 => "Brief".to_string(),
                            n => format!("Brief {n}"),
                        })
                        .disabled(selected == 0 || !enabled)
                        .on_click(cx.listener(|this, _, _, cx| this.briefs_for_selection(cx))),
                )
                .into_any_element()
        } else {
            div().into_any_element()
        };
        (status, buttons)
    }

    /// The table card: the filters and batch controls in its header, the rows in
    /// the middle, and the page footer at the bottom. It is the only white
    /// surface on a postings page, so it reads as the content.
    fn render_table(&self, cx: &mut Context<Self>) -> AnyElement {
        let (status, buttons) = self.render_batch_controls(cx);
        v_flex()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .rounded(px(10.))
            .border_1()
            .border_color(cx.theme().border)
            .bg(crate::theme::card(self.tint))
            .overflow_hidden()
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        div().w(px(260.)).child(
                            Input::new(&self.search)
                                .prefix(
                                    Icon::new(IconName::Search)
                                        .with_size(px(13.))
                                        .text_color(cx.theme().muted_foreground),
                                )
                                .cleanable(true),
                        ),
                    )
                    .child(
                        div().w(px(190.)).child(
                            Select::new(&self.location)
                                .placeholder("All locations")
                                .small()
                                .cleanable(true)
                                .accessibility_label("Location"),
                        ),
                    )
                    .child(div().flex_1())
                    .child(status)
                    .child(buttons),
            )
            .child(
                div().flex_1().min_h_0().child(
                    DataTable::new(&self.table)
                        .with_size(Size::Size(px(ROW_HEIGHT)))
                        .bordered(false),
                ),
            )
            .child(self.render_pagination(cx))
            .into_any_element()
    }

    /// The pagination footer under the table, only when there is more than one
    /// page. It shows the slice bounds and lets the reader move between pages.
    fn render_pagination(&self, cx: &mut Context<Self>) -> AnyElement {
        let total = self.total_pages().max(1);
        if total <= 1 {
            return div().into_any_element();
        }
        let count = self.visible_rows().len();
        let first = (self.table_page - 1) * self.page_size + 1;
        let last = (self.table_page * self.page_size).min(count);
        let view = cx.entity();
        h_flex()
            .items_center()
            .justify_between()
            .px_3()
            .py_2()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("{first}–{last} of {count}")),
            )
            .child(
                Pagination::new("postings-pages")
                    .current_page(self.table_page)
                    .total_pages(total)
                    .on_click(move |page, _window, cx| {
                        view.update(cx, |this, cx| this.set_table_page(*page, cx));
                    }),
            )
            .into_any_element()
    }

    /// The detail drawer: what the posting is, what it asks for, and the three
    /// actions the candidate can take. The actions sit in a footer, so they stay
    /// in reach while the posting text scrolls.
    fn render_detail(&self, detail: &PostingDetail, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let row = &detail.row;
        let id = row.id.clone();
        let applied = row.applied_at.is_some();
        let brief_status = row.brief_status.clone();
        let url = row.url.clone();
        let has_url = url.is_some();
        let enabled = self
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.briefs_enabled)
            .unwrap_or(false);
        let busy = matches!(brief_status.as_deref(), Some("running"));
        let can_brief = enabled && !busy;
        let brief = detail.brief.as_ref().map(detail::brief_view);
        let facts = detail::facts(detail);
        let text = detail::posting_text(detail);
        let mark_id = id.clone();
        let apply_id = id.clone();

        v_flex()
            .w(px(DETAIL_WIDTH))
            .flex_shrink_0()
            .h_full()
            .min_w_0()
            .min_h_0()
            .rounded(px(10.))
            .border_1()
            .border_color(theme.border)
            .bg(crate::theme::card(self.tint))
            .overflow_hidden()
            .child(
                v_flex()
                    .gap_2()
                    .p_3()
                    .child(
                        h_flex()
                            .items_start()
                            .justify_between()
                            .gap_2()
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_size(px(15.))
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .whitespace_normal()
                                            .child(row.title.clone()),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .text_color(theme.muted_foreground)
                                            .whitespace_normal()
                                            .child(detail::subtitle(row)),
                                    ),
                            )
                            .child(
                                Button::new("close-detail")
                                    .ghost()
                                    .xsmall()
                                    .icon(Icon::new(IconName::Close).with_size(px(13.)))
                                    .on_click(cx.listener(|this, _, _, cx| this.close_detail(cx))),
                            ),
                    )
                    .child(
                        h_flex()
                            .flex_wrap()
                            .gap_1()
                            .children(chips(detail, applied, cx)),
                    ),
            )
            .child(div().h(px(1.)).bg(theme.border).flex_shrink_0())
            .child(
                div()
                    .id("detail-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px_3()
                    .py_3()
                    .child(
                        v_flex()
                            .gap_3()
                            .children(brief.map(|brief| render_brief(&brief, cx)))
                            .child(
                                v_flex()
                                    .gap_1()
                                    .child(widgets::section_label("Signals", cx))
                                    .children(
                                        facts
                                            .iter()
                                            .filter(|(label, _)| {
                                                !matches!(
                                                    *label,
                                                    "Source" | "Board" | "Posted" | "Expires"
                                                )
                                            })
                                            .map(|(label, value)| {
                                                widgets::fact_row(
                                                    label,
                                                    fact_value(label, value.clone()),
                                                    cx,
                                                )
                                            }),
                                    ),
                            )
                            .child(
                                v_flex()
                                    .gap_1()
                                    .child(widgets::section_label("Listing", cx))
                                    .children(
                                        facts
                                            .iter()
                                            .filter(|(label, _)| {
                                                matches!(
                                                    *label,
                                                    "Source" | "Board" | "Posted" | "Expires"
                                                )
                                            })
                                            .map(|(label, value)| {
                                                widgets::fact_row(
                                                    label,
                                                    fact_value(label, value.clone()),
                                                    cx,
                                                )
                                            }),
                                    ),
                            )
                            .when_some(text, |el, text| {
                                el.child(
                                    v_flex()
                                        .gap_1()
                                        .child(widgets::section_label("Posting", cx))
                                        .child(
                                            div()
                                                .text_size(px(12.))
                                                .line_height(gpui::relative(1.5))
                                                .whitespace_normal()
                                                .text_color(cx.theme().foreground)
                                                .child(text),
                                        ),
                                )
                            }),
                    ),
            )
            .child(div().h(px(1.)).bg(theme.border).flex_shrink_0())
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .child(
                        Button::new("toggle-applied")
                            .outline()
                            .xsmall()
                            .icon(
                                Icon::new(if applied {
                                    IconName::CircleX
                                } else {
                                    IconName::CircleCheck
                                })
                                .with_size(px(13.)),
                            )
                            .label(if applied { "Applied" } else { "Mark applied" })
                            .on_click(cx.listener(move |this, _, _, _| {
                                if applied {
                                    this.unapply(apply_id.clone())
                                } else {
                                    this.apply(apply_id.clone())
                                }
                            })),
                    )
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .when(has_url, |el| {
                                el.child(
                                    Button::new("open-posting")
                                        .ghost()
                                        .xsmall()
                                        .icon(Icon::new(IconName::ExternalLink).with_size(px(13.)))
                                        .label("Open")
                                        .on_click(cx.listener(move |_, _, _, cx| {
                                            if let Some(url) = &url {
                                                cx.open_url(url);
                                            }
                                        })),
                                )
                            })
                            .child(
                                Button::new("make-brief")
                                    .primary()
                                    .small()
                                    .icon(Icon::new(IconName::Bot).with_size(px(13.)))
                                    .label(if busy {
                                        "Writing…"
                                    } else if brief_status.as_deref() == Some("ok") {
                                        "Rewrite"
                                    } else {
                                        "Brief"
                                    })
                                    .disabled(!can_brief)
                                    .on_click(cx.listener(move |this, _, _, _| {
                                        this.make_brief(mark_id.clone())
                                    })),
                            ),
                    ),
            )
            .into_any_element()
    }

    /// The Health page: what the last scan did, how the boards stand, and where
    /// the data lives. Two columns of cards, each a short list of metrics.
    fn render_health(&self, cx: &mut Context<Self>) -> AnyElement {
        let snapshot = self.snapshot.as_ref();
        let scanning = matches!(self.scan_state.as_ref(), Some(ScanState::Running { .. }));
        let state = match &self.scan_state {
            None => "Starting".to_string(),
            Some(ScanState::Idle) => "Idle".to_string(),
            Some(ScanState::Running { mode }) => format!(
                "Scanning ({})",
                match mode {
                    ScanMode::Quick => "quick",
                    ScanMode::Full => "full",
                }
            ),
            Some(ScanState::Error(_)) => "Failed".to_string(),
        };
        let last_scan = snapshot
            .and_then(|s| s.last_scan.as_ref())
            .map(|scan| {
                format!(
                    "{} · {} · {}",
                    scan.mode,
                    scan.status,
                    short_stamp(scan.finished_at.as_deref())
                )
            })
            .unwrap_or_else(|| "no scan has run".into());
        let next_slot = self.next_slot.format("%a %b %-d, %H:%M").to_string();
        let last_error = self.last_error.clone().unwrap_or_else(|| "none".into());
        let database = snapshot
            .map(|s| {
                if s.has_database {
                    s.database_path.clone()
                } else {
                    format!("{} (no database)", s.database_path)
                }
            })
            .unwrap_or_else(|| "starting…".into());
        let config = snapshot
            .map(|s| format!("{} file{}", s.config_files, plural(s.config_files)))
            .unwrap_or_else(|| "—".into());
        let briefs = snapshot
            .map(|s| if s.briefs_enabled { "on" } else { "off" })
            .unwrap_or("—")
            .to_string();
        let live = snapshot
            .map(|s| s.live_postings.to_string())
            .unwrap_or_else(|| "—".into());
        let seen = snapshot
            .map(|s| s.seen_count.to_string())
            .unwrap_or_else(|| "—".into());
        let (ok, failing, broken, total) = snapshot
            .map(|s| (s.board_ok, s.board_failing, s.board_broken, s.board_total))
            .unwrap_or_default();
        let dot = |tone: gpui::Hsla| {
            div()
                .size(px(7.))
                .rounded_full()
                .bg(tone)
                .flex_shrink_0()
                .into_any_element()
        };
        let (info, danger, success, secondary) = (
            cx.theme().info,
            cx.theme().danger,
            cx.theme().success,
            cx.theme().secondary,
        );

        v_flex()
            .id("health")
            .flex_1()
            .min_h_0()
            .gap_3()
            .child(self.render_page_header(
                "Health",
                format!("{live} live postings · {seen} seen · briefs {briefs}"),
                None,
                cx,
            ))
            .child(
                h_flex()
                    .items_stretch()
                    .gap_3()
                    .child(widgets::card(&*cx).child(widgets::stat_card(
                        "Scan",
                        vec![
                                h_flex()
                                    .items_center()
                                    .gap_2()
                                    .child(dot(if scanning {
                                        info
                                    } else if self.last_error.is_some() {
                                        danger
                                    } else {
                                        success
                                    }))
                                    .child(
                                        div()
                                            .text_size(px(18.))
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .text_color(secondary)
                                            .child(state),
                                    )
                                    .into_any_element(),
                                widgets::fact_row("Last", last_scan, &*cx),
                                widgets::fact_row("Next", next_slot, &*cx),
                                widgets::fact_row("Error", last_error, &*cx),
                            ],
                        &*cx,
                    )))
                    .child(widgets::card(&*cx).child(widgets::stat_card(
                        "Boards",
                        vec![
                            widgets::metric("Working", ok.to_string(), &*cx),
                            widgets::metric("Failing", failing.to_string(), &*cx),
                            widgets::metric("Broken", broken.to_string(), &*cx),
                            widgets::fact_row("Total", total.to_string(), &*cx),
                        ],
                        &*cx,
                    ))),
            )
            .child(
                h_flex()
                    .items_stretch()
                    .gap_3()
                    .child(widgets::card(&*cx).child(widgets::stat_card(
                        "Data",
                        vec![
                            widgets::fact_row("Database", database, &*cx),
                            widgets::fact_row("Config", config, &*cx),
                        ],
                        &*cx,
                    )))
                    .child(widgets::card(&*cx).child(widgets::stat_card(
                        "Postings",
                        vec![
                            widgets::metric("Live", live, &*cx),
                            widgets::metric("Seen", seen, &*cx),
                        ],
                        &*cx,
                    ))),
            )
            .into_any_element()
    }

    /// The postings page for the current pipeline stage: the title, then the
    /// table card with the detail drawer beside it while a row is open.
    fn render_postings(&self, cx: &mut Context<Self>) -> AnyElement {
        let count = self.rows_for(self.stage).len();
        let subtitle = match self.stage {
            Stage::Open => format!("{count} postings · newest first"),
            Stage::Shortlisted => format!("{count} with a brief"),
            Stage::Applied => format!("{count} marked applied"),
        };
        let header = self.render_page_header(self.section.title(), subtitle, None, cx);
        let table = self.render_table(cx);
        let drawer = self
            .detail
            .clone()
            .map(|detail| self.render_detail(&detail, cx));
        v_flex()
            .flex_1()
            .min_h_0()
            .gap_3()
            .child(header)
            .child(
                h_flex()
                    .items_stretch()
                    .flex_1()
                    .min_h_0()
                    .gap_3()
                    .child(table)
                    .children(drawer),
            )
            .into_any_element()
    }

    /// The settings view: still a placeholder until the real form exists.
    /// The Settings page: the appearance, the model key, the paths, and the
    /// shortcuts this window answers to.
    fn render_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let enabled = self
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.briefs_enabled)
            .unwrap_or(false);
        let (success, warning, muted) = (
            cx.theme().success,
            cx.theme().warning,
            cx.theme().muted_foreground,
        );
        let active = self.tint;
        // Two choices, drawn as one control: the current tint is filled, the
        // other is an outline that names what pressing it would do.
        let choice = |tint: Tint, label: &'static str, cx: &mut Context<Self>| {
            let button = Button::new(match tint {
                Tint::Light => "appearance-light",
                Tint::Dark => "appearance-dark",
            })
            .small()
            .icon(
                Icon::new(match tint {
                    Tint::Light => IconName::Sun,
                    Tint::Dark => IconName::Moon,
                })
                .with_size(px(13.)),
            )
            .label(label);
            let button = if tint == active {
                button.primary()
            } else {
                button.outline()
            };
            button
                .on_click(cx.listener(move |this, _, _, cx| this.set_tint(tint, cx)))
                .into_any_element()
        };
        let key_rows = vec![
            h_flex()
                .items_center()
                .gap_2()
                .child(widgets::chip(
                    if enabled { "on" } else { "off" },
                    if enabled { success } else { warning },
                ))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(muted)
                        .whitespace_normal()
                        .child(if enabled {
                            "A model key is present, so briefs can run."
                        } else {
                            "Set DEEPSEEK_API_KEY, or store the key in the Keychain as \
                             com.muhanli.kangaroo.deepseek."
                        }),
                )
                .into_any_element(),
        ];
        let database = self
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.database_path.clone())
            .unwrap_or_else(|| "starting…".into());
        v_flex()
            .flex_1()
            .min_h_0()
            .gap_3()
            .child(self.render_page_header(
                "Settings",
                "Appearance, model key, paths and keyboard shortcuts".into(),
                None,
                cx,
            ))
            .child(
                h_flex()
                    .items_stretch()
                    .gap_3()
                    .child(widgets::card(&*cx).child(widgets::stat_card(
                        "Appearance",
                        vec![
                                h_flex()
                                    .items_center()
                                    .gap_1()
                                    .child(choice(Tint::Light, "Light", cx))
                                    .child(choice(Tint::Dark, "Dark", cx))
                                    .into_any_element(),
                                div()
                                    .text_size(px(11.))
                                    .text_color(muted)
                                    .whitespace_normal()
                                    .child("The choice is saved and used at the next launch.")
                                    .into_any_element(),
                            ],
                        &*cx,
                    )))
                    .child(widgets::card(&*cx).child(widgets::stat_card("Briefs", key_rows, &*cx))),
            )
            .child(
                h_flex()
                    .items_stretch()
                    .gap_3()
                    .child(widgets::card(&*cx).child(widgets::stat_card(
                        "Data",
                        vec![
                            widgets::fact_row("Database", database, &*cx),
                            widgets::fact_row(
                                "Socket",
                                "data/state/kangaroo.sock".to_string(),
                                &*cx,
                            ),
                        ],
                        &*cx,
                    )))
                    .child(
                        widgets::card(&*cx).child(
                            v_flex()
                                .flex_1()
                                .min_w(px(220.))
                                .gap_1p5()
                                .p_3()
                                .child(widgets::section_label("Shortcuts", &*cx))
                                .children(
                                    [
                                        ("Scan now", "⌘R"),
                                        ("Filter", "⌘F"),
                                        ("Settings", "⌘,"),
                                        ("Close the drawer", "Esc"),
                                        ("Quit", "⌘Q"),
                                    ]
                                    .into_iter()
                                    .map(|(label, keys)| {
                                        h_flex()
                                            .justify_between()
                                            .items_center()
                                            .child(div().text_size(px(12.)).child(label))
                                            .child(
                                                div()
                                                    .text_size(px(11.))
                                                    .text_color(muted)
                                                    .child(keys),
                                            )
                                            .into_any_element()
                                    }),
                                ),
                        ),
                    ),
            )
            .into_any_element()
    }
}

fn plural(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

/// A fact as the drawer shows it: a date as day, month and year, a timestamp as
/// the local day and clock time. Anything else passes through unchanged.
fn fact_value(label: &str, value: String) -> String {
    match label {
        "Posted" | "Expires" => match chrono::NaiveDate::parse_from_str(&value, "%Y-%m-%d") {
            Ok(date) => date.format("%-d %b %Y").to_string(),
            Err(_) => value,
        },
        "Seen" | "Last seen" | "Applied" => match chrono::DateTime::parse_from_rfc3339(&value) {
            Ok(time) => time
                .with_timezone(&Local)
                .format("%-d %b %H:%M")
                .to_string(),
            Err(_) => value,
        },
        _ => value,
    }
}

/// A scan timestamp as the title bar shows it: the weekday and the clock time.
/// The server may hand over an ISO timestamp or an already formatted string, so
/// a value that does not parse is shown as it came.
fn short_stamp(stamp: Option<&str>) -> String {
    let Some(stamp) = stamp.filter(|text| !text.trim().is_empty()) else {
        return "never".into();
    };
    if let Ok(time) = chrono::DateTime::parse_from_rfc3339(stamp) {
        return time.with_timezone(&Local).format("%a %H:%M").to_string();
    }
    if let Ok(time) = chrono::NaiveDateTime::parse_from_str(stamp, "%Y-%m-%d %H:%M:%S") {
        return time.format("%a %H:%M").to_string();
    }
    stamp.to_string()
}

/// The small labels under the drawer title: stage, tier, fit, and brief state.
fn chips(detail: &PostingDetail, applied: bool, cx: &App) -> Vec<AnyElement> {
    let row = &detail.row;
    let mut chips: Vec<AnyElement> = Vec::new();
    if applied {
        chips.push(chip(
            "applied",
            cx.theme().success,
            cx.theme().background,
            cx,
        ));
    } else if let Some(status) = row.brief_status.as_deref() {
        let color = match status {
            "ok" => cx.theme().success,
            "running" => cx.theme().info,
            _ => cx.theme().danger,
        };
        chips.push(chip(
            &format!("brief {status}"),
            color,
            cx.theme().background,
            cx,
        ));
    }
    if let Some(tier) = row.tier.as_deref() {
        chips.push(chip(
            &tier.replace('_', " "),
            cx.theme().muted_foreground,
            cx.theme().muted,
            cx,
        ));
    }
    if !row.has_text {
        chips.push(chip(
            "no cached text",
            cx.theme().warning,
            cx.theme().muted,
            cx,
        ));
    }
    chips
}

fn chip(text: &str, color: gpui::Hsla, bg: gpui::Hsla, _cx: &App) -> AnyElement {
    div()
        .px_2()
        .py_0p5()
        .rounded_md()
        .bg(bg)
        .text_xs()
        .text_color(color)
        .child(text.to_owned())
        .into_any_element()
}

/// The brief block: the verdict first, because that is the ten-second answer,
/// then the skills it names, the lines it would reword, and what is missing.
fn render_brief(brief: &BriefView, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let palette = crate::theme::active(cx);
    let status_tone = match brief.status.as_str() {
        "ok" => theme.success,
        "running" => theme.info,
        _ => theme.danger,
    };
    v_flex()
        .gap_2()
        .p_3()
        .rounded(px(8.))
        .bg(theme.muted)
        .child(
            h_flex()
                .justify_between()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_size(px(10.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(crate::theme::secondary(palette))
                        .child("BRIEF"),
                )
                .child(
                    h_flex()
                        .items_center()
                        .gap_1p5()
                        .child(
                            div()
                                .size(px(6.))
                                .rounded_full()
                                .bg(status_tone)
                                .flex_shrink_0(),
                        )
                        .child(
                            div()
                                .text_size(px(10.))
                                .text_color(theme.muted_foreground)
                                .child(format!(
                                    "{} · {}",
                                    brief.status,
                                    short_stamp(brief.generated_at.as_deref())
                                )),
                        ),
                ),
        )
        .when(!brief.verdict.is_empty(), |el| {
            el.child(
                div()
                    .text_size(px(13.))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .whitespace_normal()
                    .child(brief.verdict.clone()),
            )
        })
        .when(!brief.key_skills.is_empty(), |el| {
            el.child(
                h_flex().flex_wrap().gap_1().children(
                    brief.key_skills.iter().map(|skill| {
                        widgets::chip(skill.clone(), crate::theme::secondary(palette))
                    }),
                ),
            )
        })
        .when_some(brief.lead_with.clone(), |el, lead| {
            el.child(
                div()
                    .text_size(px(12.))
                    .text_color(theme.muted_foreground)
                    .whitespace_normal()
                    .child(lead),
            )
        })
        .children(brief.rewords.iter().map(|reword| {
            v_flex()
                .gap_1()
                .p_2()
                .rounded(px(6.))
                .bg(theme.background)
                .child(
                    div()
                        .text_size(px(10.))
                        .text_color(theme.muted_foreground)
                        .child(format!("{} · {}", reword.requirement, reword.location)),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .whitespace_normal()
                        .child(reword.after.clone()),
                )
                .into_any_element()
        }))
        .when(!brief.missing.is_empty(), |el| {
            el.child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(10.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(theme.muted_foreground)
                            .child("GAPS"),
                    )
                    .children(brief.missing.iter().map(|gap| {
                        h_flex()
                            .items_start()
                            .gap_1p5()
                            .child(
                                div()
                                    .mt(px(6.))
                                    .size(px(3.))
                                    .rounded_full()
                                    .bg(theme.muted_foreground)
                                    .flex_shrink_0(),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .whitespace_normal()
                                    .child(gap.clone()),
                            )
                            .into_any_element()
                    })),
            )
        })
        .when_some(brief.notes.clone(), |el, notes| {
            el.child(
                div()
                    .text_size(px(12.))
                    .text_color(theme.muted_foreground)
                    .whitespace_normal()
                    .child(notes),
            )
        })
        .when(brief.dropped > 0, |el| {
            el.child(
                div()
                    .text_size(px(11.))
                    .text_color(theme.warning)
                    .whitespace_normal()
                    .child(format!(
                        "{} proposed edit{} dropped: the quoted line was not in the resume.",
                        brief.dropped,
                        plural(brief.dropped)
                    )),
            )
        })
        .when(!brief.flags.is_empty(), |el| {
            el.child(
                div()
                    .text_size(px(11.))
                    .text_color(theme.warning)
                    .whitespace_normal()
                    .child(format!(
                        "Not found in either text: {}",
                        brief.flags.join(", ")
                    )),
            )
        })
        .when_some(brief.error.clone(), |el, error| {
            el.child(
                div()
                    .text_size(px(11.))
                    .text_color(theme.danger)
                    .whitespace_normal()
                    .child(error),
            )
        })
        .into_any_element()
}

impl Render for AppView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The page size follows the window height, so a resize re-cuts the rows
        // instead of leaving blank space under the last one.
        self.sync_layout(window, cx);
        let title_bar = self.render_title_bar(window, cx);
        let sidebar = self.render_sidebar(cx);
        let body = match self.section {
            Section::Settings => self.render_settings(cx),
            Section::Health => self.render_health(cx),
            Section::Postings | Section::Shortlist | Section::Applied => self.render_postings(cx),
        };
        h_flex()
            .on_action(cx.listener(|this, _: &ScanNow, _, _| {
                let _ = this.commands.send(Command::Scan {
                    mode: ScanMode::Quick,
                });
            }))
            .on_action(
                cx.listener(|this, _: &OpenSettings, _, cx| {
                    this.set_section(Section::Settings, cx)
                }),
            )
            .on_action(cx.listener(|_, _: &QuitApp, _, cx| cx.quit()))
            .on_action(cx.listener(|this, _: &CloseDetail, _, cx| this.close_detail(cx)))
            .on_action(cx.listener(|this, _: &FocusSearch, window, cx| {
                let search = this.search.clone();
                search.update(cx, |state, cx| state.focus(window, cx));
            }))
            .flex()
            .flex_col()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .font_family(cx.theme().font_family.clone())
            .child(title_bar)
            .child(
                h_flex().flex_1().w_full().min_h_0().child(sidebar).child(
                    v_flex()
                        .flex_1()
                        .h_full()
                        .min_w_0()
                        .min_h_0()
                        .p_4()
                        .gap_3()
                        .child(body),
                ),
            )
    }
}

fn main() {
    // `--open-first` opens the top posting's drawer once the first snapshot
    // lands. It exists for screenshots and manual checks, like the section
    // flags beside it; nothing in normal use passes it.
    let open_first = std::env::args().any(|arg| arg == "--open-first");
    // `--tint light|dark` starts in that tint. Temporary, beside the switch in
    // the title bar.
    let tint_flag = std::env::args()
        .skip_while(|arg| arg != "--tint")
        .nth(1)
        .and_then(|name| Tint::from_arg(&name));
    let tint = tint_flag.unwrap_or_default();
    let start_section = if std::env::args().any(|arg| arg == "--settings") {
        Some(Section::Settings)
    } else if std::env::args().any(|arg| arg == "--health") {
        Some(Section::Health)
    } else {
        None
    };
    let client = match client::Client::connect_or_start() {
        Ok(client) => client,
        Err(error) => {
            eprintln!("could not reach the menu-bar server: {error:#}");
            eprintln!("start it with: cargo run -p kangaroo-server");
            return;
        }
    };
    let commands = client.commands.clone();
    let events = client.events.clone();

    application()
        .with_assets(gpui_kit_assets::Assets)
        .run(move |cx| {
            gpui_component::init(cx);
            theme::apply(cx, tint);
            cx.bind_keys([
                KeyBinding::new("cmd-r", ScanNow, None),
                KeyBinding::new("cmd-f", FocusSearch, None),
                KeyBinding::new("cmd-,", OpenSettings, None),
                KeyBinding::new("cmd-q", QuitApp, None),
                KeyBinding::new("escape", CloseDetail, None),
            ]);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(1240.), px(780.)), cx)),
                window_min_size: Some(size(px(900.), px(560.))),
                // The app draws the title bar, so AppKit keeps only the traffic
                // lights, placed where our own bar leaves room for them. AppKit
                // still owns the drag and the double-click zoom.
                titlebar: Some(gpui::TitlebarOptions {
                    title: None,
                    appears_transparent: true,
                    traffic_light_position: Some(gpui::point(px(16.), px(13.))),
                }),
                ..Default::default()
            };
            let app_commands = commands.clone();
            let view_handle = Arc::new(Mutex::new(None::<Entity<AppView>>));
            let created_view = view_handle.clone();
            let handle = cx
                .open_window(options, move |window, cx| {
                    window.set_window_title("Kangaroo");
                    window.on_window_should_close(cx, |_, cx| {
                        cx.quit();
                        true
                    });
                    let view = cx.new(|cx| {
                        AppView::new(
                            window,
                            cx,
                            app_commands,
                            open_first,
                            tint,
                            tint_flag.is_some(),
                        )
                    });
                    if let Ok(mut slot) = created_view.lock() {
                        *slot = Some(view.clone());
                    }
                    // The window's root is a gpui-component Root, which mounts the
                    // WindowState plugin that owns dialogs, notifications and the
                    // theme surface. Without it, `open_dialog` has no state to
                    // write to and panics.
                    cx.new(|cx| gpui_component::Root::new(view, window, cx))
                })
                .expect("failed to open Kangaroo window");
            let _window: AnyWindowHandle = handle.into();
            if let Some(section) = start_section {
                let view = view_handle.clone();
                let _ = _window.update(cx, move |_, _, cx| {
                    if let Ok(slot) = view.lock()
                        && let Some(view) = slot.clone()
                    {
                        view.update(cx, |this, cx| this.set_section(section, cx));
                    }
                });
            }
            let event_view = view_handle.clone();
            cx.spawn(move |cx: &mut gpui::AsyncApp| {
                let cx = cx.clone();
                async move {
                    while let Ok(event) = events.recv_async().await {
                        let target = event_view.lock().ok().and_then(|view| view.clone());
                        let Some(target) = target else {
                            continue;
                        };
                        cx.update(|cx| {
                            target.update(cx, |view, cx| {
                                match event {
                                    EngineEvent::Dashboard(snapshot) => {
                                        let stored = snapshot.settings.clone();
                                        view.apply_stored_tint(&stored, cx);
                                        view.settings = stored;
                                        if view.open_first
                                            && let Some(first) = snapshot.open.first()
                                        {
                                            view.open_first = false;
                                            let id = first.id.clone();
                                            view.open_detail(id, cx);
                                        }
                                        view.scan_state = Some(snapshot.scan_state.clone());
                                        view.snapshot = Some(snapshot);
                                        view.refresh_location_options(cx);
                                        view.sync_table(cx);
                                        // An action that wrote to the database makes
                                        // the open drawer stale: ask for it again.
                                        view.reload_detail();
                                    }
                                    EngineEvent::PostingDetail(detail) => {
                                        if view.detail_id.as_deref() == Some(detail.row.id.as_str())
                                        {
                                            view.detail = Some(detail);
                                        }
                                    }
                                    EngineEvent::ScanStatus(state) => {
                                        view.scan_state = Some(state);
                                        view.note = None;
                                    }
                                    EngineEvent::Error { message, .. } => {
                                        view.last_error = Some(message);
                                        view.note = None;
                                    }
                                    EngineEvent::BriefUpdated { posting_id, state } => {
                                        if view.detail_id.as_deref() == Some(posting_id.as_str()) {
                                            view.detail = None;
                                        }
                                        view.note = Some(match state.as_str() {
                                            "ok" => "Brief written.".into(),
                                            "skipped" => "That posting already has a brief.".into(),
                                            _ => "The brief failed. Open the posting to read why."
                                                .to_string(),
                                        });
                                    }
                                }
                                cx.notify();
                            });
                        });
                    }
                }
            })
            .detach();
        });
}

/// The next 09:00 / 13:00 / 17:00 weekday slot, shown as a hint. The server
/// owns the actual schedule; this copy is display-only.
fn kangaroo_engine_next_slot() -> DateTime<Local> {
    use chrono::{Datelike, Duration, NaiveTime, Weekday};
    let now = Local::now();
    let slots = [
        NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
        NaiveTime::from_hms_opt(13, 0, 0).unwrap(),
        NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
    ];
    for day_offset in 0..8 {
        let date = now.date_naive() + Duration::days(day_offset);
        if matches!(date.weekday(), Weekday::Sat | Weekday::Sun) {
            continue;
        }
        for slot in slots {
            if let Some(candidate) = date
                .and_time(slot)
                .and_local_timezone(Local)
                .single()
                .filter(|candidate| *candidate > now)
            {
                return candidate;
            }
        }
    }
    now.date_naive()
        .and_hms_opt(9, 0, 0)
        .unwrap()
        .and_local_timezone(Local)
        .single()
        .unwrap()
}
