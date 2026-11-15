//! The sign-in and start-up screens: a frosted glass card over the aurora
//! backdrop ([`crate::ui::backdrop`]), an orbit animation while the app signs
//! in, finds the account's servers and connects, and the pairing code as large
//! tiles.

use egui::{pos2, vec2, Align2, CornerRadius, Rect, Sense, Stroke};

use crate::ui::marks::icon::{self, Icon, Size};
use crate::ui::{glass, theme};

/// The card's width.
const CARD_WIDTH: f32 = 460.0;
/// The card's corner radius.
const CARD_RADIUS: f32 = 24.0;
/// The card's inner padding.
const CARD_PAD: f32 = 32.0;
/// The type size of the name in a card's [`masthead`]; the mark matches it.
const MASTHEAD: f32 = 30.0;
/// The orbit animation's radius.
const ORBIT: f32 = 26.0;
/// The card height its top is placed for: the tallest sign-in step, so on a
/// window tall enough for that step no step moves the card's title.
const ANCHOR_HEIGHT: f32 = 560.0;
/// Room kept above the card: the window buttons' strip.
const TOP_ROOM: f32 = crate::ui::chrome::TITLE_BAR;
/// Room kept under the card: the error strip and the not-affiliated line.
const BOTTOM_ROOM: f32 = 72.0;
/// The not-affiliated strip's distance from the window's bottom edge.
const NOT_AFFILIATED_BOTTOM: f32 = 16.0;
/// The scroll area's inset from the card's sides: its bar runs in the card's
/// padding, clear of the contents.
const SCROLL_INSET: f32 = 12.0;

/// A glass card `CARD_WIDTH` wide holding `add`, as tall as its contents.
///
/// The card's top stays where a card [`ANCHOR_HEIGHT`] tall would be centred,
/// so a step of another height grows or shrinks it downwards and the title
/// stays put. It moves up only when the card would otherwise run into the
/// bottom strips, and a card taller than the window allows scrolls.
pub fn card(ui: &mut egui::Ui, area: Rect, add: impl FnOnce(&mut egui::Ui)) {
    let id = ui.id().with("splash-card-height");
    let wanted = ui.data(|d| d.get_temp::<f32>(id)).unwrap_or(320.0);
    let width = CARD_WIDTH.min(area.width() - 32.0);
    let top_limit = area.top() + TOP_ROOM;
    let bottom_limit = area.bottom() - BOTTOM_ROOM;
    let height = wanted.min(bottom_limit - top_limit);
    let top = (area.center().y - ANCHOR_HEIGHT * 0.5)
        .min(bottom_limit - height)
        .max(top_limit);
    let rect = Rect::from_min_size(
        pos2(area.center().x - width * 0.5, top),
        vec2(width, height),
    );
    glass::panel(
        ui.painter(),
        rect,
        CARD_RADIUS,
        theme::current().glass.splash_card,
    );
    // One point in from the top and bottom so scrolled contents never cover
    // the glass rim.
    let viewport = rect.shrink2(vec2(SCROLL_INSET, 1.0));
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(viewport)
            .layout(egui::Layout::top_down(egui::Align::Center)),
    );
    let pad = egui::Margin::symmetric((CARD_PAD - SCROLL_INSET) as i8, (CARD_PAD - 1.0) as i8);
    let used = egui::ScrollArea::vertical()
        .id_salt("splash-card-scroll")
        .max_height(viewport.height())
        .auto_shrink([false, true])
        .show(&mut child, |ui| {
            egui::Frame::NONE.inner_margin(pad).show(ui, add)
        })
        .content_size
        .y
        + 2.0;
    if (used - wanted).abs() > 0.5 {
        ui.data_mut(|d| d.insert_temp(id, used));
        // Laid out again before painting, so the glass never trails its
        // contents by a frame.
        ui.ctx().request_discard("splash card height changed");
    }
}

