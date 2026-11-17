//! Small shared pieces of the interface: the card, the chip, the section label
//! and the label/value row. They are free functions on purpose — each one is
//! drawn from values, never from app state, so a page can mix them freely.

use gpui::prelude::*;
use gpui::{AnyElement, App, Hsla, IntoElement, SharedString, div, px};
use gpui_component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, v_flex};

/// A white panel on the page: the table, the detail drawer, a health card.
pub fn card(cx: &App) -> gpui::Div {
    let palette = crate::theme::active(cx);
    div()
        .flex_1()
        .rounded(px(10.))
        .border_1()
        .border_color(cx.theme().border)
        .bg(crate::theme::card(palette))
}

/// A short status word with a soft fill. `tone` is the ink; the fill is the
/// same color at low opacity, so the pair always belongs together.
pub fn chip(text: impl Into<SharedString>, tone: Hsla) -> AnyElement {
    div()
        .px_2()
        .py_0p5()
        .rounded_full()
        .bg(tone.opacity(0.12))
        .text_size(px(11.))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(tone)
        .child(text.into())
        .into_any_element()
}

/// A small caps heading above a group of rows.
pub fn section_label(title: &str, cx: &App) -> AnyElement {
    div()
        .pb_1()
        .text_size(px(10.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(cx.theme().muted_foreground)
        .child(title.to_uppercase())
        .into_any_element()
}

/// One label/value pair. The label keeps a fixed column so a list of facts reads
/// as a list, and the value wraps instead of pushing the panel wider.
pub fn fact_row(label: &'static str, value: String, cx: &App) -> AnyElement {
    h_flex()
        .items_start()
        .gap_2()
        .child(
            div()
                .w(px(88.))
                .flex_shrink_0()
                .text_size(px(11.))
                .text_color(cx.theme().muted_foreground)
                .child(label),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(px(12.))
                .whitespace_normal()
                .child(value),
        )
        .into_any_element()
}

/// The inside of a card: a small caps heading and a stack of rows. The caller
/// wraps it in [`card`], because the fill belongs to the frame, not the content.
pub fn stat_card(title: &str, rows: Vec<AnyElement>, cx: &App) -> AnyElement {
    v_flex()
        .flex_1()
        .min_w(px(220.))
        .gap_2()
        .p_3()
        .child(
            div()
                .text_size(px(10.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(cx.theme().muted_foreground)
                .child(title.to_uppercase()),
        )
        .children(rows)
        .into_any_element()
}

/// The empty state: an icon, one line of what is missing, and how to fill it.
pub fn empty_state(
    icon: IconName,
    title: &str,
    hint: &str,
    action: Option<AnyElement>,
    cx: &App,
) -> AnyElement {
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap_2()
        .p_10()
        .child(
            Icon::new(icon)
                .with_size(px(24.))
                .text_color(cx.theme().primary.opacity(0.55)),
        )
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::MEDIUM)
                .child(title.to_owned()),
        )
        .when(!hint.is_empty(), |el| {
            el.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .whitespace_normal()
                    .child(hint.to_owned()),
            )
        })
        .children(action)
        .into_any_element()
}

/// A metric with its name above it, for the Health page.
pub fn metric(label: &str, value: impl Into<SharedString>, cx: &App) -> AnyElement {
    let value: SharedString = value.into();
    v_flex()
        .gap_0p5()
        .child(
            div()
                .text_size(px(11.))
                .text_color(cx.theme().muted_foreground)
                .child(label.to_owned()),
        )
        .child(
            div()
                .text_size(px(18.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(cx.theme().secondary)
                .child(value),
        )
        .into_any_element()
}
