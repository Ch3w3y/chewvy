//! The dark token set the app draws with, applied once to the `egui::Context`.
//!
//! [`apply`] runs where the window host creates its `egui::Context`
//! ([`crate::shell::host`]) and rewrites both egui styles, so every panel,
//! frame and widget inherits these colors, radii, type scale and spacing
//! instead of egui's defaults. Named text styles (`display`, `h1`, `h2`,
//! `body`, `meta`, `caption`) plus [`primary`] give the UI one place to take
//! sizes and the white pill button from.

use std::collections::BTreeMap;
use std::sync::{Arc, LazyLock};

use egui::{
    style::WidgetVisuals, Color32, CornerRadius, FontId, Margin, RichText, Stroke, Style,
    TextStyle, Visuals,
};

/// Window/background fill: the night sky under the backdrop photo.
pub const WINDOW: Color32 = Color32::from_rgb(0x05, 0x06, 0x1a);
/// Panel fill where no glass or backdrop shows through.
pub const SURFACE: Color32 = Color32::from_rgb(0x0b, 0x0d, 0x22);
/// Card, field and inactive-widget fill.
pub const CARD: Color32 = Color32::from_rgb(0x16, 0x1a, 0x33);
/// Hovered card fill: `CARD` lifted one step.
pub const CARD_HOVER: Color32 = Color32::from_rgb(0x20, 0x25, 0x4a);
/// Primary text.
pub const TEXT: Color32 = Color32::from_rgb(0xee, 0xf0, 0xff);
/// Secondary text (`weak` labels, meta lines).
pub const SECONDARY: Color32 = Color32::from_rgb(0xa3, 0xa9, 0xcc);
/// Small capitals over groups (library and settings captions).
pub const MUTED: Color32 = Color32::from_rgb(0x8a, 0x8f, 0xb6);
/// Accent for selection, progress and highlighted rows: aurora teal.
pub const ACCENT: Color32 = Color32::from_rgb(0x38, 0xe1, 0xb8);
/// Accent selection fill: [`ACCENT`] at ~18% over [`CARD`], so [`TEXT`] on it
/// stays high contrast. A solid `ACCENT` fill under light text is unreadable.
pub const ACCENT_TINT: Color32 = Color32::from_rgb(0x1c, 0x3e, 0x4b);
/// Primary button fill: near-white, so the one main action reads first.
pub const PRIMARY_BG: Color32 = Color32::from_rgb(0xf4, 0xf6, 0xff);
/// Primary button text.
pub const PRIMARY_TEXT: Color32 = Color32::from_rgb(0x0b, 0x0c, 0x22);
/// Secondary button fill: white at 12% over whatever is behind it.
pub const SECONDARY_BG: Color32 = Color32::from_rgba_premultiplied(31, 31, 31, 31);
/// Hairlines between rows and along panel edges: white at 8%.
pub const RULE: Color32 = Color32::from_rgba_premultiplied(20, 20, 20, 20);
/// Failure text, kept distinct from the accent.
pub const DANGER: Color32 = Color32::from_rgb(0xff, 0x8a, 0x80);
/// Status dots: reachable, degraded.
pub const OK: Color32 = Color32::from_rgb(0x5f, 0xe3, 0xa1);
/// A server reachable only with limits (relay, low bandwidth).
pub const WARN: Color32 = Color32::from_rgb(0xf3, 0xc0, 0x63);

/// Radius used for cards, panels and normal widgets (design spec: 8).
pub const CARD_RADIUS_U8: u8 = 8;
/// [`CARD_RADIUS_U8`] as an egui corner radius.
pub const CARD_RADIUS: CornerRadius = CornerRadius::same(CARD_RADIUS_U8);
/// Buttons are small-radius rectangles, not pills.
pub const PILL_RADIUS_U8: u8 = CARD_RADIUS_U8;

/// Spacing unit every gap is a multiple of.
pub const SPACING_BASE: f32 = 4.0;
/// Gap between widgets and between rows.
pub const GAP: f32 = 16.0;
/// Padding inside a panel/page.
pub const PAGE_PADDING: f32 = 24.0;
/// The gutter a page keeps on both sides: from the rail's edge and from the
/// window's right edge. Content and the last grid column never touch an edge.
pub const PAGE_GUTTER: f32 = 48.0;
/// Where a page's content starts below the window's top edge, clear of the
/// window buttons.
pub const PAGE_TOP: f32 = 64.0;
/// Hairline used for strokes.
pub const HAIRLINE: f32 = 1.0;

