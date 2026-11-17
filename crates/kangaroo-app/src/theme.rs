//! The Slate palette, in the light and in the dark.
//!
//! A tint has six colours: page, card, border, text, muted text, and one accent
//! with a warm second voice beside it. Everything else is mixed from those, so
//! the two tints keep the same relation between their surfaces: the page is the
//! flat ground, cards sit one step off it, hairlines separate them, and the
//! accent carries what the reader can press.
//!
//! Headings take the text colour, not the accent. The first accent is reserved
//! for what the reader can press and for where they are; the second is spent on
//! the Shortlist row and the brief it holds.

use gpui::{App, Global, Hsla};
use gpui_component::{Theme, ThemeConfig, ThemeConfigColors, ThemeMode, try_parse_color};
use std::rc::Rc;

/// The two tints of the one palette. Temporary: the switch beside the scan
/// button exists so the two can be compared.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Tint {
    /// Light: a white page, ink text, cool teal.
    #[default]
    Light,
    /// Dark: a near-black page, off-white text, the same teal lifted.
    Dark,
}

impl Tint {
    /// The name to show beside the switch.
    pub fn name(self) -> &'static str {
        match self {
            Tint::Light => "Light",
            Tint::Dark => "Dark",
        }
    }

    /// The tint a flag names.
    pub fn from_arg(text: &str) -> Option<Self> {
        match text.trim().to_lowercase().as_str() {
            "light" => Some(Tint::Light),
            "dark" => Some(Tint::Dark),
            _ => None,
        }
    }

    /// How much stronger the tints have to be on a dark ground. The same weight
    /// that reads as a hairline on white disappears on near-black, so every mix
    /// toward the border or the accent gains a third.
    fn weight(self) -> f32 {
        match self {
            Tint::Light => 1.,
            Tint::Dark => 1.35,
        }
    }

    fn base(self) -> Base {
        match self {
            Tint::Light => LIGHT,
            Tint::Dark => DARK,
        }
    }
}

/// One tint's named colours. Seven of the twelve are the palette; the four
/// status colours carry meaning rather than taste, so each tint has to state
/// them rather than derive them, and so does the colour that sits on the accent.
#[derive(Clone, Copy)]
struct Base {
    page: &'static str,
    card: &'static str,
    border: &'static str,
    text: &'static str,
    muted: &'static str,
    accent: &'static str,
    second: &'static str,
    on_accent: &'static str,
    success: &'static str,
    danger: &'static str,
    info: &'static str,
    warning: &'static str,
}

impl Base {
    /// Every named colour, for the tests that check them all.
    #[cfg(test)]
    fn all(self) -> [&'static str; 12] {
        [
            self.page,
            self.card,
            self.border,
            self.text,
            self.muted,
            self.accent,
            self.second,
            self.on_accent,
            self.success,
            self.danger,
            self.info,
            self.warning,
        ]
    }
}

const LIGHT: Base = Base {
    page: "#F7F7F7",
    card: "#FFFFFF",
    border: "#E4E4E4",
    text: "#171717",
    muted: "#6B6B6B",
    accent: "#3F7D73",
    second: "#B4793A",
    on_accent: "#FFFFFF",
    success: "#2E7D4F",
    danger: "#B3261E",
    info: "#2F6E8F",
    warning: "#8A5A00",
};

const DARK: Base = Base {
    page: "#141414",
    card: "#1C1C1C",
    border: "#2E2E2E",
    text: "#EDEDED",
    muted: "#9A9A9A",
    // The same teal, lifted: on near-black the light-mode value loses contrast.
    accent: "#4F9A8E",
    second: "#C98F4E",
    // A near-black label reads on the lifted teal; white would not.
    on_accent: "#0B1412",
    success: "#4FA46A",
    danger: "#E06C5E",
    info: "#5A9BC0",
    warning: "#D6A23E",
};

/// The tint the theme last applied, so a helper that only has an `App` can find
/// it.
pub struct ActiveTint(pub Tint);

