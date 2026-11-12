//! The Home screen: the hero, and one horizontally scrolling shelf per
//! `/hubs` hub, with the resume offset each hub item carries.
//!
//! Every value drawn here is one the server's hub answer named
//! (`hubIdentifier`, `title`, `style`, `viewOffset`, `duration`, `index`,
//! `parentIndex`, `grandparentTitle`, `year`, `contentRating`, `Image[]`):
//! a field an answer does not name simply leaves that part out.

use egui::containers::scroll_area::{ScrollBarVisibility, ScrollSource};
use egui::Sense;
use plex_core::library::{Hub, Item};

use crate::ui::{cards, hero, images, theme};

mod card;

/// A 16:9 shelf card.
pub const CARD: [f32; 2] = [260.0, 146.0];
/// A 2:3 shelf card.
pub const MOVIE_CARD: [f32; 2] = [150.0, 225.0];
/// The resume bar drawn along a card's bottom edge.
pub const PROGRESS_HEIGHT: f32 = 4.0;

/// Artwork requested from `/photo/:/transcode` per card kind. Disk-cache keys
/// are distinct per requested size, so a card and the hero never share a frame.
const CARD_ART: [u32; 2] = [520, 292];
const MOVIE_CARD_ART: [u32; 2] = [300, 450];

/// The space between two cards in a row.
const SHELF_GAP: f32 = 18.0;
/// The caption block under one card's artwork.
const CAPTION_HEIGHT: f32 = 44.0;
/// A row's card shape. Every card in one row takes the row's shape, so a
/// shelf is top-aligned whatever the server mixed into it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowShape {
    /// 16:9 `CARD` artwork: the row is the server's continue-watching or
    /// on-deck row, or it holds episodes.
    Wide,
    /// 2:3 `MOVIE_CARD` artwork: a row of posters (a recently-added row).
    Poster,
}
/// The gap from one shelf's reserved caption block to the next shelf's title.
const ROW_GAP: f32 = 14.0;
/// `hubIdentifier` values naming the continue shelf, lowercased without
/// separators: `home.continue` (home hubs) and `continueWatching` (search).
const CONTINUE_IDS: [&str; 2] = ["homecontinue", "continuewatching"];
/// The same for the on-deck shelf (`home.ondeck`).
const DECK_IDS: [&str; 2] = ["homeondeck", "ondeck"];
/// The part of a `hubIdentifier` naming a recently-added shelf: the captured
/// home answer names one `home.movies.recent`.
const RECENT_SUFFIX: &str = "recent";
/// The fraction of a row's visible width one pager arrow pages.
const ROW_PAGE: f32 = 0.8;
/// Seconds a row takes to glide to its next page.
const ROW_GLIDE: f32 = 0.25;

/// One card or the hero asked the player to open an item; `start` is where.
pub struct Resume {
    pub rating_key: String,
    pub title: String,
    /// Seconds into the item from its `viewOffset`; `None` plays from the start.
    pub start: Option<f64>,
    /// Open the item's overview page instead of playing it: every shelf but
    /// continue watching and on deck, whose cards resume straight away.
    pub overview: bool,
}

/// Whether a shelf's cards play at once (continue watching, on deck) rather
/// than opening the item's overview first.
fn plays_directly(hub: &Hub) -> bool {
    matches(hub.hub_id.as_ref(), &CONTINUE_IDS) || matches(hub.hub_id.as_ref(), &DECK_IDS)
}

// ------------------------------------------------------------- label rules

/// The whole minutes still to watch, rounded to the nearest minute: a server
/// answers `viewOffset`/`duration` in milliseconds, and 1 925 936 ms remaining
/// reads as 32 min, 1 298 736 ms as 22 min. `None` when there is no resume
/// point or nothing left.
#[must_use]
pub fn minutes_left(view_offset: Option<u64>, duration: Option<u64>) -> Option<u64> {
    let offset = view_offset?;
    let total = duration?;
    // A `viewOffset` of 0 is the server saying "not started", the same reading
    // [`progress_fraction`] and [`resume_start`] give it.
    if total == 0 || offset == 0 || offset >= total {
        return None;
    }
    Some(((total - offset) as f64 / 60_000.0).round() as u64)
}

