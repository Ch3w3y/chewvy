//! The detail page's head: the item's art full-bleed behind it, the title art,
//! the meta row, the summary and the play buttons on the left, and the
//! versions the item can play from in a frosted panel on the right (under
//! them in a narrow window).

use egui::text::LayoutJob;
use egui::{pos2, vec2, Color32, CornerRadius, Painter, Pos2, Rect, Sense};
use plex_core::library::{Item, Media};

use super::{
    backdrop, episode_heading, logo, pill_text, play_target, runtime, version_badges, Action, Page,
    Target,
};
use crate::source::Origin;
use crate::ui::overlay::{self, Heading, BUTTON};
use crate::ui::{bleed, glass, glass_tints, images, marks, theme};

/// The art's height under the head.
const ART_HEIGHT: f32 = 640.0;
/// The head's distance below the Back button's row.
const TOP: f32 = 28.0;
/// The text column's widest.
const COLUMN: f32 = 620.0;
/// The versions panel's width, and the window content width below which it
/// moves under the text column.
const PANEL: [f32; 2] = [400.0, 900.0];
/// The gap between the column and the panel, and after the head.
const GAP: f32 = 40.0;
/// The box a `clearLogo` is fitted into.
const LOGO_BOX: [f32; 2] = [380.0, 140.0];
/// Rows the summary may use.
const SUMMARY_ROWS: usize = 5;
/// A version row's height inside the panel.
const VERSION_ROW: f32 = 60.0;
/// The panel's inner padding.
const PAD: f32 = 18.0;
/// The selected version's row fill.
const ROW_FILL: Color32 = Color32::from_rgba_premultiplied(20, 20, 20, 20);

/// Draw the head from the cursor and answer with a play button's click.
pub fn show(
    ui: &mut egui::Ui,
    page: &Page,
    artwork: &mut images::Artwork,
    playing: bool,
    servers: crate::source::Servers<'_>,
) -> Option<Action> {
    let item = &page.item;
    let window = ui.ctx().content_rect();
    let origin = ui.cursor().min;
    // The art starts at the window's top while the page is scrolled to its
    // top, and scrolls away with it.
    let scrolled = ui.clip_rect().top() - ui.min_rect().top();
    let art_rect = bleed::rect(window, window.top() - scrolled, ART_HEIGHT);
    let backdrop_art = backdrop(item);
    let texture = artwork.texture(
        backdrop_art.as_deref(),
        art_rect.width() as u32,
        art_rect.height() as u32,
    );
    bleed::paint(ui.ctx(), art_rect, texture, bleed::Mask::Detail);

    let width = ui.available_width();
    let beside = width >= PANEL[1] && !item.media.is_empty();
    let column = if beside {
        (width - PANEL[0] - GAP).min(COLUMN)
    } else {
        width.min(COLUMN)
    };
    let top = origin.y + TOP;
    let (column_height, action) =
        column_block(ui, pos2(origin.x, top), column, page, artwork, playing);
    let mut height = TOP + column_height;
    if !item.media.is_empty() {
        let panel_origin = if beside {
            pos2(origin.x + width - PANEL[0], top)
        } else {
            pos2(origin.x, top + column_height + GAP)
        };
        let panel_height = versions(
            ui.painter(),
            panel_origin,
            PANEL[0].min(width),
            item,
            servers.of(item),
        );
        height = height.max(panel_origin.y - origin.y + panel_height);
    }
    ui.allocate_exact_size(vec2(width, height + GAP), Sense::hover());
    action
}

/// The text column from `origin`: heading, episode line, meta, summary and
/// buttons. Answers its height and a click.
fn column_block(
    ui: &mut egui::Ui,
    origin: Pos2,
    width: f32,
    page: &Page,
    artwork: &mut images::Artwork,
    playing: bool,
) -> (f32, Option<Action>) {
    let item = &page.item;
    let painter = ui.painter().clone();
    let heading = Heading::new(
        &painter,
        item,
        logo(item).as_deref(),
        artwork,
        LOGO_BOX,
        width,
    );
    let mut y = origin.y;
    let heading_height = heading.height();
    heading.paint(&painter, pos2(origin.x, y));
    y += heading_height + 16.0;
    if let Some(line) = episode_heading(item) {
        let galley = painter.layout(line, theme::semibold(16.0), theme::ACCENT, width);
        let height = galley.rect.height();
        overlay::shadowed(&painter, pos2(origin.x, y), galley);
        y += height + 12.0;
    }
    let meta = painter.layout_no_wrap(meta_text(item), theme::medium(14.0), overlay::META_TEXT);
    y += overlay::meta_row(&painter, pos2(origin.x, y), width, meta, item) + 16.0;
    if let Some(summary) = item
        .summary
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let mut job = LayoutJob::simple(
            summary.to_owned(),
            theme::medium(15.0),
            overlay::BODY_TEXT,
            width,
        );
        job.wrap.max_rows = SUMMARY_ROWS;
        let galley = painter.layout_job(job);
        let height = galley.rect.height();
        overlay::shadowed(&painter, pos2(origin.x, y), galley);
        y += height + 20.0;
    }
    let action = match play_target(page) {
        Some(target) => buttons(ui, pos2(origin.x, y), page, target, playing),
        None => None,
    };
    if play_target(page).is_some() {
        y += BUTTON[0];
    }
    (y - origin.y, action)
}

