//! **query-building** tests: what a browse option puts on the wire, which
//! sort values are refused before any request, and the shape of the
//! `/photo/:/transcode` artwork target.
//!
//! Same real-loopback setup as `tests/library_endpoints.rs` (see
//! `tests/p1b/mod.rs`): the mock answers with the **captured bytes** from
//! `tests/fixtures/real/`, so the parsed readings below come from a real
//! server's answer while the query keys are checked against what this client
//! actually sent. `sort` and `unwatched` are keys no capture records as request
//! lines, so they are verified here as wire behaviour only; whether a live
//! server honours them for this account is a live-driver check.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)] // test scaffolding; product code keeps these lints on

mod common;
mod mock;
mod p1b;

use plex_core::client_info::TOKEN_HEADER;
use plex_core::error::CoreError;
use plex_core::library::{
    browse_items, hubs, photo_transcode_path, search_hubs, validate_sort, BrowseOptions, Page,
    MAX_HUB_ROWS, MAX_PHOTO_DIMENSION, PHOTO_TRANSCODE_PATH, QUERY_PHOTO_HEIGHT,
    QUERY_PHOTO_MIN_SIZE, QUERY_PHOTO_UPSCALE, QUERY_PHOTO_URL, QUERY_PHOTO_WIDTH, QUERY_SORT,
    QUERY_UNWATCHED, QUERY_UNWATCHED_ON, SECTION_ITEMS_PATH,
};

use crate::mock::Route;
use crate::p1b::{capture_script, p1b_session, query_pairs, CAPTURED_SECTION};

/// The wire query of one browse request.
fn browse_wire(pms: &mock::MockPlex, baseline: usize) -> Vec<(String, String)> {
    let calls = p1b::driven(
        pms,
        baseline,
        &format!("{SECTION_ITEMS_PATH}/{CAPTURED_SECTION}/all"),
    );
    query_pairs(&calls[0])
}

#[test]
fn a_sort_option_becomes_a_sort_query_key_and_nothing_else() {
    let (pms, server) = p1b_session("p1b-browse-sort");
    pms.set_script(capture_script(
        Route::SectionItems(CAPTURED_SECTION.to_owned()),
        "real/films_all.json",
    ));
    let baseline = pms.requests().len();

    let options = BrowseOptions {
        sort: Some("titleSort:desc".to_owned()),
        unwatched: false,
    };
    let page = browse_items(&server, CAPTURED_SECTION, Page::first(), &options)
        .expect("the captured films page must parse");
    let pairs = browse_wire(&pms, baseline);

    assert!(
        pairs
            .iter()
            .any(|(key, value)| (key.as_str(), value.as_str()) == (QUERY_SORT, "titleSort:desc")),
        "the sort value reaches the wire unchanged: {pairs:?}"
    );
    assert!(
        !pairs.iter().any(|(key, _)| key == QUERY_UNWATCHED),
        "an unwatched filter was not asked for: {pairs:?}"
    );
    assert_eq!(page.total, Some(1654), "the unfiltered films capture");
    assert_eq!(page.items.len(), 3);
    assert_eq!(page.items[0].title.as_deref(), Some("The 'Brook"));
}

#[test]
fn an_unwatched_option_becomes_unwatched_1_and_answers_a_smaller_total() {
    let (pms, server) = p1b_session("p1b-browse-unwatched");
    pms.set_script(capture_script(
        Route::SectionItems(CAPTURED_SECTION.to_owned()),
        "real/films_unwatched.json",
    ));
    let baseline = pms.requests().len();

    let options = BrowseOptions {
        sort: None,
        unwatched: true,
    };
    let page = browse_items(&server, CAPTURED_SECTION, Page::first(), &options)
        .expect("the captured unwatched films page must parse");
    let pairs = browse_wire(&pms, baseline);

    assert!(
        pairs
            .iter()
            .any(|(key, value)| (key.as_str(), value.as_str())
                == (QUERY_UNWATCHED, QUERY_UNWATCHED_ON)),
        "the unwatched filter goes on the wire as {QUERY_UNWATCHED}=1: {pairs:?}"
    );
    assert!(
        !pairs.iter().any(|(key, _)| key == QUERY_SORT),
        "no sort was asked for: {pairs:?}"
    );
    assert_eq!(
        page.total,
        Some(1611),
        "this capture answers a smaller totalSize than the unfiltered browse"
    );
    assert_eq!(page.items[0].rating_key.as_deref(), Some("30874"));
}

#[test]
fn a_plain_browse_asks_neither_sort_nor_unwatched() {
    let (pms, server) = p1b_session("p1b-browse-plain");
    pms.set_script(capture_script(
        Route::SectionItems(CAPTURED_SECTION.to_owned()),
        "real/films_all.json",
    ));
    let baseline = pms.requests().len();

    let page = browse_items(
        &server,
        CAPTURED_SECTION,
        Page::first(),
        &BrowseOptions::default(),
    )
    .expect("the default browse must still parse the films page");
    let pairs = browse_wire(&pms, baseline);
    assert!(
        pairs
            .iter()
            .all(|(key, _)| key != QUERY_SORT && key != QUERY_UNWATCHED),
        "an empty option set adds no query key: {pairs:?}"
    );
    assert_eq!(page.total, Some(1654));
}

