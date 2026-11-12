//! Detail page label rules read over the captured movie answer and the captured
//! show answer with its seasons: the metadata pills, the version badges, and
//! which season a show page opens on.
//!
//! The wire names here are the names those captures use, because `plex_core`'s
//! metadata parser is crate-internal.

use plex_core::library::{Item, ItemImage, Media, MediaPart, Role, Stream};
use serde_json::Value;

use super::*;
use crate::ui::theme;

fn fixture(name: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../plex-core/tests/fixtures/real/{name}"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// `MediaContainer.Metadata[0]` — the item one answer is about.
fn captured(name: &str) -> Value {
    fixture(name)
        .get("MediaContainer")
        .and_then(|mc| mc.get("Metadata"))
        .and_then(Value::as_array)
        .map(|rows| rows.first().cloned().unwrap_or_default())
        .unwrap_or_default()
}

fn text(row: &Value, field: &str) -> Option<String> {
    row.get(field).and_then(Value::as_str).map(str::to_owned)
}

fn number(row: &Value, field: &str) -> Option<u64> {
    row.get(field).and_then(Value::as_u64)
}

fn flag(row: &Value, field: &str) -> bool {
    row.get(field).and_then(Value::as_bool).unwrap_or(false)
}

fn tags(row: &Value, field: &str) -> Vec<String> {
    row.get(field)
        .and_then(Value::as_array)
        .map(|rows| rows.iter().filter_map(|entry| text(entry, "tag")).collect())
        .unwrap_or_default()
}

fn wire_stream(row: &Value) -> Stream {
    Stream {
        id: number(row, "id"),
        index: number(row, "index"),
        stream_type: number(row, "streamType"),
        codec: text(row, "codec"),
        display_title: text(row, "displayTitle"),
        language: text(row, "language"),
        language_tag: text(row, "languageTag"),
        channels: number(row, "channels"),
        selected: flag(row, "selected"),
        default: flag(row, "default"),
        forced: flag(row, "forced"),
        title: text(row, "title"),
        key: text(row, "key"),
    }
}

fn wire_part(row: &Value) -> MediaPart {
    MediaPart {
        key: text(row, "key"),
        container: text(row, "container"),
        size: number(row, "size"),
        duration: number(row, "duration"),
        streams: row
            .get("Stream")
            .and_then(Value::as_array)
            .map(|rows| rows.iter().map(wire_stream).collect())
            .unwrap_or_default(),
        ..MediaPart::default()
    }
}

fn wire_media(row: &Value) -> Vec<Media> {
    row.get("Media")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .map(|media| Media {
                    id: number(media, "id"),
                    duration: number(media, "duration"),
                    bitrate: number(media, "bitrate"),
                    video_resolution: text(media, "videoResolution"),
                    video_codec: text(media, "videoCodec"),
                    video_profile: text(media, "videoProfile"),
                    audio_codec: text(media, "audioCodec"),
                    audio_channels: number(media, "audioChannels"),
                    container: text(media, "container"),
                    parts: media
                        .get("Part")
                        .and_then(Value::as_array)
                        .map(|rows| rows.iter().map(wire_part).collect())
                        .unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn wire_item(row: &Value) -> Item {
    let images = row
        .get("Image")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .map(|image| ItemImage {
                    image_type: image.get("type").and_then(Value::as_str).map(str::to_owned),
                    url: image.get("url").and_then(Value::as_str).map(str::to_owned),
                    alt: image.get("alt").and_then(Value::as_str).map(str::to_owned),
                })
                .collect()
        })
        .unwrap_or_default();
    let roles = row
        .get("Role")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .map(|role| Role {
                    tag: text(role, "tag"),
                    role: text(role, "role"),
                    thumb: text(role, "thumb"),
                })
                .collect()
        })
        .unwrap_or_default();
    Item {
        rating_key: text(row, "ratingKey"),
        key: text(row, "key"),
        kind: text(row, "type"),
        title: text(row, "title"),
        index: number(row, "index"),
        parent_index: number(row, "parentIndex"),
        parent_title: text(row, "parentTitle"),
        parent_rating_key: text(row, "parentRatingKey"),
        grandparent_title: text(row, "grandparentTitle"),
        grandparent_rating_key: text(row, "grandparentRatingKey"),
        parent_thumb: text(row, "parentThumb"),
        grandparent_thumb: text(row, "grandparentThumb"),
        thumb: text(row, "thumb"),
        art: text(row, "art"),
        images,
        year: number(row, "year"),
        summary: text(row, "summary"),
        tagline: text(row, "tagline"),
        content_rating: text(row, "contentRating"),
        audience_rating: row.get("audienceRating").and_then(Value::as_f64),
        rating: row.get("rating").and_then(Value::as_f64),
        genres: tags(row, "Genre"),
        directors: tags(row, "Director"),
        roles,
        view_offset: number(row, "viewOffset"),
        view_count: number(row, "viewCount"),
        leaf_count: number(row, "leafCount"),
        viewed_leaf_count: number(row, "viewedLeafCount"),
        child_count: number(row, "childCount"),
        duration: number(row, "duration"),
        media: wire_media(row),
        ..Item::default()
    }
}

