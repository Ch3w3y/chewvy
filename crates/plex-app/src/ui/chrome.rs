//! The app's own window chrome: the system title bar is turned off
//! ([`DECORATED`]), so the window draws its own minimise / maximise / close
//! buttons, moves when its top strip is dragged, double-click maximises, and
//! resizes from its edges.
//!
//! [`behind`] is called before any panel so every widget drawn afterwards wins
//! the pointer over the drag strip and the resize edges; [`controls`] is called
//! last so the buttons sit above everything.

use std::sync::atomic::{AtomicBool, Ordering};

use egui::viewport::ResizeDirection;
use egui::{
    pos2, vec2, Align2, Color32, CursorIcon, Id, Order, Rect, Sense, Stroke, ViewportCommand,
};

use crate::ui::theme;

/// Whether the window keeps the desktop's own title bar and border.
pub const DECORATED: bool = false;
/// The height of the title band along the top: it holds the window buttons
/// and nothing else, so the whole band moves the window.
pub const TITLE_BAR: f32 = 40.0;
/// The height of the strip along the top that moves the window.
const DRAG_STRIP: f32 = TITLE_BAR;
/// The width of the edge band that resizes the window.
const EDGE: f32 = 6.0;
/// One window button's hit box.
const BUTTON: [f32; 2] = [40.0, TITLE_BAR];
/// The round hover fill's radius.
const HOVER_RADIUS: f32 = 14.0;
/// The group's inset from the window's top-right corner.
const INSET: [f32; 2] = [10.0, 6.0];
/// The glyph's half-size inside a button.
const GLYPH: f32 = 5.0;
/// The close button's hover fill.
const CLOSE_HOVER: Color32 = Color32::from_rgb(0xc4, 0x2b, 0x1c);
/// The dark line under each glyph, so it reads over bright hero art.
const GLYPH_SHADOW: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 120);

/// Set by the close button; the host exits its event loop when it reads it.
static QUIT: AtomicBool = AtomicBool::new(false);

/// Close the window, as the close button does.
pub fn request_quit() {
    QUIT.store(true, Ordering::Relaxed);
}

/// Whether the close button was pressed since the last call.
pub fn take_quit() -> bool {
    QUIT.swap(false, Ordering::Relaxed)
}

/// Register the drag strip and the resize edges under everything drawn later.
pub fn behind(root: &mut egui::Ui) {
    if DECORATED {
        return;
    }
    let ctx = root.ctx().clone();
    let screen = ctx.content_rect();
    let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));

    let strip = Rect::from_min_size(screen.min, vec2(screen.width(), DRAG_STRIP));
    let drag = root.interact(strip, Id::new("chrome-drag"), Sense::click_and_drag());
    if drag.drag_started() {
        ctx.send_viewport_cmd(ViewportCommand::StartDrag);
    }
    if drag.double_clicked() {
        ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
    }
    if maximized {
        return;
    }
    for (rect, direction, icon) in edges(screen) {
        let edge = root.interact(
            rect,
            Id::new(("chrome-edge", direction as u8)),
            Sense::drag(),
        );
        if edge.hovered() || edge.dragged() {
            ctx.set_cursor_icon(icon);
        }
        if edge.drag_started() {
            ctx.send_viewport_cmd(ViewportCommand::BeginResize(direction));
        }
    }
}

/// The resize bands: four corners first so they win over the sides.
fn edges(screen: Rect) -> [(Rect, ResizeDirection, CursorIcon); 8] {
    let (l, r, t, b) = (screen.left(), screen.right(), screen.top(), screen.bottom());
    let corner = EDGE * 2.0;
    let band = |x0: f32, y0: f32, x1: f32, y1: f32| Rect::from_min_max(pos2(x0, y0), pos2(x1, y1));
    [
        (
            band(l, t, l + corner, t + corner),
            ResizeDirection::NorthWest,
            CursorIcon::ResizeNorthWest,
        ),
        (
            band(r - corner, t, r, t + corner),
            ResizeDirection::NorthEast,
            CursorIcon::ResizeNorthEast,
        ),
        (
            band(l, b - corner, l + corner, b),
            ResizeDirection::SouthWest,
            CursorIcon::ResizeSouthWest,
        ),
        (
            band(r - corner, b - corner, r, b),
            ResizeDirection::SouthEast,
            CursorIcon::ResizeSouthEast,
        ),
        (
            band(l, t, l + EDGE, b),
            ResizeDirection::West,
            CursorIcon::ResizeWest,
        ),
        (
            band(r - EDGE, t, r, b),
            ResizeDirection::East,
            CursorIcon::ResizeEast,
        ),
        (
            band(l, t, r, t + EDGE),
            ResizeDirection::North,
            CursorIcon::ResizeNorth,
        ),
        (
            band(l, b - EDGE, r, b),
            ResizeDirection::South,
            CursorIcon::ResizeSouth,
        ),
    ]
}

