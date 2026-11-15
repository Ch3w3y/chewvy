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
use egui::{pos2, vec2, Align2, CursorIcon, Id, Order, Rect, Sense, ViewportCommand};

use crate::ui::marks::icon::{self, Icon, Size};
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
const HOVER_RADIUS: f32 = 16.0;
/// The group's inset from the window's top-right corner.
const INSET: [f32; 2] = [10.0, 6.0];

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
    // The strip and the edges are the window's frame, not controls: the
    // keyboard's Tab never stops on them.
    let drag = root.interact(strip, Id::new("chrome-drag"), Sense::CLICK | Sense::DRAG);
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
        let edge = root.interact(rect, Id::new(("chrome-edge", direction as u8)), Sense::DRAG);
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

/// One button: a hover fill and its icon; true when clicked.
fn button(ui: &mut egui::Ui, cell: Rect, kind: Kind, maximized: bool) -> bool {
    let response = ui.interact(cell, Id::new(("chrome-button", kind as u8)), Sense::click());
    let painter = ui.painter();
    // The disc's centre on a device pixel, so its edge is as crisp as the
    // glyph the icon set already places on whole pixels.
    let center = pos2(
        painter.round_to_pixel_center(cell.center().x),
        painter.round_to_pixel_center(cell.center().y),
    );
    let pressed = response.is_pointer_button_down_on();
    if response.hovered() || pressed {
        let palette = theme::palette();
        let fill = match (kind, pressed) {
            (Kind::Close, false) => palette.danger_fill,
            (Kind::Close, true) => theme::mix(palette.danger_fill, palette.shadow, 0.2),
            (_, false) => palette.wash(30),
            (_, true) => palette.wash(52),
        };
        painter.circle_filled(center, HOVER_RADIUS, fill);
    }
    crate::ui::controls::round_ring_when_focused(painter, &response, center, HOVER_RADIUS);
    let glyph = match kind {
        Kind::Minimise => Icon::WinMinimise,
        Kind::Maximise if maximized => Icon::WinRestore,
        Kind::Maximise => Icon::WinMaximise,
        Kind::Close => Icon::Close,
    };
    icon::paint(painter, glyph, Size::Regular, center, theme::palette().text);
    let response = crate::ui::cards::pointer(response);
    response.clicked()
}
