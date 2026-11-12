//! **core endpoint** tests: the captured answer each new read-only call
//! parses — home hubs, search hubs, on-deck, a show's seasons, a season's
//! episodes, a section's sorts.
//!
//! Every assertion here runs against real loopback HTTP: a mock PMS
//! (`tests/mock/mod.rs`) answers with the **captured bytes** in
//! `tests/fixtures/real/` (recorded from a live Plex Media Server through this
//! client's own transport), `plex_core::server::connect` connects to it with
//! that row's own token, and the product call under test parses the answer. So
//! a model that names a field the server does not send, or misses one it does,
//! fails here, and each route is driven with traffic the mock actually recorded
//! (`GET` only, credential in the `X-Plex-Token` header only; see
//! `tests/p1b/mod.rs` for the request-line guards).
//!
//! This is **not** a Plex integration test: whether a live server answers the
//! same way for this account is a live-driver check, not something a replay of
//! recorded bytes can prove.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)] // test scaffolding; product code keeps these lints on

mod common;
mod mock;
mod p1b;

use plex_core::library::{
    children, hubs, on_deck, search_hubs, sorts, validate_sort, CHILDREN_PATH, HUBS_PATH,
    ON_DECK_PATH, QUERY_HUB_COUNT, QUERY_SEARCH_LIMIT, SEARCH_HUBS_PATH, SECTION_SORTS_PATH,
};

use crate::mock::Route;
use crate::p1b::{
    capture_script, driven, p1b_session, pair, query_pairs, CAPTURED_SEASON, CAPTURED_SECTION,
    CAPTURED_SHOW,
};

