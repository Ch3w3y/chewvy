//! The detail page's drawing: the 16:9 backdrop band with its bottom gradient,
//! the `clearLogo` or title hanging from it, the Play / Resume pill, the version
//! badges, the overview, genre chips, the cast row, the season chips and the
//! episode rows.
//!
//! Every label comes from a label function in the parent module, so a field the
//! server did not name draws nothing here either. A click answers with one
//! [`Action`]: what to put in the player, or which season's episodes to read.

use std::sync::Arc;

use egui::text::LayoutJob;

use egui::{pos2, vec2, Color32, CornerRadius, Galley, Response, ScrollArea, Sense, Stroke, Ui};
use plex_core::library::Item;

use super::{
    cast, episode_meta, episode_pill_text, row_caption, season_label, seasons, selected_season,
    watched, Action, Page, Target, BAR_HEIGHT, CAST_COLUMN, CAST_THUMB, EPISODE_SUMMARY_ROWS,
    EPISODE_THUMB, PILL,
};
use crate::ui::{cards, chips, home, images, theme};

/// Every pill on the page — the play pill and each meta / badge / genre /
/// season chip — is drawn this tall and with [`chips::RADIUS`], so they read as
/// one shape family instead of a pill sitting on shorter tags.
const PILL_HEIGHT: f32 = PILL[1];
/// Width of one cast entry's column.
const CAST_ENTRY: f32 = CAST_COLUMN;
/// Cast rows shown before "Show all".
const CAST_ROWS: usize = 2;
/// Width of the text beside an episode row's thumbnail.
const EPISODE_TEXT_WIDTH: f32 = 360.0;
/// The gap between the lines of an episode row's text.
const EPISODE_TEXT_GAP: f32 = theme::SPACING_BASE;

/// Draw one detail page and answer with what the user clicked.
///
/// `playing` disables every play affordance: the window already has an item in
/// its player, and one click is one playback request. A season chip answers
/// with the season whose episodes to read, and takes the selection in place.
pub fn show(
    ui: &mut Ui,
    page: &mut Page,
    artwork: &mut images::Artwork,
    playing: bool,
    servers: crate::source::Servers<'_>,
) -> Option<Action> {
    let mut action = None;
    // The head scrolls with the page: a pinned one would cover the episodes a
    // narrow window has room for.
    ScrollArea::vertical()
        .auto_shrink([false; 2])
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
        .show(ui, |ui| {
            action = super::head::show(ui, page, artwork, playing, servers);
            // A show's season pills sit right above the episodes they pick, so
            // choosing a season unfolds its episodes under the pill.
            if let Some(key) = season_chips(ui, page) {
                action = Some(Action::Season { rating_key: key });
            }
            if let Some(target) = episode_rows(ui, page, artwork, playing) {
                action = Some(Action::Open {
                    rating_key: target.rating_key,
                    title: target.title,
                });
            }
            cast_row(ui, &page.item, artwork);
        });
    action
}

/// One measured chip: its label galley and its drawn width.
#[derive(Clone)]
struct Chip {
    galley: Arc<Galley>,
    width: f32,
}

/// Measure `labels` as chips in `font`: each one `PAD_X` of padding either side
/// of its own one-line galley, so [`paint_chips`] draws them at the widths the
/// labels actually need.
fn chips_row(ui: &Ui, labels: Vec<String>, font: egui::FontId) -> Vec<Chip> {
    labels
        .into_iter()
        .map(|label| {
            let galley = ui
                .painter()
                .layout_no_wrap(label, font.clone(), Color32::WHITE);
            let width = galley.size().x + 2.0 * chips::PAD_X;
            Chip { galley, width }
        })
        .collect()
}

