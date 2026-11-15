//! The scale of a Plex rating, read off the captured answers, and the catalog
//! read the theme art library pages a section with.
//!
//! `rating` and `audienceRating` carry no unit on the wire. The captures name
//! the source of each value in `ratingImage` / `audienceRatingImage`, and
//! Rotten Tomatoes draws its ripe/rotten and upright/spilled marks at 60 %:
//! if the values were percentages or a 5-star scale the marks would not split
//! at 6.0, so finding that split in every capture is what says both fields are
//! out of 10.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)] // test scaffolding; product code keeps these lints on

mod common;
mod mock;
mod p1b;

use plex_core::library::{catalog_page, Page, QUERY_ENABLED, QUERY_INCLUDE_GUIDS};

use crate::mock::{Route, Script, Step};
use crate::p1b::{capture_script, p1b_session, query_pairs, CAPTURED_SECTION};

/// Every `(value, image)` pair of one rating field across the captures.
fn rated(field: &str, image: &str) -> Vec<(f64, String)> {
    let dir = mock::fixture_path("real");
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let text = std::fs::read_to_string(entry.unwrap().path()).unwrap();
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        let container = &json["MediaContainer"];
        let hubs = container["Hub"].as_array().cloned().unwrap_or_default();
        let rows = container["Metadata"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .chain(
                hubs.iter()
                    .flat_map(|hub| hub["Metadata"].as_array().cloned().unwrap_or_default()),
            );
        for row in rows {
            if let (Some(value), Some(mark)) = (row[field].as_f64(), row[image].as_str()) {
                out.push((value, mark.to_owned()));
            }
        }
    }
    out
}

fn split_at_six(pairs: &[(f64, String)], below: &str, above: &str) {
    let marked = |mark: &str| -> Vec<f64> {
        pairs
            .iter()
            .filter(|(_, m)| m == mark)
            .map(|(v, _)| *v)
            .collect()
    };
    let (low, high) = (marked(below), marked(above));
    assert!(!low.is_empty() && !high.is_empty(), "{below} / {above}");
    assert!(low.iter().all(|v| *v < 6.0), "{below}: {low:?}");
    assert!(
        high.iter().all(|v| (6.0..=10.0).contains(v)),
        "{above}: {high:?}"
    );
}

#[test]
fn both_ratings_are_tenths_of_rotten_tomatoes_percentages() {
    let critics = rated("rating", "ratingImage");
    split_at_six(
        &critics,
        "rottentomatoes://image.rating.rotten",
        "rottentomatoes://image.rating.ripe",
    );
    let audience = rated("audienceRating", "audienceRatingImage");
    split_at_six(
        &audience,
        "rottentomatoes://image.rating.spilled",
        "rottentomatoes://image.rating.upright",
    );
    // TMDb's own scores sit inside the same 0 to 10 range.
    assert!(audience
        .iter()
        .filter(|(_, m)| m == "themoviedb://image.rating")
        .all(|(v, _)| (0.0..=10.0).contains(v)));
}

#[test]
fn a_catalog_page_asks_for_external_ids_and_reads_the_titles() {
    let (pms, server) = p1b_session("ratings-catalog");
    pms.set_script(capture_script(
        Route::SectionItems(CAPTURED_SECTION.to_owned()),
        "real/films_all.json",
    ));
    let baseline = pms.requests().len();
    let page = catalog_page(&server, CAPTURED_SECTION, Page::new(0, 50).unwrap()).unwrap();

    let calls = p1b::driven(
        &pms,
        baseline,
        &format!("/library/sections/{CAPTURED_SECTION}/all"),
    );
    let pairs = query_pairs(&calls[0]);
    assert!(
        pairs
            .iter()
            .any(|(k, v)| (k.as_str(), v.as_str()) == (QUERY_INCLUDE_GUIDS, QUERY_ENABLED)),
        "{pairs:?}"
    );
    assert_eq!(page.total, Some(1654));
    let adrift = page
        .items
        .iter()
        .find(|item| item.title.as_deref() == Some("#Adrift"))
        .unwrap();
    assert_eq!(adrift.original_title.as_deref(), Some("#표류"));
    assert_eq!(adrift.rating, Some(8.8));
    assert_eq!(adrift.audience_rating, Some(6.3));
    assert!(
        page.items
            .iter()
            .filter(|item| item.title.as_deref() != Some("#Adrift"))
            .all(|item| item.original_title.is_none()),
        "a title the server names no original for has none"
    );
}

#[test]
fn guids_and_collections_are_read_as_their_tags() {
    let (pms, server) = p1b_session("ratings-guids");
    pms.set_script(capture_script(
        Route::SectionItems(CAPTURED_SECTION.to_owned()),
        "real/movie_metadata.json",
    ));
    let page = catalog_page(&server, CAPTURED_SECTION, Page::new(0, 1).unwrap()).unwrap();
    assert_eq!(
        page.items[0].guids,
        ["imdb://tt2172380", "tmdb://22969", "tvdb://1648"]
    );
    assert!(
        page.items[0].collections.is_empty(),
        "the capture names none"
    );

    // No capture is of a film in a collection; the server names one in the
    // `Genre[]` tag shape.
    pms.set_script(Script::new().with(
        Route::SectionItems(CAPTURED_SECTION.to_owned()),
        &[Step::Respond(
            200,
            r#"{"MediaContainer":{"size":1,"totalSize":1,"offset":0,"Metadata":[
                {"ratingKey":"7","type":"movie","title":"Home Alone",
                 "Collection":[{"tag":"Christmas Classics"},{"tag":""}]}]}}"#,
        )],
    ));
    let page = catalog_page(&server, CAPTURED_SECTION, Page::new(0, 1).unwrap()).unwrap();
    assert_eq!(page.items[0].collections, ["Christmas Classics"]);
    assert!(page.items[0].guids.is_empty());
}
