//! The look of the app: every colour and material value the UI draws with,
//! in one place, and the one place egui's own styles are built from them.
//!
//! # The data shape
//!
//! The app has one look, smoky glass: neutral greys and blacks, with no hue
//! but the danger red. A background theme changes the photograph behind the
//! app and nothing about the UI's colours, so there is one [`Theme`] value
//! ([`SMOKE`]) and it has two kinds of part.
//!
//! The same whatever is behind the app:
//!
//! - [`Palette`]: opaque colours by role. Surfaces (`window`, `surface`,
//!   `card`, `card_hover`, `outline`, `floating`, `plate`), text levels
//!   (`text`, `secondary`, `muted`, the sidebar's `nav` and `nav_sub`, the
//!   three levels drawn over art, the app mark), the neutral `accent` with
//!   the ink that goes on it, the primary action's fill and inks, the two
//!   danger colours, and the two overlay inks `highlight` and `shadow`.
//! - [`Glass`]: the frosted-glass material. One tint per glass surface, the
//!   rim and sheen, the saturation of the blur, and the opacity added where
//!   the blur cannot run.
//! - `art_shadow`: the [`TextShadow`]s under text drawn over an item's own
//!   artwork.
//!
//! Following the photograph:
//!
//! - [`Backdrop`]: how much of the window colour lies over each copy of the
//!   photograph, and the shadows under text drawn straight on it. A brighter
//!   photograph needs more of both, and [`set_backdrop`] is the hook the code
//!   that changes the photograph calls.
//!
//! What can be derived is a method, not a field, so it cannot be stated
//! wrongly: [`Palette::accent_tint`] mixes the accent into the card colour,
//! and [`Palette::wash`] and [`Palette::scrim`] are the overlay inks at a
//! strength the component names. The strength of a hover fill or a track is
//! part of a component's shape, like its radius; the ink is the palette's.
//!
//! Radii, spacing, the type scale and the fonts are constants of this module.
//!
//! Two colours are fixed and are not tokens: [`UNTINTED`] (artwork is never
//! tinted) and [`LETTERBOX`] (what surrounds video). They live here too, so
//! this module and its children are the only place under `ui` that names a
//! colour; a test holds the rest of `ui` to it.
//!
//! # Reading and changing the theme
//!
//! [`current`] answers the theme in effect and [`palette`] its palette; both
//! are a copy out of a thread-local, cheap enough to call wherever a colour is
//! needed, every frame. Nothing caches a colour across frames. The value is
//! per thread because the UI runs on one thread.
//!
//! No screen replaces the palette or the glass. [`install`] exists so a test
//! or a probe build can swap every token at once and see that nothing on
//! screen keeps a colour of its own; it replaces the current theme and
//! rebuilds both egui styles from it.
//!
//! [`apply`] runs where the window host creates its `egui::Context`
//! ([`crate::shell::host`]): it loads the fonts and styles egui from the
//! current theme, so every panel, frame and widget inherits the tokens, radii,
//! type scale and spacing instead of egui's defaults.

mod tokens;

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::sync::{Arc, LazyLock};

use egui::{
    style::WidgetVisuals, CornerRadius, FontId, Margin, RichText, Stroke, Style, TextStyle, Visuals,
};

pub use tokens::{
    contrast, current, luminance, mix, palette, pixel, set_backdrop, Backdrop, Glass, Palette,
    TextShadow, Theme, LETTERBOX, SMOKE, UNTINTED,
};

/// Radius used for cards, panels and normal widgets (design spec: 8).
pub const CARD_RADIUS_U8: u8 = 8;
/// [`CARD_RADIUS_U8`] as an egui corner radius.
pub const CARD_RADIUS: CornerRadius = CornerRadius::same(CARD_RADIUS_U8);
/// Buttons and fields are small-radius rectangles; only a chip, which holds
/// a state rather than starting an action, is a full pill.
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

/// Text drawn in the palette's `secondary`.
#[must_use]
pub fn secondary(text: impl Into<RichText>) -> RichText {
    text.into().color(palette().secondary)
}

/// An item title: the palette's `text` stated as an explicit colour.
///
/// egui resolves `RichText::strong()` through `Visuals::strong_text_color()`,
/// which is the *active widget* foreground. Leaving titles to that resolution
/// makes them follow whatever widget state they happen to be drawn in, so the
/// colour is pinned here.
#[must_use]
pub fn title(text: impl Into<RichText>) -> RichText {
    text.into().color(palette().text).strong()
}

/// A meta line under a title: the palette's `secondary` at the [`META`] size.
#[must_use]
pub fn meta_text(text: impl Into<RichText>) -> RichText {
    text.into().color(palette().secondary).size(META_SIZE)
}

