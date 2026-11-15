//! One poster card of the grid: its artwork with the unwatched badge or
//! marker, and the caption under it.

use egui::{Align, CornerRadius, Rect, Sense};

use crate::ui::{cards, images, rail, theme};

use super::title::title_lines;
use super::{card_id, marker, poster_size, Card, Marker, POSTER_GAP, TITLE_GAP};

/// The pill that counts a show's unwatched episodes.
const BADGE: [f32; 2] = [26.0, 22.0];
/// Badge and marker inset from the poster's top-right corner.
const BADGE_INSET: f32 = 8.0;
/// The accent dot that marks a movie never played.
const MARK_RADIUS: f32 = 5.0;

/// One poster card: 2:3 artwork with its badge or marker, then title and year,
/// clickable while nothing is playing. Answers whether the user clicked it.
///
/// `mark_unwatched` is the [`super::Chips::unwatched`] filter in effect: only rows the
/// server already said are unwatched get the accent dot. `focused` asks egui to
/// hold the keyboard focus on this card, which is what draws its ring when the
/// screen's keyboard selection is here.
pub(crate) fn poster(
    ui: &mut egui::Ui,
    artwork: &mut images::Artwork,
    card: &Card,
    size: [f32; 2],
    playing: bool,
    mark_unwatched: bool,
    focused: bool,
) -> bool {
    let [width, height] = size;
    let poster = poster_size(width);
    let clickable = !playing && (card.playable() || card.browsable());
    // Every card is given the same box - `width` across, `height` down, the
    // height [`Self::row_height`] reserves - so a long caption costs its own
    // card nothing and a row's posters, titles and meta lines stay in line.
    // The caption steps by [`Self::TITLE_GAP`]; the poster's own artwork,
    // badge and marker are painted into one allocation, so the one stepped
    // space before them is the [`Self::POSTER_GAP`] gap to the title.
    let inner = ui.allocate_ui_with_layout(
        egui::vec2(width, height),
        egui::Layout::top_down(Align::Min),
        |ui| {
            ui.spacing_mut().item_spacing.y = TITLE_GAP;
            let (rect, _) = ui.allocate_exact_size(poster, Sense::hover());
            poster_artwork(ui, artwork, card, rect);
            match marker(card, mark_unwatched) {
                Marker::Count(count) => badge(ui, rect, &count.to_string()),
                Marker::Dot => mark(ui, rect),
                Marker::None => {}
            }
            ui.add_space(POSTER_GAP - TITLE_GAP);
            caption(ui, card, width)
        },
    );
    let (caption_top, elided) = inner.inner;
    let response = ui.interact(
        inner.response.rect,
        card_id(ui.id(), card),
        if clickable {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    if focused {
        response.request_focus();
    }
    cards::focus_ring(
        ui,
        inner.response.rect,
        &response,
        cards::CARD_RADIUS,
        focused,
    );
    if elided && (response.hovered() || response.has_focus()) {
        full_title(
            ui,
            card,
            egui::pos2(inner.response.rect.min.x, caption_top),
            width,
        );
    }
    let response = if clickable {
        cards::pointer(response)
    } else {
        response
    };
    response.clicked()
}

/// A title too long for its card's two lines, whole, on a glass plate laid
/// over the card's own caption while the card is pointed at or focused. It
/// covers only its own card's caption and grows downwards, so it never sits
/// on a neighbour's title the way a tooltip at the pointer would.
fn full_title(ui: &egui::Ui, card: &Card, top_left: egui::Pos2, width: f32) {
    let pad = egui::vec2(8.0, 6.0);
    let painter = ui
        .ctx()
        .layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            ui.id().with("full-title"),
        ))
        .with_clip_rect(ui.clip_rect());
    let galley = painter.layout(
        card.title.clone(),
        theme::card_title(),
        theme::palette().text,
        width,
    );
    let plate = Rect::from_min_size(top_left - pad, galley.size() + pad * 2.0)
        .with_max_x(top_left.x + width + pad.x);
    crate::ui::glass::panel(&painter, plate, 8.0, theme::current().glass.panel);
    painter.galley(top_left, galley, theme::palette().text);
}

fn poster_artwork(ui: &mut egui::Ui, artwork: &mut images::Artwork, card: &Card, rect: Rect) {
    ui.painter()
        .rect_filled(rect, theme::CARD_RADIUS, theme::palette().card);
    if let Some((id, tex)) = artwork.texture(
        card.thumb.as_deref(),
        rect.width() as u32,
        rect.height() as u32,
    ) {
        cards::rounded_art(
            ui,
            id,
            rect,
            cards::cover_uv_top(tex, rect.size()),
            theme::CARD_RADIUS,
            theme::UNTINTED,
        );
    }
}

/// The unwatched-episodes badge: an accent pill with the count in dark ink, in
/// the poster's top-right corner (a poster's own title art sits at its foot,
/// where a badge would read as part of the title). It widens for 3-digit counts.
fn badge(ui: &mut egui::Ui, rect: Rect, text: &str) {
    let font = theme::semibold(theme::CAPTION_SIZE);
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font, theme::palette().primary_text);
    let width = (galley.size().x + 12.0).max(BADGE[1]);
    let box_ = Rect::from_min_size(
        egui::pos2(rect.max.x - width - BADGE_INSET, rect.min.y + BADGE_INSET),
        egui::vec2(width, BADGE[1]),
    );
    ui.painter().rect_filled(
        box_.expand(1.5),
        CornerRadius::same(255),
        theme::palette().scrim(90),
    );
    ui.painter()
        .rect_filled(box_, CornerRadius::same(255), theme::palette().accent);
    ui.painter().galley(
        box_.center() - galley.size() * 0.5,
        galley,
        theme::palette().primary_text,
    );
}

/// A leaf marker: an accent dot ringed in dark, top-right like [`badge`].
fn mark(ui: &mut egui::Ui, rect: Rect) {
    let center = egui::pos2(
        rect.max.x - BADGE_INSET - MARK_RADIUS,
        rect.min.y + BADGE_INSET + MARK_RADIUS,
    );
    ui.painter()
        .circle_filled(center, MARK_RADIUS + 2.0, theme::palette().scrim(180));
    ui.painter()
        .circle_filled(center, MARK_RADIUS, theme::palette().accent);
}

/// Title and its year or episode word, each on its own line and stepped by
/// [`TITLE_GAP`]: the title in at most [`CAPTION_TITLE_ROWS`] lines that each
/// fit `width`, the meta line elided to `width` too. Nothing a caption draws
/// is ever wider than the poster above it. Answers where the title starts and
/// whether it had to be cut short.
fn caption(ui: &mut egui::Ui, card: &Card, width: f32) -> (f32, bool) {
    let font = theme::card_title();
    let (lines, elided) = title_lines(ui.painter(), &card.title, width, &font);
    let top = ui.cursor().min.y;
    ui.with_layout(egui::Layout::top_down(Align::Min), |ui| {
        ui.spacing_mut().item_spacing.y = TITLE_GAP;
        for line in lines {
            // Drawn in the font the lines were measured in, so none wraps.
            crate::ui::legible::label(ui, theme::title(line).font(font.clone()));
        }
        if !card.subtitle.is_empty() {
            let meta = rail::elide(ui.painter(), &card.subtitle, width, &theme::meta());
            crate::ui::legible::label(ui, theme::meta_text(meta));
        }
    });
    (top, elided)
}
