//! The pill controls a browsing screen carries: an outlined chip for a control
//! that is off, an accent-filled one for the control in effect or whose menu is
//! up, and the menu a chip opens with a painted check on the row that is.
//!
//! Home's pager arrows ([`cards::arrow`](super::cards)) are a different control:
//! a chip changes what the next request asks for, so it is the state, not an
//! action, that it paints.

use egui::{
    text::LayoutJob, Align2, Color32, CornerRadius, CursorIcon, Pos2, Rect, Response, Sense,
    Stroke, StrokeKind, Ui, Vec2,
};

use crate::ui::theme;

/// One chip, and one row of the menu it opens, is this tall: a chip and its
/// menu read as one control.
pub const HEIGHT: f32 = 30.0;
/// The room a chip's label gets on each side of it.
pub const PAD_X: f32 = 12.0;
/// Chips in a row sit this far apart.
pub const GAP: f32 = 8.0;
/// A chip's ends are fully round.
pub const RADIUS: CornerRadius = CornerRadius::same(255);
/// The hairline edge of a chip that is not in effect: readable on
/// [`theme::SURFACE`], quieter than the accent edge of one that is.
pub const OUTLINE: Color32 = Color32::from_rgb(0x39, 0x40, 0x4D);
/// The width a chip's menu asks for: its rows are this wide plus
/// [`CHECK_COLUMN`], or whatever the chip's own row offers, whichever is wider,
/// and a label longer than that row is elided inside it.
pub const MENU_WIDTH: f32 = 210.0;
/// The space a menu row gives its check, and the gap from the check to the label.
pub const CHECK_COLUMN: f32 = 22.0;

