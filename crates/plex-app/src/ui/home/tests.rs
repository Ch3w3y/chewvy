//! Home label rules read over the captured home hub answer and the captured
//! on-deck listing: the progress bar fraction, the remaining-minutes label, the
//! card caption, and which shelf the hero comes from.
//!
//! The wire names here are the names those captures use, because
//! `plex_core`'s hub parser is crate-internal.

use plex_core::library::{Hub, Item, ItemImage};
use serde_json::Value;

use super::*;

fn fixture(name: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../plex-core/tests/fixtures/real/{name}"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn text(row: &Value, field: &str) -> Option<String> {
    row.get(field).and_then(Value::as_str).map(str::to_owned)
}

fn number(row: &Value, field: &str) -> Option<u64> {
    row.get(field).and_then(Value::as_u64)
}

fn wire_item(row: &Value) -> Item {
    let images = row
        .get("Image")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|image| {
                    let url = image.get("url").and_then(Value::as_str)?;
                    Some(ItemImage {
                        image_type: image.get("type").and_then(Value::as_str).map(str::to_owned),
                        url: Some(url.to_owned()),
                        alt: image.get("alt").and_then(Value::as_str).map(str::to_owned),
                    })
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
        grandparent_art: text(row, "grandparentArt"),
        parent_thumb: text(row, "parentThumb"),
        grandparent_thumb: text(row, "grandparentThumb"),
        thumb: text(row, "thumb"),
        art: text(row, "art"),
        year: number(row, "year"),
        summary: text(row, "summary"),
        content_rating: text(row, "contentRating"),
        view_offset: number(row, "viewOffset"),
        duration: number(row, "duration"),
        images,
        ..Item::default()
    }
}