/// Largest page title size (design spec: 32).
pub const DISPLAY_SIZE: f32 = 32.0;
/// Section title size (design spec: 24).
pub const H1_SIZE: f32 = 24.0;
/// Row and group heading size (design spec: 19).
pub const H2_SIZE: f32 = 19.0;
/// Default text size (design spec: 15).
pub const BODY_SIZE: f32 = 15.0;
/// Meta line size (design spec: 13).
pub const META_SIZE: f32 = 13.0;
/// Smallest label size (design spec: 12).
pub const CAPTION_SIZE: f32 = 12.0;
/// Type-scale size for a shelf card's own title line.
pub const CARD_TITLE_SIZE: f32 = 14.0;

/// Type-scale name for the largest page title.
pub static DISPLAY: LazyLock<TextStyle> = LazyLock::new(|| TextStyle::Name(Arc::from("display")));
/// Type-scale name for section titles.
pub static H1: LazyLock<TextStyle> = LazyLock::new(|| TextStyle::Name(Arc::from("h1")));
/// Type-scale name for row and group headings.
pub static H2: LazyLock<TextStyle> = LazyLock::new(|| TextStyle::Name(Arc::from("h2")));
/// Type-scale name for meta lines.
pub static META: LazyLock<TextStyle> = LazyLock::new(|| TextStyle::Name(Arc::from("meta")));
/// Type-scale name for the smallest labels.
pub static CAPTION: LazyLock<TextStyle> = LazyLock::new(|| TextStyle::Name(Arc::from("caption")));

/// WCAG 2.2 relative luminance of a colour: what its brightness is worth to a
/// reader, the half of the contrast ratio a surface is measured against.
#[must_use]
pub fn luminance(c: Color32) -> f32 {
    fn channel(v: u8) -> f32 {
        let s = f32::from(v) / 255.0;
        if s <= 0.040_45 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    }
    let [r, g, b, _] = c.to_array();
    0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
}

/// The WCAG 2.2 contrast ratio between two colours, in either order: the
/// `4.5:1` body-text and `3:1` secondary-text bars are what a page is held to.
#[must_use]
pub fn contrast(a: Color32, b: Color32) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    let (hi, lo) = (la.max(lb), la.min(lb));
    (hi + 0.05) / (lo + 0.05)
}

const fn proportional(size: f32) -> FontId {
    FontId::new(size, egui::FontFamily::Proportional)
}

/// The UI typeface, Plus Jakarta Sans (SIL OFL 1.1,
/// `assets/fonts/OFL-PlusJakartaSans.txt`), in the weights the app draws.
const PJS_REGULAR: &[u8] = include_bytes!("../../assets/fonts/PlusJakartaSans-Regular.ttf");
const PJS_MEDIUM: &[u8] = include_bytes!("../../assets/fonts/PlusJakartaSans-Medium.ttf");
const PJS_SEMIBOLD: &[u8] = include_bytes!("../../assets/fonts/PlusJakartaSans-SemiBold.ttf");
const PJS_BOLD: &[u8] = include_bytes!("../../assets/fonts/PlusJakartaSans-Bold.ttf");
const PJS_EXTRABOLD: &[u8] = include_bytes!("../../assets/fonts/PlusJakartaSans-ExtraBold.ttf");

/// The font family name of the Medium weight (titles).
pub const MEDIUM: &str = "medium";
/// The font family name of the SemiBold weight (emphasis, badges, buttons).
pub const SEMIBOLD: &str = "semibold";
/// The font family name of the Bold weight (headings, the wordmark).
pub const BOLD: &str = "bold";
/// The font family name of the ExtraBold weight (large display titles).
pub const EXTRABOLD: &str = "extrabold";

/// A [`FontFamily`](egui::FontFamily) by one of the weight names above.
#[must_use]
pub fn family(name: &str) -> egui::FontFamily {
    egui::FontFamily::Name(Arc::from(name))
}

/// A font id in the Medium weight.
#[must_use]
pub fn medium(size: f32) -> FontId {
    FontId::new(size, family(MEDIUM))
}

/// A font id in the SemiBold weight.
#[must_use]
pub fn semibold(size: f32) -> FontId {
    FontId::new(size, family(SEMIBOLD))
}

/// A font id in the Bold weight.
#[must_use]
pub fn bold(size: f32) -> FontId {
    FontId::new(size, family(BOLD))
}