/// The `Style` egui gets for `theme`: its tokens, the type scale and spacing.
#[must_use]
pub fn style(theme: &Theme) -> Style {
    let mut style = Style::default();
    apply_to(&mut style, theme);
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

/// Rebuild an existing style from `theme`; both egui styles get the same one,
/// so no widget can fall back to egui's light defaults.
fn apply_to(style: &mut Style, theme: &Theme) {
    style.text_styles = text_styles();
    style.override_text_style = Some(TextStyle::Body);
    style.spacing.item_spacing = egui::Vec2::new(GAP, GAP);
    style.spacing.window_margin = Margin::same(PAGE_PADDING as i8);
    style.spacing.button_padding = egui::Vec2::new(GAP / 2.0, GAP / 4.0);
    style.spacing.indent = SPACING_BASE * 4.0;
    style.spacing.menu_margin = Margin::symmetric(10, 6);
    style.spacing.scroll = scroll();
    style.interaction.tooltip_delay = TOOLTIP_DELAY;
    style.visuals = visuals(&theme.palette);
}

/// How long the pointer rests on a control before its tooltip shows.
const TOOLTIP_DELAY: f32 = 0.45;

/// One scroll bar for every scrolling page: a thin handle that floats over
/// the content's edge with no gutter, brighter while it is used.
fn scroll() -> egui::style::ScrollStyle {
    egui::style::ScrollStyle {
        bar_width: 6.0,
        floating_width: 3.0,
        floating_allocated_width: 0.0,
        dormant_background_opacity: 0.0,
        active_background_opacity: 0.0,
        interact_background_opacity: 0.0,
        dormant_handle_opacity: 0.0,
        active_handle_opacity: 0.35,
        interact_handle_opacity: 0.6,
        ..egui::style::ScrollStyle::floating()
    }
}

/// egui's own widget colours, from the palette.
fn visuals(palette: &Palette) -> Visuals {
    let widget = |fill, edge| WidgetVisuals {
        bg_fill: fill,
        weak_bg_fill: fill,
        bg_stroke: Stroke::new(HAIRLINE, edge),
        corner_radius: CARD_RADIUS,
        fg_stroke: Stroke::new(0.0, palette.text),
        expansion: 0.0,
    };
    let mut visuals = Visuals::dark();
    visuals.window_fill = palette.window;
    visuals.panel_fill = palette.surface;
    visuals.extreme_bg_color = palette.window;
    visuals.faint_bg_color = palette.card;
    visuals.code_bg_color = palette.card;
    visuals.text_edit_bg_color = Some(palette.card);
    visuals.window_corner_radius = CARD_RADIUS;
    visuals.menu_corner_radius = CARD_RADIUS;
    visuals.window_stroke = Stroke::new(HAIRLINE, palette.outline);
    visuals.popup_shadow = egui::Shadow {
        offset: [0, 4],
        blur: 16,
        spread: 0,
        color: palette.scrim(110),
    };
    visuals.hyperlink_color = palette.accent;
    visuals.override_text_color = Some(palette.text);
    visuals.weak_text_color = Some(palette.secondary);
    visuals.warn_fg_color = palette.accent;
    visuals.error_fg_color = palette.danger;
    visuals.selection.bg_fill = palette.accent_tint();
    visuals.selection.stroke = Stroke::new(HAIRLINE, palette.accent);
    visuals.text_cursor.stroke.color = palette.text;
    visuals.ime_composition.active_underline_stroke.color = palette.text;
    visuals.ime_composition.inactive_underline_stroke.color = palette.muted;
    visuals.widgets.noninteractive = widget(palette.surface, palette.card);
    visuals.widgets.inactive = widget(palette.card, palette.outline);
    visuals.widgets.hovered = widget(palette.card_hover, palette.accent);
    visuals.widgets.active = widget(palette.accent_tint(), palette.accent);
    visuals.widgets.open = visuals.widgets.active;
    visuals
}

/// Load the fonts and style both egui themes from the current theme, so the
/// app is in its own look from the first frame.
pub fn apply(ctx: &egui::Context) {
    ctx.set_fonts(fonts());
    restyle(ctx);
}

/// Make `theme` the current one and restyle `ctx` from it. Everything the app
/// paints itself reads the theme each frame, so the next frame is in `theme`.
/// The app has one look and never calls this; tests and probe builds do.
pub fn install(ctx: &egui::Context, theme: Theme) {
    tokens::set_current(theme);
    restyle(ctx);
    ctx.request_repaint();
}

fn restyle(ctx: &egui::Context) {
    let theme = current();
    ctx.all_styles_mut(|style| apply_to(style, &theme));
}