/// The not-affiliated line along the bottom edge of a first-run screen, on a
/// strip of the card's glass so it reads over a bright part of the backdrop.
pub fn not_affiliated(ui: &egui::Ui, area: Rect) {
    let painter = ui.painter();
    let text = painter.layout_no_wrap(
        crate::ui::brand::NOT_AFFILIATED.to_owned(),
        theme::caption(),
        theme::palette().text,
    );
    let size = text.size() + vec2(20.0, 8.0);
    let centre = pos2(
        area.center().x,
        area.max.y - NOT_AFFILIATED_BOTTOM - size.y / 2.0,
    );
    let strip = Rect::from_center_size(centre, size);
    glass::panel(painter, strip, 6.0, theme::current().glass.splash_card);
    painter.galley(centre - text.size() * 0.5, text, theme::palette().text);
}

/// The app's mark and name, centred at the top of a first-run card.
pub fn masthead(ui: &mut egui::Ui) {
    let width = crate::ui::brand::wordmark_width(ui.painter(), MASTHEAD);
    let (rect, _) = ui.allocate_exact_size(vec2(width, MASTHEAD * 1.6), Sense::hover());
    crate::ui::brand::wordmark(ui.painter(), rect.min.x, rect.center().y, MASTHEAD);
    ui.add_space(18.0);
}

/// A card's title in SemiBold with a line under it.
pub fn heading(ui: &mut egui::Ui, title: &str, subtitle: &str) {
    ui.label(
        egui::RichText::new(title)
            .font(theme::semibold(30.0))
            .color(theme::palette().text),
    );
    ui.add_space(6.0);
    ui.label(
        egui::RichText::new(subtitle)
            .font(theme::body())
            .color(theme::palette().secondary),
    );
    ui.add_space(24.0);
}

/// Three satellites on a faint ring, each trailing a comet tail, at
/// three speeds: the app is working.
pub fn orbit(ui: &mut egui::Ui) {
    let (rect, _) =
        ui.allocate_exact_size(vec2(ORBIT * 2.0 + 16.0, ORBIT * 2.0 + 16.0), Sense::hover());
    let painter = ui.painter();
    let center = rect.center();
    let time = ui.input(|i| i.time) as f32;
    painter.circle_stroke(center, ORBIT, Stroke::new(1.5, theme::palette().wash(28)));
    painter.circle_filled(center, 4.0, theme::palette().accent.gamma_multiply(0.5));
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
                theme::palette().accent.gamma_multiply(alpha),
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
        painter.rect_filled(cell, CornerRadius::same(12), theme::palette().wash(14));
        painter.rect_stroke(
            cell,
            CornerRadius::same(12),
            Stroke::new(1.0, theme::palette().wash(40)),
            egui::StrokeKind::Inside,
        );
        painter.text(
            cell.center(),
            Align2::CENTER_CENTER,
            ch.to_string(),
            theme::semibold(34.0),
            theme::palette().text,
        );
    }
}

/// One server in the picker: a full-width glass row; true when clicked.
pub fn server_row(ui: &mut egui::Ui, name: &str) -> bool {
    let (rect, press) = ui.allocate_exact_size(vec2(ui.available_width(), 52.0), Sense::click());
    let press = crate::ui::cards::pointer(press);
    let fill = if press.hovered() || press.has_focus() {
        theme::palette().wash(26)
    } else {
        theme::palette().wash(12)
    };
    ui.painter().rect_filled(rect, CornerRadius::same(14), fill);
    ui.painter().text(
        rect.left_center() + vec2(18.0, 0.0),
        Align2::LEFT_CENTER,
        name,
        theme::medium(theme::BODY_SIZE),
        theme::palette().text,
    );
    let ink = if press.hovered() {
        theme::palette().accent
    } else {
        theme::palette().secondary
    };
    crate::ui::controls::ring_when_focused(ui.painter(), &press, rect, 14.0);
    let chevron = rect.right_center() - vec2(12.0 + icon::SIZE / 2.0, 0.0);
    icon::paint(
        ui.painter(),
        Icon::ChevronRight,
        Size::Regular,
        chevron,
        ink,
    );
    ui.add_space(8.0);
    press.clicked()
}