/// How far into an item the server says the user is, as `0.0..=1.0`. `None`
/// when the item has no `duration` or the user has not started it.
#[must_use]
pub fn progress_fraction(view_offset: Option<u64>, duration: Option<u64>) -> Option<f32> {
    let offset = view_offset?;
    let total = duration?;
    if total == 0 || offset == 0 {
        return None;
    }
    Some(((offset as f64 / total as f64).clamp(0.0, 1.0)) as f32)
}

/// `S4 E2 · Blue Rafter`, skipping any index the answer did not name.
#[must_use]
pub fn episode_caption(item: &Item) -> String {
    let title = item
        .title
        .clone()
        .unwrap_or_else(|| "(untitled)".to_owned());
    let mut prefix = String::new();
    if let Some(season) = item.parent_index {
        prefix.push_str(&format!("S{season}"));
    }
    if let Some(number) = item.index {
        if !prefix.is_empty() {
            prefix.push(' ');
        }
        prefix.push_str(&format!("E{number}"));
    }
    if prefix.is_empty() {
        title
    } else {
        format!("{prefix} · {title}")
    }
}

/// The subtitle a 2:3 poster card carries: an episode its `S1 E2 · title`, a
/// season its `S1`, a movie or show its `32 min left` where the server gave a
/// resume point and its `year` otherwise. An item the answer gave none of those
/// for leaves the line out rather than repeating the card's own title.
#[must_use]
pub fn poster_subtitle(item: &Item) -> String {
    match item.kind.as_deref() {
        Some("movie") | Some("show") => minutes_left(item.view_offset, item.duration)
            .map(|minutes| format!("{minutes} min left"))
            .or_else(|| item.year.map(|year| year.to_string()))
            .unwrap_or_default(),
        Some("season") => item
            .parent_index
            .map(|season| format!("S{season}"))
            .unwrap_or_default(),
        Some("episode") => episode_caption(item),
        _ => String::new(),
    }
}

/// The hero's episode line (F30): only an episode names one, so a movie or
/// show hero draws no line and reserves no height for one.
#[must_use]
pub fn hero_episode_line(item: &Item) -> Option<String> {
    (item.kind.as_deref() == Some("episode")).then(|| episode_caption(item))
}

/// The name the player is given for one item: a show's episode keeps both the
/// show and the episode, so a resumed episode's window title is the card's own
/// caption instead of only the show.
#[must_use]
pub fn play_title(item: &Item) -> String {
    match (
        item.grandparent_title.as_deref(),
        item.parent_rating_key.as_ref(),
    ) {
        (Some(show), Some(_)) => format!("{show} - {}", episode_caption(item)),
        _ => item
            .title
            .clone()
            .unwrap_or_else(|| "(untitled)".to_owned()),
    }
}

/// The type word the design spec shows for an item: a movie reads `Movie`, a
/// show, season or episode reads `TV Show`. A `type` the spec has no word for is
/// left out rather than drawn as the server's raw word, because an episode hero
/// labelled `episode` reads as a claim about the row, not the item.
#[must_use]
pub fn type_label(item: &Item) -> Option<&'static str> {
    match item.kind.as_deref() {
        Some("movie") => Some("Movie"),
        Some("show") | Some("season") | Some("episode") => Some("TV Show"),
        _ => None,
    }
}

/// The seconds to start playback at, from the server's own `viewOffset`.
#[must_use]
pub fn resume_start(item: &Item) -> Option<f64> {
    let offset = item.view_offset?;
    if offset == 0 {
        return None;
    }
    Some(offset as f64 / 1000.0)
}

/// Lowercase a server name so a `hubIdentifier` can be compared with the two
/// spellings the captures use (`home.onDeck` and `home.ondeck` are the same).
fn normalize(name: &str) -> String {
    name.chars()
        .filter(char::is_ascii_alphanumeric)
        .flat_map(char::to_lowercase)
        .collect()
}

fn matches(name: Option<&String>, wanted: &[&str]) -> bool {
    name.map(|value| wanted.contains(&normalize(value).as_str()))
        .unwrap_or(false)
}

