//! One shelf card: rounded artwork with a soft drop shadow, then two caption
//! lines. A wide card carries its show's title art, the service it comes from
//! and a resume track; a poster in a recently-added row carries a dot while
//! nothing of it has been watched.

use egui::epaint::Shadow;
use egui::text::LayoutJob;
use egui::{pos2, vec2, Align2, Color32, CornerRadius, Direction, Rect, Sense, Shape};
use plex_core::library::Item;

use super::{
    card_art, card_size, episode_caption, hero_logo, play_title, poster_subtitle,
    progress_fraction, resume_start, wide_subtitle, Resume, RowShape, CAPTION_HEIGHT,
    PROGRESS_HEIGHT,
};
use crate::source::Origin;
use crate::ui::{cards, images, marks, theme};

/// The shadow every card's art casts on the page.
const SHADOW: Shadow = Shadow {
    offset: [0, 10],
    blur: 24,
    spread: 0,
    color: Color32::from_rgba_premultiplied(0, 0, 0, 110),
};
/// The darkening under a wide card's title art.
const SHADE: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 178);
/// The resume track's unfilled part.
const TRACK: Color32 = Color32::from_rgba_premultiplied(51, 51, 51, 51);
/// The service chip's fill.
const CHIP: Color32 = Color32::from_rgba_premultiplied(5, 7, 19, 140);
/// The colour Jellyfin uses for itself, where this app ships no mark of its own.
const JELLYFIN_BLUE: Color32 = Color32::from_rgb(0x12, 0x99, 0xb8);
/// The title art's box on a wide card: a share of its width, and a height.
const LOGO_BOX: [f32; 2] = [0.46, 30.0];
/// A title art request's size.
const LOGO_ART: [u32; 2] = [360, 120];
/// The "new" dot's radius and its inset from the poster's top-right corner.
const DOT: [f32; 2] = [4.5, 12.5];

/// One card at its row's shape. Returns what a click asks the player to open,
/// and the interaction it answered so the row can page to a card the keyboard
/// focus landed on.
pub fn show(
    ui: &mut egui::Ui,
    artwork: &mut images::Artwork,
    item: &Item,
    shape: RowShape,
    id: egui::Id,
    fresh: bool,
    playable: bool,
) -> (Option<Resume>, egui::Response) {
    let ([width, height], [art_w, art_h]) = card_size(shape);
    let heading = item
        .grandparent_title
        .clone()
        .or_else(|| item.title.clone())
        .unwrap_or_else(|| "(untitled)".to_owned());
    let caption = match shape {
        RowShape::Poster => poster_subtitle(item),
        RowShape::Wide => wide_subtitle(item),
    };
    let mut art = Rect::NOTHING;
    let inner = ui.allocate_ui_with_layout(
        vec2(width, height + CAPTION_HEIGHT),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.spacing_mut().item_spacing.y = 9.0;
            let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
            art = rect;
            // Cards scrolled out of a row's view ask the loader for nothing.
            if ui.is_rect_visible(rect) {
                artwork_box(ui, artwork, item, shape, rect, (art_w, art_h));
                // A poster names its source only when that is not Plex, which is
                // what a card with no mark on it means.
                if crate::source::origin_of(item).is_jellyfin() {
                    let chip = Rect::from_min_size(rect.min + vec2(8.0, 8.0), vec2(18.0, 18.0));
                    ui.painter().rect_filled(chip, CornerRadius::same(5), CHIP);
                    ui.painter().text(
                        chip.center(),
                        Align2::CENTER_CENTER,
                        "J",
                        theme::semibold(10.0),
                        JELLYFIN_BLUE,
                    );
                }
                if fresh {
                    let center = pos2(rect.max.x - DOT[1], rect.min.y + DOT[1]);
                    ui.painter().circle_filled(
                        center,
                        DOT[0] + 3.0,
                        theme::ACCENT.gamma_multiply(0.25),
                    );
                    ui.painter().circle_filled(center, DOT[0], theme::ACCENT);
                }
            }
            caption_lines(ui, &heading, &caption, width);
        },
    );
    let sense = if playable {
        Sense::click()
    } else {
        Sense::hover()
    };
    let response = ui.interact(inner.response.rect, id, sense);
    // The ring is the art's, not the caption's: text a ring touches is worse
    // than no ring.
    cards::ring(ui, art, &response, cards::CARD_RADIUS);
    let tip = match shape {
        RowShape::Wide => format!("{heading}\n{}", episode_caption(item)),
        RowShape::Poster => format!("{heading}\n{caption}"),
    };
    let response = cards::pointer(response).on_hover_text(tip);
    let play = response.clicked().then(|| Resume {
        rating_key: item.rating_key.clone().unwrap_or_default(),
        title: play_title(item),
        start: resume_start(item),
        overview: false,
    });
    (play, response)
}