/// Draw one measured chip as a frosted plate of [`PILL_HEIGHT`] at `box_rect`,
/// its label centred in it.
fn paint_chip(ui: &Ui, box_rect: egui::Rect, galley: &Arc<Galley>) {
    crate::ui::glass::panel(
        ui.painter(),
        box_rect,
        f32::from(theme::CARD_RADIUS_U8),
        crate::ui::glass_tints::PANEL,
    );
    ui.painter().rect_stroke(
        box_rect,
        chips::RADIUS,
        Stroke::new(theme::HAIRLINE, chips::OUTLINE),
        egui::StrokeKind::Middle,
    );
    ui.painter().galley(
        box_rect.center() - galley.size() * 0.5,
        galley.clone(),
        theme::SECONDARY,
    );
}

/// Paint measured chips from the cursor as rows that fit the width available
/// here: a chip that would run past the right edge starts a new row instead of
/// leaving the page. Answers with each chip's box in order and the height those
/// rows take, so a caller can put its own click targets on them.
fn chips_block(ui: &mut Ui, measured: &[Chip]) -> (Vec<egui::Rect>, f32) {
    if measured.is_empty() {
        return (Vec::new(), 0.0);
    }
    let origin = ui.cursor().min;
    let mut boxes: Vec<egui::Rect> = Vec::with_capacity(measured.len());
    let mut top = origin.y;
    for row in &chip_rows(measured, ui.available_width()) {
        let mut x = origin.x;
        for chip in row {
            let box_rect = egui::Rect::from_min_size(pos2(x, top), vec2(chip.width, PILL_HEIGHT));
            paint_chip(ui, box_rect, &chip.galley);
            x += chip.width + chips::GAP;
            boxes.push(box_rect);
        }
        top += PILL_HEIGHT + chips::GAP;
    }
    let height = boxes.iter().map(|rect| rect.max.y).fold(origin.y, f32::max) - origin.y;
    (boxes, height)
}

/// Split measured chips into rows that fit `width`: a chip that would run past
/// the right edge starts a new row instead of leaving the page.
fn chip_rows(measured: &[Chip], width: f32) -> Vec<Vec<Chip>> {
    let mut rows: Vec<Vec<Chip>> = Vec::new();
    for chip in measured {
        let wraps = rows
            .last()
            .is_some_and(|row| row_width(row) + chips::GAP + chip.width > width);
        if wraps || rows.is_empty() {
            rows.push(Vec::new());
        }
        if let Some(row) = rows.last_mut() {
            row.push(chip.clone());
        }
    }
    rows
}

/// The width a chip row's labels and the gaps between them take.
fn row_width(row: &[Chip]) -> f32 {
    row.iter().map(|chip| chip.width).sum::<f32>() + chips::GAP * (row.len() - 1) as f32
}

