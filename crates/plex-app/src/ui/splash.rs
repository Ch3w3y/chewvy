//! The sign-in and start-up screens: a frosted glass card over the aurora
//! backdrop ([`crate::ui::backdrop`]), an orbit animation while the app signs
//! in, finds the account's servers and connects, and the pairing code as large
//! tiles.

use egui::{pos2, vec2, Align2, Color32, CornerRadius, Rect, Sense, Stroke};

use crate::ui::{glass, theme};

/// The card's width.
const CARD_WIDTH: f32 = 460.0;
/// The card's corner radius.
const CARD_RADIUS: f32 = 24.0;
/// The card's inner padding.
const CARD_PAD: f32 = 32.0;
/// Dark enough that body text over the brightest nebula keeps AA contrast.
const CARD_TINT: Color32 = crate::ui::glass_tints::SPLASH_CARD;
/// The orbit animation's radius.
const ORBIT: f32 = 26.0;
/// A centred glass card `CARD_WIDTH` wide holding `add`; its height is the
/// height its contents took last frame.
pub fn card(ui: &mut egui::Ui, area: Rect, add: impl FnOnce(&mut egui::Ui)) {
    let id = ui.id().with("splash-card-height");
    let height = ui.data(|d| d.get_temp::<f32>(id)).unwrap_or(320.0);
    let width = CARD_WIDTH.min(area.width() - 32.0);
    let rect = Rect::from_center_size(area.center(), vec2(width, height));
    glass::panel(ui.painter(), rect, CARD_RADIUS, CARD_TINT);
    let inner = rect.shrink(CARD_PAD);
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(Rect::from_min_size(
                inner.min,
                vec2(inner.width(), f32::INFINITY),
            ))
            .layout(egui::Layout::top_down(egui::Align::Center)),
    );
    add(&mut child);
    let used = child.min_rect().height() + 2.0 * CARD_PAD;
    if (used - height).abs() > 0.5 {
        ui.data_mut(|d| d.insert_temp(id, used));
        ui.ctx().request_repaint();
    }
}

/// The not-affiliated line along the bottom edge of a first-run screen, on a
/// strip of the card's glass so it reads over a bright part of the backdrop.
pub fn not_affiliated(ui: &egui::Ui, area: Rect) {
    let painter = ui.painter();
    let text = painter.layout_no_wrap(
        crate::ui::brand::NOT_AFFILIATED.to_owned(),
        theme::caption(),
        theme::TEXT,
    );
    let centre = pos2(area.center().x, area.max.y - 13.0);
    let strip = Rect::from_center_size(centre, text.size() + vec2(20.0, 8.0));
    glass::panel(painter, strip, 6.0, CARD_TINT);
    painter.galley(centre - text.size() * 0.5, text, theme::TEXT);
}

/// The app's mark: the title in SemiBold with a line under it.
pub fn heading(ui: &mut egui::Ui, title: &str, subtitle: &str) {
    ui.label(
        egui::RichText::new(title)
            .font(theme::semibold(30.0))
            .color(theme::TEXT),
    );
    ui.add_space(6.0);
    ui.label(
        egui::RichText::new(subtitle)
            .font(theme::body())
            .color(theme::SECONDARY),
    );
    ui.add_space(24.0);
}

/// Three amber satellites on a faint ring, each trailing a comet tail, at
/// three speeds: the app is working.
pub fn orbit(ui: &mut egui::Ui) {
    let (rect, _) =
        ui.allocate_exact_size(vec2(ORBIT * 2.0 + 16.0, ORBIT * 2.0 + 16.0), Sense::hover());
    let painter = ui.painter();
    let center = rect.center();
    let time = ui.input(|i| i.time) as f32;
    painter.circle_stroke(
        center,
        ORBIT,
        Stroke::new(1.5, Color32::from_white_alpha(28)),
    );
    painter.circle_filled(center, 4.0, theme::ACCENT.gamma_multiply(0.5));
    for (speed, offset, size, radius) in [
        (1.6_f32, 0.0_f32, 5.0_f32, ORBIT),
        (1.1, 2.1, 3.5, ORBIT * 0.62),
        (2.3, 4.2, 3.0, ORBIT * 1.0),
    ] {
        let angle = time * speed + offset;
        for step in 0..14 {
            let t = step as f32 / 14.0;
            let a = angle - t * 1.1;
            let p = center + vec2(a.cos(), a.sin()) * radius;
            let alpha = (1.0 - t).powi(2);
            painter.circle_filled(
                p,
                size * (1.0 - 0.6 * t),
                theme::ACCENT.gamma_multiply(alpha),
            );
        }
    }
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_millis(16));
}

