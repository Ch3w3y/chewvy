//! The card chrome a Home shelf card and the hero have in common: the
//! hover/keyboard-focus ring, the pointer cursor, the two corner radii, the
//! rounded artwork quad a poster or backdrop is drawn with, and the chevron a
//! scrollable row and its pager arrows are marked with.
//!
//! It also holds the two pure functions that decide how a Home row moves for
//! one wheel event, so those rules are testable without a window.

use egui::{
    Color32, CornerRadius, CursorIcon, Pos2, Rect, Response, Shape, Stroke, StrokeKind, TextureId,
    Ui, Vec2,
};

use super::theme;

/// Card and poster radius: the design spec's 8, which [`theme::CARD_RADIUS`] already is.
pub const CARD_RADIUS: CornerRadius = theme::CARD_RADIUS;
/// Thickness of the accent ring a hovered or keyboard-focused card gets.
pub const RING_STROKE: f32 = 2.0;
/// Radius of a pager arrow's round background: a 40px plate.
pub const ARROW_RADIUS: f32 = 20.0;
/// A pager arrow's plate sits this far inside the row or hero it pages, so it
/// reads as part of that band instead of hanging off its edge.
pub const ARROW_INSET: f32 = 8.0;
/// The width of the column a row gives one pager arrow: exactly its clickable
/// box, so an arrow allocated in such a column cannot overlap a card.
pub const ARROW_BOX: f32 = 2.0 * ARROW_RADIUS;
/// `[run, arm]` of a painted pager chevron: how far its arms run from the tip,
/// and how far each arm sits above and below it. The proportional fonts egui
/// bundles name no ‹/› glyph, so the arrow is drawn, not typed.
pub const ARROW: [f32; 2] = [5.0, 8.0];
/// `[run, arm]` of the small chevron that marks a scrollable row: how far its
/// arms run from the tip and how far each arm sits above and below it.
pub const CHEVRON: [f32; 2] = [4.0, 6.0];
/// The space a row title's chevron is given: wide enough for [`CHEVRON`] and
/// tall enough to read against an [H2](super::theme::H2_SIZE) title.
pub const CHEVRON_BOX: [f32; 2] = [14.0, 22.0];

/// The ring a card shows while the pointer is over it or it holds the keyboard
/// focus: the accent reads as "this is the card a click or an Enter acts on".
pub fn ring(ui: &Ui, rect: Rect, response: &Response, radius: CornerRadius) {
    focus_ring(ui, rect, response, radius, false);
}

/// [`ring`], plus the ring a browsing screen's own keyboard selection draws.
///
/// A card the arrows have selected is the card an Enter acts on, so it is
/// ringed whether or not egui holds the keyboard focus on it this frame: only
/// the rows a scroll area has in view are laid out, so a selection scrolled out
/// of view holds no egui focus to read.
pub fn focus_ring(ui: &Ui, rect: Rect, response: &Response, radius: CornerRadius, selected: bool) {
    if !selected && !response.hovered() && !response.has_focus() {
        return;
    }
    ui.painter().rect_stroke(
        rect,
        radius,
        Stroke::new(RING_STROKE, theme::ACCENT),
        StrokeKind::Outside,
    );
}

/// The cursor over a card the user can open. A card that cannot be opened
/// (while the player holds an item) keeps the default arrow.
#[must_use]
pub fn pointer(response: Response) -> Response {
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// The artwork of `rect` cropped to `uv`, drawn with rounded corners instead of
/// four right angles; the crop is unchanged and only the corners are clipped.
pub fn rounded_art(
    ui: &Ui,
    texture: TextureId,
    rect: Rect,
    uv: Rect,
    radius: CornerRadius,
    tint: Color32,
) {
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    // epaint's own textured rounded rect: anti-aliased edges and exact corners,
    // where a hand-built fan rasterised with steps along its straight runs.
    ui.painter()
        .add(egui::epaint::RectShape::filled(rect, radius, tint).with_texture(texture, uv));
}

/// The clickable box of a pager arrow centred on `center`: [`ARROW_BOX`] square.
#[must_use]
pub fn arrow_rect(center: Pos2) -> Rect {
    Rect::from_center_size(center, egui::vec2(ARROW_BOX, ARROW_BOX))
}

/// The full-texture uv rect: egui crops by sub-rect of `0.0..=1.0`.
pub const FULL_UV: Rect = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));