/// The `Role[]` row: one [`CAST_THUMB`] plate of the person's own initials over
/// their name and the role the answer credited them with.
///
/// The captured answers name cast thumbs as absolute `https://metadata-static
/// .plex.tv/…` URLs, which [`images`] does not request (it only asks for the
/// server-named `/library/…` paths through `/photo/:/transcode`), so the plate
/// is drawn here from the name instead of fetched.
fn cast_row(ui: &mut Ui, item: &Item, artwork: &mut images::Artwork) {
    let people = cast(item);
    if people.is_empty() {
        return;
    }
    section_heading(ui, "Cast");
    // The cast wraps into rows across the page instead of scrolling sideways.
    let width = ui.available_width();
    let columns = ((width + theme::GAP) / (CAST_ENTRY + theme::GAP))
        .floor()
        .max(1.0) as usize;
    // Each column stretches so the rows fill the page width exactly.
    let cell = ((width + theme::GAP) / columns as f32 - theme::GAP).max(CAST_ENTRY);
    // Two rows until asked for the whole credit list.
    let expanded_id = ui.id().with("cast-expanded");
    let expanded = ui
        .data(|d| d.get_temp::<bool>(expanded_id))
        .unwrap_or(false);
    let limit = if expanded {
        people.len()
    } else {
        (columns * CAST_ROWS).min(people.len())
    };
    let shown = people.get(..limit).unwrap_or(&people);
    for row in shown.chunks(columns) {
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
            // The column count above already holds the gaps.
            ui.spacing_mut().item_spacing.x = 0.0;
            for (index, person) in row.iter().enumerate() {
                if index > 0 {
                    ui.add_space(theme::GAP);
                }
                let entry = ui.vertical(|ui| {
                    ui.set_width(cell);
                    cast_photo(ui, &person.name, person.thumb.as_deref(), artwork);
                    ui.add_space(theme::SPACING_BASE * 2.0);
                    let name = ui.painter().layout(
                        person.name.clone(),
                        theme::card_title(),
                        theme::TEXT,
                        cell,
                    );
                    let role = person.role.as_deref().map(|role| {
                        ui.painter()
                            .layout(role.to_owned(), theme::meta(), theme::SECONDARY, cell)
                    });
                    let height = name.size().y + role.as_ref().map_or(0.0, |g| g.size().y + 2.0);
                    let (box_rect, _) = ui.allocate_at_least(vec2(cell, height), Sense::hover());
                    ui.painter().galley(box_rect.min, name.clone(), theme::TEXT);
                    if let Some(galley) = role {
                        ui.painter().galley(
                            pos2(box_rect.min.x, box_rect.min.y + name.size().y + 2.0),
                            galley,
                            theme::SECONDARY,
                        );
                    }
                });
                cast_link(ui, &person.name, entry.response.rect);
            }
        });
        ui.add_space(theme::GAP * 1.5);
    }
    if people.len() > columns * CAST_ROWS {
        let label = if expanded {
            "Show less".to_owned()
        } else {
            format!("Show all {} cast and crew", people.len())
        };
        let press =
            ui.add(egui::Button::new(theme::strong(label).color(theme::ACCENT)).frame(false));
        if crate::ui::cards::pointer(press).clicked() {
            ui.data_mut(|d| d.insert_temp(expanded_id, !expanded));
        }
        ui.add_space(theme::GAP);
    }
}

/// A cast entry is a link: hovering rings the photo, a click opens the
/// person's IMDb name search in the default browser (only the name is sent).
fn cast_link(ui: &mut Ui, name: &str, rect: egui::Rect) {
    let press = ui.interact(rect, ui.id().with(("cast-link", name)), Sense::click());
    let press = crate::ui::cards::pointer(press);
    if press.hovered() {
        let center = pos2(rect.min.x + CAST_THUMB / 2.0, rect.min.y + CAST_THUMB / 2.0);
        ui.painter().circle_stroke(
            center,
            CAST_THUMB / 2.0 + 2.0,
            egui::Stroke::new(2.0, theme::ACCENT),
        );
    }
    if press.clicked() {
        ui.ctx()
            .open_url(egui::OpenUrl::new_tab(super::imdb_search(name)));
    }
}

/// One cast plate: the person's photo in a [`CAST_THUMB`] circle, or their
/// initials while it loads or when the server has none.
fn cast_photo(ui: &mut Ui, name: &str, thumb: Option<&str>, artwork: &mut images::Artwork) {
    let radius = CAST_THUMB / 2.0;
    let size = vec2(CAST_THUMB, CAST_THUMB);
    let (box_rect, _) = ui.allocate_at_least(size, Sense::hover());
    let plate = egui::Rect::from_min_size(box_rect.min, size);
    // The server fetches the photo; a texture is asked only once its circle is
    // on screen, and the initials stand in until it arrives.
    let photo = ui
        .is_rect_visible(plate)
        .then(|| artwork.texture(thumb, CAST_THUMB as u32, CAST_THUMB as u32))
        .flatten();
    match photo {
        // A rounded rect whose radius is half its side is a disc.
        Some((id, tex)) => cards::rounded_art(
            ui,
            id,
            plate,
            cards::cover_uv(tex, size),
            CornerRadius::same(radius as u8),
            Color32::WHITE,
        ),
        None => initials(ui, name, plate),
    }
}