/// The art with its shadow, and on a wide card its overlays.
fn artwork_box(
    ui: &mut egui::Ui,
    artwork: &mut images::Artwork,
    item: &Item,
    shape: RowShape,
    rect: Rect,
    (art_w, art_h): (u32, u32),
) {
    ui.painter().add(SHADOW.as_shape(rect, cards::CARD_RADIUS));
    match artwork.texture(card_art(item, shape), art_w, art_h) {
        Some((texture, tex)) => cards::rounded_art(
            ui,
            texture,
            rect,
            cards::cover_uv(tex, rect.size()),
            cards::CARD_RADIUS,
            Color32::WHITE,
        ),
        None => {
            ui.painter()
                .rect_filled(rect, cards::CARD_RADIUS, theme::CARD);
        }
    }
    if shape == RowShape::Wide {
        wide_overlays(ui, artwork, item, rect);
    }
    ui.painter().rect_stroke(
        rect,
        cards::CARD_RADIUS,
        (1.0, Color32::from_white_alpha(20)),
        egui::StrokeKind::Inside,
    );
}

/// A wide card's shade, title art, service chip and resume track.
fn wide_overlays(ui: &mut egui::Ui, artwork: &mut images::Artwork, item: &Item, rect: Rect) {
    let painter = ui.painter();
    let shade = Rect::from_min_max(pos2(rect.min.x, rect.center().y - 8.0), rect.max);
    painter.add(Shape::gradient_rect(
        shade,
        Direction::BottomUp,
        [SHADE, Color32::TRANSPARENT],
    ));
    if let Some((texture, tex)) =
        hero_logo(item).and_then(|logo| artwork.texture(Some(logo), LOGO_ART[0], LOGO_ART[1]))
    {
        let size = cards::fit_size(tex, vec2(rect.width() * LOGO_BOX[0], LOGO_BOX[1]));
        let logo = Rect::from_min_size(pos2(rect.min.x + 12.0, rect.max.y - 14.0 - size.y), size);
        ui.painter()
            .image(texture, logo, cards::FULL_UV, Color32::WHITE);
    }
    let painter = ui.painter();
    let chip = Rect::from_min_size(
        rect.min + vec2(8.0, 8.0),
        vec2(marks::width_at(marks::PLEX, 9.0) + 16.0, 22.0),
    );
    painter.rect_filled(chip, CornerRadius::same(6), CHIP);
    match crate::source::origin_of(item) {
        Origin::Plex => {
            marks::paint_tinted(
                painter,
                marks::PLEX,
                chip.min.x + 8.0,
                chip.center().y,
                9.0,
                Color32::WHITE,
            );
        }
        // A source this app has no mark for is named by its letter, in the
        // colour it uses for itself.
        Origin::Jellyfin(_) => {
            painter.text(
                chip.center(),
                Align2::CENTER_CENTER,
                "J",
                theme::semibold(11.0),
                JELLYFIN_BLUE,
            );
        }
    }
    if let Some(fraction) = progress_fraction(item.view_offset, item.duration) {
        let track = Rect::from_min_max(pos2(rect.min.x, rect.max.y - PROGRESS_HEIGHT), rect.max);
        let bottom = CornerRadius {
            nw: 0,
            ne: 0,
            sw: cards::CARD_RADIUS.sw,
            se: cards::CARD_RADIUS.se,
        };
        painter.rect_filled(track, bottom, TRACK);
        let filled =
            Rect::from_min_size(track.min, vec2(track.width() * fraction, PROGRESS_HEIGHT));
        painter.rect_filled(filled, CornerRadius { se: 0, ..bottom }, theme::ACCENT);
    }
}

/// A card's two caption lines: title then subtitle, close together.
fn caption_lines(ui: &mut egui::Ui, heading: &str, subtitle: &str, width: f32) {
    ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
        ui.spacing_mut().item_spacing.y = 3.0;
        caption(ui, heading, theme::semibold(14.0), theme::TEXT, 1, width);
        caption(
            ui,
            subtitle,
            theme::medium(12.5),
            theme::SECONDARY,
            2,
            width,
        );
    });
}

/// One wrapped caption line inside a card's own width.
fn caption(
    ui: &mut egui::Ui,
    text: &str,
    font: egui::FontId,
    color: Color32,
    max_rows: usize,
    width: f32,
) {
    if text.is_empty() {
        return;
    }
    let mut job = LayoutJob::simple(text.to_owned(), font, color, width);
    job.wrap.max_rows = max_rows;
    // Only a one-row label may cut mid-word (it ends in an ellipsis); wrapped
    // text breaks between words.
    job.wrap.break_anywhere = max_rows == 1;
    ui.label(job);
}
