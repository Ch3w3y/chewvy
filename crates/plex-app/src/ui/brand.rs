//! The Chewvy mark (a film reel whose fifth hole opens into a lower-case c)
//! and its lockup with the name. The mark is an SVG drawn through
//! [`marks`], rasterised at the size it is drawn so it stays sharp at any
//! scale, with its holes clear so the glass under it shows through.

use std::sync::Arc;

use egui::{pos2, Galley, Painter};

use crate::ui::{marks, theme};

/// Shown on first run and in Settings: Plex and Jellyfin are other people's
/// names, used here only to say which service a source belongs to.
pub const NOT_AFFILIATED: &str =
    "Chewvy is an independent project, not affiliated with Plex, Inc. or the Jellyfin project.";

/// Attribution the licences of the bundled font and icon ask for; the
/// background photographs' credits come from their own table
/// ([`crate::ui::backdrop::PHOTOS`]).
pub const CREDITS: [&str; 2] = [
    "Typeface: Plus Jakarta Sans, SIL Open Font License 1.1.",
    "Jellyfin icon: the Jellyfin project, CC BY-SA 4.0.",
];

/// The gap between the mark and the name, as a fraction of the mark's size.
const GAP: f32 = 0.3;
/// How far above the baseline the middle of the name's x-height sits, as a
/// fraction of the type size (Plus Jakarta Sans Bold).
const X_MIDDLE: f32 = 0.272;

/// The mark `size` points tall from `left`, centred on `center_y`; answers
/// its width.
pub fn mark(painter: &Painter, left: f32, center_y: f32, size: f32) -> f32 {
    marks::paint_tinted(
        painter,
        marks::BRAND,
        left,
        center_y,
        size,
        theme::palette().mark,
    )
}

/// "chewvy" in bold `size` points, and how far down its galley the middle of
/// its x-height is.
fn name(painter: &Painter, size: f32) -> (Arc<Galley>, f32) {
    let galley = painter.layout_no_wrap(
        "chewvy".to_owned(),
        theme::bold(size),
        theme::palette().text,
    );
    let baseline = galley
        .rows
        .first()
        .and_then(|row| Some(row.pos.y + row.glyphs.first()?.pos.y))
        .unwrap_or(size);
    (galley, baseline - X_MIDDLE * size)
}

/// The width [`wordmark`] takes at `size`.
#[must_use]
pub fn wordmark_width(painter: &Painter, size: f32) -> f32 {
    let mark = marks::width_at(marks::BRAND, size);
    mark + GAP * size + name(painter, size).0.rect.width()
}

/// The lockup from `left`: the mark as tall as the name's type is large, then
/// "chewvy" in bold `size` points, both centred on the name's x-height, which
/// sits on `center_y`. Answers the drawn width.
pub fn wordmark(painter: &Painter, left: f32, center_y: f32, size: f32) -> f32 {
    let mark = mark(painter, left, center_y, size);
    let (galley, middle) = name(painter, size);
    let width = galley.rect.width();
    let x = left + mark + GAP * size;
    painter.galley(pos2(x, center_y - middle), galley, theme::palette().text);
    x + width - left
}