impl Global for ActiveTint {}

/// The tint in use. Falls back to light before the theme is installed, which
/// only happens on the first frame.
pub fn active(cx: &App) -> Tint {
    cx.try_global::<ActiveTint>()
        .map(|active| active.0)
        .unwrap_or_default()
}

fn rgb(text: &str) -> [f32; 3] {
    let text = text.trim_start_matches('#');
    let channel = |at: usize| {
        u8::from_str_radix(&text[at..at + 2], 16).expect("tint colors are valid hex") as f32
    };
    [channel(0), channel(2), channel(4)]
}

fn hex_of(color: [f32; 3]) -> String {
    let channel = |value: f32| value.round().clamp(0., 255.) as u8;
    format!(
        "#{:02X}{:02X}{:02X}",
        channel(color[0]),
        channel(color[1]),
        channel(color[2])
    )
}

/// `toward` over `base` at `weight`, for the tints that sit between two named
/// colours.
fn mix(base: &str, toward: &str, weight: f32) -> String {
    let (base, toward) = (rgb(base), rgb(toward));
    let mut out = [0.; 3];
    for (index, channel) in out.iter_mut().enumerate() {
        *channel = base[index] * (1. - weight) + toward[index] * weight;
    }
    hex_of(out)
}

fn hsla(text: &str) -> Hsla {
    try_parse_color(text).expect("tint colors are valid hex")
}

/// The tints mixed from the named colours, computed once per call.
struct Mixed {
    border_soft: String,
    fill: String,
    hover: String,
    header: String,
    row_hover: String,
    row_active: String,
    sidebar: String,
    sidebar_accent: String,
    section_ink: String,
    accent_hover: String,
    selection: String,
}

fn mixed(tint: Tint) -> Mixed {
    let base = tint.base();
    let w = tint.weight();
    let sidebar = mix(base.page, base.border, 0.35 * w);
    Mixed {
        border_soft: mix(base.card, base.border, 0.6 * w),
        fill: mix(base.card, base.border, 0.45 * w),
        hover: mix(base.page, base.border, 0.45 * w),
        header: mix(base.card, base.border, 0.35 * w),
        row_hover: mix(base.card, base.border, 0.5 * w),
        row_active: mix(base.card, base.accent, 0.12 * w),
        sidebar_accent: mix(&sidebar, base.accent, 0.12 * w),
        sidebar,
        section_ink: mix(base.muted, base.text, 0.25),
        // Toward the text colour, so the hover darkens in light and lightens in
        // dark without a second rule.
        accent_hover: mix(base.accent, base.text, 0.18),
        selection: mix(base.card, base.accent, 0.16 * w),
    }
}

fn sh(value: &str) -> gpui::SharedString {
    value.to_owned().into()
}