/// The minimise / maximise / close buttons in the top-right corner, drawn above
/// every panel. `shown` lets playback hide them with its own controls.
pub fn controls(ctx: &egui::Context, shown: bool) {
    if DECORATED || !shown {
        return;
    }
    let fullscreen = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
    // Fullscreen outlives the player, so the browsing screens keep the
    // buttons; there the middle one draws as restore and leaves fullscreen.
    let maximized = fullscreen || ctx.input(|i| i.viewport().maximized.unwrap_or(false));
    let screen = ctx.content_rect();
    egui::Area::new(Id::new("chrome-controls"))
        .order(Order::Foreground)
        .anchor(Align2::RIGHT_TOP, vec2(0.0, 0.0))
        .show(ctx, |ui| {
            let size = vec2(BUTTON[0] * 3.0, BUTTON[1]);
            let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
            let rect = Rect::from_min_size(
                pos2(screen.right() - size.x - INSET[0], screen.top() + INSET[1]),
                rect.size(),
            );
            for (index, kind) in [Kind::Minimise, Kind::Maximise, Kind::Close]
                .into_iter()
                .enumerate()
            {
                let cell = Rect::from_min_size(
                    pos2(rect.left() + index as f32 * BUTTON[0], rect.top()),
                    vec2(BUTTON[0], BUTTON[1]),
                );
                if button(ui, cell, kind, maximized) {
                    match kind {
                        Kind::Minimise => ctx.send_viewport_cmd(ViewportCommand::Minimized(true)),
                        Kind::Maximise if fullscreen => {
                            ctx.send_viewport_cmd(ViewportCommand::Fullscreen(false))
                        }
                        Kind::Maximise => {
                            ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized))
                        }
                        Kind::Close => QUIT.store(true, Ordering::Relaxed),
                    }
                }
            }
        });
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Minimise,
    Maximise,
    Close,
}

/// One button: a hover fill and a thin glyph; true when clicked.
fn button(ui: &mut egui::Ui, cell: Rect, kind: Kind, maximized: bool) -> bool {
    let response = ui.interact(cell, Id::new(("chrome-button", kind as u8)), Sense::click());
    let painter = ui.painter();
    if response.hovered() {
        let fill = if kind == Kind::Close {
            CLOSE_HOVER
        } else {
            Color32::from_white_alpha(30)
        };
        painter.circle_filled(cell.center(), HOVER_RADIUS, fill);
    } else {
        glyph(
            painter,
            cell.center() + vec2(0.0, 1.0),
            kind,
            maximized,
            Stroke::new(2.4, GLYPH_SHADOW),
        );
    }
    glyph(
        painter,
        cell.center(),
        kind,
        maximized,
        Stroke::new(1.3, theme::TEXT),
    );
    let response = crate::ui::cards::pointer(response);
    response.clicked()
}

/// One button's glyph centred on `c`.
fn glyph(painter: &egui::Painter, c: egui::Pos2, kind: Kind, maximized: bool, stroke: Stroke) {
    match kind {
        Kind::Minimise => {
            painter.line_segment([pos2(c.x - GLYPH, c.y), pos2(c.x + GLYPH, c.y)], stroke);
        }
        Kind::Maximise if maximized => {
            let back = Rect::from_center_size(c + vec2(1.5, -1.5), vec2(GLYPH * 1.6, GLYPH * 1.6));
            let front = Rect::from_center_size(c + vec2(-1.5, 1.5), vec2(GLYPH * 1.6, GLYPH * 1.6));
            painter.rect_stroke(back, 1.0, stroke, egui::StrokeKind::Inside);
            painter.rect_stroke(front, 1.0, stroke, egui::StrokeKind::Inside);
        }
        Kind::Maximise => {
            let square = Rect::from_center_size(c, vec2(GLYPH * 2.0, GLYPH * 2.0));
            painter.rect_stroke(square, 1.0, stroke, egui::StrokeKind::Inside);
        }
        Kind::Close => {
            painter.line_segment([c + vec2(-GLYPH, -GLYPH), c + vec2(GLYPH, GLYPH)], stroke);
            painter.line_segment([c + vec2(-GLYPH, GLYPH), c + vec2(GLYPH, -GLYPH)], stroke);
        }
    }
}