#[test]
fn home_hubs_are_the_captured_shelves_with_their_items() {
    let (pms, server) = p1b_session("p1b-hubs-home");
    pms.set_script(capture_script(Route::Hubs, "real/hubs_home.json"));
    let baseline = pms.requests().len();

    let shelves = hubs(&server, 6).expect("the captured home answer must parse as hubs");
    let calls = driven(&pms, baseline, HUBS_PATH);
    assert_eq!(
        query_pairs(&calls[0]),
        vec![pair(QUERY_HUB_COUNT, "6")],
        "/hubs is asked with count only (no credential)"
    );

    let ids: Vec<&str> = shelves
        .iter()
        .map(|hub| hub.hub_id.as_deref().unwrap_or_default())
        .collect();
    assert_eq!(ids, ["home.continue", "home.ondeck", "home.movies.recent"]);
    let titles: Vec<&str> = shelves
        .iter()
        .map(|hub| hub.title.as_deref().unwrap_or_default())
        .collect();
    assert_eq!(
        titles,
        ["Continue Watching", "On Deck", "Recently Added Movies"]
    );
    let kinds: Vec<&str> = shelves
        .iter()
        .map(|hub| hub.kind.as_deref().unwrap_or_default())
        .collect();
    assert_eq!(kinds, ["mixed", "episode", "movie"]);
    let styles: Vec<&str> = shelves
        .iter()
        .map(|hub| hub.style.as_deref().unwrap_or_default())
        .collect();
    assert_eq!(styles, ["hero", "shelf", "shelf"]);
    let contexts: Vec<&str> = shelves
        .iter()
        .map(|hub| hub.context.as_deref().unwrap_or_default())
        .collect();
    assert_eq!(
        contexts,
        [
            "hub.home.continue",
            "hub.home.ondeck",
            "hub.home.movies.recent"
        ]
    );
    assert!(
        shelves.iter().all(|hub| hub.more == Some(true)),
        "every captured home hub offers a full list"
    );
    assert!(
        shelves.iter().all(|hub| hub.promoted == Some(true)),
        "the home answer names promoted on every shelf"
    );
    assert_eq!(
        shelves[0].path.as_deref(),
        Some("/hubs/home/continueWatching"),
        "a hub path is the hub's own request path"
    );
    assert_eq!(
        shelves[2].path.as_deref(),
        Some("/hubs/home/recentlyAdded?type=1")
    );

    for hub in &shelves {
        assert_eq!(hub.available, Some(6), "each home shelf declares size 6");
        assert_eq!(
            hub.items.len(),
            3,
            "the capture embeds 3 of the 6 rows the shelf declares"
        );
        let keys = hub
            .item_keys
            .as_deref()
            .expect("a home shelf names a hubKey");
        assert!(
            keys.starts_with("/library/metadata/"),
            "hubKey is a metadata key list, not a hub path"
        );
        assert_eq!(
            keys.split(',').count(),
            6,
            "the declared shelf size and the hubKey entry count agree here"
        );
    }

    let first = &shelves[0].items[0];
    assert_eq!(first.rating_key.as_deref(), Some("43492"));
    assert_eq!(first.title.as_deref(), Some("Blue Rafter"));
    assert_eq!(first.kind.as_deref(), Some("episode"));
    assert_eq!(first.grandparent_title.as_deref(), Some("Tolland Baron"));
    assert_eq!(first.view_offset, Some(574_000), "a partially watched row");
    assert_eq!(shelves[0].items[1].rating_key.as_deref(), Some("14431"));
    assert_eq!(shelves[0].items[1].kind.as_deref(), Some("movie"));
    assert_eq!(shelves[0].items[1].view_offset, Some(4_714_000));

    // The row fields a home shelf is drawn from: the caption indices, the
    // resume maths, and the artwork kinds a backdrop or logo lookup asks for.
    let film = &shelves[0].items[1];
    assert_eq!((first.index, first.parent_index), (Some(2), Some(4)));
    assert_eq!(first.duration, Some(2_499_936));
    assert_eq!(
        (first.year, first.content_rating.as_deref()),
        (Some(2026), Some("TV-MA"))
    );
    assert_eq!(
        first
            .images
            .iter()
            .filter_map(|image| image.image_type.as_deref())
            .collect::<Vec<_>>(),
        ["coverPoster", "snapshot", "background"],
        "the capture names the backdrop kind the hero asks for"
    );
    assert!(first.image_url("background").is_some());
    assert!(first.image_url("clearLogo").is_none());
    assert_eq!((film.index, film.parent_index), (None, None));
    assert_eq!(film.duration, Some(6_012_736));
    assert_eq!(
        (film.year, film.content_rating.as_deref()),
        (Some(2013), Some("gb/15"))
    );
    assert!(film.image_url("clearLogo").is_some());

    let added: Vec<i64> = shelves[2]
        .items
        .iter()
        .map(|item| item.added_at.unwrap_or_default())
        .collect();
    assert_eq!(
        added,
        [1_789_252_738, 1_788_933_530, 1_788_926_209],
        "recently-added shelves keep the capture's order"
    );
}