/// `MediaContainer.Metadata[]` of one `/children` capture.
fn wire_children(name: &str) -> Vec<Item> {
    fixture(name)
        .get("MediaContainer")
        .and_then(|mc| mc.get("Metadata"))
        .and_then(Value::as_array)
        .map(|rows| rows.iter().map(wire_item).collect())
        .unwrap_or_default()
}

fn movie() -> Item {
    wire_item(&captured("movie_metadata.json"))
}

fn show() -> Item {
    wire_item(&captured("show_metadata.json"))
}

#[test]
fn the_meta_line_names_the_answers_own_scalars() {
    let item = movie();
    let meta = super::head::meta_text(&item);
    assert!(meta.starts_with("1989"), "{meta}");
    assert!(meta.ends_with("1h 41m"), "{meta}");
    // A show's `duration` is one episode's length, so its line names no
    // runtime even though the answer carries a duration.
    let show = super::head::meta_text(&show());
    assert!(show.starts_with("2014"), "{show}");
    assert_eq!(show, "2014  ·  Drama, Mystery");
}

#[test]
fn a_resume_point_reads_as_a_clock() {
    assert_eq!(super::head::clock(5832.0), "1:37:12");
    assert_eq!(super::head::clock(754.0), "12:34");
    assert_eq!(super::head::clock(-3.0), "0:00");
}

#[test]
fn the_badges_name_the_version_the_player_opens() {
    assert_eq!(badges(&movie()), ["1080p", "H.264", "AAC · 2.0"]);
    assert_eq!(badges(&show()), Vec::<String>::new());
}

#[test]
fn a_movie_page_targets_the_movie_itself() {
    let page = Page::open(movie(), &[]);
    let target = play_target(&page).unwrap_or_else(|| panic!("a movie page names a target"));
    assert_eq!(target.rating_key, "30874");
    assert_eq!(target.title, "The 'Brook");
    assert_eq!(target.start, None);
    assert_eq!(page.pending(), None);
}

#[test]
fn a_show_page_opens_on_the_season_with_an_episode_left() {
    let children = wire_children("show_children_seasons.json");
    let page = Page::open(show(), &children);
    assert_eq!(
        page.seasons
            .iter()
            .map(|s| s.title.clone())
            .collect::<Vec<_>>(),
        vec![
            Some("Season 1".to_owned()),
            Some("Season 2".to_owned()),
            Some("Season 3".to_owned())
        ]
    );
    assert_eq!(page.episodes, Vec::<Item>::new());
    assert_eq!(page.season_key.as_deref(), Some("36099"));
    assert_eq!(page.pending(), Some("36099"));
    // The season the page opens on is the one its own answer listed first.
    assert_eq!(
        default_season(&children).and_then(|s| s.title.clone()),
        Some("Season 1".to_owned())
    );
    // No episodes read yet, so the page's pill has nothing to put in the player.
    assert_eq!(play_target(&page), None);
}

#[test]
fn a_season_page_fills_from_its_own_children_answer() {
    let children = wire_children("season_children_episodes.json");
    let season = wire_children("show_children_seasons.json")
        .into_iter()
        .find(|item| item.kind.as_deref() == Some("season"))
        .unwrap_or_else(|| panic!("the season capture lists no season"));
    let mut page = Page::open(season.clone(), &[]);
    assert_eq!(page.pending(), season.rating_key.as_deref());
    page.fill(season.rating_key.as_deref().unwrap_or_default(), &children);
    assert!(!page.episodes.is_empty());
    assert_eq!(page.pending(), None);
    let target =
        play_target(&page).unwrap_or_else(|| panic!("a filled season page names a target"));
    assert_eq!(target.rating_key, "36100");
    assert_eq!(target.title, "The 404 - S1 E1 · Pilot");
    assert_eq!(target.start, None);
    let listed = episodes(&children).len();
    assert_eq!(page.episodes.len(), listed);
}

