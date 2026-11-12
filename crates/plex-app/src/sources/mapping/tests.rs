//! Mapping tests, read over the live 12.4 capture J2a took: what a Jellyfin
//! row becomes as a browse row, which fields the captured rows leave unset, and
//! what the two home shelves of that server look like.
//!
//! The names here are the names that capture uses, because the mapping's whole
//! job is to answer for them.

use std::path::Path;

use serde_json::{json, Value};

use jellyfin_core::browse::image_address;

use super::*;

const SERVER: &str = "a57074336d774261bb9c10e93bbe1139";

/// The movie, the series and the episode of that capture, by their real ids.
const MOVIE: &str = "5197d80dc0c848286caeb3d345930689";
const SERIES: &str = "323fe870cae813165d6a87e8b47c9693";
const SEASON: &str = "600b7329d762180dc41c3b2c893a6526";
const EPISODE: &str = "28c2ea280a096582d8661f8ac0062da3";

fn fixture(name: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../jellyfin-core/tests/fixtures/live-12.4/{name}"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// One captured row — `/Items/{id}`, or one row of a captured listing.
fn row(name: &str, index: usize) -> JfItem {
    let value = fixture(name);
    // An item capture is the row; a listing capture lends one of its rows.
    let one = match name {
        "item_movie.json" | "item_series.json" | "item_episode.json" => value,
        _ => value
            .get("Items")
            .and_then(|rows| rows.get(index))
            .cloned()
            .unwrap_or(serde_json::Value::Null),
    };
    match serde_json::from_value(one) {
        Ok(row) => row,
        Err(why) => panic!("`{name}` does not parse as one item: {why}"),
    }
}

/// One captured listing whole, as a shelf.
fn shelf_from(name: &str, hub_id: &str, title: &str) -> Hub {
    let answer: QueryResult<JfItem> = match serde_json::from_value(fixture(name)) {
        Ok(answer) => answer,
        Err(why) => panic!("`{name}` does not parse as a listing: {why}"),
    };
    shelf(SERVER, hub_id, title, &answer)
}

/// Where a tagged image of a captured series lives on its server.
fn addressed(series: &str, kind: ImageKind, tag: &str) -> String {
    image_address(series, kind, tag)
        .unwrap_or_else(|| panic!("a tagged {kind:?} image has no address"))
}

fn named<'a>(items: &'a [Item], kind: &str) -> Option<&'a Item> {
    items.iter().find(|item| item.kind.as_deref() == Some(kind))
}

#[test]
fn a_captured_movie_row_arrives_as_a_playable_poster_card() {
    let item = item(SERVER, &row("item_movie.json", 0));
    assert_eq!(item.kind.as_deref(), Some("movie"));
    assert_eq!(item.title.as_deref(), Some("Aurora Test (2024)"));
    assert_eq!(item.year, Some(2024));
    // 300000000 ticks is 30 s, the whole of that generated test clip.
    assert_eq!(item.duration, Some(30_000));
    // Its id and its artwork name the server they came from, so a later read of
    // that row goes back to Jellyfin and not to the Plex session.
    assert_eq!(
        item.rating_key.as_deref(),
        Some(&*source::jellyfin_id(SERVER, MOVIE))
    );
    let thumb = item.thumb.clone().unwrap_or_else(|| {
        panic!("that movie carries a Primary tag");
    });
    assert_eq!(
        thumb,
        source::artwork_address(
            SERVER,
            &format!("/Items/{MOVIE}/Images/Primary?tag=f575702cdb1f1f0c154421dbe6b03885")
        )
    );
    // Never in the address: the token that authorises the artwork request.
    assert!(!thumb.to_lowercase().contains("token"));
    // The capture names no backdrop, no logo, no overview and no rating.
    assert_eq!(item.art, None);
    assert_eq!(item.summary, None);
    assert_eq!(item.genres, Vec::<String>::new());
    assert_eq!(item.content_rating, None);
    assert_eq!(item.image_url("coverPoster"), Some(thumb.as_str()));
    assert_eq!(item.image_url("clearLogo"), None);
    // Watched nothing, so there is no resume point and no watch count.
    assert_eq!(item.view_offset, None);
    assert_eq!(item.view_count, None);
}