#[test]
fn search_hubs_group_the_matches_by_type() {
    let (pms, server) = p1b_session("p1b-hubs-search");
    pms.set_script(capture_script(Route::HubsSearch, "real/hubs_search.json"));
    let baseline = pms.requests().len();

    let found = search_hubs(&server, "Aphelion", 5).expect("the captured search answer must parse");
    let calls = driven(&pms, baseline, SEARCH_HUBS_PATH);
    assert_eq!(
        query_pairs(&calls[0]),
        vec![pair("query", "Aphelion"), pair(QUERY_SEARCH_LIMIT, "5")],
        "/hubs/search is asked with the query and its per-hub limit"
    );

    let ids: Vec<&str> = found
        .iter()
        .map(|hub| hub.hub_id.as_deref().unwrap_or_default())
        .collect();
    assert_eq!(ids, ["movie", "show", "artist"]);
    let titles: Vec<&str> = found
        .iter()
        .map(|hub| hub.title.as_deref().unwrap_or_default())
        .collect();
    assert_eq!(titles, ["Movies", "Shows", "Artists"]);
    let kinds: Vec<&str> = found
        .iter()
        .map(|hub| hub.kind.as_deref().unwrap_or_default())
        .collect();
    assert_eq!(kinds, ["movie", "show", "artist"]);
    let rows: Vec<usize> = found.iter().map(|hub| hub.items.len()).collect();
    assert_eq!(
        rows,
        [3, 1, 0],
        "the group the server found nothing for is still a group"
    );
    let declared: Vec<Option<usize>> = found.iter().map(|hub| hub.available).collect();
    assert_eq!(declared, [Some(4), Some(1), Some(0)]);
    for hub in &found {
        assert_eq!(
            hub.context, None,
            "a search hub answers context \"\" as no context"
        );
        assert_eq!(hub.style.as_deref(), Some("shelf"));
        assert_eq!(
            hub.more,
            Some(false),
            "the captured search hubs name more=false"
        );
        assert_eq!(hub.promoted, None, "the search answer names no promoted");
        assert_eq!(hub.path, None, "the search answer names no hub key path");
        assert_eq!(hub.item_keys, None, "the search answer names no hubKey");
    }

    let movie_keys: Vec<&str> = found[0]
        .items
        .iter()
        .map(|item| item.rating_key.as_deref().unwrap_or_default())
        .collect();
    assert_eq!(movie_keys, ["2184", "8662", "4646"]);
    assert_eq!(found[0].items[0].title.as_deref(), Some("Aphelion"));
    assert_eq!(found[0].items[0].library_section_id, Some(2));
    assert_eq!(found[1].items[0].title.as_deref(), Some("Aphelion: Ember"));
    assert_eq!(found[1].items[0].kind.as_deref(), Some("show"));
    assert_eq!(found[1].items[0].library_section_id, Some(1));
    assert!(found[2].items.is_empty());
}

#[test]
fn on_deck_is_one_declared_page_of_mixed_rows() {
    let (pms, server) = p1b_session("p1b-on-deck");
    pms.set_script(capture_script(Route::OnDeck, "real/on_deck.json"));
    let baseline = pms.requests().len();

    let deck = on_deck(&server).expect("the captured on-deck answer must parse as a page");
    let calls = driven(&pms, baseline, ON_DECK_PATH);
    assert!(
        query_pairs(&calls[0]).is_empty(),
        "on-deck is asked without a query"
    );

    assert_eq!(
        deck.items.len(),
        3,
        "the capture embeds 3 of 4 declared rows"
    );
    assert_eq!(deck.total, Some(21), "on-deck reports a windowed totalSize");
    assert_eq!(deck.start, Some(0));
    assert_eq!(
        deck.identifier.as_deref(),
        Some("com.plexapp.plugins.library")
    );

    let kinds: Vec<&str> = deck
        .items
        .iter()
        .map(|item| item.kind.as_deref().unwrap_or_default())
        .collect();
    assert_eq!(
        kinds,
        ["episode", "episode", "movie"],
        "on-deck is not episode-only: the capture holds a movie too"
    );
    let offsets: Vec<Option<u64>> = deck.items.iter().map(|item| item.view_offset).collect();
    assert_eq!(
        offsets,
        [Some(574_000), None, Some(4_714_000)],
        "viewOffset is named on some rows and null on another"
    );
    let titles: Vec<&str> = deck
        .items
        .iter()
        .map(|item| item.title.as_deref().unwrap_or_default())
        .collect();
    assert_eq!(titles, ["Blue Rafter", "Boat Yard", "Moth"]);
    assert_eq!(
        deck.items[0].grandparent_title.as_deref(),
        Some("Tolland Baron")
    );
    assert_eq!(deck.items[0].parent_title.as_deref(), Some("Season 4"));
    assert_eq!(deck.items[0].index, Some(2));
    assert_eq!(deck.items[0].parent_index, Some(4));
    assert_eq!(
        deck.items[1].grandparent_title.as_deref(),
        Some("The Five Harbours (2025)")
    );
    assert_eq!(deck.items[1].index, Some(4));
    assert_eq!(deck.items[2].kind.as_deref(), Some("movie"));
    assert_eq!(
        deck.items[2].parent_title, None,
        "a movie row names no parent season"
    );
    assert_eq!(deck.items[2].index, None);
    let sections: Vec<Option<u64>> = deck
        .items
        .iter()
        .map(|item| item.library_section_id)
        .collect();
    assert_eq!(sections, [Some(1), Some(1), Some(2)]);
}

