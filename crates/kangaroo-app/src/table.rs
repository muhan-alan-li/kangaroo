//! The postings table. It draws four columns — date, location, company and
//! title — and, in the Open stage, a checkbox that marks a row for a batch
//! brief. The Date column is sortable. A row click is the table's own
//! `SelectRow` event; a checkbox click stops there and sends
//! [`RowEvent::Toggle`] instead.
//!
//! The title column takes the width the other columns leave, so the table fills
//! its card at any window size instead of ending in an empty sliver.

use crate::Stage;
use gpui::prelude::*;
use gpui::{AnyElement, App, Context, SharedString, Window, div, px};
use gpui_component::{
    ActiveTheme as _, Icon, IconName, Sizable as _,
    table::{Column, ColumnSort, TableDelegate, TableState},
};
use kangaroo_protocol::PostingRow;
use std::collections::HashSet;

/// What a click inside a cell asks the view to do.
#[derive(Debug, Clone)]
pub enum RowEvent {
    /// The reader ticked or unticked one row.
    Toggle(String),
    /// The reader asked to sort the table by the Date column.
    Sort(ColumnSort),
}

pub struct PostingTable {
    pub rows: Vec<PostingRow>,
    pub selected: HashSet<String>,
    pub stage: Stage,
    pub empty: SharedString,
    /// The current sort of the Date column. The view mirrors this value, so it
    /// can re-apply the sort after a new snapshot replaces the rows.
    pub sort: ColumnSort,
    /// The width the table draws into, set by the view on every render. The
    /// title column takes whatever the fixed columns leave.
    pub table_width: f32,
    tx: flume::Sender<RowEvent>,
}

impl PostingTable {
    pub fn new(tx: flume::Sender<RowEvent>) -> Self {
        Self {
            rows: Vec::new(),
            selected: HashSet::new(),
            stage: Stage::Open,
            empty: "Nothing here yet.".into(),
            sort: ColumnSort::Descending,
            table_width: 900.,
            tx,
        }
    }

    /// The Open stage is the only one that can start a brief, so it is the only
    /// one that draws a checkbox.
    fn marks_rows(&self) -> bool {
        self.stage == Stage::Open
    }
}

/// Header label and width for the fields that do not stretch, in the order they
/// are drawn. The title column follows them and takes the rest. The width is the
/// Calm one; [`widths`] adjusts it per style.
const COLUMNS: [(&str, f32); 3] = [("Date", 92.), ("Location", 210.), ("Company", 180.)];

/// The title column's index among the fields, after the fixed ones.
const TITLE_INDEX: usize = COLUMNS.len();

/// How many columns the table draws: the fixed fields, the title, and the
/// marker when the stage draws one.
fn column_count(marks: bool) -> usize {
    COLUMNS.len() + 1 + usize::from(marks)
}

/// One column's header, width and alignment. The Date column is sortable and
/// reports the current [`ColumnSort`], so its header shows the sort direction.
fn column_spec(marks: bool, index: usize, sort: ColumnSort, table_width: f32) -> Column {
    if marks && index == 0 {
        return Column::new("mark", "").width(px(34.)).p_0();
    }
    let field = index - usize::from(marks);
    // The title column is the one that grows; everything else keeps its width.
    // The drawer takes 400 points from the table, so the fixed columns shrink
    // with it: a narrow table shows fewer characters, not a squeezed title.
    let compact = table_width < 780.;
    fn base(compact: bool, name: &'static str, width: f32) -> (&'static str, f32) {
        let (date, location, company): (f32, f32, f32) = (92., 210., 180.);
        match (compact, name) {
            (true, "Location") => (name, (location * 0.72).round()),
            (true, "Company") => (name, (company * 0.74).round()),
            (true, "Date") => (name, (date * 0.9).round()),
            (_, "Date") => (name, date),
            (_, "Location") => (name, location),
            (_, "Company") => (name, company),
            _ => (name, width),
        }
    }
    let (name, width) = if field == TITLE_INDEX {
        let fixed: f32 = COLUMNS
            .iter()
            .map(|(name, width)| base(compact, name, *width).1)
            .sum();
        let marker = if marks { 34. } else { 0. };
        ("Title", (table_width - fixed - marker).max(200.))
    } else {
        let offset = usize::from(field > TITLE_INDEX);
        let (name, width) = COLUMNS[field - offset];
        base(compact, name, width)
    };
    let column = Column::new(name.to_lowercase(), name).width(px(width));
    match name {
        "Date" => column.sort(sort),
        _ => column,
    }
}