#[test]
fn a_captured_episode_row_bring_its_season_and_its_series_with_it() {
    let item = item(SERVER, &row("item_episode.json", 0));
    assert_eq!(item.kind.as_deref(), Some("episode"));
    assert_eq!(item.title.as_deref(), Some("Starlight Signals S01E01"));
    assert_eq!(item.index, Some(1));
    assert_eq!(item.parent_index, Some(1));
    // The detail screen and the back-out key off these three, so they name the
    // server too — a season of Jellyfin opens as a page of Jellyfin.
    assert_eq!(item.parent_title.as_deref(), Some("Season 1"));
    assert_eq!(
        item.parent_rating_key.as_deref(),
        Some(&*source::jellyfin_id(SERVER, SEASON))
    );
    assert_eq!(item.grandparent_title.as_deref(), Some("Starlight Signals"));
    assert_eq!(
        item.grandparent_rating_key.as_deref(),
        Some(&*source::jellyfin_id(SERVER, SERIES))
    );
    assert_eq!(
        item.rating_key.as_deref(),
        Some(&*source::jellyfin_id(SERVER, EPISODE))
    );
    // The row's own `thumb` is the one its cards are drawn from.
    assert!(item
        .thumb
        .as_deref()
        .is_some_and(|thumb| thumb.starts_with(source::JELLYFIN_PREFIX)));
}

#[test]
fn a_captured_series_row_counts_the_leaves_the_grid_and_detail_want() {
    let item = item(SERVER, &row("item_series.json", 0));
    assert_eq!(item.kind.as_deref(), Some("show"));
    assert_eq!(item.title.as_deref(), Some("Starlight Signals"));
    // The ▶ badge and the detail progress bar read the pair: three leaves, none
    // of them watched.
    assert_eq!(item.leaf_count, Some(3));
    assert_eq!(item.viewed_leaf_count, Some(0));
    // That capture's series has no poster of its own — the placeholder card.
    assert_eq!(item.thumb, None);
    assert!(item.images.is_empty());
}

#[test]
fn the_resume_listing_arrives_as_one_shelf_of_the_rows_it_holds() {
    let hub = shelf_from(
        "user_items_resume.json",
        crate::sources::browse::RESUME_ID,
        crate::sources::browse::RESUME_TITLE,
    );
    assert_eq!(
        hub.hub_id.as_deref(),
        Some(crate::sources::browse::RESUME_ID)
    );
    assert_eq!(
        hub.title.as_deref(),
        Some(crate::sources::browse::RESUME_TITLE)
    );
    assert_eq!(hub.style.as_deref(), Some("shelf"));
    assert_eq!(hub.items.len(), 2);
    assert_eq!(hub.available, Some(2));
    // That answer resumes a partly-watched series by naming its season and the
    // series itself, so the shelf as a whole names no one kind — as the
    // captured Plex home answer does too.
    assert_eq!(hub.kind.as_deref(), Some("mixed"));
    assert!(named(&hub.items, "season").is_some());
    assert!(named(&hub.items, "show").is_some());
    // The season row points back at the series, and both are Jellyfin rows.
    let season =
        named(&hub.items, "season").unwrap_or_else(|| panic!("the season that answer names"));
    assert_eq!(season.index, Some(1));
    assert_eq!(season.parent_title.as_deref(), Some("Starlight Signals"));
    assert_eq!(
        season.parent_rating_key.as_deref(),
        Some(&*source::jellyfin_id(SERVER, SERIES))
    );
}