/// One `/children`-style episode row carrying the two watch scalars none of the
/// captures put on a leaf: `viewOffset` and `viewCount`, spelled as the server
/// spells them on the items that do carry them.
fn wire_episode(
    index: u64,
    title: &str,
    view_offset: Option<u64>,
    view_count: Option<u64>,
) -> Item {
    let mut row = serde_json::json!({
        "ratingKey": format!("synthetic-episode-{index}"),
        "type": "episode",
        "index": index,
        "title": title,
        "duration": 2_476_480,
        "parentIndex": 1,
        "parentTitle": "Season 1",
        "parentRatingKey": "synthetic-season-1",
        "grandparentTitle": "The 404",
        "grandparentRatingKey": "synthetic-show-1",
    });
    for (name, value) in [("viewOffset", view_offset), ("viewCount", view_count)] {
        if let Some(count) = value {
            if let Some(fields) = row.as_object_mut() {
                fields.insert(name.to_owned(), serde_json::json!(count));
            }
        }
    }
    wire_item(&row)
}

/// A season page holding `episodes` as its own filled children answer.
fn season_page(episodes: &[Item]) -> Page {
    let season = wire_item(&serde_json::json!({
        "ratingKey": "synthetic-season-1",
        "type": "season",
        "index": 1,
        "title": "Season 1",
        "parentTitle": "The 404",
        "parentIndex": 1,
        "leafCount": 4,
        "viewedLeafCount": 2,
    }));
    let mut page = Page::open(season.clone(), &[]);
    page.fill("synthetic-season-1", episodes);
    page
}

#[test]
fn an_episode_row_heads_with_its_index_and_title() {
    let listed = wire_children("season_children_episodes.json");
    let episodes = episodes(&listed);
    assert_eq!(
        episodes
            .iter()
            .map(|&episode| episode_label(episode))
            .collect::<Vec<String>>(),
        ["E1", "E2", "E3"]
    );
    assert_eq!(
        episodes
            .iter()
            .map(|&episode| row_caption(episode))
            .collect::<Vec<String>>(),
        ["E1 · Pilot", "E2 · Ember Calls", "E3 · Ember Falls"]
    );
    // An answer naming no `index` names no prefix rather than a false `E0`.
    let Some(first) = episodes.first().copied() else {
        panic!("the capture lists no episode")
    };
    let mut unnamed = first.clone();
    unnamed.index = None;
    assert_eq!(episode_label(&unnamed), "");
    assert_eq!(row_caption(&unnamed), "Pilot");
}

#[test]
fn an_episode_row_names_its_own_runtime_in_whole_minutes() {
    let captured = wire_item(&captured("episode_metadata.json"));
    assert_eq!(episode_runtime(&captured).as_deref(), Some("41m"));
    assert_eq!(episode_meta(&captured), "41m");
    let listed = wire_children("season_children_episodes.json");
    assert_eq!(
        episodes(&listed)
            .iter()
            .map(|&episode| episode_runtime(episode))
            .collect::<Vec<Option<String>>>(),
        [
            Some("41m".to_owned()),
            Some("42m".to_owned()),
            Some("42m".to_owned())
        ]
    );
    assert_eq!(runtime(Some(6_060_000)).as_deref(), Some("1h 41m"));
    // A shorter answer than a minute, and an answer naming no duration at all,
    // name no runtime rather than inventing a `0m`.
    assert_eq!(runtime(Some(30_000)), None);
    assert_eq!(runtime(None), None);
    let mut no_duration = captured;
    no_duration.duration = None;
    assert_eq!(episode_meta(&no_duration), "");
}

#[test]
fn the_next_episode_of_a_wholly_unwatched_season_is_its_first() {
    let listed = wire_children("season_children_episodes.json");
    // Nothing in this capture carries `viewOffset` or `viewCount`, so every
    // leaf reads as unwatched and the season's own order decides.
    let next = next_episode(&listed).unwrap_or_else(|| panic!("the season lists no episode"));
    assert_eq!(next.rating_key.as_deref(), Some("36100"));
    assert_eq!(row_caption(next), "E1 · Pilot");
    let page = season_page(&listed);
    let target = play_target(&page).unwrap_or_else(|| panic!("the season page names a target"));
    assert_eq!(target.rating_key, "36100");
    assert_eq!(target.start, None);
    assert_eq!(pill_text(&page).as_deref(), Some("S1 E1 · Pilot"));
}