#[test]
fn children_of_a_show_are_its_seasons_without_the_browse_directory() {
    let (pms, server) = p1b_session("p1b-children-seasons");
    pms.set_script(capture_script(
        Route::Children(CAPTURED_SHOW.to_owned()),
        "real/show_children_seasons.json",
    ));
    let baseline = pms.requests().len();

    let kids =
        children(&server, CAPTURED_SHOW).expect("the captured show children answer must parse");
    let calls = driven(
        &pms,
        baseline,
        &format!("{CHILDREN_PATH}/{CAPTURED_SHOW}/children"),
    );
    assert!(
        query_pairs(&calls[0]).is_empty(),
        "children are asked without a query"
    );

    assert_eq!(
        kids.items.len(),
        3,
        "the capture embeds 3 of 8 declared rows"
    );
    assert_eq!(
        kids.available,
        Some(8),
        "the show's container declares size 8"
    );
    assert_eq!(
        kids.total, None,
        "a children answer names no totalSize, so it is not a paged listing"
    );
    assert_eq!(
        kids.identifier.as_deref(),
        Some("com.plexapp.plugins.library")
    );

    let titles: Vec<&str> = kids
        .items
        .iter()
        .map(|item| item.title.as_deref().unwrap_or_default())
        .collect();
    assert_eq!(titles, ["Season 1", "Season 2", "Season 3"]);
    assert!(
        kids.items
            .iter()
            .all(|item| item.key.as_deref() != Some("/library/metadata/35773/allLeaves")),
        "the capture's navigational Directory row (\"All episodes\") is not a child"
    );
    assert!(
        kids.items
            .iter()
            .all(|item| item.rating_key.is_some() && item.parent_rating_key.is_some()),
        "a child row is an item with its parent"
    );
    let index: Vec<u64> = kids
        .items
        .iter()
        .map(|item| item.index.unwrap_or_default())
        .collect();
    assert_eq!(index, [1, 2, 3]);
    let leaf: Vec<u64> = kids
        .items
        .iter()
        .map(|item| item.leaf_count.unwrap_or_default())
        .collect();
    assert_eq!(leaf, [13, 16, 16]);
    let viewed: Vec<u64> = kids
        .items
        .iter()
        .map(|item| item.viewed_leaf_count.unwrap_or_default())
        .collect();
    assert_eq!(viewed, [0, 0, 0]);
    let keys: Vec<&str> = kids
        .items
        .iter()
        .map(|item| item.rating_key.as_deref().unwrap_or_default())
        .collect();
    assert_eq!(keys, ["36099", "35774", "35791"]);
    assert!(kids
        .items
        .iter()
        .all(|item| item.parent_rating_key.as_deref() == Some(CAPTURED_SHOW)));
    assert_eq!(
        kids.items[0].key.as_deref(),
        Some("/library/metadata/36099/children"),
        "a season row names where its own children live"
    );
}