#[test]
fn the_nextup_listing_arrives_as_an_episode_shelf_with_posters() {
    let hub = shelf_from(
        "shows_nextup.json",
        crate::sources::browse::NEXTUP_ID,
        crate::sources::browse::NEXTUP_TITLE,
    );
    assert_eq!(
        hub.hub_id.as_deref(),
        Some(crate::sources::browse::NEXTUP_ID)
    );
    assert_eq!(hub.kind.as_deref(), Some("episode"));
    assert_eq!(hub.items.len(), 1);
    assert_eq!(hub.available, Some(1));
    for episode in &hub.items {
        assert_eq!(episode.kind.as_deref(), Some("episode"));
        assert!(episode
            .thumb
            .as_deref()
            .is_some_and(|thumb| thumb.starts_with(source::JELLYFIN_PREFIX)));
        assert_eq!(
            episode.grandparent_rating_key.as_deref(),
            Some(&*source::jellyfin_id(SERVER, SERIES))
        );
    }
}

#[test]
fn a_row_naming_what_the_screens_read_arrives_complete() {
    let row: JfItem = serde_json::from_value(json!({
        "Id": SERIES,
        "Name": "Starlight Signals",
        "Type": "Series",
        "Overview": "A band plays, a signal goes out.",
        "OfficialRating": "PG",
        "Genres": ["Music", "Drama"],
        "ImageTags": {"Primary": "p", "Logo": "l", "Thumb": "t"},
        "BackdropImageTags": ["b1", "b2"],
        "People": [
            {"Name": "Ada", "Role": "Singer", "Type": "Actor"},
            {"Name": "Grace", "Type": "Director"},
            {"Type": "Actor"}
        ],
        "UserData": {"PlaybackPositionTicks": 600000000, "PlayCount": 2, "UnplayedItemCount": 1},
        "RecursiveItemCount": 3
    }))
    .unwrap_or_else(|why| panic!("a minimal BaseItemDto: {why}"));
    let item = item(SERVER, &row);
    assert_eq!(
        item.summary.as_deref(),
        Some("A band plays, a signal goes out.")
    );
    assert_eq!(item.content_rating.as_deref(), Some("PG"));
    assert_eq!(item.genres, vec!["Music".to_owned(), "Drama".to_owned()]);
    // A credit with no name is not a cast member.
    let singer = item
        .roles
        .first()
        .unwrap_or_else(|| panic!("one credited role"));
    assert_eq!(singer.tag.as_deref(), Some("Ada"));
    assert_eq!(singer.role.as_deref(), Some("Singer"));
    assert_eq!(item.directors.len(), 1);
    assert_eq!(item.directors.first().map(String::as_str), Some("Grace"));
    // 600000000 ticks is 60 s of a resume point, and two finished watches.
    assert_eq!(item.view_offset, Some(60_000));
    assert_eq!(item.view_count, Some(2));
    assert_eq!(item.leaf_count, Some(3));
    assert_eq!(item.viewed_leaf_count, Some(2));
    // Every image role the hero and the cards ask for by name.
    for (role, kind, tag) in [
        ("coverPoster", ImageKind::Primary, "p"),
        ("background", ImageKind::Backdrop, "b1"),
        ("clearLogo", ImageKind::Logo, "l"),
        ("thumb", ImageKind::Thumb, "t"),
    ] {
        let url = item
            .image_url(role)
            .unwrap_or_else(|| panic!("no {role} artwork"));
        assert_eq!(
            url,
            source::artwork_address(SERVER, &addressed(SERIES, kind, tag))
        );
    }
}

#[test]
fn a_type_the_browse_model_has_no_kind_for_keeps_its_own_name() {
    let minimal = json!({"Id": "1", "Type": "Audio", "Name": "A song"});
    let row: JfItem = match serde_json::from_value(minimal) {
        Ok(row) => row,
        Err(why) => panic!("a minimal BaseItemDto: {why}"),
    };
    let item = item(SERVER, &row);
    // Listed as itself, which is neither a kind the grid opens nor one it plays.
    assert_eq!(item.kind.as_deref(), Some("audio"));
    assert_eq!(item.thumb, None);
    assert_eq!(item.leaf_count, None);
    assert_eq!(item.viewed_leaf_count, None);
}