#[test]
fn a_resume_point_wins_only_when_the_server_does_not_count_that_episode_watched() {
    let listed = vec![
        // A leaf carrying both: counted watched, so its leftover offset is not
        // a claim that this is the episode in progress.
        wire_episode(1, "Pilot", Some(1_800_000), Some(1)),
        wire_episode(2, "Ember Calls", None, Some(1)),
        wire_episode(3, "Ember Falls", Some(600_000), None),
        wire_episode(4, "Waste Land", None, None),
    ];
    let next = next_episode(&listed).unwrap_or_else(|| panic!("the season lists no episode"));
    assert_eq!(row_caption(next), "E3 · Ember Falls");
    let page = season_page(&listed);
    let target = play_target(&page).unwrap_or_else(|| panic!("the season page names a target"));
    assert_eq!(target.title, "The 404 - S1 E3 · Ember Falls");
    assert_eq!(target.start, Some(600.0));
    assert_eq!(pill_text(&page).as_deref(), Some("Resume · 31 min left"));
    // Once the episode in progress is itself watched, the first unwatched one
    // is what the pill plays.
    let mut watched_resume = listed.clone();
    if let Some(row) = watched_resume.get_mut(2) {
        row.view_count = Some(1);
    } else {
        panic!("the listed season holds no third episode");
    }
    let next = next_episode(&watched_resume).unwrap_or_else(|| panic!("no episode left"));
    assert_eq!(row_caption(next), "E4 · Waste Land");
}

#[test]
fn a_resume_offset_draws_the_bar_and_a_view_count_draws_the_tick() {
    let started = wire_episode(1, "Pilot", Some(1_238_240), None);
    let fresh = wire_episode(2, "Ember Calls", None, None);
    let seen = wire_episode(3, "Ember Falls", None, Some(2));
    // What an episode row reads for its thumbnail's resume bar and its tick.
    assert_eq!(
        home::progress_fraction(started.view_offset, started.duration),
        Some(0.5)
    );
    assert_eq!(
        home::progress_fraction(fresh.view_offset, fresh.duration),
        None
    );
    assert_eq!(
        home::progress_fraction(seen.view_offset, seen.duration),
        None
    );
    assert!(!watched(&started));
    assert!(!watched(&fresh));
    assert!(watched(&seen));
    assert_eq!(episode_pill_text(&started), "Resume · 21 min left");
    assert_eq!(episode_pill_text(&fresh), "Play");
    assert_eq!(episode_pill_text(&seen), "Play");
}

/// Paint one episode row on its own, in a frame the width of a normal window,
/// with an artwork loader that has nothing to fetch: the row falls back to its
/// flat [`EPISODE_THUMB`] plate.
fn painted_row(episode: &Item) -> egui::FullOutput {
    let ctx = egui::Context::default();
    ctx.set_fonts(crate::ui::theme::fonts());
    let mut artwork = crate::ui::images::Artwork::with_cache(None, 4);
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1_280.0, 720.0),
        )),
        ..egui::RawInput::default()
    };
    let mut frame = ctx.run_ui(input, |ui| {
        let _ = drawing::episode_row(ui, episode, &mut artwork, false);
    });
    // No render backend here: epaint 0.36.1 `debug_assert`s that a dropped
    // `TexturesDelta` was cleared first (`epaint-0.36.1/src/textures.rs:335-341`).
    frame.textures_delta.clear();
    frame
}

fn painted_rects(out: &egui::FullOutput, fill: egui::Color32) -> Vec<egui::Rect> {
    out.shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::Rect(rect) if rect.fill == fill && rect.stroke.is_empty() => {
                Some(rect.rect)
            }
            _ => None,
        })
        .collect()
}

/// The row's [`EPISODE_THUMB`] plate, which the resume bar and the tick sit on.
fn thumb_plate(out: &egui::FullOutput) -> egui::Rect {
    painted_rects(out, theme::CARD)
        .into_iter()
        .find(|rect| rect.width() == EPISODE_THUMB[0] && rect.height() == EPISODE_THUMB[1])
        .unwrap_or_else(|| {
            panic!(
                "the row painted no {}x{} plate",
                EPISODE_THUMB[0], EPISODE_THUMB[1]
            )
        })
}

/// The watched tick's plate: the only filled circle in the frame.
fn tick_plate(out: &egui::FullOutput) -> Option<(egui::Pos2, f32)> {
    out.shapes.iter().find_map(|clipped| match &clipped.shape {
        egui::Shape::Circle(circle) if circle.stroke.is_empty() => {
            Some((circle.center, circle.radius))
        }
        _ => None,
    })
}