/// The shape one hub's whole row is drawn in: its own `hubIdentifier` or
/// `type`, never an individual item's `type`. The captured home answer puts
/// one episode (`43492`) and two movies (`14431`, `42793`) in the
/// `type: mixed` `home.continue` hub; drawing each row in one shape is what
/// keeps that shelf top-aligned instead of mixing 16:9 with 2:3 heights.
/// A `type` of `episode` or `mixed` is an episode row (16:9); `movie`, `show`
/// and `artist` (the captured search hubs) are poster rows (2:3). A
/// recently-added shelf ([`recent_hub`]) is always a poster row: the captured TV
/// version of one answers seasons and episodes, and both read as posters.
#[must_use]
pub fn row_shape(hub: &Hub) -> RowShape {
    if recent_hub(hub) {
        return RowShape::Poster;
    }
    if matches(hub.hub_id.as_ref(), &CONTINUE_IDS) || matches(hub.hub_id.as_ref(), &DECK_IDS) {
        return RowShape::Wide;
    }
    match hub.kind.as_deref() {
        Some("episode") | Some("mixed") => RowShape::Wide,
        _ => RowShape::Poster,
    }
}

/// Whether one hub is the server's Recently Added shelf, from its own
/// `hubIdentifier`. A Recently Added shelf draws 2:3 posters whatever its `type`
/// says: the captured TV row answers seasons and episodes, both of which
/// [`poster_thumb`] draws as the show's poster.
#[must_use]
pub fn recent_hub(hub: &Hub) -> bool {
    hub.hub_id
        .as_deref()
        .is_some_and(|id| normalize(id).ends_with(RECENT_SUFFIX))
}

/// `CARD` or `MOVIE_CARD`, plus the `/photo/:/transcode` size to fetch for it.
fn card_size(shape: RowShape) -> ([f32; 2], [u32; 2]) {
    match shape {
        RowShape::Wide => (CARD, CARD_ART),
        RowShape::Poster => (MOVIE_CARD, MOVIE_CARD_ART),
    }
}

/// Whether a shelf is worth Home's room: the server's music and photo shelves
/// (`artist`, `album`, `track`, `photo`) hold nothing this client plays, and a
/// `playlist` counts only when its own rows name a movie, episode, show or
/// season.
#[must_use]
pub fn shows_video(hub: &Hub) -> bool {
    match hub.kind.as_deref() {
        Some("artist") | Some("album") | Some("track") | Some("photo") => false,
        Some("playlist") => hub.items.iter().any(|item| {
            matches!(
                item.kind.as_deref(),
                Some("movie") | Some("episode") | Some("show") | Some("season")
            )
        }),
        _ => true,
    }
}

/// The shelves Home may draw from: the video ones.
fn video_shelves(hubs: &[Hub]) -> impl Iterator<Item = &Hub> {
    hubs.iter().filter(|hub| shows_video(hub))
}

/// The hub the hero comes from: Continue Watching when the server named one,
/// else On Deck. A hub is chosen by its stable `hubIdentifier` first; the
/// server-worded `title` only decides when no hub names a known identifier,
/// because a localized `title` is not a name this client can rely on.
#[must_use]
pub fn hero_hub(hubs: &[Hub]) -> Option<&Hub> {
    let by_id =
        |wanted: &[&str]| video_shelves(hubs).find(|hub| matches(hub.hub_id.as_ref(), wanted));
    let by_title =
        |wanted: &[&str]| video_shelves(hubs).find(|hub| matches(hub.title.as_ref(), wanted));
    by_id(&CONTINUE_IDS)
        .or_else(|| by_id(&DECK_IDS))
        .or_else(|| by_title(&CONTINUE_IDS))
        .or_else(|| by_title(&DECK_IDS))
        .or_else(|| video_shelves(hubs).find(|hub| hub.style.as_deref() == Some("hero")))
        .or_else(|| video_shelves(hubs).find(|hub| !hub.items.is_empty()))
}

/// The item the hero starts on: the first item of the hero hub this app can
/// open. It is also the one item taken out of that shelf, so the carousel's
/// first slide is not drawn twice on the same screen.
#[must_use]
pub fn hero_item(hubs: &[Hub]) -> Option<&Item> {
    hero_hub(hubs)
        .into_iter()
        .flat_map(|hub| hub.items.iter())
        .find(|item| has_rating_key(item))
}