/// The pairing code as one glass-edged tile per character.
pub fn code_tiles(ui: &mut egui::Ui, code: &str) {
    let chars: Vec<char> = code.chars().filter(|c| !c.is_whitespace()).collect();
    if chars.is_empty() {
        return;
    }
    let tile = vec2(52.0, 64.0);
    let gap = 10.0;
    let width = chars.len() as f32 * tile.x + (chars.len() - 1) as f32 * gap;
    let (rect, _) = ui.allocate_exact_size(vec2(width, tile.y), Sense::hover());
    let painter = ui.painter();
    for (index, ch) in chars.iter().enumerate() {
        let min = pos2(rect.min.x + index as f32 * (tile.x + gap), rect.min.y);
        let cell = Rect::from_min_size(min, tile);
        painter.rect_filled(cell, CornerRadius::same(12), Color32::from_white_alpha(14));
        painter.rect_stroke(
            cell,
            CornerRadius::same(12),
            Stroke::new(1.0, Color32::from_white_alpha(40)),
            egui::StrokeKind::Inside,
        );
        painter.text(
            cell.center(),
            Align2::CENTER_CENTER,
            ch.to_string(),
            theme::semibold(34.0),
            theme::TEXT,
        );
    }
}

/// The white primary pill; true when clicked.
pub fn primary(ui: &mut egui::Ui, label: &str) -> bool {
    let (rect, press) =
        ui.allocate_exact_size(vec2(ui.available_width().min(320.0), 44.0), Sense::click());
    let press = crate::ui::cards::pointer(press);
    let fill = if press.is_pointer_button_down_on() {
        theme::SECONDARY
    } else if press.hovered() {
        theme::TEXT
    } else {
        theme::PRIMARY_BG
    };
    ui.painter()
        .rect_filled(rect, CornerRadius::same(theme::PILL_RADIUS_U8), fill);
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        theme::semibold(theme::BODY_SIZE),
        theme::PRIMARY_TEXT,
    );
    press.clicked()
}

/// A quiet text action under the primary one; true when clicked.
pub fn secondary(ui: &mut egui::Ui, label: &str) -> bool {
    let press = ui.add(
        egui::Button::new(
            egui::RichText::new(label)
                .font(theme::body())
                .color(theme::SECONDARY),
        )
        .frame(false),
    );
    crate::ui::cards::pointer(press).clicked()
}

/// One server in the picker: a full-width glass row; true when clicked.
pub fn server_row(ui: &mut egui::Ui, name: &str) -> bool {
    let (rect, press) = ui.allocate_exact_size(vec2(ui.available_width(), 52.0), Sense::click());
    let press = crate::ui::cards::pointer(press);
    let fill = if press.hovered() {
        Color32::from_white_alpha(26)
    } else {
        Color32::from_white_alpha(12)
    };
    ui.painter().rect_filled(rect, CornerRadius::same(14), fill);
    ui.painter().text(
        rect.left_center() + vec2(18.0, 0.0),
        Align2::LEFT_CENTER,
        name,
        theme::medium(theme::BODY_SIZE),
        theme::TEXT,
    );
    ui.painter().text(
        rect.right_center() - vec2(18.0, 0.0),
        Align2::RIGHT_CENTER,
        "›",
        theme::semibold(20.0),
        if press.hovered() {
            theme::ACCENT
        } else {
            theme::SECONDARY
        },
    );
    ui.add_space(8.0);
    press.clicked()
}

#[cfg(test)]
mod tests {
    use super::CARD_TINT;

    /// Body text on the card must keep WCAG AA (4.5:1) even when the blur
    /// behind it is pure white: the tint alone has to carry the contrast.
    #[test]
    fn card_text_stays_legible_over_a_white_backdrop() {
        let [r, g, b, a] = CARD_TINT.to_array();
        let keep = 1.0 - f64::from(a) / 255.0;
        let over_white = |c: u8| f64::from(c) / 255.0 + keep;
        let lin = |c: f64| {
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        let luminance =
            0.2126 * lin(over_white(r)) + 0.7152 * lin(over_white(g)) + 0.0722 * lin(over_white(b));
        let text = 0.2126 * lin(0xee as f64 / 255.0)
            + 0.7152 * lin(0xf0 as f64 / 255.0)
            + 0.0722 * lin(0xff as f64 / 255.0);
        let ratio = (text + 0.05) / (luminance + 0.05);
        assert!(ratio >= 4.5, "contrast {ratio:.2}");
    }
}
