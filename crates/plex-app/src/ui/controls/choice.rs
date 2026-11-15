//! The controls that set a value rather than start an action: a row that
//! picks one option, a segmented choice, a switch row and a slider.

use std::ops::RangeInclusive;

use egui::text::LayoutJob;
use egui::{pos2, vec2, CornerRadius, CursorIcon, Painter, Rect, Response, Sense, Stroke, Vec2};

use crate::ui::marks::icon::{self, Icon, Size};
use crate::ui::theme;

use super::Look;

/// An option row's height in a menu or a popover.
const ROW: f32 = 34.0;
/// Where an option row's check is centred, and where its label starts.
const CHECK_X: f32 = 15.0;
const LABEL_X: f32 = 32.0;
/// The strength of the wash under a pointed-at row.
const ROW_HOVER: u8 = 26;
/// A switch's track, and the room its knob keeps off the track's edge.
const SWITCH: Vec2 = Vec2::new(40.0, 22.0);
const KNOB_PAD: f32 = 3.0;
/// The strengths of an off switch's track at rest and pointed at.
const TRACK_OFF: [u8; 2] = [40, 64];
/// A switch row's height with one line of text and with a note under it.
const SWITCH_ROW: [f32; 2] = [36.0, 52.0];
/// The room a switch row keeps between its text and its switch, and the
/// gap between its two lines of text.
const SWITCH_GAP: f32 = 16.0;
const LINE_GAP: f32 = 3.0;
/// How far a pointed-at switch row's fill reaches outside the row, so the
/// row's text stays on the section's own left edge.
const ROW_BLEED: f32 = 8.0;
/// A segmented choice's height, and the strengths of its track, of a
/// pointed-at segment and of the chosen one.
const SEGMENT: f32 = 32.0;
const SEGMENTS: [u8; 3] = [16, 30, 80];
/// A slider's label column, value column, track thickness and knob radius.
const SLIDER_LABEL: f32 = 104.0;
const SLIDER_VALUE: f32 = 58.0;
const TRACK: f32 = 4.0;
const KNOB: f32 = 7.0;
/// The strength of a slider's unfilled track.
const TRACK_WASH: u8 = 46;

/// One choice in a list: a check when it is the one in effect, its label
/// elided to the row (the full label then shows on hover). The row is
/// `ui`'s full width.
pub fn option_row(ui: &mut egui::Ui, label: &str, selected: bool) -> Response {
    option_row_sized(ui, label, selected, vec2(ui.available_width(), ROW))
}