#[test]
fn a_sort_the_section_does_not_offer_is_refused_before_any_request() {
    let (pms, server) = p1b_session("p1b-browse-bad-sort");
    pms.set_script(capture_script(
        Route::SectionItems(CAPTURED_SECTION.to_owned()),
        "real/films_all.json",
    ));
    let baseline = pms.requests().len();

    for rejected in [
        "addedAt:DESC",
        "titleSort:",
        ":desc",
        "a;b",
        "titleSort:desc,addedAt",
        "title sort",
        "",
    ] {
        let error = validate_sort(rejected).expect_err("{rejected} must not be a sort value");
        assert!(
            matches!(error, CoreError::InvalidInput),
            "{rejected}: {error:?}"
        );
    }

    let options = BrowseOptions {
        sort: Some("titleSort:desc;unwatched=1".to_owned()),
        unwatched: false,
    };
    let error = browse_items(&server, CAPTURED_SECTION, Page::first(), &options)
        .expect_err("a sort value carrying a query separator must be refused");
    assert!(matches!(error, CoreError::InvalidInput), "{error:?}");
    assert_eq!(
        pms.requests().len(),
        baseline,
        "a refused sort value must not reach the server at all"
    );
}

#[test]
fn hub_row_bounds_are_refused_before_any_request() {
    let (pms, server) = p1b_session("p1b-hub-bounds");
    pms.set_script(capture_script(Route::Hubs, "real/hubs_home.json"));
    let baseline = pms.requests().len();

    assert!(matches!(
        hubs(&server, 0).expect_err("count 0 is not a row bound"),
        CoreError::InvalidInput
    ));
    assert!(matches!(
        hubs(&server, MAX_HUB_ROWS + 1).expect_err("an oversized row bound is refused"),
        CoreError::InvalidInput
    ));
    assert!(matches!(
        search_hubs(&server, "Aphelion", 0).expect_err("a search limit of 0 is refused"),
        CoreError::InvalidInput
    ));
    assert_eq!(
        pms.requests().len(),
        baseline,
        "a refused row bound must not reach the server at all"
    );
}

#[test]
fn photo_transcode_path_is_a_token_free_request_target() {
    let thumb = "/library/metadata/36100/thumb/1774055077";
    let target =
        photo_transcode_path(thumb, 300, 168).expect("a captured episode thumb must build");
    assert_eq!(
        target,
        "/photo/:/transcode?width=300&height=168&minSize=1&upscale=0&url=%2Flibrary%2Fmetadata%2F36100%2Fthumb%2F1774055077",
        "the artwork request target, with the thumb form-encoded inside its own parameter"
    );
    for key in [
        QUERY_PHOTO_WIDTH,
        QUERY_PHOTO_HEIGHT,
        QUERY_PHOTO_MIN_SIZE,
        QUERY_PHOTO_UPSCALE,
        QUERY_PHOTO_URL,
    ] {
        assert!(
            target.contains(&format!("{key}=")),
            "{key} must name a parameter"
        );
    }
    assert!(target.starts_with(PHOTO_TRANSCODE_PATH));
    assert!(
        !target.contains(TOKEN_HEADER),
        "no credential name may appear in an artwork URL"
    );

    for rejected in [
        ("", 300, 168),
        ("library/metadata/36100", 300, 168),
        ("/library/metadata/36100?X-Plex-Token=abc", 300, 168),
        ("/library/metadata/36100#frag", 300, 168),
        ("/library/metadata/36100 1", 300, 168),
        ("/library/metadata/36100", 0, 168),
        ("/library/metadata/36100", 300, 0),
        ("/library/metadata/36100", MAX_PHOTO_DIMENSION + 1, 168),
    ] {
        let error = photo_transcode_path(rejected.0, rejected.1, rejected.2)
            .expect_err("a target that is not a server-named artwork path must be refused");
        assert!(
            matches!(error, CoreError::InvalidInput),
            "{rejected:?}: {error:?}"
        );
    }
}

#[test]
fn a_plex_static_photo_is_asked_through_the_server_and_nothing_else_is() {
    let cast = "https://metadata-static.plex.tv/d/people/4a8cd29ce336b3923c59841892b17d05.jpg";
    let path = photo_transcode_path(cast, 72, 72)
        .unwrap_or_else(|e| panic!("a Plex static cast photo is accepted: {e:?}"));
    assert!(path.starts_with("/photo/:/transcode?"));
    assert!(path.contains("url=https%3A%2F%2Fmetadata-static.plex.tv%2F"));
    for refused in [
        "http://metadata-static.plex.tv/d/people/a.jpg",
        "https://example.com/a.jpg",
        "https://plex.tv.evil.example/a.jpg",
        "https://metadata-static.plex.tv:8443/a.jpg",
        "https://user@metadata-static.plex.tv/a.jpg",
        "https://metadata-static.plex.tv/a.jpg?X-Plex-Token=abc",
        "ftp://metadata-static.plex.tv/a.jpg",
    ] {
        assert!(
            photo_transcode_path(refused, 72, 72).is_err(),
            "{refused} must be refused"
        );
    }
}
