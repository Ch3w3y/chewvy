//! The Home hero carousel: the first few Continue Watching / On Deck items,
//! one at a time, with pager dots and an auto-advance that pauses while the
//! pointer is over it.
//!
//! The item itself, its artwork and its labels are the server's home hubs'
//! own fields ([`home`] decides which hubs the pool comes from and which words
//! an answer entitles); a field an answer does not name leaves that part of
//! the hero out.
//!
//! The carousel is driven by frames, not a thread: a frame's
//! [`egui::Context::request_repaint_after_secs`] is what wakes the window when
//! nothing else would, and a frame's `predicted_dt` is what the timer is
//! measured in, so the countdown only counts frames that were actually drawn.

use egui::{Color32, Id, Sense};
use plex_core::library::Item;

use crate::ui::{bleed, cards, home, images, theme};

#[path = "hero/block.rs"]
mod block;

/// The hero's height bounds; between them it is [`HEIGHT_SHARE`] of the window.
const HEIGHT_RANGE: [f32; 2] = [440.0, 560.0];
/// The share of the window's height the hero takes.
const HEIGHT_SHARE: f32 = 0.6;
/// How far the art runs on under the first shelf before its mask fades it out.
const ART_OVERHANG: f32 = 120.0;
/// A pager dot's painted radius.
const DOT_RADIUS: f32 = 3.5;
/// A pager dot's clickable box: wider than [`DOT_RADIUS`] so a dot is easy to hit.
const DOT_BOX: f32 = 18.0;
/// The dot row's inset from the hero's bottom and right edges.
const DOT_INSET: f32 = 12.0;
/// A dot the carousel is not on.
const DOT_DIM: Color32 = Color32::from_rgba_premultiplied(110, 110, 110, 110);

/// How many hero items the carousel rotates through.
pub const SLOTS: usize = 5;
/// Seconds one hero item stays up before the carousel moves on.
pub const AUTO_ADVANCE_SECS: f32 = 8.0;

/// The hero's height in a window `window_height` points tall.
#[must_use]
pub fn height(window_height: f32) -> f32 {
    (window_height * HEIGHT_SHARE).clamp(HEIGHT_RANGE[0], HEIGHT_RANGE[1])
}

/// The hero item one index down from `index`, wrapping at the end back to the
/// first. A carousel of none or one has nowhere to go.
#[must_use]
pub fn next_index(index: usize, count: usize) -> usize {
    if count == 0 {
        return 0;
    }
    (index + 1) % count
}

/// The index to show when the pool has shrunk under the index being shown.
#[must_use]
pub fn clamp_index(index: usize, count: usize) -> usize {
    if count == 0 {
        return 0;
    }
    index.min(count - 1)
}

/// One frame of the countdown: how much has elapsed since the last advance, and
/// whether this frame advances. The timer is frozen while the pointer is over
/// the hero (paused on hover) and a carousel of one item has nothing to
/// advance to, so it accumulates nothing either. `dt` is a drawn frame's length;
/// a negative one (a clock adjustment) counts as no time.
#[must_use]
pub fn advance(elapsed: f32, dt: f32, count: usize, hovered: bool, interval: f32) -> (f32, bool) {
    if count < 2 || hovered || interval <= 0.0 {
        return (elapsed, false);
    }
    let elapsed = elapsed + dt.max(0.0);
    if elapsed >= interval {
        return (0.0, true);
    }
    (elapsed, false)
}

/// Seconds until the next advance, or `None` when nothing is counting down.
/// This is what a frame asks the window to repaint after, so an idle browsing
/// screen still turns the carousel.
#[must_use]
pub fn repaint_after(elapsed: f32, count: usize, hovered: bool, interval: f32) -> Option<f32> {
    if count < 2 || hovered || interval <= 0.0 {
        return None;
    }
    Some((interval - elapsed.max(0.0)).clamp(0.0, interval))
}

/// The clickable box of the `ordinal` pager dot of `count`, laid out left to
/// right in a group held [`DOT_INSET`] from the hero's bottom-right corner, so
/// dot order reads the way the carousel steps.
#[must_use]
pub fn dot_rects(hero: egui::Rect, count: usize) -> Vec<egui::Rect> {
    let right = hero.max.x - DOT_INSET;
    let cy = hero.max.y - DOT_INSET - DOT_BOX / 2.0;
    (0..count)
        .map(|ordinal| {
            let x = right - (count - 1 - ordinal) as f32 * DOT_BOX;
            egui::Rect::from_center_size(egui::pos2(x, cy), egui::vec2(DOT_BOX, DOT_BOX))
        })
        .collect()
}

/// The hero's place in the carousel and how long the current item has been up.
#[derive(Clone, Copy)]
struct Carousel {
    index: usize,
    elapsed: f32,
}