/// `2024 · Science Fiction, Adventure · 2h 47m`: what the answer names.
#[must_use]
pub fn meta_text(item: &Item) -> String {
    let mut parts: Vec<String> = Vec::new();
    parts.extend(item.year.map(|year| year.to_string()));
    let genres: Vec<&str> = item.genres.iter().take(2).map(String::as_str).collect();
    if !genres.is_empty() {
        parts.push(genres.join(", "));
    }
    if matches!(item.kind.as_deref(), Some("movie") | Some("episode")) {
        parts.extend(runtime(item.duration));
    }
    parts.join("  ·  ")
}

/// `Resume from 1:37:12` and `Play from start`, or the single play button.
fn buttons(
    ui: &mut egui::Ui,
    origin: Pos2,
    page: &Page,
    target: Target,
    playing: bool,
) -> Option<Action> {
    let id = ui
        .id()
        .with(("detail-play", page.item.rating_key.as_deref()));
    let label = match target.start {
        Some(start) => format!("Resume from {}", clock(start)),
        None => pill_text(page).unwrap_or_else(|| "Play".to_owned()),
    };
    let play = overlay::primary(ui, origin, id, &label, None, !playing);
    if !playing && play.clicked() {
        return Some(Action::Play(target));
    }
    if target.start.is_some() {
        let from_start = overlay::secondary(
            ui,
            pos2(play.rect.max.x + 12.0, origin.y),
            id.with("start"),
            None,
            "Play from start",
            !playing,
        );
        if !playing && from_start.clicked() {
            return Some(Action::Play(Target {
                start: None,
                ..target
            }));
        }
    }
    None
}

/// `1:37:12`, or `37:12` under an hour.
#[must_use]
pub fn clock(seconds: f64) -> String {
    let total = seconds.max(0.0) as u64;
    let (h, m, s) = (total / 3600, total / 60 % 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// The frosted "Play from" panel listing the item's versions; answers its
/// height. The first version is the one the play buttons open.
fn versions(
    painter: &Painter,
    origin: Pos2,
    width: f32,
    item: &Item,
    source: (Origin, &str),
) -> f32 {
    let count = item.media.len();
    let note = if count > 1 {
        "The ticked version plays. Choose a quality in the player to have the server convert it."
    } else {
        "Plays the file as it is. Choose a quality in the player to have the server convert it."
    };
    let note = painter.layout(
        note.to_owned(),
        theme::medium(12.5),
        theme::SECONDARY,
        width - 2.0 * PAD - 8.0,
    );
    let height = PAD + 30.0 + count as f32 * (VERSION_ROW + 6.0) + 8.0 + note.rect.height() + PAD;
    let rect = Rect::from_min_size(origin, vec2(width, height));
    glass::panel(painter, rect, 12.0, glass_tints::PANEL);

    let inner = rect.shrink(PAD);
    painter.text(
        pos2(inner.min.x + 4.0, inner.min.y + 10.0),
        egui::Align2::LEFT_CENTER,
        "Play from",
        theme::bold(15.0),
        theme::TEXT,
    );
    let found = if count == 1 {
        "1 version".to_owned()
    } else {
        format!("{count} versions")
    };
    painter.text(
        pos2(inner.max.x - 4.0, inner.min.y + 10.0),
        egui::Align2::RIGHT_CENTER,
        found,
        theme::medium(12.5),
        theme::SECONDARY,
    );
    let mut y = inner.min.y + 30.0;
    for (index, media) in item.media.iter().enumerate() {
        let row = Rect::from_min_size(pos2(inner.min.x, y), vec2(inner.width(), VERSION_ROW));
        version_row(painter, row, media, index == 0, &source.0, source.1);
        y += VERSION_ROW + 6.0;
    }
    painter.galley(pos2(inner.min.x + 4.0, y + 8.0), note, theme::SECONDARY);
    height
}

/// One version: a tick on the one that plays, the service and server, then
/// what the version is.
fn version_row(
    painter: &Painter,
    row: Rect,
    media: &Media,
    chosen: bool,
    service: &Origin,
    server: &str,
) {
    if chosen {
        painter.rect_filled(row, CornerRadius::same(8), ROW_FILL);
        marks::paint_tinted(
            painter,
            marks::icon::CHECK,
            row.min.x + 14.0,
            row.center().y,
            18.0,
            theme::ACCENT,
        );
    }
    let x = row.min.x + 46.0;
    let line = row.min.y + 20.0;
    let mut name_x = x + marks::paint_source(painter, service, x, line, 10.0) + 8.0;
    if server.is_empty() {
        name_x = x;
    }
    painter.text(
        pos2(name_x, line),
        egui::Align2::LEFT_CENTER,
        server,
        theme::bold(14.0),
        theme::TEXT,
    );
    let spec = version_badges(media).join("  ·  ");
    let mut job = LayoutJob::simple_singleline(spec, theme::medium(12.5), theme::SECONDARY);
    job.wrap.max_width = row.max.x - 12.0 - x;
    job.wrap.max_rows = 1;
    let spec = painter.layout_job(job);
    painter.galley(
        pos2(x, row.min.y + 42.0 - spec.rect.height() / 2.0),
        spec,
        theme::SECONDARY,
    );
}