/// A font id in the ExtraBold weight.
#[must_use]
pub fn extrabold(size: f32) -> FontId {
    FontId::new(size, family(EXTRABOLD))
}

/// Plus Jakarta Sans first in every family, egui's own fonts kept behind it
/// for the glyphs it lacks (emoji, symbols).
#[must_use]
pub fn fonts() -> egui::FontDefinitions {
    let mut fonts = egui::FontDefinitions::default();
    let fallbacks = fonts
        .families
        .get(&egui::FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    for (name, bytes) in [
        ("pjs-regular", PJS_REGULAR),
        ("pjs-medium", PJS_MEDIUM),
        ("pjs-semibold", PJS_SEMIBOLD),
        ("pjs-bold", PJS_BOLD),
        ("pjs-extrabold", PJS_EXTRABOLD),
    ] {
        fonts.font_data.insert(
            name.to_owned(),
            Arc::new(egui::FontData::from_static(bytes)),
        );
    }
    let with = |first: &str| {
        std::iter::once(first.to_owned())
            .chain(fallbacks.iter().cloned())
            .collect::<Vec<_>>()
    };
    fonts
        .families
        .insert(egui::FontFamily::Proportional, with("pjs-regular"));
    for (name, data) in [
        (MEDIUM, "pjs-medium"),
        (SEMIBOLD, "pjs-semibold"),
        (BOLD, "pjs-bold"),
        (EXTRABOLD, "pjs-extrabold"),
    ] {
        fonts.families.insert(family(name), with(data));
    }
    fonts
}

/// Largest page title, in Bold.
pub fn display() -> FontId {
    bold(DISPLAY_SIZE)
}

/// Section-title font id, in Bold.
pub fn h1() -> FontId {
    bold(H1_SIZE)
}

/// Row and group heading font id, in SemiBold.
pub fn h2() -> FontId {
    semibold(H2_SIZE)
}

/// Default body font id (`TextStyle::Body`).
pub fn body() -> FontId {
    proportional(BODY_SIZE)
}

/// Meta line font id (`TextStyle::Small`).
pub fn meta() -> FontId {
    proportional(META_SIZE)
}

/// Smallest label font id.
pub fn caption() -> FontId {
    proportional(CAPTION_SIZE)
}

/// Shelf card title line font id, in Medium.
pub fn card_title() -> FontId {
    medium(CARD_TITLE_SIZE)
}

/// Emphasised text: strong colour in SemiBold.
#[must_use]
pub fn strong(text: impl Into<RichText>) -> RichText {
    text.into().strong().family(family(SEMIBOLD))
}

/// Text drawn in [`SECONDARY`].
#[must_use]
pub fn secondary(text: impl Into<RichText>) -> RichText {
    text.into().color(SECONDARY)
}

/// An item title: [`TEXT`] stated as an explicit colour.
///
/// egui resolves `RichText::strong()` through `Visuals::strong_text_color()`,
/// which is the *active widget* foreground. Leaving titles to that resolution
/// makes them follow whatever widget state they happen to be drawn in, so the
/// colour is pinned here.
#[must_use]
pub fn title(text: impl Into<RichText>) -> RichText {
    text.into().color(TEXT).strong()
}

/// A meta line under a title: [`SECONDARY`] at the [`META`] size.
#[must_use]
pub fn meta_text(text: impl Into<RichText>) -> RichText {
    text.into().color(SECONDARY).size(META_SIZE)
}

/// The near-white primary button.
pub fn primary(text: impl Into<RichText>) -> egui::Button<'static> {
    egui::Button::new(text.into().color(PRIMARY_TEXT))
        .fill(PRIMARY_BG)
        .stroke(Stroke::new(0.0, PRIMARY_BG))
        .corner_radius(CornerRadius::same(PILL_RADIUS_U8))
}

/// The `Style` [`apply`] installs: dark tokens, type scale and spacing.
#[must_use]
pub fn style() -> Style {
    let mut style = Style::default();
    apply_to(&mut style);
    style
}

/// Every `TextStyle` name the UI may draw with, at the type-scale size.
fn text_styles() -> BTreeMap<TextStyle, FontId> {
    BTreeMap::from([
        (DISPLAY.clone(), display()),
        (H1.clone(), h1()),
        (H2.clone(), h2()),
        (META.clone(), meta()),
        (CAPTION.clone(), caption()),
        (TextStyle::Heading, h1()),
        (TextStyle::Body, body()),
        (TextStyle::Button, body()),
        (TextStyle::Small, meta()),
        (TextStyle::Monospace, meta()),
    ])
}