/// [`option_row`] in a box of `size`.
pub fn option_row_sized(ui: &mut egui::Ui, label: &str, selected: bool, size: Vec2) -> Response {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let response = response.on_hover_cursor(CursorIcon::PointingHand);
    let look = Look::of(&response);
    let palette = theme::palette();
    let painter = ui.painter();
    if look.hovered || look.focused {
        painter.rect_filled(rect, theme::CARD_RADIUS, palette.wash(ROW_HOVER));
    }
    if selected {
        icon::paint(
            painter,
            Icon::Check,
            Size::Regular,
            pos2(rect.min.x + CHECK_X, rect.center().y),
            palette.accent,
        );
    }
    let ink = if selected || look.hovered {
        palette.text
    } else {
        palette.secondary
    };
    let mut job = LayoutJob::simple(
        label.to_owned(),
        theme::body(),
        ink,
        (rect.width() - LABEL_X - 10.0).max(0.0),
    );
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    let galley = painter.layout_job(job);
    let elided = galley.elided;
    painter.galley(
        pos2(
            rect.min.x + LABEL_X,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        ink,
    );
    super::ring_when_focused(painter, &response, rect, f32::from(theme::CARD_RADIUS_U8));
    if elided {
        response.on_hover_text(label)
    } else {
        response
    }
}

/// A row of `labels` as one control, the `selected` one lit; answers the
/// segment clicked. The segments share the row's width equally.
pub fn segmented(ui: &mut egui::Ui, labels: &[&str], selected: Option<usize>) -> Option<usize> {
    let (rect, row) = ui.allocate_exact_size(vec2(ui.available_width(), SEGMENT), Sense::hover());
    let palette = theme::palette();
    let [track, hover, chosen] = SEGMENTS;
    let radius = f32::from(theme::PILL_RADIUS_U8);
    ui.painter().rect_filled(rect, radius, palette.wash(track));
    let count = labels.len().max(1) as f32;
    let width = rect.width() / count;
    let mut clicked = None;
    for (index, label) in labels.iter().enumerate() {
        let cell = Rect::from_min_size(
            pos2(rect.min.x + width * index as f32, rect.min.y),
            vec2(width, SEGMENT),
        );
        let response = ui
            .interact(cell, row.id.with(index), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand);
        let on = selected == Some(index);
        let plate = cell.shrink(2.0);
        if on {
            ui.painter()
                .rect_filled(plate, radius - 2.0, palette.wash(chosen));
        } else if response.hovered() {
            ui.painter()
                .rect_filled(plate, radius - 2.0, palette.wash(hover));
        }
        let (font, ink) = if on {
            (theme::semibold(theme::META_SIZE), palette.text)
        } else {
            (theme::medium(theme::META_SIZE), palette.secondary)
        };
        ui.painter().text(
            cell.center(),
            egui::Align2::CENTER_CENTER,
            *label,
            font,
            ink,
        );
        super::ring_when_focused(ui.painter(), &response, plate, radius - 2.0);
        if response.clicked() {
            clicked = Some(index);
        }
    }
    clicked
}

/// One setting as a row: its label, an optional note under it that wraps to
/// two lines, and its switch at the right edge. The whole row is the control.
pub fn switch_row(ui: &mut egui::Ui, label: &str, note: Option<&str>, on: bool) -> Response {
    let width = ui.available_width();
    let text_width = (width - SWITCH.x - SWITCH_GAP).max(0.0);
    let palette = theme::palette();
    let mut title = LayoutJob::simple(label.to_owned(), theme::body(), palette.text, text_width);
    title.wrap.max_rows = 1;
    let title = ui.painter().layout_job(title);
    let note = note.map(|note| {
        let mut job = LayoutJob::simple(
            note.to_owned(),
            theme::meta(),
            palette.secondary,
            text_width,
        );
        job.wrap.max_rows = 2;
        ui.painter().layout_job(job)
    });
    let text_height = title.size().y + note.as_ref().map_or(0.0, |n| LINE_GAP + n.size().y);
    let floor = if note.is_some() {
        SWITCH_ROW[1]
    } else {
        SWITCH_ROW[0]
    };
    let height = floor.max(text_height + 16.0);
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    let response = response.on_hover_cursor(CursorIcon::PointingHand);
    let look = Look::of(&response);
    let painter = ui.painter();
    let bleed = rect.expand2(vec2(ROW_BLEED, 0.0));
    if look.hovered {
        painter.rect_filled(bleed, theme::CARD_RADIUS, palette.wash(12));
    }
    let top = rect.center().y - text_height / 2.0;
    let title_height = title.size().y;
    painter.galley(pos2(rect.min.x, top), title, palette.text);
    if let Some(note) = note {
        painter.galley(
            pos2(rect.min.x, top + title_height + LINE_GAP),
            note,
            palette.secondary,
        );
    }
    let track = Rect::from_min_size(
        pos2(rect.max.x - SWITCH.x, rect.center().y - SWITCH.y / 2.0),
        SWITCH,
    );
    let t = ui.ctx().animate_bool_responsive(response.id, on);
    paint_switch(painter, track, t, look.hovered);
    super::ring_when_focused(painter, &response, bleed, f32::from(theme::CARD_RADIUS_U8));
    response
}

/// A switch in `track`, `t` of the way from off (0) to on (1): a pill track,
/// bright when on and quiet when off, with its round knob at that end.
fn paint_switch(painter: &Painter, track: Rect, t: f32, hovered: bool) {
    let palette = theme::palette();
    let radius = track.height() / 2.0;
    let [rest, hover] = TRACK_OFF;
    let off = f32::from(if hovered { hover } else { rest }) / 255.0;
    let fill = theme::mix(
        theme::mix(palette.surface, palette.highlight, off),
        palette.accent,
        t,
    );
    painter.rect_filled(track, radius, fill);
    let knob = radius - KNOB_PAD;
    let x = egui::lerp(
        (track.left() + radius)..=(track.right() - radius),
        t.clamp(0.0, 1.0),
    );
    let ink = theme::mix(palette.secondary, palette.on_accent, t);
    painter.circle_filled(pos2(x, track.center().y), knob, ink);
}

/// A labelled slider over `range`, moving in `step`s, with its value shown
/// at its right as `format` writes it. Dragging, a click on the track, and
/// the arrow keys while it holds the focus all move it; the response says
/// `changed` when the value did.
pub fn slider(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f64,
    range: RangeInclusive<f64>,
    step: f64,
    format: impl Fn(f64) -> String,
) -> Response {
    let (rect, row) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::hover());
    let palette = theme::palette();
    let track_left = rect.min.x + SLIDER_LABEL + KNOB;
    let track_right = (rect.max.x - SLIDER_VALUE - KNOB).max(track_left + 1.0);
    let hit = Rect::from_min_max(
        pos2(track_left - KNOB, rect.min.y),
        pos2(track_right + KNOB, rect.max.y),
    );
    let mut response = ui
        .interact(hit, row.id.with("knob"), Sense::click_and_drag())
        .on_hover_cursor(CursorIcon::PointingHand);
    let (lo, hi) = (*range.start(), *range.end());
    let snap = |v: f64| {
        let stepped = if step > 0.0 {
            lo + ((v - lo) / step).round() * step
        } else {
            v
        };
        stepped.clamp(lo, hi)
    };
    let before = *value;
    if let Some(pointer) = response.interact_pointer_pos() {
        if response.is_pointer_button_down_on() || response.clicked() {
            let t = f64::from((pointer.x - track_left) / (track_right - track_left));
            *value = snap(lo + t.clamp(0.0, 1.0) * (hi - lo));
        }
    }
    if response.has_focus() {
        let (left, right) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::ArrowLeft),
                i.key_pressed(egui::Key::ArrowRight),
            )
        });
        if left {
            *value = snap(*value - step);
        }
        if right {
            *value = snap(*value + step);
        }
    }
    if (*value - before).abs() > f64::EPSILON {
        response.mark_changed();
    }
    let painter = ui.painter();
    let cy = rect.center().y;
    let mut name = LayoutJob::simple(
        label.to_owned(),
        theme::body(),
        palette.text,
        SLIDER_LABEL - 8.0,
    );
    name.wrap.max_rows = 1;
    let name = painter.layout_job(name);
    painter.galley(
        pos2(rect.min.x, cy - name.size().y / 2.0),
        name,
        palette.text,
    );
    let t = if hi > lo {
        ((*value - lo) / (hi - lo)).clamp(0.0, 1.0) as f32
    } else {
        0.0
    };
    let x = egui::lerp(track_left..=track_right, t);
    let rail = CornerRadius::same((TRACK / 2.0) as u8);
    painter.rect_filled(
        Rect::from_min_max(
            pos2(track_left, cy - TRACK / 2.0),
            pos2(track_right, cy + TRACK / 2.0),
        ),
        rail,
        palette.wash(TRACK_WASH),
    );
    painter.rect_filled(
        Rect::from_min_max(
            pos2(track_left, cy - TRACK / 2.0),
            pos2(x, cy + TRACK / 2.0),
        ),
        rail,
        palette.accent,
    );
    let look = Look::of(&response);
    let knob = if look.hovered || look.pressed {
        KNOB + 1.0
    } else {
        KNOB
    };
    painter.circle_filled(pos2(x, cy), knob, palette.accent);
    painter.circle_stroke(
        pos2(x, cy),
        knob,
        Stroke::new(theme::HAIRLINE, palette.scrim(90)),
    );
    super::round_ring_when_focused(painter, &response, pos2(x, cy), knob);
    painter.text(
        pos2(rect.max.x, cy),
        egui::Align2::RIGHT_CENTER,
        format(*value),
        theme::meta(),
        palette.secondary,
    );
    response
}