/// The hero carousel's pool: up to [`hero::SLOTS`] openable items, taken from
/// the hero shelf first, then every other video shelf in the order Home draws
/// them, deduplicated by `ratingKey` (the home capture repeats 43492 across
/// `home.continue` and `home.ondeck`).
///
/// Only [`hero_item`]'s item leaves a shelf: the carousel spotlights items the
/// row still lists, so a shelf never loses its cards to the hero.
#[must_use]
pub fn hero_pool(hubs: &[Hub]) -> Vec<&Item> {
    let mut pool: Vec<&Item> = Vec::new();
    for hub in hero_hub(hubs).into_iter().chain(video_shelves(hubs)) {
        for item in hub.items.iter() {
            if !has_rating_key(item) || pool.iter().any(|held| held.rating_key == item.rating_key) {
                continue;
            }
            pool.push(item);
            if pool.len() == hero::SLOTS {
                return pool;
            }
        }
    }
    pool
}

fn has_rating_key(item: &Item) -> bool {
    item.rating_key.is_some()
}

/// The artwork one wide card draws: the `snapshot` or `background` the item
/// names (the capture gives both to an episode, only `background` to a movie
/// in a 16:9 row), else its own artwork. A movie's `thumb` is its 2:3 poster,
/// which is the wrong crop for a wide row.
fn wide_art(item: &Item) -> Option<&str> {
    item.image_url("snapshot")
        .or(item.image_url("background"))
        .or(item.thumb.as_deref())
        .or(item.art.as_deref())
        .or(item.grandparent_art.as_deref())
}

/// The poster one 2:3 card draws, from the artwork the captures give each kind:
/// a movie its own `thumb`; an episode the show's `grandparentThumb` (an
/// episode's own `thumb` is its 16:9 still); a season the show's `parentThumb`
/// (a season answers its own `thumb` and no `grandparentThumb`). Only when an
/// answer names none of those does it fall back to the item's own artwork.
#[must_use]
pub fn poster_thumb(item: &Item) -> Option<&str> {
    if item.kind.as_deref() == Some("movie") {
        return item
            .thumb
            .as_deref()
            .or(item.art.as_deref())
            .or(item.parent_thumb.as_deref())
            .or(item.grandparent_thumb.as_deref());
    }
    item.grandparent_thumb
        .as_deref()
        .or(item.parent_thumb.as_deref())
        .or(item.thumb.as_deref())
        .or(item.art.as_deref())
}

/// The artwork one poster card draws.
fn poster_art(item: &Item) -> Option<&str> {
    poster_thumb(item).or(item.grandparent_art.as_deref())
}

fn card_art(item: &Item, shape: RowShape) -> Option<&str> {
    match shape {
        RowShape::Wide => wide_art(item),
        RowShape::Poster => poster_art(item),
    }
}

/// The hero's backdrop: the `background` artwork, else `art`, else the show's.
pub(crate) fn hero_art(item: &Item) -> Option<&str> {
    item.image_url("background")
        .or(item.art.as_deref())
        .or(item.grandparent_art.as_deref())
}

/// The logo the hero draws, when the item names a `clearLogo`.
pub(crate) fn hero_logo(item: &Item) -> Option<&str> {
    item.image_url("clearLogo")
}

// -------------------------------------------------------------- the screen

/// Draw Home: the hero, then one horizontally scrolling shelf per hub. Returns
/// the item the user clicked to (re)play, or `None` while nothing was clicked.
///
/// `playable` is whether nothing else is open: while an item is playing the
/// cards and the hero pill are hover-only, as the item list's ▶ Play is.
pub fn show(
    ui: &mut egui::Ui,
    hubs: &[Hub],
    artwork: &mut images::Artwork,
    playable: bool,
    server: &str,
) -> Option<Resume> {
    if hubs.is_empty() {
        ui.label(theme::secondary("Your server gave no home shelves yet."));
        return None;
    }
    let mut play = None;
    // The wheel, keys and touch scroll the page; a bar would only draw a line
    // down the hero art.
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .scroll_bar_visibility(ScrollBarVisibility::AlwaysHidden)
        .show(ui, |ui| {
            let pool = hero_pool(hubs);
            if !pool.is_empty() {
                if let Some(resume) = hero::show(ui, &pool, artwork, playable, server) {
                    play = Some(resume);
                }
                ui.add_space(ROW_GAP);
            }
            let hero = hero_hub(hubs);
            let hero_key = hero_item(hubs).and_then(|item| item.rating_key.clone());
            for (row, hub) in hubs.iter().enumerate() {
                if hub.items.is_empty() || !shows_video(hub) {
                    continue;
                }
                // The hero already shows the hero hub's first item, so that
                // one shelf starts at the next item instead of drawing the
                // same card twice on the same screen.
                let hidden = if hero.is_some_and(|h| std::ptr::eq(h, hub)) {
                    hero_key.as_deref()
                } else {
                    None
                };
                if let Some(resume) = shelf(ui, hub, artwork, row_shape(hub), row, hidden, playable)
                {
                    play = Some(resume);
                }
                ui.add_space(ROW_GAP);
            }
        });
    play
}