/// Draw the hero over `pool` (its first [`SLOTS`] openable items). Returns the
/// item a button asked to open, or `None` while nothing was clicked.
///
/// `playable` is whether nothing else is playing: while an item plays, the
/// buttons are hover-only, as the shelf cards are. `server` names where the
/// item comes from.
pub fn show(
    ui: &mut egui::Ui,
    pool: &[&Item],
    artwork: &mut images::Artwork,
    playable: bool,
    servers: crate::source::Servers<'_>,
) -> Option<home::Resume> {
    let items: Vec<&Item> = pool
        .iter()
        .copied()
        .filter(|item| item.rating_key.is_some())
        .take(SLOTS)
        .collect();
    let count = items.len();
    if count == 0 {
        return None;
    }
    let window = ui.ctx().content_rect();
    let width = ui.available_width().max(home::CARD[0]);
    let (hero, _) =
        ui.allocate_exact_size(egui::vec2(width, height(window.height())), Sense::hover());
    let id = Id::new("home-hero");
    let index = carousel(ui, hero, id, count);
    let item = items.get(index).copied()?;

    let art_rect = bleed::rect(window, hero.min.y, hero.height() + ART_OVERHANG);
    let texture = artwork.texture(
        home::hero_art(item),
        art_rect.width() as u32,
        art_rect.height() as u32,
    );
    bleed::paint(ui.ctx(), art_rect, texture, bleed::Mask::Hero);
    let pressed = block::show(ui, hero, id, item, artwork, playable, servers.of(item));
    if count > 1 {
        paint_dots(ui, hero, count, index);
    }
    (pressed.play || pressed.details).then(|| home::Resume {
        rating_key: item.rating_key.clone().unwrap_or_default(),
        title: home::play_title(item),
        start: home::resume_start(item),
        overview: pressed.details,
    })
}

/// Step the carousel for this frame (auto-advance, dot clicks) and answer the
/// index to show.
fn carousel(ui: &mut egui::Ui, hero: egui::Rect, id: Id, count: usize) -> usize {
    // Whether the pointer is over the hero decides whether the countdown runs
    // this frame, and it is read before anything is drawn.
    let hovered = ui
        .ctx()
        .pointer_interact_pos()
        .is_some_and(|pos| hero.contains(pos));
    let dt = ui.ctx().input(|input| input.predicted_dt);
    let state = ui.ctx().data_mut(|data| {
        let carousel =
            data.get_temp_mut_or_insert_with::<Carousel>(id.with("state"), || Carousel {
                index: 0,
                elapsed: 0.0,
            });
        let (elapsed, advanced) = advance(carousel.elapsed, dt, count, hovered, AUTO_ADVANCE_SECS);
        carousel.elapsed = elapsed;
        if advanced {
            carousel.index = next_index(carousel.index, count);
        }
        carousel.index = clamp_index(carousel.index, count);
        *carousel
    });
    let mut index = state.index;
    let mut elapsed = state.elapsed;
    if count > 1 {
        if let Some(ordinal) = dot_clicks(ui, hero, id, count) {
            index = clamp_index(ordinal, count);
            // A click the user made themselves starts the countdown afresh.
            elapsed = 0.0;
        }
    }
    ui.ctx().data_mut(|data| {
        data.insert_temp(id.with("state"), Carousel { index, elapsed });
    });
    if let Some(after) = repaint_after(elapsed, count, hovered, AUTO_ADVANCE_SECS) {
        ui.ctx().request_repaint_after_secs(after);
    }
    index
}

/// Read a click on each of the `count` pager dots, answering with the ordinal of
/// the one clicked. The clickable box is [`DOT_BOX`] wide around a [`DOT_RADIUS`]
/// dot, so a dot is a target and not a hairline.
fn dot_clicks(ui: &mut egui::Ui, hero: egui::Rect, id: Id, count: usize) -> Option<usize> {
    let mut clicked = None;
    for (ordinal, rect) in dot_rects(hero, count).into_iter().enumerate() {
        let box_id = id.with(("hero-dot", ordinal));
        let response = cards::pointer(ui.interact(rect, box_id, Sense::click()));
        if response.clicked() {
            clicked = Some(ordinal);
        }
    }
    clicked
}

/// Paint the `count` pager dots: they say how many items the carousel holds.
fn paint_dots(ui: &mut egui::Ui, hero: egui::Rect, count: usize, index: usize) {
    for (ordinal, rect) in dot_rects(hero, count).into_iter().enumerate() {
        let color = if ordinal == index {
            theme::ACCENT
        } else {
            DOT_DIM
        };
        ui.painter().circle_filled(rect.center(), DOT_RADIUS, color);
    }
}

#[cfg(test)]
#[path = "hero/tests.rs"]
mod tests;
