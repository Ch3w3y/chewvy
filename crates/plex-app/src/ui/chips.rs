//! The pill controls a browsing screen carries: an outlined chip for a control
//! that is off, an accent-filled one for the control in effect or whose menu is
//! up, and the menu a chip opens with a check on the row that is.
//!
//! Home's pager arrows ([`cards::arrow`](super::cards)) are a different control:
//! a chip changes what the next request asks for, so it is the state, not an
//! action, that it paints.

use egui::{
    Align2, Color32, CornerRadius, CursorIcon, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui,
    Vec2,
};

use crate::ui::marks::icon::{self, Icon, Size};
use crate::ui::{controls, glass, theme};

/// One chip, and one row of the menu it opens, is this tall: a chip and its
/// menu read as one control.
pub const HEIGHT: f32 = 32.0;
/// The room a chip's label gets on each side of it.
pub const PAD_X: f32 = 12.0;
/// Chips in a row sit this far apart.
pub const GAP: f32 = 8.0;
/// A chip's ends are fully round.
pub const RADIUS: CornerRadius = CornerRadius::same(255);
/// The strengths of the wash an off chip is filled with while pointed at and
/// while pressed, and that lifts an on chip while pointed at.
const HOVER: [u8; 3] = [20, 34, 16];
/// The width a chip's menu asks for: its rows are this wide plus
/// [`CHECK_COLUMN`], or whatever the chip's own row offers, whichever is wider,
/// and a label longer than that row is elided inside it.
pub const MENU_WIDTH: f32 = 210.0;
/// The space a menu row gives its check, and the gap from the check to the label.
pub const CHECK_COLUMN: f32 = 22.0;

/// The fill, edge and ink of a chip that is in effect (`active`) or whose menu is
/// up (`open`): the accent tint under the accent, which stays
/// readable where a solid accent fill would not.
#[must_use]
pub fn chip_colors(active: bool, open: bool) -> (Color32, Color32, Color32) {
    let palette = theme::palette();
    if active || open {
        (palette.accent_tint(), palette.accent, palette.accent)
    } else {
        (Color32::TRANSPARENT, palette.outline, palette.secondary)
    }
}

/// A chip's slot in the current row: the box it was given and its click.
pub struct Slot {
    pub rect: Rect,
    pub response: Response,
}

/// A chip laid out in the current row, not yet painted: the caller paints it
/// with [`paint`] once it knows whether the chip is in effect or whose menu is
/// up, which for a menu chip is only known after [`menu`] has run.
#[must_use]
pub fn slot(ui: &mut Ui, label: &str) -> Slot {
    let text = ui
        .painter()
        .layout_no_wrap(label.to_owned(), theme::body(), Color32::PLACEHOLDER);
    let size = Vec2::new(text.size().x + 2.0 * PAD_X, HEIGHT);
    let response = ui
        .allocate_response(size, Sense::click())
        .on_hover_cursor(CursorIcon::PointingHand);
    Slot {
        rect: response.rect,
        response,
    }
}

/// Paint `chip`: an outline when it is off and its menu closed, the accent
/// tint under the accent when it is in effect or its menu is up. Pointed at,
/// it lifts; pressed, it lifts further; holding the keyboard focus, it is
/// ringed.
pub fn paint(ui: &Ui, chip: &Slot, label: &str, active: bool, open: bool) {
    let rect = chip.rect;
    let look = controls::Look::of(&chip.response);
    let (fill, edge, mut ink) = chip_colors(active, open);
    ui.painter().rect_filled(rect, RADIUS, fill);
    let [hover, pressed, lift] = HOVER;
    if look.pressed || look.hovered {
        let wash = match (active || open, look.pressed) {
            (_, true) => pressed,
            (false, false) => hover,
            (true, false) => lift,
        };
        ui.painter()
            .rect_filled(rect, RADIUS, theme::palette().wash(wash));
        if !(active || open) {
            ink = theme::palette().text;
        }
    }
    ui.painter().rect_stroke(
        rect,
        RADIUS,
        Stroke::new(theme::HAIRLINE, edge),
        StrokeKind::Middle,
    );
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        theme::body(),
        ink,
    );
    controls::ring_when_focused(ui.painter(), &chip.response, rect, rect.height() / 2.0);
}

/// A chip that flips a state on and off; answers whether it was clicked.
pub fn toggle(ui: &mut Ui, label: &str, active: bool, hover: &str) -> bool {
    let chip = slot(ui, label);
    paint(ui, &chip, label, active, false);
    chip.response.on_hover_text(hover).clicked()
}

/// The menu [`slot`] opens when it is clicked, closed by a row that was clicked
/// or by [`Ui::close`]. Answers whether the menu is open this frame, which is
/// the state its caller paints the chip with.
///
/// The open state lives in egui memory keyed to the chip's own [`Response`], the
/// same mechanism [`egui::Ui::menu_button`] uses, so a chip keeps menu behaviour
/// without giving up its own visuals.
pub fn menu(chip: &Slot, contents: impl FnOnce(&mut Ui)) -> bool {
    // `Popup::menu` toggles its own open state on a click of the chip, so it is
    // shown every frame; its state is keyed by `Popup::default_response_id`.
    let shown = egui::Popup::menu(&chip.response)
        .frame(egui::Frame::NONE)
        .show(|ui| {
            let slot = glass::reserve(ui.painter());
            let menu = egui::Frame::NONE
                .inner_margin(egui::Margin::same(4))
                .show(ui, contents);
            glass::fill(
                ui.painter(),
                slot,
                menu.response.rect,
                f32::from(theme::CARD_RADIUS_U8) + 2.0,
                theme::current().glass.panel,
            );
        });
    shown.is_some()
}

/// One row of a chip's menu: `label` with a check where `checked`. Answers
/// whether it was clicked.
pub fn menu_row(ui: &mut Ui, label: &str, checked: bool) -> bool {
    let width = ui.available_width().max(MENU_WIDTH + CHECK_COLUMN);
    controls::option_row_sized(ui, label, checked, Vec2::new(width, HEIGHT)).clicked()
}

/// The tick that marks the row of a chip's menu that is in effect.
pub fn check(ui: &Ui, center: Pos2) {
    icon::paint(
        ui.painter(),
        Icon::Check,
        Size::Regular,
        center,
        theme::palette().accent,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_control_in_effect_gets_the_accent_fill() {
        assert_eq!(chip_colors(false, false).0, Color32::TRANSPARENT);
        assert_eq!(chip_colors(false, false).1, theme::palette().outline);
        assert_eq!(
            chip_colors(true, false),
            (
                theme::palette().accent_tint(),
                theme::palette().accent,
                theme::palette().accent
            )
        );
        // A menu that is up reads as in effect even while its filter is off.
        assert_eq!(chip_colors(false, true).0, theme::palette().accent_tint());
    }

    /// A chip and its menu rows are one height, and a chip gives its label the
    /// room [`PAD_X`] promises, so the chips row reads as one control.
    #[test]
    fn a_chip_and_its_menu_rows_share_one_height() {
        let chip = Vec2::new(100.0 + 2.0 * PAD_X, HEIGHT);
        let row = Vec2::new(MENU_WIDTH + CHECK_COLUMN, HEIGHT);
        assert_eq!(chip.y, row.y);
        assert!(chip.x > 100.0, "the label keeps its padding");
        assert!(row.x > chip.x, "a menu row keeps a column for its check");
    }
}