#[test]
fn a_half_watched_episode_row_paints_the_resume_bar_on_its_thumb_edge() {
    let started = wire_episode(1, "Pilot", Some(1_238_240), None);
    let frame = painted_row(&started);
    let plate = thumb_plate(&frame);
    let bars = painted_rects(&frame, theme::ACCENT);
    let Some(bar) = bars.first() else {
        panic!("a row whose offset the server named painted no resume bar");
    };
    assert_eq!(bars.len(), 1, "one bar, not a second one beside it");
    assert_eq!(bar.height(), BAR_HEIGHT);
    assert!(
        (bar.min.x - plate.min.x).abs() < 0.01,
        "the bar starts at the thumb's left edge: {bar:?} vs {plate:?}"
    );
    assert!(
        (plate.max.y - bar.max.y).abs() < 0.01,
        "the bar lies along the thumb's bottom edge: {bar:?} vs {plate:?}"
    );
    assert!(
        (bar.width() - plate.width() * 0.5).abs() < 0.5,
        "the bar covers the half the offset and duration give: {bar:?}"
    );
    assert!(
        tick_plate(&frame).is_none(),
        "a row the server does not count watched paints no tick plate"
    );
}

#[test]
fn a_watched_episode_row_paints_the_tick_over_its_thumb_corner() {
    let seen = wire_episode(3, "Ember Falls", None, Some(2));
    let frame = painted_row(&seen);
    let plate = thumb_plate(&frame);
    assert!(
        painted_rects(&frame, theme::ACCENT).is_empty(),
        "a row with no `viewOffset` paints no bar"
    );
    let Some((center, radius)) = tick_plate(&frame) else {
        panic!("a row the server counts watched painted no tick plate");
    };
    assert!(radius > 0.0);
    assert!(
        (center.x - (plate.max.x - 14.0)).abs() < 0.01
            && (center.y - (plate.min.y + 14.0)).abs() < 0.01,
        "the tick sits over the thumb's top-right corner: {center:?} vs {plate:?}"
    );
}

#[test]
fn an_untouched_episode_row_paints_neither_bar_nor_tick() {
    let fresh = wire_episode(2, "Ember Calls", None, None);
    let frame = painted_row(&fresh);
    let _ = thumb_plate(&frame);
    assert!(
        painted_rects(&frame, theme::ACCENT).is_empty(),
        "no offset, no bar"
    );
    assert!(tick_plate(&frame).is_none(), "no view count, no tick plate");
}

#[test]
fn a_cast_link_searches_imdb_by_name_only() {
    assert_eq!(
        imdb_search("Zoe Saldaña"),
        "https://www.imdb.com/find/?q=Zoe+Salda%C3%B1a&s=nm"
    );
    assert_eq!(
        imdb_search(" CCH Pounder "),
        "https://www.imdb.com/find/?q=CCH+Pounder&s=nm"
    );
    assert_eq!(
        imdb_search("a&b"),
        "https://www.imdb.com/find/?q=a%26b&s=nm"
    );
}

#[test]
fn a_season_chip_names_the_season_and_progress_stands_apart() {
    let mut season = Item {
        title: Some("Season 1".to_owned()),
        leaf_count: Some(5),
        viewed_leaf_count: Some(0),
        ..Item::default()
    };
    assert_eq!(season_label(&season), "Season 1");
    assert_eq!(season_progress(&season).as_deref(), Some("5 episodes"));
    season.viewed_leaf_count = Some(2);
    assert_eq!(season_progress(&season).as_deref(), Some("2 of 5 watched"));
    season.viewed_leaf_count = Some(5);
    assert_eq!(season_progress(&season).as_deref(), Some("Watched"));
}

#[test]
fn browsing_another_season_keeps_the_shows_next_up() {
    let item = |kind: &str, key: &str, index: u64, viewed: Option<u64>| Item {
        kind: Some(kind.to_owned()),
        rating_key: Some(key.to_owned()),
        index: Some(index),
        view_count: viewed,
        ..Item::default()
    };
    let show = item("show", "s", 0, None);
    let mut page = Page::open(
        show,
        &[item("season", "s1", 1, None), item("season", "s2", 2, None)],
    );
    page.fill(
        "s1",
        &[
            item("episode", "e11", 1, Some(1)),
            item("episode", "e12", 2, None),
        ],
    );
    assert_eq!(
        play_item(&page).and_then(|i| i.rating_key.as_deref()),
        Some("e12")
    );
    page.select_season("s2");
    page.fill("s2", &[item("episode", "e21", 1, None)]);
    assert_eq!(page.episodes.len(), 1);
    assert_eq!(
        play_item(&page).and_then(|i| i.rating_key.as_deref()),
        Some("e12")
    );
}