/// Mutate an existing style in place; used by [`apply`] so both egui styles end
/// up identical and no widget can fall back to egui's light defaults.
pub fn apply_to(style: &mut Style) {
    style.text_styles = text_styles();
    style.override_text_style = Some(TextStyle::Body);
    style.spacing.item_spacing = egui::Vec2::new(GAP, GAP);
    style.spacing.window_margin = Margin::same(PAGE_PADDING as i8);
    style.spacing.button_padding = egui::Vec2::new(GAP / 2.0, GAP / 4.0);
    style.spacing.indent = SPACING_BASE * 4.0;

    let mut visuals = Visuals::dark();
    visuals.window_fill = WINDOW;
    visuals.panel_fill = SURFACE;
    visuals.extreme_bg_color = WINDOW;
    visuals.faint_bg_color = CARD;
    visuals.code_bg_color = CARD;
    visuals.text_edit_bg_color = Some(CARD);
    visuals.window_corner_radius = CARD_RADIUS;
    visuals.menu_corner_radius = CARD_RADIUS;
    visuals.window_stroke = Stroke::new(HAIRLINE, CARD);
    visuals.hyperlink_color = ACCENT;
    visuals.override_text_color = Some(TEXT);
    visuals.weak_text_color = Some(SECONDARY);
    visuals.warn_fg_color = ACCENT;
    visuals.error_fg_color = DANGER;
    visuals.selection.bg_fill = ACCENT_TINT;
    visuals.selection.stroke = Stroke::new(HAIRLINE, ACCENT);
    visuals.widgets.noninteractive = WidgetVisuals {
        bg_fill: SURFACE,
        weak_bg_fill: SURFACE,
        bg_stroke: Stroke::new(HAIRLINE, CARD),
        corner_radius: CARD_RADIUS,
        fg_stroke: Stroke::new(0.0, TEXT),
        expansion: 0.0,
    };
    visuals.widgets.inactive = WidgetVisuals {
        bg_fill: CARD,
        weak_bg_fill: CARD,
        bg_stroke: Stroke::new(HAIRLINE, SECONDARY),
        corner_radius: CARD_RADIUS,
        fg_stroke: Stroke::new(0.0, TEXT),
        expansion: 0.0,
    };
    visuals.widgets.hovered = WidgetVisuals {
        bg_fill: CARD_HOVER,
        weak_bg_fill: CARD_HOVER,
        bg_stroke: Stroke::new(HAIRLINE, ACCENT),
        corner_radius: CARD_RADIUS,
        fg_stroke: Stroke::new(0.0, TEXT),
        expansion: 0.0,
    };
    visuals.widgets.active = WidgetVisuals {
        bg_fill: ACCENT_TINT,
        weak_bg_fill: ACCENT_TINT,
        bg_stroke: Stroke::new(HAIRLINE, ACCENT),
        corner_radius: CARD_RADIUS,
        fg_stroke: Stroke::new(0.0, TEXT),
        expansion: 0.0,
    };
    visuals.widgets.open = visuals.widgets.active;
    style.visuals = visuals;
}