/// The row viewport's id. It also seeds the row's cards' ids, so paging,
/// keyboard focus and the row's stored offset all name the same row.
fn row_id(row: usize) -> egui::Id {
    egui::Id::new(("home-row", row))
}

/// One shelf: its label with a chevron marking it as a row that pages sideways,
/// then every card at the shape the whole row takes.
///
/// `hidden` is the item the hero already shows; `row` identifies this shelf for
/// widget ids (the capture repeats `ratingKey` 43492 across `home.continue` and
/// `home.ondeck`).
///
/// The row keeps its own sideways offset. The wheel is read here rather than
/// left to the row's area: the outer vertical area has already read it by the
/// time a row is drawn and egui zeroes `smooth_scroll_delta` once it has used
/// it (containers/scroll_area.rs), so a nested area would never see it. The row
/// takes the wheel only while the pointer is over its band, and zeroes it then,
/// which is what keeps the page scrolling for a pointer over nothing.
fn shelf(
    ui: &mut egui::Ui,
    hub: &Hub,
    artwork: &mut images::Artwork,
    shape: RowShape,
    row: usize,
    hidden: Option<&str>,
    playable: bool,
) -> Option<Resume> {
    let title = hub
        .title
        .clone()
        .or_else(|| hub.hub_id.clone())
        .unwrap_or_else(|| "Shelf".to_owned());
    let visible: Vec<&Item> = hub
        .items
        .iter()
        .filter(|item| has_rating_key(item) && item.rating_key.as_deref() != hidden)
        .collect();
    let id = row_id(row);
    let ([card_w, card_h], _) = card_size(shape);
    let content = visible.len() as f32 * (card_w + SHELF_GAP);

    ui.horizontal(|ui| {
        ui.label(theme::strong(title).size(theme::H2_SIZE));
        // The chevron that says this row goes on that way.
        let (plate, _) = ui.allocate_exact_size(
            egui::vec2(cards::CHEVRON_BOX[0], cards::CHEVRON_BOX[1]),
            Sense::hover(),
        );
        cards::chevron(ui, plate.center(), false, theme::SECONDARY);
    });

    let left = ui.cursor().min.x;
    let top = ui.available_rect_before_wrap().min.y;
    let band_width = ui.available_width();
    let clamp = (content - band_width).max(0.0);
    let step = (band_width * ROW_PAGE).clamp(card_w + SHELF_GAP, content.max(card_w + SHELF_GAP));
    let offset = ui
        .ctx()
        .data(|data| data.get_temp::<f32>(id.with("offset")))
        .unwrap_or(0.0)
        .min(clamp);
    // The row glides to its new offset, so a page reads as movement one way.
    let shown = ui
        .ctx()
        .animate_value_with_time(id.with("glide"), offset, ROW_GLIDE);

    let mut play = None;
    let mut reveal = 0.0_f32;
    let area = egui::ScrollArea::horizontal()
        .id_salt(id)
        .auto_shrink([false, true])
        // The row's own pager arrows and wheel reader page it, so its area
        // takes no wheel of its own and shows no bar of its own.
        .scroll_source(ScrollSource {
            mouse_wheel: false,
            ..Default::default()
        })
        .scroll_bar_visibility(ScrollBarVisibility::AlwaysHidden)
        .horizontal_scroll_offset(shown)
        .show(ui, |ui| {
            // `Align::Min` on a horizontal layout's cross axis is its top edge:
            // every card in the row hangs from the same line.
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
                for (ordinal, item) in visible.iter().enumerate() {
                    let fresh = shape == RowShape::Poster && recent_hub(hub) && unseen(item);
                    let card_id = id.with(("card", ordinal));
                    let (resume, response) =
                        card::show(ui, artwork, item, shape, card_id, fresh, playable);
                    if let Some(mut resume) = resume {
                        resume.overview = !plays_directly(hub);
                        play = Some(resume);
                    }
                    // Keyboard focus must not land on a card the band cannot
                    // show: pull the row to it.
                    if response.gained_focus() {
                        let rect = response.rect;
                        reveal += (rect.max.x - (left + band_width)).max(0.0);
                        reveal -= (left - rect.min.x).max(0.0);
                    }
                    ui.add_space(SHELF_GAP);
                }
            });
        });

    // The band is the whole area the row was given: its title line through its
    // cards. Both ends of it get a pager arrow, and only while the row has
    // cards the band cannot show.
    let band = egui::Rect::from_min_max(egui::pos2(left, top), area.inner_rect.max);
    let mut paging = reveal;
    if clamp > 0.0 {
        // Each arrow shows only while there is more of the row its way: none
        // pointing back at the start, none pointing on at the end.
        let more_back = offset > 0.5;
        let more_on = offset < clamp - 0.5;
        let reach = band.width() * 0.5 - cards::ARROW_RADIUS - cards::ARROW_INSET;
        for (flip, sign, more) in [(false, 1.0, more_on), (true, -1.0, more_back)] {
            if !more {
                continue;
            }
            // Level with the middle of the artwork, not of artwork and captions.
            let center = egui::pos2(band.center().x + sign * reach, top + card_h * 0.5);
            let response = ui.interact(
                cards::arrow_rect(center),
                id.with(("arrow", flip)),
                Sense::click(),
            );
            let response = cards::pointer(response);
            if response.clicked() {
                paging += sign * step;
            }
            cards::arrow(ui, center, flip, crate::ui::glass_tints::SHELF_ARROW);
        }
    }
    // The wheel (F38). A plain vertical wheel belongs to the page, never to a
    // row: the row only takes a delta that came sideways, or one held with
    // Shift, and only while the pointer is over its band and the row has cards
    // left to page. A delta the row does not take is left in place for the page.
    let over = ui
        .ctx()
        .pointer_interact_pos()
        .is_some_and(|pos| band.contains(pos));
    let step = ui.ctx().input_mut(|input| {
        let wheel = cards::row_wheel_delta(input.modifiers.shift, input.smooth_scroll_delta);
        let taken = over
            .then(|| cards::row_advance(offset, clamp, wheel))
            .flatten();
        if wheel != 0.0 && taken.is_some() {
            input.smooth_scroll_delta = egui::Vec2::ZERO;
        }
        taken.unwrap_or(0.0)
    });
    ui.ctx().data_mut(|data| {
        *data.get_temp_mut_or_default::<f32>(id.with("offset")) =
            (offset + paging + step).clamp(0.0, clamp);
    });
    play
}

