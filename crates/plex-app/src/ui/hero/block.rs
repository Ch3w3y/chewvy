//! The hero's text block, hung from the bottom of the hero: the title art (or
//! the title), the meta row with its rating and format marks, the episode line,
//! the summary, the Resume and Details buttons, then which server it is from.
//!
//! Every part is measured before anything is drawn, so the stack can be hung
//! from the bottom edge and a tall logo pushes the block up instead of into the
//! shelves below.

use egui::text::LayoutJob;
use egui::{pos2, Color32, Id, Painter, Pos2, Rect};
use plex_core::library::Item;

use crate::source::Origin;
use crate::ui::overlay::{self, Heading, Progress, BUTTON, META_HEIGHT};
use crate::ui::{detail, home, images, marks, theme};

/// The widest the block reads; the summary wraps inside it.
const WIDTH: f32 = 600.0;
/// The summary's measure, narrower than the block so it reads as prose.
const SUMMARY_WIDTH: f32 = 520.0;
/// Rows the summary may use before it is elided.
const SUMMARY_ROWS: usize = 3;
/// The box a `clearLogo` is fitted into.
const LOGO_BOX: [f32; 2] = [400.0, 140.0];
/// The block's distance from the hero's bottom edge.
const BOTTOM: f32 = 36.0;
/// The gap between the block's rows.
const ROW_GAP: f32 = 16.0;

const SOURCE_TEXT: Color32 = Color32::from_rgb(0xc4, 0xc8, 0xe4);

/// What the block's buttons answered this frame.
pub struct Pressed {
    pub play: bool,
    pub details: bool,
}

/// Draw the block for `item` inside `hero`, hung [`BOTTOM`] above its bottom.
pub fn show(
    ui: &mut egui::Ui,
    hero: Rect,
    id: Id,
    item: &Item,
    artwork: &mut images::Artwork,
    playable: bool,
    source: (Origin, &str),
) -> Pressed {
    let painter = ui.painter().clone();
    let left = hero.min.x;
    let width = WIDTH.min(hero.width());
    let heading = Heading::new(
        &painter,
        item,
        home::hero_logo(item),
        artwork,
        LOGO_BOX,
        width,
    );
    let meta = painter.layout_no_wrap(meta_text(item), theme::medium(14.0), overlay::META_TEXT);
    let episode = home::hero_episode_line(item)
        .map(|line| painter.layout(line, theme::semibold(15.0), theme::ACCENT, width));
    let summary = item
        .summary
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(|summary| {
            let mut job = LayoutJob::simple(
                summary.to_owned(),
                theme::medium(15.5),
                overlay::BODY_TEXT,
                SUMMARY_WIDTH.min(width),
            );
            job.wrap.max_rows = SUMMARY_ROWS;
            painter.layout_job(job)
        });

    let rows = [
        heading.height(),
        META_HEIGHT,
        episode.as_ref().map_or(0.0, |g| g.rect.height()),
        summary.as_ref().map_or(0.0, |g| g.rect.height()),
        BUTTON[0],
        14.0,
    ];
    let present = rows.iter().filter(|h| **h > 0.0).count() as f32;
    let block = rows.iter().sum::<f32>() + ROW_GAP * (present - 1.0) + 6.0;
    let mut top = (hero.max.y - BOTTOM - block).max(hero.min.y + theme::PAGE_TOP);

    let heading_height = heading.height();
    heading.paint(&painter, pos2(left, top));
    top += heading_height + ROW_GAP;
    top += overlay::meta_row(&painter, pos2(left, top), width, meta, item) + ROW_GAP;
    for galley in [episode, summary].into_iter().flatten() {
        let height = galley.rect.height();
        overlay::shadowed(&painter, pos2(left, top), galley);
        top += height + ROW_GAP;
    }
    top += 6.0;
    let pressed = buttons(ui, pos2(left, top), id, item, playable);
    top += BUTTON[0] + ROW_GAP - 4.0;
    source_line(&painter, pos2(left, top), &source.0, source.1);
    pressed
}

/// `Movie · 2024 · 2h 47m · Science Fiction`: what the answer names, in order.
#[must_use]
pub fn meta_text(item: &Item) -> String {
    let mut parts: Vec<String> = Vec::new();
    parts.extend(home::type_label(item).map(str::to_owned));
    parts.extend(item.year.map(|year| year.to_string()));
    if matches!(item.kind.as_deref(), Some("movie") | Some("episode")) {
        parts.extend(detail::runtime(item.duration));
    }
    parts.extend(item.genres.first().cloned());
    parts.join("  ·  ")
}

/// Resume (or Play) and Details, side by side from `origin`.
fn buttons(ui: &mut egui::Ui, origin: Pos2, id: Id, item: &Item, playable: bool) -> Pressed {
    let start = home::resume_start(item);
    let progress = start
        .and(home::progress_fraction(item.view_offset, item.duration))
        .zip(home::minutes_left(item.view_offset, item.duration))
        .map(|(fraction, minutes)| Progress {
            fraction,
            left: home::time_left(minutes),
        });
    let play = overlay::primary(
        ui,
        origin,
        id.with(("hero-play", item.rating_key.as_deref())),
        home::resume_label(start),
        progress,
        playable,
    );
    let details = overlay::secondary(
        ui,
        pos2(play.rect.max.x + 12.0, origin.y),
        id.with("hero-details"),
        Some(marks::icon::INFO),
        "Details",
        playable,
    );
    Pressed {
        play: playable && play.clicked(),
        details: playable && details.clicked(),
    }
}

/// `From [Plex] Living Room NAS`, with the mark of the service the item is on.
fn source_line(painter: &Painter, origin: Pos2, service: &Origin, server: &str) {
    if server.is_empty() {
        return;
    }
    let font = theme::medium(12.5);
    let cy = origin.y + 7.0;
    let from = painter.layout_no_wrap("From".to_owned(), font.clone(), SOURCE_TEXT);
    let mut x = origin.x + from.rect.width() + 8.0;
    overlay::shadowed(painter, pos2(origin.x, cy - from.rect.height() / 2.0), from);
    x += marks::paint_source(painter, service, x, cy, 10.0) + 8.0;
    let name = painter.layout_no_wrap(server.to_owned(), font, SOURCE_TEXT);
    overlay::shadowed(painter, pos2(x, cy - name.rect.height() / 2.0), name);
}
