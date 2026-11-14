//! One shelf card: rounded artwork with a soft drop shadow, then two caption
//! lines. A wide card carries its show's title art, the service it comes from
//! and a resume track; a poster in a recently-added row carries a dot while
//! nothing of it has been watched. The play button over the art and the two
//! caption lines are separate targets (play, the show, the episode).

use egui::epaint::Shadow;
use egui::text::LayoutJob;
use egui::{pos2, vec2, Color32, CornerRadius, Direction, Rect, Sense, Shape};
use plex_core::library::Item;

use super::{
    card_art, card_size, episode_caption, episode_page, hero_logo, item_page, play_title,
    poster_subtitle, progress_fraction, resume_label, resume_start, title_page, wide_subtitle,
    Page, Pick, RowShape, CAPTION_HEIGHT, PROGRESS_HEIGHT,
};
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
/// The Plex wordmark's height on a card's service chip.
const MARK: f32 = 9.0;
/// The title art's box on a wide card: a share of its width, and a height.
const LOGO_BOX: [f32; 2] = [0.46, 30.0];
/// A title art request's size.
const LOGO_ART: [u32; 2] = [360, 120];
/// The "new" dot's radius and its inset from the poster's top-right corner.
const DOT: [f32; 2] = [4.5, 12.5];

/// How one card behaves, besides what it shows.
#[derive(Clone, Copy)]
pub struct Flags {
    /// A poster in a recently-added row that nothing of has been watched.
    pub fresh: bool,
    /// Nothing else is playing, so the card answers clicks at all.
    pub playable: bool,
    /// A click on the art plays the item (continue watching, on deck) rather
    /// than opening its page.
    pub direct: bool,
}

/// The play button's diameter over a card's art.
const PLAY: f32 = 46.0;

/// One card at its row's shape. Each part answers for itself: the play button
/// over the art plays, the title opens its page (an episode's show), the
/// episode line opens the episode, and the rest of the art plays or opens the
/// item as the row decides. Also answers the art's interaction, so the row can
/// page to a card the keyboard focus landed on.
pub fn show(
    ui: &mut egui::Ui,
    artwork: &mut images::Artwork,
    item: &Item,
    shape: RowShape,
    id: egui::Id,
    flags: Flags,
) -> (Option<Pick>, egui::Response) {
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
    let links = Links {
        title: title_page(item).filter(|_| flags.playable),
        episode: episode_page(item).filter(|_| flags.playable),
    };
    let mut art = Rect::NOTHING;
    let mut opened = None;
    ui.allocate_ui_with_layout(
        vec2(width, height + CAPTION_HEIGHT),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.spacing_mut().item_spacing.y = 9.0;
            let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
            art = rect;
            // Cards scrolled out of a row's view ask the loader for nothing.
            if ui.is_rect_visible(rect) {
                artwork_box(ui, artwork, item, shape, rect, (art_w, art_h));
                badges(ui, item, rect, flags.fresh);
            }
            opened = caption_lines(ui, id, &heading, &caption, width, &links);
        },
    );
    let sense = if flags.playable {
        Sense::click()
    } else {
        Sense::hover()
    };
    let tip = match shape {
        RowShape::Wide => format!("{heading}\n{}", episode_caption(item)),
        RowShape::Poster => format!("{heading}\n{caption}"),
    };
    let response = cards::pointer(ui.interact(art, id, sense)).on_hover_text(tip);
    let play = (flags.playable && ui.rect_contains_pointer(art)) || response.has_focus();
    let play_button = play.then(|| play_button(ui, art, id, item));
    // The ring is the art's, not the caption's: text a ring touches is worse
    // than no ring. It stays while the pointer is on the play button too.
    let ringed = match &play_button {
        Some(button) => response.union(button.clone()),
        None => response.clone(),
    };
    cards::ring(ui, art, &ringed, cards::CARD_RADIUS);
    let pick = if play_button.is_some_and(|b| b.clicked()) {
        Some(Pick::play(item))
    } else if let Some(page) = opened {
        Some(Pick::Open(page))
    } else if response.clicked() {
        if flags.direct {
            Some(Pick::play(item))
        } else {
            item_page(item).map(Pick::Open)
        }
    } else {
        None
    };
    (pick, response)
}

/// The pages a card's two caption lines open, where they open one.
struct Links {
    title: Option<Page>,
    episode: Option<Page>,
}

/// The source chip on a poster that is not from Plex, and the "new" dot.
fn badges(ui: &egui::Ui, item: &Item, rect: Rect, fresh: bool) {
    // A poster names its source only when that is not Plex, which is what a
    // card with no mark on it means.
    let origin = crate::source::origin_of(item);
    if origin.is_jellyfin() {
        let chip = Rect::from_min_size(rect.min + vec2(8.0, 8.0), vec2(22.0, 22.0));
        ui.painter().rect_filled(chip, CornerRadius::same(6), CHIP);
        marks::paint_source(
            ui.painter(),
            &origin,
            chip.min.x + 4.0,
            chip.center().y,
            MARK,
        );
    }
    if fresh {
        let center = pos2(rect.max.x - DOT[1], rect.min.y + DOT[1]);
        ui.painter()
            .circle_filled(center, DOT[0] + 3.0, theme::ACCENT.gamma_multiply(0.25));
        ui.painter().circle_filled(center, DOT[0], theme::ACCENT);
    }
}