/// Whether the server says nothing of this item has been watched yet.
#[must_use]
pub fn unseen(item: &Item) -> bool {
    item.view_count.unwrap_or(0) == 0 && item.view_offset.unwrap_or(0) == 0
}

/// `1h 10m left`, or `22 min left` under an hour.
#[must_use]
pub fn time_left(minutes: u64) -> String {
    if minutes < 60 {
        return format!("{minutes} min left");
    }
    format!("{}h {:02}m left", minutes / 60, minutes % 60)
}

/// A wide card's second line: `S2 · E7 · 22 min left` for an episode, the year
/// for anything else, then the time left where the server gave a resume point.
#[must_use]
pub fn wide_subtitle(item: &Item) -> String {
    let mut parts: Vec<String> = Vec::new();
    if item.kind.as_deref() == Some("episode") {
        parts.extend(item.parent_index.map(|season| format!("S{season}")));
        parts.extend(item.index.map(|number| format!("E{number}")));
    } else {
        parts.extend(item.year.map(|year| year.to_string()));
    }
    parts.extend(minutes_left(item.view_offset, item.duration).map(time_left));
    parts.join(" · ")
}

/// The pill names itself from the server's own `viewOffset`.
#[must_use]
pub fn resume_label(start: Option<f64>) -> &'static str {
    if start.is_some() {
        "Resume"
    } else {
        "Play"
    }
}

#[cfg(test)]
mod tests;
