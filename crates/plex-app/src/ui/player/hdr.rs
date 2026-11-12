//! The overlay's side of the HDR window: the button that offers it while an HDR
//! video plays here, and the card this window shows while the picture is there.

use egui::{pos2, vec2, Align2, Color32, CursorIcon, Rect, Sense};

use super::{Action, GLASS};
use crate::player::hdr::{is_dolby_vision, is_hdr};
use crate::player::Player;
use crate::ui::theme;

/// Whether this item may move to the HDR window: an HDR or Dolby Vision file
/// played as it sits. A server transcode has already tone-mapped the picture,
/// so there is nothing left for the HDR window to do.
#[must_use]
pub(crate) fn offered(p: &Player) -> bool {
    !p.output().is_hdr_window() && p.request().session.is_none() && is_hdr(&p.snapshot().video)
}

/// "HDR window" under the badges while an HDR video plays here.
pub(super) fn button(root: &mut egui::Ui, screen: Rect, p: &Player) -> Option<Action> {
    if !offered(p) {
        return None;
    }
    let size = vec2(150.0, 40.0);
    let rect = Rect::from_min_size(pos2(screen.right() - 28.0 - size.x, 96.0), size);
    let hover = if is_dolby_vision(&p.snapshot().video) {
        "Play with Dolby Vision colours and real HDR in a separate window"
    } else {
        "Play with real HDR in a separate window"
    };
    glass_button(root, rect, "hdr-window", "HDR window", hover).then(|| Action::Output {
        hdr: true,
        at: p.snapshot().position.unwrap_or(0.0),
    })
}

/// The card this window shows while the picture plays in the HDR window: the
/// controls below keep working from here.
pub(super) fn card(root: &mut egui::Ui, screen: Rect, p: &Player) -> Option<Action> {
    let size = vec2(440.0, 172.0);
    let rect = Rect::from_center_size(screen.center() - vec2(0.0, 40.0), size);
    let painter = root.painter();
    painter.rect_filled(rect, theme::CARD_RADIUS, GLASS);
    painter.text(
        rect.center_top() + vec2(0.0, 26.0),
        Align2::CENTER_TOP,
        "Playing in the HDR window",
        egui::FontId::proportional(22.0),
        theme::TEXT,
    );
    painter.text(
        rect.center_top() + vec2(0.0, 62.0),
        Align2::CENTER_TOP,
        "Space pauses there, F toggles full screen, Q closes it.",
        theme::body(),
        theme::SECONDARY,
    );
    let back = Rect::from_min_size(rect.left_bottom() + vec2(28.0, -64.0), vec2(260.0, 40.0));
    let stop = Rect::from_min_size(back.right_top() + vec2(12.0, 0.0), vec2(112.0, 40.0));
    let at = p.snapshot().position.unwrap_or(0.0);
    let hover = "Close the HDR window and keep playing in Chewvy, tone-mapped";
    if glass_button(root, back, "hdr-back", "Close HDR window, play here", hover) {
        return Some(Action::Output { hdr: false, at });
    }
    glass_button(root, stop, "hdr-stop", "Stop", "Stop playing").then_some(Action::Back(None))
}

/// One small-radius, glass-filled button with a label; `true` when clicked.
fn glass_button(root: &mut egui::Ui, rect: Rect, id: &str, label: &str, hover: &str) -> bool {
    let resp = root.interact(rect, root.id().with(id), Sense::click());
    let (fill, text) = if resp.hovered() {
        (theme::PRIMARY_BG, theme::PRIMARY_TEXT)
    } else {
        (GLASS, theme::TEXT)
    };
    let painter = root.painter();
    painter.rect_filled(rect, theme::CARD_RADIUS, fill);
    painter.rect_stroke(
        rect,
        theme::CARD_RADIUS,
        egui::Stroke::new(1.0, Color32::from_white_alpha(90)),
        egui::StrokeKind::Inside,
    );
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        theme::body(),
        text,
    );
    resp.on_hover_text(hover)
        .on_hover_cursor(CursorIcon::PointingHand)
        .clicked()
}