fn colors(tint: Tint) -> ThemeConfigColors {
    let b = tint.base();
    let m = mixed(tint);

    let mut theme = ThemeConfigColors::default();
    theme.background = Some(sh(b.page));
    theme.foreground = Some(sh(b.text));
    theme.border = Some(sh(b.border));
    theme.primary = Some(sh(b.accent));
    theme.primary_hover = Some(sh(&m.accent_hover));
    theme.primary_foreground = Some(sh(b.on_accent));
    theme.secondary = Some(sh(b.text));
    // Solid secondary buttons carry the accent's label colour; the token itself
    // stays dark because gpui-component also uses it for ghost-button text.
    theme.button_secondary_foreground = Some(sh(b.on_accent));
    theme.accent = Some(sh(b.accent));
    theme.accent_foreground = Some(sh(b.on_accent));
    theme.muted = Some(sh(&m.fill));
    theme.muted_foreground = Some(sh(b.muted));
    theme.input = Some(sh(b.card));
    theme.popover = Some(sh(b.card));
    theme.popover_foreground = Some(sh(b.text));
    theme.selection = Some(sh(&m.selection));
    theme.ring = Some(sh(b.accent));
    theme.caret = Some(sh(b.accent));
    theme.link = Some(sh(b.accent));
    theme.link_hover = Some(sh(&m.accent_hover));
    theme.link_active = Some(sh(&m.accent_hover));
    theme.success = Some(sh(b.success));
    theme.danger = Some(sh(b.danger));
    theme.info = Some(sh(b.info));
    theme.warning = Some(sh(b.warning));
    // Lists and selects
    theme.list = Some(sh(b.card));
    theme.list_head = Some(sh(&m.header));
    theme.list_hover = Some(sh(&m.row_hover));
    theme.list_active = Some(sh(&m.row_active));
    theme.list_active_border = Some(sh(b.accent));
    theme.list_even = Some(sh(b.card));
    // The table sits on a card, so its own fills stay quiet.
    theme.table = Some(sh(b.card));
    theme.table_head = Some(sh(&m.header));
    theme.table_head_foreground = Some(sh(b.muted));
    theme.table_even = Some(sh(b.card));
    theme.table_hover = Some(sh(&m.row_hover));
    theme.table_active = Some(sh(&m.row_active));
    theme.table_active_border = Some(sh(b.accent));
    theme.table_row_border = Some(sh(&m.border_soft));
    theme.table_foot = Some(sh(b.card));
    theme.table_foot_foreground = Some(sh(b.muted));
    // Navigation and window chrome
    theme.sidebar = Some(sh(&m.sidebar));
    theme.sidebar_foreground = Some(sh(b.text));
    theme.sidebar_border = Some(sh(b.border));
    theme.sidebar_accent = Some(sh(&m.sidebar_accent));
    theme.sidebar_accent_foreground = Some(sh(b.text));
    theme.sidebar_primary = Some(sh(b.accent));
    theme.sidebar_primary_foreground = Some(sh(b.on_accent));
    theme.title_bar = Some(sh(&m.sidebar));
    theme.title_bar_border = Some(sh(b.border));
    theme.status_bar = Some(sh(&m.sidebar));
    theme.status_bar_border = Some(sh(b.border));
    theme.group_box = Some(sh(b.card));
    theme.group_box_foreground = Some(sh(b.text));
    theme.group_box_title_foreground = Some(sh(b.text));
    theme.scrollbar = Some(sh("#00000000"));
    // A scrollbar sits on every surface, so the thumb is one step off the card
    // toward the text: dark in light mode, light in dark mode.
    theme.scrollbar_thumb = Some(sh(&mix(b.card, b.muted, 0.5)));
    theme.scrollbar_thumb_hover = Some(sh(b.muted));
    theme
}

/// The card surface: the panels on the page.
pub fn card(tint: Tint) -> Hsla {
    hsla(tint.base().card)
}

/// The navigation column, one step off the page.
pub fn sidebar(tint: Tint) -> Hsla {
    hsla(&mixed(tint).sidebar)
}

/// A hover fill for a row that sits on the page or on a card.
pub fn hover(tint: Tint) -> Hsla {
    hsla(&mixed(tint).hover)
}

/// The small caps label above a group of navigation rows.
pub fn section_ink(tint: Tint) -> Hsla {
    hsla(&mixed(tint).section_ink)
}

/// The second accent, for the Shortlist row and the brief it holds.
pub fn secondary(tint: Tint) -> Hsla {
    hsla(tint.base().second)
}

