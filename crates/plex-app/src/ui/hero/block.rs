//! The hero's text block, hung from the bottom of the hero: the title art (or
//! the title), the meta row with its rating and format marks, the episode line,
//! the summary, the Resume and Details buttons, then which server it is from.
//! The title opens its page (an episode's show) and the episode line opens the
//! episode, so only Resume plays.
//!
//! Every part is measured before anything is drawn, so the stack can be hung
//! from the bottom edge and a tall logo pushes the block up instead of into the
//! shelves below.

use egui::text::LayoutJob;
use egui::{pos2, Id, Painter, Pos2, Rect};
use plex_core::library::Item;

use crate::source::Origin;
use crate::ui::home::Pick;
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

/// Draw the block for `item` inside `hero`, hung [`BOTTOM`] above its bottom.
pub fn show(
    ui: &mut egui::Ui,
    hero: Rect,
    id: Id,
    item: &Item,
    artwork: &mut images::Artwork,
    playable: bool,
    source: (Origin, &str),
) -> Option<Pick> {
    let painter = ui.painter().clone();
    let left = hero.min.x;
    let width = WIDTH.min(hero.width());
    // An episode's heading is its show; the episode line names the episode.
    let title = match item.kind.as_deref() {
        Some("episode") => item.grandparent_title.as_deref().or(item.title.as_deref()),
        _ => item.title.as_deref(),
    };
    let heading = Heading::new(
        &painter,
        title.unwrap_or("(untitled)"),
        home::hero_logo(item),
        artwork,
        LOGO_BOX,
        width,
    );
    let meta = painter.layout_no_wrap(
        meta_text(item),
        theme::medium(14.0),
        theme::palette().art_meta,
    );
    let episode = home::hero_episode_line(item)
        .map(|line| painter.layout(line, theme::semibold(15.0), theme::palette().accent, width));
    let summary = item
        .summary
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(|summary| {
            let mut job = LayoutJob::simple(
                summary.to_owned(),
                theme::medium(15.5),
                theme::palette().art_body,
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

    let text_block = Rect::from_min_size(pos2(left, top), egui::vec2(width, block));
    shade(&painter, text_block);
    let heading_rect = Rect::from_min_size(pos2(left, top), heading.size());
    heading.paint(&painter, heading_rect.min);
    let mut pick = link(
        ui,
        heading_rect,
        id.with("hero-title"),
        home::title_page(item),
        playable,
    );
    top += heading_rect.height() + ROW_GAP;
    top += overlay::meta_row(&painter, pos2(left, top), width, meta, item) + ROW_GAP;
    if let Some(galley) = episode {
        let rect = Rect::from_min_size(pos2(left, top), galley.rect.size());
        crate::ui::legible::on_art(&painter, rect.min, galley);
        let page = home::episode_page(item);
        pick = pick.or(link(ui, rect, id.with("hero-episode"), page, playable));
        top += rect.height() + ROW_GAP;
    }
    if let Some(galley) = summary {
        let height = galley.rect.height();
        crate::ui::legible::on_art(&painter, pos2(left, top), galley);
        top += height + ROW_GAP;
    }
    top += 6.0;
    pick = pick.or(buttons(ui, pos2(left, top), id, item, playable));
    top += BUTTON[0] + ROW_GAP - 4.0;
    source_line(&painter, pos2(left, top), &source.0, source.1);
    pick
}

/// How far the shade behind the text reaches past the text block: left,
/// right, up and down.
const SHADE_REACH: [f32; 4] = [80.0, 160.0, 70.0, 50.0];
/// Steps the shade's mesh takes across each margin and across the block.
const SHADE_STEPS: [usize; 2] = [5, 3];

/// A soft, borderless shade behind the text block: the theme's shadow ink at
/// its `art_scrim` strength over the block, fading to nothing over
/// [`SHADE_REACH`] round it, so the summary reads over a bright part of the
/// art without a visible box.
fn shade(painter: &Painter, block: Rect) {
    let [l, r, u, d] = SHADE_REACH;
    let xs = stops([block.min.x - l, block.min.x, block.max.x, block.max.x + r]);
    let ys = stops([block.min.y - u, block.min.y, block.max.y, block.max.y + d]);
    let strength = f32::from(theme::current().art_scrim) / 255.0;
    let ink = theme::palette().shadow;
    let mut mesh = egui::Mesh::default();
    for &y in &ys {
        for &x in &xs {
            let dx = ((block.min.x - x) / l).max((x - block.max.x) / r).max(0.0);
            let dy = ((block.min.y - y) / u).max((y - block.max.y) / d).max(0.0);
            let t = (1.0 - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0);
            let alpha = strength * t * t * (3.0 - 2.0 * t);
            mesh.colored_vertex(pos2(x, y), ink.gamma_multiply(alpha));
        }
    }
    let cols = xs.len() as u32;
    for row in 0..ys.len().saturating_sub(1) as u32 {
        for col in 0..cols - 1 {
            let i = row * cols + col;
            mesh.add_triangle(i, i + 1, i + cols);
            mesh.add_triangle(i + 1, i + cols + 1, i + cols);
        }
    }
    painter.add(egui::Shape::mesh(mesh));
}

/// The coordinates the shade's mesh runs through along one axis: evenly
/// across the near margin, the block and the far margin, so the edge of the
/// block itself is a row of vertices.
fn stops([outer_near, near, far, outer_far]: [f32; 4]) -> Vec<f32> {
    let [margin, inside] = SHADE_STEPS;
    let span = |from: f32, to: f32, steps: usize| {
        (0..steps).map(move |i| egui::lerp(from..=to, i as f32 / steps as f32))
    };
    span(outer_near, near, margin)
        .chain(span(near, far, inside))
        .chain(span(far, outer_far, margin))
        .chain(std::iter::once(outer_far))
        .collect()
}

/// `rect` as a link to `page`, underlined while pointed at; answers the page
/// when clicked. Nothing links while something else is playing.
fn link(ui: &mut egui::Ui, rect: Rect, id: Id, page: Option<home::Page>, on: bool) -> Option<Pick> {
    let page = page.filter(|_| on)?;
    let response = ui
        .interact(rect, id, egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(format!("Go to {}", page.title));
    if response.hovered() {
        underline(ui.painter(), rect);
    }
    response.clicked().then_some(Pick::Open(page))
}

/// The line under a pointed-at link.
fn underline(painter: &Painter, rect: Rect) {
    let y = rect.bottom() + 1.0;
    painter.line_segment(
        [pos2(rect.left(), y), pos2(rect.right(), y)],
        egui::Stroke::new(1.0, theme::palette().text),
    );
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
fn buttons(ui: &mut egui::Ui, origin: Pos2, id: Id, item: &Item, playable: bool) -> Option<Pick> {
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
        Some(marks::icon::Icon::Info),
        "Details",
        playable,
    );
    if !playable {
        None
    } else if play.clicked() {
        Some(Pick::play(item))
    } else if details.clicked() {
        home::item_page(item).map(Pick::Open)
    } else {
        None
    }
}

/// `From [Plex] Living Room NAS`, with the mark of the service the item is on.
fn source_line(painter: &Painter, origin: Pos2, service: &Origin, server: &str) {
    if server.is_empty() {
        return;
    }
    let font = theme::medium(12.5);
    let cy = origin.y + 7.0;
    let from = painter.layout_no_wrap(
        "From".to_owned(),
        font.clone(),
        theme::palette().art_caption,
    );
    let mut x = origin.x + from.rect.width() + 8.0;
    crate::ui::legible::on_art(painter, pos2(origin.x, cy - from.rect.height() / 2.0), from);
    x += marks::paint_source(painter, service, x, cy, 10.0) + 8.0;
    let name = painter.layout_no_wrap(server.to_owned(), font, theme::palette().art_caption);
    crate::ui::legible::on_art(painter, pos2(x, cy - name.rect.height() / 2.0), name);
}