/// The round play button in the middle of the art, drawn while the card is
/// pointed at or focused.
fn play_button(ui: &mut egui::Ui, art: Rect, id: egui::Id, item: &Item) -> egui::Response {
    let rect = Rect::from_center_size(art.center(), vec2(PLAY, PLAY));
    let hint = format!("{} {}", resume_label(resume_start(item)), play_title(item));
    let response =
        cards::pointer(ui.interact(rect, id.with("play"), Sense::click())).on_hover_text(hint);
    let fill = if response.hovered() {
        theme::PRIMARY_BG
    } else {
        Color32::from_black_alpha(160)
    };
    let ink = if response.hovered() {
        theme::PRIMARY_TEXT
    } else {
        Color32::WHITE
    };
    let painter = ui.painter();
    painter.circle_filled(rect.center(), PLAY / 2.0, fill);
    painter.circle_stroke(
        rect.center(),
        PLAY / 2.0,
        egui::Stroke::new(1.0, Color32::from_white_alpha(70)),
    );
    let icon = PLAY * 0.42;
    let glyph = marks::width_at(marks::icon::PLAY, icon);
    // The triangle's weight sits left of its box, so it is nudged right.
    let left = rect.center().x - glyph / 2.0 + 1.5;
    marks::paint_tinted(painter, marks::icon::PLAY, left, rect.center().y, icon, ink);
    response
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
    let origin = crate::source::origin_of(item);
    // The square icon sits closer to the chip's edge than the wordmark does.
    let pad = if origin.is_jellyfin() { 4.0 } else { 8.0 };
    let chip = Rect::from_min_size(
        rect.min + vec2(8.0, 8.0),
        vec2(marks::source_width(&origin, MARK) + 2.0 * pad, 22.0),
    );
    painter.rect_filled(chip, CornerRadius::same(6), CHIP);
    marks::paint_source(painter, &origin, chip.min.x + pad, chip.center().y, MARK);
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

/// A card's two caption lines: title then subtitle, close together. Each is a
/// link where [`Links`] names its page; answers the page clicked.
fn caption_lines(
    ui: &mut egui::Ui,
    id: egui::Id,
    heading: &str,
    subtitle: &str,
    width: f32,
    links: &Links,
) -> Option<Page> {
    ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
        ui.spacing_mut().item_spacing.y = 3.0;
        let title = Line {
            text: heading,
            font: theme::semibold(14.0),
            color: theme::TEXT,
            rows: 1,
            width,
        };
        let sub = Line {
            text: subtitle,
            font: theme::medium(12.5),
            color: theme::SECONDARY,
            rows: 2,
            width,
        };
        let a = caption(ui, id.with("title"), title, links.title.as_ref());
        let b = caption(ui, id.with("episode"), sub, links.episode.as_ref());
        a.or(b)
    })
    .inner
}

/// One caption line's text and how it is set.
struct Line<'a> {
    text: &'a str,
    font: egui::FontId,
    color: Color32,
    rows: usize,
    width: f32,
}

/// One wrapped caption line inside a card's own width; with a `page`, it is
/// a link to it, underlined while pointed at. Answers the page when clicked.
fn caption(ui: &mut egui::Ui, id: egui::Id, line: Line<'_>, page: Option<&Page>) -> Option<Page> {
    if line.text.is_empty() {
        return None;
    }
    let job = |underline: bool| {
        let mut job = LayoutJob::simple(
            line.text.to_owned(),
            line.font.clone(),
            line.color,
            line.width,
        );
        job.wrap.max_rows = line.rows;
        // Only a one-row label may cut mid-word (it ends in an ellipsis);
        // wrapped text breaks between words.
        job.wrap.break_anywhere = line.rows == 1;
        if underline {
            for section in &mut job.sections {
                section.format.underline = egui::Stroke::new(1.0, line.color);
            }
        }
        job
    };
    let galley = ui.painter().layout_job(job(false));
    let Some(page) = page else {
        ui.label(galley);
        return None;
    };
    let (rect, _) = ui.allocate_exact_size(galley.size(), Sense::hover());
    let response = cards::pointer(ui.interact(rect, id, Sense::click()))
        .on_hover_text(format!("Go to {}", page.title));
    let galley = if response.hovered() {
        ui.painter().layout_job(job(true))
    } else {
        galley
    };
    ui.painter().galley(rect.min, galley, line.color);
    response.clicked().then(|| page.clone())
}