/// `MediaContainer.Hub[]` of one capture; a capture with no `Hub` (the plain
/// on-deck listing) maps to no shelves.
fn wire_hubs(capture: &Value) -> Vec<Hub> {
    capture
        .get("MediaContainer")
        .and_then(|mc| mc.get("Hub"))
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .map(|hub| Hub {
                    hub_id: text(hub, "hubIdentifier"),
                    path: text(hub, "key"),
                    item_keys: text(hub, "hubKey"),
                    title: text(hub, "title"),
                    kind: text(hub, "type"),
                    available: number(hub, "size").map(|size| size as usize),
                    context: text(hub, "context"),
                    style: text(hub, "style"),
                    more: hub.get("more").and_then(Value::as_bool),
                    promoted: hub.get("promoted").and_then(Value::as_bool),
                    items: hub
                        .get("Metadata")
                        .and_then(Value::as_array)
                        .map(|rows| rows.iter().map(wire_item).collect())
                        .unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn home() -> Vec<Hub> {
    wire_hubs(&fixture("hubs_home.json"))
}

/// One captured row by rating key.
fn row(hubs: &[Hub], rating_key: &str) -> Item {
    hubs.iter()
        .flat_map(|hub| hub.items.iter())
        .find(|item| item.rating_key.as_deref() == Some(rating_key))
        .cloned()
        .unwrap_or_else(|| panic!("the capture has no item {rating_key}"))
}

#[test]
fn the_capture_names_three_home_shelves() {
    let hubs = home();
    let ids: Vec<String> = hubs.iter().filter_map(|h| h.hub_id.clone()).collect();
    assert_eq!(ids, ["home.continue", "home.ondeck", "home.movies.recent"]);
    assert_eq!(
        hubs.iter().map(|h| h.items.len()).collect::<Vec<_>>(),
        [3, 3, 3]
    );
}

#[test]
fn the_progress_bar_reads_the_servers_own_offsets() {
    let hubs = home();
    let episode = row(&hubs, "43492");
    let fraction = progress_fraction(episode.view_offset, episode.duration)
        .unwrap_or_else(|| panic!("a resumed episode must report a fraction"));
    assert!((f64::from(fraction) - 0.2296).abs() < 0.0005, "{fraction}");
    let movie = row(&hubs, "14431");
    let fraction = progress_fraction(movie.view_offset, movie.duration)
        .unwrap_or_else(|| panic!("a resumed film must report a fraction"));
    assert!((f64::from(fraction) - 0.7840).abs() < 0.0005, "{fraction}");
}

#[test]
fn remaining_time_is_the_nearest_whole_minute() {
    let hubs = home();
    let episode = row(&hubs, "43492");
    assert_eq!(
        minutes_left(episode.view_offset, episode.duration),
        Some(32)
    );
    let movie = row(&hubs, "14431");
    assert_eq!(minutes_left(movie.view_offset, movie.duration), Some(22));
    assert_eq!(minutes_left(Some(0), Some(60_000)), None);
    assert_eq!(minutes_left(Some(60_000), Some(60_000)), None);
    assert_eq!(minutes_left(Some(1), None), None);
}

#[test]
fn a_card_caption_skips_an_index_the_answer_did_not_name() {
    let episode = row(&home(), "43492");
    assert_eq!(episode_caption(&episode), "S4 E2 · Blue Rafter");
    assert_eq!(wide_subtitle(&episode), "S4 · E2 · 32 min left");

    let mut season_less = episode.clone();
    season_less.parent_index = None;
    assert_eq!(episode_caption(&season_less), "E2 · Blue Rafter");
    season_less.index = None;
    season_less.content_rating = None;
    assert_eq!(episode_caption(&season_less), "Blue Rafter");
    assert_eq!(wide_subtitle(&season_less), "32 min left");
}

#[test]
fn the_hero_is_the_first_continue_watching_item() {
    let hubs = home();
    let hub = hero_hub(&hubs).unwrap_or_else(|| panic!("the capture names shelves"));
    assert_eq!(hub.hub_id.as_deref(), Some("home.continue"));
    assert_eq!(
        hero_item(&hubs)
            .and_then(|item| item.rating_key.clone())
            .as_deref(),
        Some("43492")
    );
}

#[test]
fn the_hero_falls_back_to_on_deck_when_no_continue_shelf_is_named() {
    let mut hubs = home();
    hubs.retain(|hub| hub.hub_id.as_deref() != Some("home.continue"));
    let hub = hero_hub(&hubs).unwrap_or_else(|| panic!("on deck remains"));
    assert_eq!(hub.hub_id.as_deref(), Some("home.ondeck"));
    assert_eq!(
        hero_item(&hubs)
            .and_then(|item| item.rating_key.clone())
            .as_deref(),
        Some("43492")
    );
}

#[test]
fn a_hub_is_chosen_by_its_identifier_not_by_shelf_order_or_wording() {
    let mut hubs = home();
    hubs.reverse();
    assert_eq!(
        hero_hub(&hubs)
            .and_then(|hub| hub.hub_id.clone())
            .as_deref(),
        Some("home.continue")
    );
    // A localized server still names the shelf; its title is a different language.
    let renamed: Vec<Hub> = hubs
        .iter()
        .map(|hub| Hub {
            title: Some(match hub.hub_id.as_deref() {
                Some("home.continue") => "Continue Watching".to_owned(),
                _ => "Zuletzt hinzugefügt".to_owned(),
            }),
            ..hub.clone()
        })
        .collect();
    assert_eq!(
        hero_hub(&renamed)
            .and_then(|hub| hub.hub_id.clone())
            .as_deref(),
        Some("home.continue")
    );
    // Only wording is left: no shelf is misread as continue because a movie
    // shelf's title says "Recently".
    let titled: Vec<Hub> = hubs
        .iter()
        .map(|hub| Hub {
            hub_id: None,
            style: None,
            title: Some(match hub.hub_id.as_deref() {
                Some("home.continue") => "Continue Watching".to_owned(),
                Some("home.ondeck") => "Next Up".to_owned(),
                _ => "Recently Added Movies".to_owned(),
            }),
            ..hub.clone()
        })
        .collect();
    assert_eq!(
        hero_hub(&titled)
            .and_then(|hub| hub.title.clone())
            .as_deref(),
        Some("Continue Watching")
    );
}

#[test]
fn the_plain_on_deck_listing_holds_no_shelves_but_the_same_resume_offsets() {
    let capture = fixture("on_deck.json");
    let hubs = wire_hubs(&capture);
    assert!(hubs.is_empty());
    let rows: Vec<Item> = capture
        .get("MediaContainer")
        .and_then(|mc| mc.get("Metadata"))
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("the capture names Metadata rows"))
        .iter()
        .map(wire_item)
        .collect();
    let started = rows
        .iter()
        .find(|item| item.rating_key.as_deref() == Some("43492"));
    assert_eq!(started.and_then(resume_start).map(|s| s as u64), Some(574));
    let fresh = rows
        .iter()
        .find(|item| item.rating_key.as_deref() == Some("43231"));
    assert_eq!(fresh.map(resume_start), Some(None));
    assert_eq!(
        fresh.map(|item| minutes_left(item.view_offset, item.duration)),
        Some(None)
    );
    // The hero and every card read the same fields the listing names.
    assert_eq!(
        rows.iter().filter(|item| item.duration.is_some()).count(),
        3
    );
}

#[test]
fn a_row_draws_every_card_in_the_shape_the_hub_names_not_the_item() {
    let hubs = home();
    // The capture fills `home.continue` with 1 episode (43492) plus 2 movies
    // (14431, 42793); the whole shelf is 16:9, so it stays top-aligned.
    let shapes: Vec<(Option<String>, RowShape)> = hubs
        .iter()
        .map(|hub| (hub.hub_id.clone(), row_shape(hub)))
        .collect();
    assert_eq!(
        shapes,
        vec![
            (Some("home.continue".to_owned()), RowShape::Wide),
            (Some("home.ondeck".to_owned()), RowShape::Wide),
            (Some("home.movies.recent".to_owned()), RowShape::Poster),
        ]
    );
    // Both cards in the mixed row take the row's 16:9 box.
    let sizes: Vec<([f32; 2], [u32; 2])> =
        hubs.iter().map(|hub| card_size(row_shape(hub))).collect();
    assert_eq!(
        sizes,
        vec![
            (CARD, CARD_ART),
            (CARD, CARD_ART),
            (MOVIE_CARD, MOVIE_CARD_ART)
        ]
    );
}

#[test]
fn the_backdrop_a_card_asks_for_is_the_one_the_capture_names() {
    let hubs = home();
    let episode = row(&hubs, "43492");
    assert_eq!(hero_art(&episode), episode.image_url("background"));
    // Every captured episode's `background` (and `art`) is the show's own
    // `grandparentArt` path, so the hero already gets show-level artwork for
    // episodes; no reordering of the fallbacks is warranted.
    assert_eq!(
        episode.image_url("background"),
        episode.grandparent_art.as_deref()
    );
    // A wide row takes the episode's own 16:9 `snapshot`, not its 2:3 `thumb`.
    assert_eq!(
        card_art(&episode, RowShape::Wide),
        episode.image_url("snapshot")
    );
    // A poster row takes the show's 2:3 `grandparentThumb`, never the episode's
    // own 16:9 `thumb`.
    assert_eq!(
        card_art(&episode, RowShape::Poster),
        episode.grandparent_thumb.as_deref()
    );

    // A film in that same wide row has no `snapshot`: it takes the
    // `background` the capture names, never its 2:3 poster `thumb`.
    let film = row(&hubs, "14431");
    assert_eq!(hero_art(&film), film.image_url("background"));
    assert_eq!(
        card_art(&film, RowShape::Wide),
        film.image_url("background")
    );
    assert_eq!(card_art(&film, RowShape::Poster), film.thumb.as_deref());
    assert_eq!(hero_logo(&film), film.image_url("clearLogo"));
    assert_eq!(hero_logo(&episode), None);

    // What an answer does not name is left out rather than guessed.
    let bare = Item {
        rating_key: Some("1".to_owned()),
        ..Item::default()
    };
    assert_eq!(hero_art(&bare), None);
    assert_eq!(card_art(&bare, RowShape::Wide), None);
    assert_eq!(card_art(&bare, RowShape::Poster), None);
    let show_backed = Item {
        rating_key: Some("2".to_owned()),
        grandparent_art: Some("/grandparent/art".to_owned()),
        ..Item::default()
    };
    assert_eq!(
        card_art(&show_backed, RowShape::Wide),
        Some("/grandparent/art")
    );
    assert_eq!(
        card_art(&show_backed, RowShape::Poster),
        Some("/grandparent/art")
    );
    assert_eq!(hero_art(&show_backed), Some("/grandparent/art"));
}

#[test]
fn a_resumed_episode_keeps_its_own_name_in_the_playing_title() {
    let hubs = home();
    let episode = row(&hubs, "43492");
    let show = episode
        .grandparent_title
        .clone()
        .unwrap_or_else(|| panic!("the captured episode names a grandparentTitle"));
    assert_eq!(
        play_title(&episode),
        format!("{show} - {}", episode_caption(&episode))
    );
    let film = row(&hubs, "14431");
    assert_eq!(
        play_title(&film),
        film.title
            .clone()
            .unwrap_or_else(|| panic!("the captured film names a title"))
    );
}

/// `Metadata[0]` of a capture that is not a hub answer: a show or a season
/// detail answer, wired exactly as a hub row is.
fn captured_row(name: &str) -> Item {
    let capture = fixture(name);
    wire_item(
        capture
            .get("MediaContainer")
            .and_then(|container| container.get("Metadata"))
            .and_then(Value::as_array)
            .and_then(|rows| rows.first())
            .unwrap_or_else(|| panic!("{name} names no Metadata[]")),
    )
}

/// F31: the poster one 2:3 card crops, from the artwork each capture names for
/// its kind. An episode names `grandparentThumb` (the show) and `thumb` (its own
/// 16:9 still); a season names `parentThumb` and no `grandparentThumb`; a movie
/// names only its own `thumb`.
#[test]
fn a_recently_added_poster_is_the_shows_poster_not_the_items_own_art() {
    let episode = row(&home(), "43492");
    assert_eq!(
        poster_thumb(&episode),
        episode.grandparent_thumb.as_deref(),
        "an episode's own thumb is its still, not the show's poster"
    );
    assert_ne!(poster_thumb(&episode), episode.thumb.as_deref());
    assert_eq!(poster_subtitle(&episode), "S4 E2 · Blue Rafter");

    let film = row(&home(), "14431");
    assert_eq!(film.kind.as_deref(), Some("movie"));
    assert_eq!(poster_thumb(&film), film.thumb.as_deref());
    assert_eq!(poster_subtitle(&film), "22 min left");

    let season = captured_row("show_children_seasons.json");
    assert_eq!(season.kind.as_deref(), Some("season"));
    assert_eq!(
        season.grandparent_thumb, None,
        "a season answer names no grandparentThumb"
    );
    assert_eq!(poster_thumb(&season), season.parent_thumb.as_deref());
    assert_ne!(poster_thumb(&season), season.thumb.as_deref());
    assert_eq!(poster_subtitle(&season), "S1");
}

/// A movie card's subtitle is the server's resume point when it has one and the
/// year when it does not; neither reading repeats the card's own title.
#[test]
fn a_movie_card_says_time_left_or_the_year_never_its_title() {
    let resumed = row(&home(), "14431");
    assert_eq!(
        minutes_left(resumed.view_offset, resumed.duration),
        Some(22)
    );
    assert_eq!(poster_subtitle(&resumed), "22 min left");
    assert_ne!(
        poster_subtitle(&resumed),
        resumed.title.clone().unwrap_or_default(),
        "a resume point must not read back as the title"
    );

    let capture = fixture("films_recently_added.json");
    let fresh = wire_item(
        capture
            .get("MediaContainer")
            .and_then(|container| container.get("Metadata"))
            .and_then(Value::as_array)
            .and_then(|rows| {
                rows.iter().find(|row| {
                    row.get("type").and_then(Value::as_str) == Some("movie")
                        && row.get("viewOffset").and_then(Value::as_u64).is_none()
                })
            })
            .unwrap_or_else(|| panic!("the capture names no movie without a resume point")),
    );
    assert_eq!(fresh.view_offset, None);
    assert_eq!(
        poster_subtitle(&fresh),
        fresh.year.unwrap_or_default().to_string()
    );
    assert_ne!(
        poster_subtitle(&fresh),
        fresh.title.clone().unwrap_or_default(),
        "a movie card never shows its own title twice"
    );
}

/// Home draws only shelves that hold video. The captured home answer is all
/// video; add the music and photo shelves a plex server also serves and a
/// playlist whose rows are tracks, and none of them counts as a shelf.
#[test]
fn home_draws_only_the_shelves_that_hold_video() {
    let shelf = |id: &str, hub_kind: &str, item_kind: &str| Hub {
        hub_id: Some(id.to_owned()),
        path: Some(format!("/hubs/{id}")),
        item_keys: None,
        title: Some(id.to_owned()),
        kind: Some(hub_kind.to_owned()),
        available: Some(1),
        context: Some(format!("hub.{id}")),
        style: Some("shelf".to_owned()),
        more: Some(false),
        promoted: Some(false),
        items: vec![Item {
            rating_key: Some(format!("rk-{id}")),
            kind: Some(item_kind.to_owned()),
            title: Some(format!("{item_kind} row")),
            ..Item::default()
        }],
    };
    let captured = home();
    assert!(
        captured.iter().all(shows_video),
        "the captured home answer holds nothing but video shelves"
    );

    let mut shelves: Vec<Hub> = captured.clone();
    shelves.extend([
        shelf("home.artists", "artist", "artist"),
        shelf("home.albums", "album", "album"),
        shelf("home.tracks", "track", "track"),
        shelf("home.photos", "photo", "photo"),
        shelf("home.playlist.audio", "playlist", "track"),
    ]);
    let drawn = shelves.iter().filter(|hub| shows_video(hub)).count();
    assert_eq!(
        drawn,
        captured.len(),
        "a music, photo or all-audio playlist shelf never reaches Home"
    );

    let video_playlist = shelf("home.playlist.videos", "playlist", "movie");
    shelves.push(video_playlist.clone());
    assert!(
        shows_video(&video_playlist),
        "a playlist whose rows are movies is a video shelf"
    );
    assert!(
        hero_hub(&[shelf("home.playlist.audio", "playlist", "track")]).is_none(),
        "a shelf holding no video carries no hero"
    );
}

/// F30: the hero's own words. The type word is the display label, never the
/// server's raw `type`, and only an episode names which episode the hero is.
#[test]
fn the_hero_labels_and_captions_the_kind_the_capture_names() {
    let episode = row(&home(), "43492");
    assert_eq!(type_label(&episode), Some("TV Show"));
    assert_eq!(
        hero_episode_line(&episode).as_deref(),
        Some("S4 E2 · Blue Rafter")
    );

    let film = row(&home(), "14431");
    assert_eq!(type_label(&film), Some("Movie"));
    assert_eq!(
        hero_episode_line(&film),
        None,
        "a movie hero is not an episode"
    );

    let show = captured_row("show_metadata.json");
    assert_eq!(show.kind.as_deref(), Some("show"));
    assert_eq!(type_label(&show), Some("TV Show"));
    assert_eq!(
        hero_episode_line(&show),
        None,
        "a show hero is not one episode"
    );
}

/// A second source names its two shelves as the connected server names its own,
/// so they read as the rows the user already knows — and keep the shape those
/// rows are drawn in.
#[test]
fn a_second_source_fills_the_rows_it_names_the_same_way() {
    let shelves = [
        Hub {
            hub_id: Some("home.continue".to_owned()),
            kind: Some("mixed".to_owned()),
            ..Default::default()
        },
        Hub {
            hub_id: Some("home.onDeck".to_owned()),
            kind: Some("episode".to_owned()),
            ..Default::default()
        },
    ];
    let shapes: Vec<RowShape> = shelves.iter().map(row_shape).collect();
    assert_eq!(shapes, [RowShape::Wide, RowShape::Wide]);
    // The hero stays the first shelf that names itself Continue Watching: the
    // connected server's own, which was asked for first.
    let mut hubs = home();
    hubs.extend(shelves);
    assert_eq!(
        hero_hub(&hubs)
            .and_then(|hub| hub.hub_id.clone())
            .as_deref(),
        Some("home.continue")
    );
}