/// Sort a list of postings by their publish date, in the direction the Date
/// column reports. `ColumnSort::Default` keeps the server's own order. A
/// posting with no date always sorts last, matching the parity rule.
pub fn sort_rows(mut rows: Vec<PostingRow>, sort: ColumnSort) -> Vec<PostingRow> {
    let descending = match sort {
        ColumnSort::Descending => Some(true),
        ColumnSort::Ascending => Some(false),
        ColumnSort::Default => None,
    };
    let Some(descending) = descending else {
        return rows;
    };
    rows.sort_by(|a, b| postings_cmp(a, b, descending));
    rows
}

fn postings_cmp(a: &PostingRow, b: &PostingRow, descending: bool) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (&a.posted, &b.posted) {
        (Some(a), Some(b)) => {
            if descending {
                b.cmp(a)
            } else {
                a.cmp(b)
            }
        }
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

impl TableDelegate for PostingTable {
    fn columns_count(&self, _: &App) -> usize {
        column_count(self.marks_rows())
    }

    fn rows_count(&self, _: &App) -> usize {
        self.rows.len()
    }

    fn column(&self, index: usize, _: &App) -> Column {
        column_spec(self.marks_rows(), index, self.sort, self.table_width)
    }

    fn perform_sort(
        &mut self,
        _col_ix: usize,
        sort: ColumnSort,
        _window: &mut Window,
        _cx: &mut Context<TableState<Self>>,
    ) {
        self.sort = sort;
        // The rows live in the view; ask it to re-sort and redraw them.
        let _ = self.tx.send(RowEvent::Sort(sort));
    }

    /// A column header is a label, not text: small and quiet, so the rows win
    /// the eye.
    fn render_th(
        &mut self,
        col_ix: usize,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let name = self.column(col_ix, cx).name.clone();
        div()
            .w_full()
            .text_size(px(11.))
            .font_weight(gpui::FontWeight::MEDIUM)
            .text_color(cx.theme().table_head_foreground)
            .child(name)
    }

    fn cell_label(&self, row_ix: usize, col_ix: usize, _: &App) -> Option<SharedString> {
        Some(cell_text(self.rows.get(row_ix)?, self.marks_rows(), col_ix)?.into())
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        self.cell(row_ix, col_ix, cx)
    }

    fn render_empty(
        &mut self,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        crate::widgets::empty_state(
            IconName::Inbox,
            "Nothing here",
            &self.empty.clone(),
            None,
            cx,
        )
    }
}

impl PostingTable {
    fn cell(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        cx: &mut Context<TableState<Self>>,
    ) -> AnyElement {
        let marks = self.marks_rows();
        let Some(row) = self.rows.get(row_ix) else {
            return div().into_any_element();
        };
        if marks && col_ix == 0 {
            let checked = self.selected.contains(&row.id);
            let id = row.id.clone();
            let tx = self.tx.clone();
            return div()
                .id(("mark", row_ix))
                .h_full()
                .w_full()
                .flex()
                .items_center()
                .justify_center()
                .on_click(move |_, _, cx| {
                    // Keep the row click out of it: a tick must not open the drawer.
                    cx.stop_propagation();
                    let _ = tx.send(RowEvent::Toggle(id.clone()));
                })
                .child(tick(checked, cx))
                .into_any_element();
        }
        let theme = cx.theme();
        let field = col_ix - usize::from(marks);
        let Some(text) = cell_text(row, marks, col_ix) else {
            return div().into_any_element();
        };
        let (color, weight) = match field {
            0 | 1 => (theme.muted_foreground, gpui::FontWeight::NORMAL),
            2 => (theme.foreground, gpui::FontWeight::NORMAL),
            _ => (theme.foreground, gpui::FontWeight::MEDIUM),
        };
        div()
            .w_full()
            .truncate()
            .text_size(px(12.5))
            .text_color(color)
            .font_weight(weight)
            .child(text)
            .into_any_element()
    }
}

/// The text a cell draws, or `None` for the checkbox column.
fn cell_text(row: &PostingRow, marks: bool, col_ix: usize) -> Option<String> {
    if marks && col_ix == 0 {
        return None;
    }
    let dash = || "—".to_string();
    Some(match col_ix - usize::from(marks) {
        0 => row.posted.as_deref().map(day_label).unwrap_or_else(dash),
        1 => row.location.clone().unwrap_or_else(dash),
        2 => row.company.clone(),
        3 => row.title.clone(),
        _ => return None,
    })
}

/// A publish date as the table shows it: the day and the month, with the two
/// neighbouring days spelled out. The year never earns a column here.
pub fn day_label(posted: &str) -> String {
    let Ok(date) = chrono::NaiveDate::parse_from_str(posted, "%Y-%m-%d") else {
        return posted.to_owned();
    };
    let today = chrono::Local::now().date_naive();
    let days = (today - date).num_days();
    match days {
        0 => "Today".to_string(),
        1 => "Yesterday".to_string(),
        _ => date.format("%-d %b").to_string(),
    }
}

/// The selection box. On a dark ground the border has to lift off the row, so an
/// unticked box takes the muted text colour instead of the hairline colour.
fn tick(checked: bool, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let idle_border = match crate::theme::active(cx) {
        crate::theme::Tint::Light => theme.border,
        crate::theme::Tint::Dark => theme.muted_foreground.opacity(0.55),
    };
    div()
        .flex()
        .items_center()
        .justify_center()
        .size(px(15.))
        .rounded(px(4.))
        .border_1()
        .border_color(if checked { theme.primary } else { idle_border })
        .bg(if checked {
            theme.primary
        } else {
            theme.transparent
        })
        .when(checked, |el| {
            el.child(
                Icon::new(IconName::Check)
                    .with_size(px(10.))
                    .text_color(theme.primary_foreground),
            )
        })
        .into_any_element()
}

/// The header the Open stage draws above the table.
pub fn selection_note(selected: usize, rows: usize) -> SharedString {
    match selected {
        0 => format!("{rows} postings").into(),
        n => format!("{n} of {rows} selected").into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str) -> PostingRow {
        PostingRow {
            id: id.into(),
            company: "Co".into(),
            title: "Junior Engineer".into(),
            location: Some("Vancouver, BC".into()),
            url: None,
            source: "jobbank".into(),
            ats: None,
            tier: Some("vancouver".into()),
            score: None,
            age_days: Some(1),
            posted: Some("2026-11-12".into()),
            expires_on: None,
            brief_status: None,
            has_text: false,
            applied_at: None,
        }
    }

    /// The checkbox column exists only in the Open stage, and it shifts every
    /// other column by one. A wrong offset would put a title under "Company".
    #[test]
    fn the_marker_shifts_the_columns() {
        let sample = row("0123456789abcdef");
        assert_eq!(cell_text(&sample, true, 0), None);
        assert_eq!(
            cell_text(&sample, true, 1).as_deref(),
            Some(day_label("2026-11-12").as_str())
        );
        assert_eq!(
            cell_text(&sample, true, 2).as_deref(),
            Some("Vancouver, BC")
        );
        assert_eq!(cell_text(&sample, true, 3).as_deref(), Some("Co"));
        assert_eq!(
            cell_text(&sample, true, 4).as_deref(),
            Some("Junior Engineer")
        );
        assert_eq!(
            cell_text(&sample, false, 0).as_deref(),
            Some(day_label("2026-11-12").as_str())
        );
        assert_eq!(
            cell_text(&sample, false, 3).as_deref(),
            Some("Junior Engineer")
        );
        assert_eq!(cell_text(&sample, false, 4), None);

        assert_eq!(column_count(true), 5);
        assert_eq!(column_count(false), 4);
        // The marker takes the first slot, so header text must shift with it.
        assert_eq!(
            &*column_spec(true, 1, ColumnSort::Descending, 900.).name,
            "Date"
        );
        assert_eq!(
            &*column_spec(false, 0, ColumnSort::Descending, 900.).name,
            "Date"
        );
    }

    /// The title column is the one that takes the leftover width, so the table
    /// fills its card instead of leaving a gap after the last column.
    #[test]
    fn the_title_column_takes_the_leftover_width() {
        let narrow = column_spec(false, 3, ColumnSort::Descending, 700.);
        let wide = column_spec(false, 3, ColumnSort::Descending, 1100.);
        assert!(f32::from(wide.width) > f32::from(narrow.width));
        let fixed: f32 = COLUMNS.iter().map(|(_, width)| width).sum();
        assert_eq!(f32::from(wide.width), 1100. - fixed);
        // A window too narrow for the fixed columns still keeps the title readable.
        assert_eq!(
            f32::from(column_spec(false, 3, ColumnSort::Descending, 100.).width),
            200.
        );
    }

    /// A missing value must read as a dash, not as a blank cell that looks like
    /// a rendering bug.
    #[test]
    fn missing_values_read_as_a_dash() {
        let mut sample = row("ffffffffffffffff");
        sample.posted = None;
        sample.location = None;
        assert_eq!(cell_text(&sample, false, 0).as_deref(), Some("—"));
        assert_eq!(cell_text(&sample, false, 1).as_deref(), Some("—"));
    }

    /// The day label spells out the two days a reader thinks in, and uses the
    /// day and month for anything older.
    #[test]
    fn the_day_label_is_relative_only_nearby() {
        let today = chrono::Local::now().date_naive();
        assert_eq!(day_label(&today.format("%Y-%m-%d").to_string()), "Today");
        let yesterday = (today - chrono::Duration::days(1)).format("%Y-%m-%d");
        assert_eq!(day_label(&yesterday.to_string()), "Yesterday");
        assert_eq!(day_label("2026-11-12"), "12 Nov");
        assert_eq!(day_label("not a date"), "not a date");
    }

    /// The Date column is the only sortable one, and it starts on descending so
    /// the newest postings come first.
    #[test]
    fn the_date_column_is_sortable_descending() {
        assert!(
            column_spec(false, 0, ColumnSort::Descending, 900.)
                .sort
                .is_some()
        );
        assert!(
            column_spec(false, 1, ColumnSort::Descending, 900.)
                .sort
                .is_none()
        );
        assert_eq!(
            PostingTable::new(flume::unbounded().0).sort,
            ColumnSort::Descending
        );
    }

    #[test]
    fn rows_sort_newest_first_and_undated_last() {
        let mut newest = row("a");
        newest.posted = Some("2026-11-14".into());
        let mut middle = row("b");
        middle.posted = Some("2026-11-10".into());
        let mut undated = row("c");
        undated.posted = None;

        let descending = sort_rows(
            vec![middle.clone(), undated.clone(), newest.clone()],
            ColumnSort::Descending,
        );
        let descending_ids: Vec<_> = descending.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(descending_ids, vec!["a", "b", "c"]);

        let ascending = sort_rows(
            vec![middle.clone(), undated.clone(), newest.clone()],
            ColumnSort::Ascending,
        );
        let ascending_ids: Vec<_> = ascending.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ascending_ids, vec!["b", "a", "c"]);

        let untouched = sort_rows(
            vec![middle.clone(), undated.clone(), newest.clone()],
            ColumnSort::Default,
        );
        let untouched_ids: Vec<_> = untouched.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(untouched_ids, vec!["b", "c", "a"]);
    }

    #[test]
    fn the_selection_note_counts() {
        assert_eq!(selection_note(0, 12), "12 postings");
        assert_eq!(selection_note(3, 12), "3 of 12 selected");
    }
}