#[test]
fn children_of_a_season_are_its_episodes() {
    let (pms, server) = p1b_session("p1b-children-episodes");
    pms.set_script(capture_script(
        Route::Children(CAPTURED_SEASON.to_owned()),
        "real/season_children_episodes.json",
    ));
    let baseline = pms.requests().len();

    let kids =
        children(&server, CAPTURED_SEASON).expect("the captured season children answer must parse");
    let calls = driven(
        &pms,
        baseline,
        &format!("{CHILDREN_PATH}/{CAPTURED_SEASON}/children"),
    );
    assert_eq!(calls[0].method, "GET");

    assert_eq!(
        kids.items.len(),
        3,
        "the capture embeds 3 of 13 declared rows"
    );
    assert_eq!(kids.available, Some(13));
    assert_eq!(kids.total, None);
    assert_eq!(
        kids.identifier.as_deref(),
        Some("com.plexapp.plugins.library")
    );

    let titles: Vec<&str> = kids
        .items
        .iter()
        .map(|item| item.title.as_deref().unwrap_or_default())
        .collect();
    assert_eq!(titles, ["Pilot", "Ember Calls", "Ember Falls"]);
    let index: Vec<u64> = kids
        .items
        .iter()
        .map(|item| item.index.unwrap_or_default())
        .collect();
    assert_eq!(index, [1, 2, 3]);
    assert!(kids.items.iter().all(|item| item.parent_index == Some(1)));
    assert!(kids
        .items
        .iter()
        .all(|item| item.parent_title.as_deref() == Some("Season 1")));
    let thumbs: Vec<Option<&str>> = kids
        .items
        .iter()
        .map(|item| item.thumb.as_deref())
        .collect();
    assert_eq!(
        thumbs,
        [
            Some("/library/metadata/36100/thumb/1774055077"),
            Some("/library/metadata/36101/thumb/1774055088"),
            Some("/library/metadata/36102/thumb/1774055069"),
        ],
        "episode rows name the artwork paths an artwork request would use"
    );
    assert!(kids.items.iter().all(|item| item.leaf_count.is_none()));
    assert_eq!(kids.items[0].year, Some(2014));
}

#[test]
fn a_section_names_the_sorts_it_offers() {
    let (pms, server) = p1b_session("p1b-sorts");
    pms.set_script(capture_script(
        Route::SectionSorts(CAPTURED_SECTION.to_owned()),
        "real/films_sorts.json",
    ));
    let baseline = pms.requests().len();

    let offered = sorts(&server, CAPTURED_SECTION).expect("the captured sorts answer must parse");
    let calls = driven(
        &pms,
        baseline,
        &format!("{SECTION_SORTS_PATH}/{CAPTURED_SECTION}/sorts"),
    );
    assert!(
        query_pairs(&calls[0]).is_empty(),
        "sorts are asked without a query"
    );

    let keys: Vec<&str> = offered.iter().map(|sort| sort.key.as_str()).collect();
    assert_eq!(keys, ["titleSort", "originallyAvailableAt", "rating"]);
    let titles: Vec<&str> = offered
        .iter()
        .map(|sort| sort.title.as_deref().unwrap_or_default())
        .collect();
    assert_eq!(titles, ["Title", "Release Date", "Critic Rating"]);
    let desc: Vec<Option<&str>> = offered
        .iter()
        .map(|sort| sort.desc_key.as_deref())
        .collect();
    assert_eq!(
        desc,
        [
            Some("titleSort:desc"),
            Some("originallyAvailableAt:desc"),
            Some("rating:desc"),
        ]
    );
    let direction: Vec<Option<&str>> = offered
        .iter()
        .map(|sort| sort.default_direction.as_deref())
        .collect();
    assert_eq!(direction, [Some("asc"), Some("desc"), Some("desc")]);
    let first_character: Vec<Option<&str>> = offered
        .iter()
        .map(|sort| sort.first_character_path.as_deref())
        .collect();
    assert_eq!(
        first_character,
        [Some("/library/sections/2/firstCharacter"), None, None],
        "only the title sort names a first-character browse path in this capture"
    );

    // Every sort value this section names must survive [`validate_sort`]
    // unchanged: validating the list must not silently pick the other order.
    for sort in &offered {
        assert_eq!(validate_sort(&sort.key).expect("key"), sort.key);
        let descending = sort
            .desc_key
            .clone()
            .expect("every captured sort names its own descKey");
        assert_eq!(validate_sort(&descending).expect("descKey"), descending);
    }
}