/// A cast member's initials on an accent disc filling `plate`.
fn initials(ui: &Ui, name: &str, plate: egui::Rect) {
    let letters: String = name
        .split_whitespace()
        .take(2)
        .filter_map(|word| word.chars().next())
        .flat_map(char::to_uppercase)
        .collect();
    let center = plate.center();
    ui.painter()
        .circle_filled(center, plate.width() / 2.0, theme::ACCENT_TINT);
    let galley = ui
        .painter()
        .layout_no_wrap(letters, theme::h2(), theme::ACCENT);
    ui.painter()
        .galley(center - galley.size() * 0.5, galley, theme::ACCENT);
}

/// The season chips of a show page, clickable while the page has seasons: a
/// click selects that season in place and answers with the rating key whose
/// episodes to read.
fn season_chips(ui: &mut Ui, page: &mut Page) -> Option<String> {
    let current = selected_season(&*page).and_then(|season| season.rating_key.clone());
    let listed: Vec<(String, String, bool)> = seasons(&page.seasons)
        .into_iter()
        .filter_map(|season| {
            let key = season.rating_key.clone()?;
            Some((
                key.clone(),
                season_label(season),
                season.rating_key == current,
            ))
        })
        .collect();
    if listed.is_empty() {
        return None;
    }
    section_heading(ui, "Seasons");
    let measured = chips_row(
        ui,
        listed.iter().map(|(_, label, _)| label.clone()).collect(),
        theme::body(),
    );
    let (boxes, height) = chips_block(ui, &measured);
    let mut asked = None;
    for ((key, _, active), box_rect) in listed.iter().zip(&boxes) {
        let press = ui.interact(
            *box_rect,
            ui.id().with(("season", key)),
            if *active {
                Sense::hover()
            } else {
                Sense::click()
            },
        );
        let press = crate::ui::cards::pointer(press);
        season_state(
            ui,
            *box_rect,
            measured_label(&measured, box_rect, &boxes),
            *active,
            &press,
        );
        if press.clicked() {
            page.select_season(key);
            asked = Some(key.clone());
        }
    }
    ui.add_space(height + theme::SPACING_BASE * 2.0);
    let progress = selected_season(&*page).and_then(super::season_progress);
    if let Some(progress) = progress {
        ui.label(theme::secondary(progress).font(theme::meta()));
    }
    ui.add_space(theme::GAP);
    asked
}

/// The galley of the chip drawn in `box_rect`.
fn measured_label<'a>(
    measured: &'a [Chip],
    box_rect: &egui::Rect,
    boxes: &[egui::Rect],
) -> Option<&'a Arc<Galley>> {
    let index = boxes.iter().position(|b| b == box_rect)?;
    measured.get(index).map(|chip| &chip.galley)
}

/// Repaint a season pill for its state: the selected season in the accent, a
/// hovered one lifted, a pressed one darkened, so a click is felt at once.
fn season_state(
    ui: &Ui,
    box_rect: egui::Rect,
    galley: Option<&Arc<Galley>>,
    active: bool,
    press: &Response,
) {
    let (fill, stroke, ink) = if active {
        (theme::ACCENT_TINT, theme::ACCENT, theme::TEXT)
    } else if press.is_pointer_button_down_on() {
        (theme::SURFACE, chips::OUTLINE, theme::TEXT)
    } else if press.hovered() {
        (theme::CARD_HOVER, theme::SECONDARY, theme::TEXT)
    } else {
        return;
    };
    let painter = ui.painter();
    painter.rect_filled(box_rect, chips::RADIUS, fill);
    painter.rect_stroke(
        box_rect,
        chips::RADIUS,
        Stroke::new(1.5, stroke),
        egui::StrokeKind::Middle,
    );
    if let Some(galley) = galley {
        painter.galley(box_rect.center() - galley.size() * 0.5, galley.clone(), ink);
    }
}

