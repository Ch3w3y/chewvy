//! The Chewvy mark (a peak with a star over it) and its wordmark, drawn as
//! shapes so they stay sharp at any scale.

use egui::{pos2, Align2, Color32, Painter, Pos2, Shape, Stroke};

use crate::ui::theme;

/// Shown on first run and in Settings: Plex and Jellyfin are other people's
/// names, used here only to say which service a source belongs to.
pub const NOT_AFFILIATED: &str =
    "Chewvy is an independent project, not affiliated with Plex, Inc. or the Jellyfin project.";

/// Attribution the licences of the bundled artwork and font ask for.
pub const CREDITS: [&str; 3] = [
    "Backdrop: “Aurora” by Russo Francesco, CC BY 3.0, cropped, colour-graded and blurred.",
    "Typeface: Plus Jakarta Sans, SIL Open Font License 1.1.",
    "Jellyfin icon: the Jellyfin project, CC BY-SA 4.0.",
];

/// The mark's design grid: every point below is on a 34-unit square.
const GRID: f32 = 34.0;
const RIDGE: [[f32; 2]; 5] = [
    [2.5, 29.0],
    [12.5, 12.5],
    [18.0, 20.0],
    [21.0, 16.0],
    [31.5, 29.0],
];
const SNOW: [[f32; 2]; 3] = [[9.8, 17.0], [12.5, 12.5], [15.4, 16.4]];
const STAR: [f32; 2] = [24.5, 7.5];

/// The mark `size` points square with its top-left at `origin`.
pub fn mark(painter: &Painter, origin: Pos2, size: f32) {
    let k = size / GRID;
    let at = |[x, y]: [f32; 2]| pos2(origin.x + x * k, origin.y + y * k);
    let width = (2.3 * k).max(1.2);
    painter.add(Shape::closed_line(
        RIDGE.iter().copied().map(at).collect(),
        Stroke::new(width, Color32::from_rgb(0xf1, 0xf3, 0xff)),
    ));
    painter.add(Shape::line(
        SNOW.iter().copied().map(at).collect(),
        Stroke::new(width, theme::ACCENT),
    ));
    painter.circle_filled(at(STAR), 2.3 * k, theme::ACCENT);
}

/// The mark followed by the lower-case name; answers the drawn width.
pub fn wordmark(painter: &Painter, origin: Pos2, mark_size: f32, text_size: f32) -> f32 {
    mark(painter, origin, mark_size);
    let text = painter.text(
        pos2(origin.x + mark_size + 10.0, origin.y + mark_size * 0.5),
        Align2::LEFT_CENTER,
        "chewvy",
        theme::bold(text_size),
        theme::TEXT,
    );
    text.max.x - origin.x
}