/// The fill, edge and ink of a chip that is in effect (`active`) or whose menu is
/// up (`open`): [`theme::ACCENT_TINT`] under [`theme::ACCENT`], which stays
/// readable where a solid accent fill would not.
#[must_use]
pub fn chip_colors(active: bool, open: bool) -> (Color32, Color32, Color32) {
    if active || open {
        (theme::ACCENT_TINT, theme::ACCENT, theme::ACCENT)
    } else {
        (Color32::TRANSPARENT, OUTLINE, theme::SECONDARY)
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
        .layout_no_wrap(label.to_owned(), theme::body(), Color32::WHITE);
    let size = Vec2::new(text.size().x + 2.0 * PAD_X, HEIGHT);
    let response = ui
        .allocate_response(size, Sense::click())
        .on_hover_cursor(CursorIcon::PointingHand);
    Slot {
        rect: response.rect,
        response,
    }
}

/// Paint the chip in `rect`: an outline when it is off and its menu closed,
/// [`theme::ACCENT_TINT`] under [`theme::ACCENT`] when it is in effect or its
/// menu is up.
pub fn paint(ui: &Ui, rect: Rect, label: &str, active: bool, open: bool) {
    let (fill, edge, ink) = chip_colors(active, open);
    ui.painter().rect_filled(rect, RADIUS, fill);
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
}

/// A chip that flips a state on and off; answers whether it was clicked.
pub fn toggle(ui: &mut Ui, label: &str, active: bool, hover: &str) -> bool {
    let chip = slot(ui, label);
    paint(ui, chip.rect, label, active, false);
    chip.response.on_hover_text(hover).clicked()
}

/// The menu [`slot`] opens when it is clicked, closed by a row that was clicked
/// or by [`Ui::close`]. Answers whether the menu is open this frame, which is
/// the state its caller paints the chip with.
///
/// The open state lives in egui memory keyed to the chip's own [`Response`], the
/// same mechanism [`egui::Ui::menu_button`] uses, so a chip keeps menu behaviour
/// without giving up its own visuals.
pub fn menu(ui: &mut Ui, chip: &Slot, contents: impl FnOnce(&mut Ui)) -> bool {
    // `Popup::menu` toggles its own open state on a click of the chip, so it is
    // shown every frame; its state is keyed by `Popup::default_response_id`.
    let shown = egui::Popup::menu(&chip.response)
        .frame(
            egui::Frame::menu(ui.style())
                .fill(theme::SURFACE)
                .stroke(Stroke::new(theme::HAIRLINE, OUTLINE))
                .corner_radius(theme::CARD_RADIUS)
                .inner_margin(egui::Margin::same(4)),
        )
        .show(|ui| contents(ui));
    shown.is_some()
}

/// One row of a chip's menu: `label` with [`check`] where `checked`. Answers
/// whether it was clicked.
pub fn menu_row(ui: &mut Ui, label: &str, checked: bool) -> bool {
    let width = ui.available_width().max(MENU_WIDTH + CHECK_COLUMN);
    let response = ui.allocate_response(Vec2::new(width, HEIGHT), Sense::click());
    if response.hovered() {
        ui.painter()
            .rect_filled(response.rect, theme::CARD_RADIUS, theme::CARD_HOVER);
    }
    if checked {
        check(
            ui,
            response.rect.left_center() + egui::vec2(CHECK_COLUMN * 0.5, 0.0),
        );
    }
    let ink = if checked {
        theme::TEXT
    } else {
        theme::SECONDARY
    };
    let label_min = response.rect.min.x + CHECK_COLUMN + theme::SPACING_BASE;
    let galley = ui.painter().layout_job(label_job(
        label,
        ink,
        (response.rect.max.x - label_min - PAD_X).max(0.0),
    ));
    ui.painter().galley(
        Pos2::new(label_min, response.rect.center().y - galley.size().y * 0.5),
        galley,
        ink,
    );
    response.clicked()
}

/// `label` laid out for a menu row: one row, broken anywhere, ended with `…`
/// when a server-supplied title is longer than the row's label box.
#[must_use]
pub fn label_job(label: &str, ink: Color32, max_width: f32) -> LayoutJob {
    let mut job = LayoutJob::simple(label.to_owned(), theme::body(), ink, max_width);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job
}

/// The tick that marks the row of a chip's menu that is in effect. The
/// proportional fonts egui builds its family from name no ✓, so it is drawn.
pub fn check(ui: &Ui, center: Pos2) {
    ui.painter().line(
        check_points(center).to_vec(),
        Stroke::new(1.8, theme::ACCENT),
    );
}

/// The three points of a check centred on `center`: the short arm down from the
/// left, its corner at the bottom, then the long arm up to the right.
#[must_use]
pub fn check_points(center: Pos2) -> [Pos2; 3] {
    [
        center + egui::vec2(-6.5, -0.5),
        center + egui::vec2(-2.0, 4.5),
        center + egui::vec2(6.5, -5.5),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_control_in_effect_gets_the_accent_fill() {
        assert_eq!(chip_colors(false, false).0, Color32::TRANSPARENT);
        assert_eq!(chip_colors(false, false).1, OUTLINE);
        assert_eq!(
            chip_colors(true, false),
            (theme::ACCENT_TINT, theme::ACCENT, theme::ACCENT)
        );
        // A menu that is up reads as in effect even while its filter is off.
        assert_eq!(chip_colors(false, true).0, theme::ACCENT_TINT);
    }

    /// A tick is drawn, not typed: the fonts name no glyph for one, and its
    /// corner must sit below both arm ends so it reads as a check and not a v.
    #[test]
    fn a_check_is_painted_with_its_corner_at_the_bottom() {
        let [left, corner, right] = check_points(egui::pos2(0.0, 0.0));
        assert!(
            left.x < corner.x && corner.x < right.x,
            "{left} {corner} {right}"
        );
        assert!(corner.y > left.y && corner.y > right.y);
        assert!(right.y < left.y);
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

    /// A menu row's label is laid out for the row it is drawn in, so a title
    /// longer than that row ends in `…` on one line instead of being painted
    /// past the menu box or wrapped over the rows under it.
    #[test]
    fn a_menu_label_is_laid_out_for_one_elided_row() {
        let job = label_job("Release Date", theme::TEXT, 60.0);
        assert_eq!(job.wrap.max_rows, 1, "a menu row takes one line only");
        assert!(job.wrap.break_anywhere, "a long title breaks where it must");
        assert_eq!(job.wrap.max_width, 60.0, "the label box is the row's own");
        assert_eq!(
            job.wrap.overflow_character,
            Some('…'),
            "epaint's default overflow character ends the elided row"
        );
        let fits = label_job("Release Date", theme::TEXT, MENU_WIDTH);
        assert_eq!(fits.wrap.max_width, MENU_WIDTH);
    }
}