/// While a newly chosen season's episodes load: placeholder rows that pulse,
/// so the click visibly did something.
fn loading_rows(ui: &mut Ui) {
    let time = ui.input(|i| i.time) as f32;
    let pulse = 0.5 + 0.5 * (time * 3.0).sin();
    let fill = theme::CARD.lerp_to_gamma(theme::CARD_HOVER, pulse);
    for _ in 0..3 {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 96.0), Sense::hover());
        let thumb = egui::Rect::from_min_size(rect.min, vec2(170.0, 96.0));
        ui.painter().rect_filled(thumb, theme::CARD_RADIUS, fill);
        for (row, width) in [(0.0, 260.0), (28.0, 420.0), (50.0, 360.0)] {
            let line = egui::Rect::from_min_size(
                pos2(thumb.max.x + theme::GAP, rect.min.y + 8.0 + row),
                vec2(width, 14.0),
            );
            ui.painter().rect_filled(line, 7.0, fill);
        }
        ui.add_space(theme::GAP);
    }
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_millis(33));
}

/// The episodes of the season in view, one row each; a row's click opens that
/// episode's overview, where Play / Resume starts it.
fn episode_rows(
    ui: &mut Ui,
    page: &Page,
    artwork: &mut images::Artwork,
    playing: bool,
) -> Option<Target> {
    let mut asked = None;
    if page.episodes.is_empty() && !page.seasons.is_empty() && page.season_key.is_some() {
        loading_rows(ui);
        return None;
    }
    for episode in &page.episodes {
        if episode_row(ui, episode, artwork, playing) {
            asked = Some(Target {
                rating_key: episode.rating_key.clone()?,
                title: super::play_title(episode),
                start: home::resume_start(episode),
            });
        }
        ui.add_space(theme::GAP);
    }
    asked
}

/// An episode row's own text, measured before the row is placed.
struct EpisodeText {
    /// `E{index}` · title.
    heading: Arc<Galley>,
    /// The answer's own `summary`, cut to [`EPISODE_SUMMARY_ROWS`] lines.
    summary: Option<Arc<Galley>>,
    /// The runtime, plus the resume line the server's own `viewOffset` implies.
    meta: Option<Arc<Galley>>,
}

impl EpisodeText {
    /// The height the three lines take, gaps between them included.
    fn height(&self) -> f32 {
        self.heading.size().y
            + self
                .summary
                .as_ref()
                .map_or(0.0, |galley| EPISODE_TEXT_GAP + galley.size().y)
            + self
                .meta
                .as_ref()
                .map_or(0.0, |galley| EPISODE_TEXT_GAP + galley.size().y)
    }
}

/// Measure an episode row's text: its `E{index}` · title, its summary cut to
/// [`EPISODE_SUMMARY_ROWS`] lines, and a meta line holding its runtime and, only
/// where the answer named a resume point, the `Resume · N min left` the page's
/// own pill would say. An unwatched row says nothing its click does not already
/// say, so its meta line is its runtime alone or nothing at all.
fn episode_text(ui: &mut Ui, episode: &Item) -> EpisodeText {
    let caption = row_caption(episode);
    let heading = ui.painter().layout(
        caption,
        theme::card_title(),
        theme::TEXT,
        EPISODE_TEXT_WIDTH,
    );
    let summary = episode
        .summary
        .as_deref()
        .map(str::trim)
        .filter(|summary| !summary.is_empty())
        .map(|summary| {
            let mut job = LayoutJob::simple(
                summary.to_owned(),
                theme::body(),
                theme::SECONDARY,
                EPISODE_TEXT_WIDTH,
            );
            job.wrap.max_rows = EPISODE_SUMMARY_ROWS;
            ui.painter().layout_job(job)
        });
    let resume = home::resume_start(episode).map(|_| episode_pill_text(episode));
    let meta = episode_meta(episode);
    let meta_text = [(!meta.is_empty()).then_some(meta), resume]
        .into_iter()
        .flatten()
        .collect::<Vec<String>>()
        .join("  ·  ");
    let meta_line = (!meta_text.is_empty()).then(|| {
        ui.painter().layout(
            meta_text,
            theme::meta(),
            theme::SECONDARY,
            EPISODE_TEXT_WIDTH,
        )
    });
    EpisodeText {
        heading,
        summary,
        meta: meta_line,
    }
}