/// Installs the tint and reloads the theme. Calling it again with the other
/// tint is the whole of a switch.
pub fn apply(cx: &mut App, tint: Tint) {
    cx.set_global(ActiveTint(tint));
    let mut config = ThemeConfig::default();
    config.name = format!("Kangaroo Slate {}", tint.name()).into();
    config.mode = match tint {
        Tint::Light => ThemeMode::Light,
        Tint::Dark => ThemeMode::Dark,
    };
    config.colors = colors(tint);
    let config = Rc::new(config);
    let mode = config.mode;
    Theme::update(cx, |theme| match mode {
        ThemeMode::Light => theme.light_theme = config,
        ThemeMode::Dark => theme.dark_theme = config,
    });
    // The UI font is left at `.SystemUIFont`, so functional text draws with the
    // platform UI font. The wordmark keeps Optima on its own element in main.rs.
    Theme::change(mode, None, cx);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mixing is what every derived surface rests on, so the endpoints and the
    /// midpoint have to be exact.
    #[test]
    fn mixing_walks_from_one_color_to_the_other() {
        assert_eq!(mix("#FFFFFF", "#000000", 0.), "#FFFFFF");
        assert_eq!(mix("#FFFFFF", "#000000", 1.), "#000000");
        assert_eq!(mix("#FFFFFF", "#000000", 0.5), "#808080");
        assert_eq!(mix("#F7F7F7", "#E4E4E4", 0.5), "#EEEEEE");
    }

    /// A tint is twelve colours; a malformed one would panic on the first frame
    /// instead of at launch.
    #[test]
    fn every_tint_is_well_formed() {
        for tint in [Tint::Light, Tint::Dark] {
            for color in tint.base().all() {
                assert_eq!(color.len(), 7, "{color} is not #RRGGBB");
                assert!(color.starts_with('#'));
                rgb(color);
            }
        }
    }

    /// Relative luminance, so contrast can be measured rather than eyeballed.
    fn luminance(color: &str) -> f32 {
        let rgba = gpui::Rgba::from(hsla(color));
        let channel = |value: f32| {
            if value <= 0.03928 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(rgba.r) + 0.7152 * channel(rgba.g) + 0.0722 * channel(rgba.b)
    }

    fn contrast(a: &str, b: &str) -> f32 {
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    /// The card has to separate from the page in both tints, or the window loses
    /// its panels, and the text has to stay readable on both surfaces.
    #[test]
    fn each_tint_stays_readable() {
        for tint in [Tint::Light, Tint::Dark] {
            let base = tint.base();
            assert!(
                luminance(base.card) > luminance(base.page),
                "{} has a card that does not lift off the page",
                tint.name()
            );
            assert!(
                contrast(base.card, base.page) >= 1.05,
                "{} has a card that vanishes into the page",
                tint.name()
            );
            // Body text clears AAA, secondary text clears AA.
            assert!(
                contrast(base.text, base.card) >= 7.,
                "{} body text is under AAA: {:.1}",
                tint.name(),
                contrast(base.text, base.card)
            );
            assert!(
                contrast(base.muted, base.card) >= 4.5,
                "{} muted text is under AA: {:.1}",
                tint.name(),
                contrast(base.muted, base.card)
            );
            // A button's label on the accent has to read too.
            assert!(
                contrast(base.on_accent, base.accent) >= 4.5,
                "{} accent label is under AA: {:.1}",
                tint.name(),
                contrast(base.on_accent, base.accent)
            );
            assert_ne!(base.accent, base.second);
        }
    }

    /// The error in one tint is the error in the other: a status colour that
    /// flips hue would read as a different state.
    #[test]
    fn the_status_colors_stay_in_range() {
        for tint in [Tint::Light, Tint::Dark] {
            let base = tint.base();
            assert!(
                rgb(base.success)[1] > rgb(base.success)[0],
                "success is not green"
            );
            assert!(
                rgb(base.danger)[0] > rgb(base.danger)[1],
                "danger is not red"
            );
            assert!(
                rgb(base.warning)[0] > rgb(base.warning)[2],
                "warning is not warm"
            );
        }
    }

    /// The switch and the flag have to agree on the names.
    #[test]
    fn names_round_trip() {
        for tint in [Tint::Light, Tint::Dark] {
            assert_eq!(Tint::from_arg(tint.name()), Some(tint));
            assert_eq!(Tint::from_arg(&tint.name().to_lowercase()), Some(tint));
        }
        assert_eq!(Tint::from_arg("nope"), None);
        assert_eq!(Tint::default(), Tint::Light);
    }
}