/// The uv sub-rect of a `tex`-sized texture that covers `target` with the least
/// cropping (no letterboxing, no stretching), the crop held at `anchor` of the
/// space the cover leaves over: `0.5` centres it, `0.0` pins it to the art's top
/// edge. Home keeps the centred rule for its wide backdrops, where the middle of
/// a 16:9 frame is the part worth keeping.
#[must_use]
fn cover_uv_at(tex: Vec2, target: Vec2, anchor: f32) -> Rect {
    if tex.x <= 0.0 || tex.y <= 0.0 || target.x <= 0.0 || target.y <= 0.0 {
        return FULL_UV;
    }
    let scale = (target.x / tex.x).max(target.y / tex.y);
    let crop = egui::vec2(target.x / scale, target.y / scale);
    let spare = egui::vec2(1.0 - crop.x / tex.x, 1.0 - crop.y / tex.y);
    let uv = Rect::from_min_size(
        egui::pos2(spare.x * 0.5, spare.y * anchor),
        egui::vec2(crop.x / tex.x, crop.y / tex.y),
    );
    uv.intersect(FULL_UV)
}

/// The centred cover crop: [`cover_uv_at`] at its default anchor.
#[must_use]
pub fn cover_uv(tex: Vec2, target: Vec2) -> Rect {
    cover_uv_at(tex, target, 0.5)
}

/// The largest `tex`-sized drawing that fits `box_size` whole: title art is
/// fitted, never cropped.
#[must_use]
pub fn fit_size(tex: Vec2, box_size: Vec2) -> Vec2 {
    if tex.x <= 0.0 || tex.y <= 0.0 {
        return box_size;
    }
    let scale = (box_size.x / tex.x).min(box_size.y / tex.y);
    egui::vec2(tex.x * scale, tex.y * scale)
}

/// The cover crop pinned to the art's top edge. A grid of fixed-shape poster
/// cells crops every poster this way, so a row shares one artwork top and one
/// caption baseline whatever each poster's own aspect ratio is.
#[must_use]
pub fn cover_uv_top(tex: Vec2, target: Vec2) -> Rect {
    cover_uv_at(tex, target, 0.0)
}

/// The three points of a pager arrow's chevron, centred on `center`: the tip
/// points right (toward the content further along the row) unless `flip`
/// points it left.
#[must_use]
pub fn arrow_points(center: Pos2, flip: bool) -> [Pos2; 3] {
    let [run, arm] = ARROW;
    let side = if flip { -1.0 } else { 1.0 };
    [
        center + egui::vec2(-side * run * 0.5, -arm),
        center + egui::vec2(side * run * 0.5, 0.0),
        center + egui::vec2(-side * run * 0.5, arm),
    ]
}

/// One pager arrow, drawn over the row it pages: a round glass plate tinted by
/// `tint`, with a filled chevron in [`theme::TEXT`] so it reads over art.
pub fn arrow(ui: &Ui, center: Pos2, flip: bool, tint: Color32) {
    crate::ui::glass::panel(ui.painter(), arrow_rect(center), ARROW_RADIUS, tint);
    let [top, tip, bottom] = arrow_points(center, flip);
    ui.painter().add(Shape::convex_polygon(
        vec![top, tip, bottom],
        theme::TEXT,
        Stroke::new(1.5, theme::TEXT),
    ));
}

/// The small stroked chevron that says "this row goes on that way", centred on
/// `center` and pointing right unless `flip` points it left.
pub fn chevron(ui: &Ui, center: Pos2, flip: bool, color: Color32) {
    let [run, arm] = CHEVRON;
    let side = if flip { -1.0 } else { 1.0 };
    ui.painter().add(Shape::line(
        vec![
            center + egui::vec2(-side * run * 0.5, -arm),
            center + egui::vec2(side * run * 0.5, 0.0),
            center + egui::vec2(-side * run * 0.5, arm),
        ],
        Stroke::new(1.5, color),
    ));
}

/// How far one wheel event moves a Home row, in points: only a sideways intent
/// pages a row (F38). A plain vertical wheel belongs to the page and answers
/// 0.0 here, a horizontal wheel or trackpad flick counts as it is, and a wheel
/// held with Shift pages the row along whichever axis it arrived on. The sign is
/// egui's own - a positive delta is the content moving right - so a row and a
/// [`egui::ScrollArea`] answer one event alike.
#[must_use]
pub fn row_wheel_delta(shift: bool, delta: Vec2) -> f32 {
    if shift {
        delta.x + delta.y
    } else {
        delta.x
    }
}

/// Whether a row at `offset` of `max` should take that wheel step at all, and
/// by how much its left-hand content grows. A row that cannot page any further
/// leaves the wheel to the page, which is what a nested [`egui::ScrollArea`]
/// does with a wheel it cannot use.
#[must_use]
pub fn row_advance(offset: f32, max: f32, delta: f32) -> Option<f32> {
    let step = -delta;
    if (step > 0.0 && offset < max) || (step < 0.0 && offset > 0.0) {
        Some(step)
    } else {
        None
    }
}

#[cfg(test)]
#[path = "cards/tests.rs"]
mod tests;