/// Install the tokens on a context (both themes) so the app is dark from the
/// first frame.
pub fn apply(ctx: &egui::Context) {
    ctx.set_fonts(fonts());
    ctx.all_styles_mut(apply_to);
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{text::LayoutJob, FontSelection};

    #[test]
    fn tokens_match_the_design_spec() {
        assert_eq!(WINDOW, Color32::from_rgb(0x05, 0x06, 0x1a));
        assert_eq!(SURFACE, Color32::from_rgb(0x0b, 0x0d, 0x22));
        assert_eq!(CARD, Color32::from_rgb(0x16, 0x1a, 0x33));
        assert_eq!(TEXT, Color32::from_rgb(0xee, 0xf0, 0xff));
        assert_eq!(SECONDARY, Color32::from_rgb(0xa3, 0xa9, 0xcc));
        assert_eq!(ACCENT, Color32::from_rgb(0x38, 0xe1, 0xb8));
        assert_eq!(PRIMARY_BG, Color32::from_rgb(0xf4, 0xf6, 0xff));
        assert_eq!(PRIMARY_TEXT, Color32::from_rgb(0x0b, 0x0c, 0x22));
    }

    /// WCAG relative luminance ratio, so the contrast a user actually gets is
    /// checked rather than eyeballed.
    fn contrast(a: Color32, b: Color32) -> f64 {
        fn channel(v: u8) -> f64 {
            let s = f64::from(v) / 255.0;
            if s <= 0.040_45 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            }
        }
        fn luminance(c: Color32) -> f64 {
            let [r, g, b, _] = c.to_array();
            0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
        }
        let (hi, lo) = (
            luminance(a).max(luminance(b)),
            luminance(a).min(luminance(b)),
        );
        (hi + 0.05) / (lo + 0.05)
    }

    #[test]
    fn text_and_selected_fills_are_readable() {
        assert!(contrast(TEXT, WINDOW) > 15.0);
        assert!(contrast(TEXT, SURFACE) > 12.0);
        assert!(contrast(TEXT, ACCENT_TINT) > 7.0);
        assert!(contrast(SECONDARY, SURFACE) > 4.5);
        assert!(contrast(MUTED, SURFACE) > 4.5);
        assert!(contrast(PRIMARY_TEXT, PRIMARY_BG) > 15.0);
        // A solid accent fill under light text is what the live run reported.
        assert!(contrast(TEXT, ACCENT) < 4.5);
        let s = style();
        assert_eq!(s.visuals.selection.bg_fill, ACCENT_TINT);
        assert_eq!(s.visuals.widgets.active.bg_fill, ACCENT_TINT);
        assert_eq!(s.visuals.widgets.active.fg_stroke.color, TEXT);
        assert_eq!(s.visuals.widgets.open.bg_fill, ACCENT_TINT);
        // What the widgets actually get painted with, read out of the layout job.
        let s = style();
        let drawn = |rt: RichText| {
            let mut job = LayoutJob::default();
            rt.append_to(&mut job, &s, FontSelection::Default, egui::Align::Center);
            job.sections.last().map(|section| section.format.color)
        };
        assert_eq!(drawn(title("Heat")), Some(TEXT));
        assert_eq!(drawn(meta_text("movie · 1h41m")), Some(SECONDARY));
        assert_eq!(drawn(strong("Heat")), Some(TEXT));
    }

    #[test]
    fn type_scale_sizes() {
        assert_eq!(display(), FontId::new(32.0, family(BOLD)));
        assert_eq!(h1(), FontId::new(24.0, family(BOLD)));
        assert_eq!(h2(), FontId::new(19.0, family(SEMIBOLD)));
        assert_eq!(body(), FontId::new(15.0, egui::FontFamily::Proportional));
        assert_eq!(meta(), FontId::new(13.0, egui::FontFamily::Proportional));
        assert_eq!(caption(), FontId::new(12.0, egui::FontFamily::Proportional));
    }

    #[test]
    fn every_named_style_is_in_the_scale() {
        let styles = style().text_styles;
        for name in [&*DISPLAY, &*H1, &*H2, &*META, &*CAPTION] {
            assert!(styles.contains_key(name), "{name:?} missing");
        }
        assert_eq!(styles.get(&TextStyle::Body), Some(&body()));
        assert_eq!(styles.get(&TextStyle::Heading), Some(&h1()));
    }

    #[test]
    fn radii_and_spacing_follow_the_grid() {
        let s = style();
        assert_eq!(s.visuals.window_corner_radius, CornerRadius::same(8));
        assert_eq!(
            s.visuals.widgets.noninteractive.corner_radius,
            CornerRadius::same(8)
        );
        assert_eq!(
            s.visuals.widgets.inactive.corner_radius,
            CornerRadius::same(8)
        );
        assert_eq!(s.spacing.item_spacing, egui::Vec2::new(16.0, 16.0));
        assert_eq!(s.spacing.window_margin, Margin::same(24));
        assert_eq!(s.spacing.indent, 16.0);
    }

    #[test]
    fn apply_replaces_visuals_on_the_context() {
        let ctx = egui::Context::default();
        apply(&ctx);
        let s = ctx.style_of(egui::Theme::Dark);
        assert_eq!(s.visuals.window_fill, WINDOW);
        assert_eq!(s.visuals.panel_fill, SURFACE);
        assert_eq!(s.visuals.widgets.inactive.bg_fill, CARD);
        assert_eq!(s.visuals.hyperlink_color, ACCENT);
        assert_eq!(s.text_styles.get(&TextStyle::Body), Some(&body()));
        let light = ctx.style_of(egui::Theme::Light);
        assert_eq!(light.visuals.window_fill, WINDOW);
    }
}