/// One episode row: its 16:9 [`EPISODE_THUMB`] with the resume bar along its
/// bottom edge and the watched tick over it, then [`EpisodeText`] beside it.
/// The whole row is the click target, so its text belongs to the row rather
/// than being a second control.
pub(super) fn episode_row(
    ui: &mut Ui,
    episode: &Item,
    artwork: &mut images::Artwork,
    playing: bool,
) -> bool {
    let text = episode_text(ui, episode);
    let row_size = vec2(
        EPISODE_THUMB[0] + theme::GAP + EPISODE_TEXT_WIDTH,
        EPISODE_THUMB[1].max(text.height()),
    );
    let (rect, press) = ui.allocate_exact_size(
        row_size,
        if playing {
            Sense::hover()
        } else {
            Sense::click()
        },
    );
    let thumb = egui::Rect::from_min_size(rect.min, vec2(EPISODE_THUMB[0], EPISODE_THUMB[1]));
    match artwork.texture(
        episode.thumb.as_deref(),
        EPISODE_THUMB[0] as u32,
        EPISODE_THUMB[1] as u32,
    ) {
        Some((id, tex)) => {
            cards::rounded_art(
                ui,
                id,
                thumb,
                cards::cover_uv(tex, thumb.size()),
                cards::CARD_RADIUS,
                Color32::WHITE,
            );
        }
        None => {
            ui.painter()
                .rect_filled(thumb, cards::CARD_RADIUS, theme::CARD);
        }
    }
    if let Some(fraction) = home::progress_fraction(episode.view_offset, episode.duration) {
        ui.painter().rect_filled(
            egui::Rect::from_min_size(
                thumb.left_bottom() - vec2(0.0, BAR_HEIGHT),
                vec2(thumb.width() * fraction, BAR_HEIGHT),
            ),
            CornerRadius {
                nw: 0,
                ne: 0,
                sw: cards::CARD_RADIUS.sw,
                se: 0,
            },
            theme::ACCENT,
        );
    }
    if watched(episode) {
        tick(ui, thumb.right_top() + vec2(-14.0, 14.0));
    }
    let x = thumb.max.x + theme::GAP;
    let mut y = rect.min.y + (row_size.y - text.height()) * 0.5;
    ui.painter()
        .galley(pos2(x, y), text.heading.clone(), theme::TEXT);
    y += text.heading.size().y;
    for galley in text.summary.into_iter().chain(text.meta) {
        y += EPISODE_TEXT_GAP;
        ui.painter()
            .galley(pos2(x, y), galley.clone(), theme::SECONDARY);
        y += galley.size().y;
    }
    if !playing {
        press
            .clone()
            .on_hover_cursor(egui::CursorIcon::PointingHand);
    }
    press.clicked()
}

/// The watched tick: the three arms [`chips::check_points`] draws, as a filled
/// circle plate over an episode thumbnail's corner.
fn tick(ui: &Ui, center: egui::Pos2) {
    ui.painter()
        .circle_filled(center, 11.0, egui::Color32::from_black_alpha(200));
    chips::check(ui, center);
}

/// A body section's own heading, in the theme's `h2`.
fn section_heading(ui: &mut Ui, text: &str) {
    ui.label(theme::strong(text).size(theme::H2_SIZE));
    ui.add_space(theme::SPACING_BASE);
}
