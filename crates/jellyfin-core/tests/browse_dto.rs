//! Every browse DTO parsed from its own live 12.4 answer.
//!
//! A fixture that stops parsing, or stops answering the field the UI reads,
//! fails here before it can fail as a blank card in plex-app. The assertions
//! quote the fixtures, including what they do *not* answer.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::time::Duration;

use jellyfin_core::browse::ImageKind;
use jellyfin_core::dto::{ImageInfo, Item, PlaybackInfo, QueryResult, SearchHintResult, User};
use jellyfin_core::{server, ticks};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/live-12.4/");

/// The ids and tags the capture was taken with, spelled out once so a fixture
/// that changes identity fails loudly.
const MOVIE_ID: &str = "5197d80dc0c848286caeb3d345930689";
const MOVIE_TAG: &str = "f575702cdb1f1f0c154421dbe6b03885";
const SERIES_ID: &str = "323fe870cae813165d6a87e8b47c9693";
const SEASON_ID: &str = "600b7329d762180dc41c3b2c893a6526";
const EPISODE_ID: &str = "28c2ea280a096582d8661f8ac0062da3";
const SERVER_ID: &str = "a57074336d774261bb9c10e93bbe1139";
const USER_ID: &str = "70eb2746aca443c098159c5d6e66d886";
/// Every fixture item runs 300000000 ticks — 30 seconds of generated media.
const CLIP: Duration = Duration::from_secs(30);

fn parse<T: serde::de::DeserializeOwned>(name: &str) -> T {
    let text = std::fs::read_to_string(format!("{FIXTURES}{name}.json")).expect("fixture present");
    serde_json::from_str(&text).expect("fixture parses")
}

/// One item detail fixture, which is a bare `BaseItemDto` and not an envelope.
fn detail(name: &str) -> Item {
    parse::<Item>(name)
}

/// A `QueryResult` fixture, with its rows.
fn page(name: &str) -> (QueryResult<Item>, serde_json::Value) {
    let raw: serde_json::Value = parse(name);
    let count = raw["TotalRecordCount"].as_u64().expect("a record count");
    let parsed: QueryResult<Item> = parse(name);
    assert_eq!(parsed.total_record_count, Some(count), "{name} count");
    assert_eq!(parsed.start_index, Some(0), "{name} start index");
    assert_eq!(
        parsed.items.len() as u64,
        count,
        "{name} answers every row it counts"
    );
    (parsed, raw)
}

#[test]
fn a_configured_server_answers_pascal_case_and_these_dtos_are_pinned_to_that() {
    // The 12.4 capture came from a wizard-finished server, and every key in it
    // is PascalCase. J1 pinned `PublicInfo` to the same rule; this is the same
    // pin for the browse side.
    let public: server::PublicInfo = parse("system_info_public");
    assert_eq!(public.server_name, "1e2cae5acadd");
    assert_eq!(public.id, SERVER_ID);
    assert_eq!(public.version, "12.4.0");
    // The add-a-source step refuses a server that has not finished its own setup,
    // so the flag has to survive a parse of the captured answer — and a server
    // that omits the key is unknown, not unfinished.
    assert_eq!(public.startup_wizard_completed, Some(true));
    let unnamed: server::PublicInfo =
        serde_json::from_str(r#"{"ServerName":"Jellyfin","Id":"x","Version":"10.9"}"#)
            .expect("a public info answer that says nothing about the wizard");
    assert_eq!(unnamed.startup_wizard_completed, None);

    // A first-run server answers the same endpoints in camelCase, and these
    // DTOs do not accept it: casing is pinned, not matched.
    let camelcase = r#"{"serverName":"Jellyfin","Id":"x"}"#;
    assert!(serde_json::from_str::<server::PublicInfo>(camelcase).is_err());
    let camelcase_row: Result<Item, _> = serde_json::from_str(r#"{"id":"x","name":"A"}"#);
    assert!(
        camelcase_row.is_err(),
        "a row is only a row when it carries `Id`"
    );

    // A row is identified or it is not a row: `Id` is required, so an item
    // that lost its id is a shape error rather than a card with no title, and
    // a PascalCase row that names nothing else still parses.
    assert!(serde_json::from_str::<Item>(r#"{"Id":"x","Name":"A"}"#).is_ok());
    assert!(serde_json::from_str::<Item>(r#"{"Name":"A"}"#).is_err());

    // The one honest soft spot of case-insensitive-on-the-server,
    // case-sensitive-on-the-client: an envelope whose rows are absent parses
    // as an empty page under either casing, because `items` defaults. The
    // camelCase server's *empty* answer is therefore accepted.
    let camelcase_empty: QueryResult<Item> =
        serde_json::from_str(r#"{"totalRecordCount":0,"items":[]}"#)
            .expect("an empty camelCase page parses");
    assert!(camelcase_empty.items.is_empty());
    assert_eq!(
        camelcase_empty.total_record_count, None,
        "count is PascalCase"
    );
}

#[test]
fn users_me_answers_the_id_every_other_call_is_scoped_by() {
    let me: User = parse("users_me");
    assert_eq!(me.user_id(), USER_ID);
    assert_eq!(me.name.as_deref(), Some("fixture"));
    assert_eq!(me.server_id.as_deref(), Some(SERVER_ID));
    assert_eq!(me.has_password, Some(true));

    // The password-sign-in fixture wraps the same kind of row in a `User`
    // object; that inner row is what a `Client` is built from.
    let wrapped: serde_json::Value = parse("users_authenticate_by_name");
    let inner = serde_json::to_string(&wrapped["User"]).expect("serialisable");
    let user: User = serde_json::from_str(&inner).expect("the wrapped user parses");
    assert_eq!(user.user_id(), USER_ID);
    assert_eq!(
        wrapped["ServerId"].as_str(),
        Some(SERVER_ID),
        "the session names the same server as the user"
    );
}

#[test]
fn user_views_answers_library_sections_and_their_collection_type() {
    let (views, _raw) = page("user_views");
    // `CollectionType` is what decides whether a view opens as movies or as
    // shows, and the server names it on the folder row itself.
    let sections: Vec<(&str, &str, Option<&str>)> = views
        .items
        .iter()
        .map(|view| {
            (
                view.id.as_str(),
                view.name.as_deref().unwrap_or_default(),
                view.collection_type.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        sections,
        vec![
            ("f137a2dd21bbc1b99aa5c0f6bf02a805", "Movies", Some("movies")),
            ("a656b907eb3a73532e40e44b968d0225", "Shows", Some("tvshows")),
        ]
    );
    assert_eq!(views.items[0].kind.as_deref(), Some("CollectionFolder"));
    assert_eq!(views.items[0].child_count, Some(2));
    // The Movies view has artwork; the Shows view answers an empty image map,
    // so its card has nothing to ask for.
    assert_eq!(
        views.items[0].image_tag(ImageKind::Primary),
        Some("9a477e608ba75f5d7c5d21fb7316e9ee"),
        "a view has its own artwork tag"
    );
    assert!(views.items[1].image_tags.is_empty());
    assert_eq!(views.items[1].image_path(ImageKind::Primary, 640), None);
}

#[test]
fn a_movie_detail_names_its_version_and_its_track() {
    let item = detail("item_movie");
    assert_eq!(item.id, MOVIE_ID);
    assert_eq!(item.name.as_deref(), Some("Aurora Test (2024)"));
    assert_eq!(item.kind.as_deref(), Some("Movie"));
    assert_eq!(item.production_year, Some(2024));
    assert_eq!(item.runtime(), Some(CLIP));
    assert_eq!(
        item.primary_image_aspect_ratio,
        Some(1.7777777777777777),
        "layout hint for the card"
    );
    assert!(item.is_playback_item());
    assert_eq!(item.image_tag(ImageKind::Primary), Some(MOVIE_TAG));
    assert_eq!(item.child_count, None, "a leaf counts no children");
    // The server omits `Overview` for generated media: the name is there,
    // the text is not, and a detail page renders without a blurb.
    assert_eq!(item.overview, None);
    assert!(item.genres.is_empty());
    assert!(item.people.is_empty());

    // The source is what a stream URL is built from. Its id happens to equal
    // the item's id for a `File` source, and is read from the source.
    let source = item.media_sources.first().expect("one version");
    assert_eq!(source.id.as_deref(), Some(MOVIE_ID));
    assert_eq!(source.container.as_deref(), Some("mkv"));
    assert_eq!(source.protocol.as_deref(), Some("File"));
    assert_eq!(source.size, Some(54_564_552));
    assert_eq!(source.bitrate, Some(14_550_547));
    assert_eq!(source.runtime(), Some(CLIP));
    assert_eq!(source.supports_direct_play, Some(true));
    assert_eq!(source.requires_opening, Some(false));
    let stream = source.media_streams.first().expect("one track");
    assert_eq!(stream.codec.as_deref(), Some("h264"));
    assert_eq!(stream.kind.as_deref(), Some("Video"));
    assert_eq!(stream.display_title.as_deref(), Some("1080p H264 SDR"));
    assert_eq!((stream.width, stream.height), (Some(1920), Some(1080)));
    assert_eq!(stream.is_default, Some(false));
}

#[test]
fn an_episode_row_points_back_at_its_season_and_its_show() {
    let item = detail("item_episode");
    assert_eq!(item.id, EPISODE_ID);
    assert_eq!(item.kind.as_deref(), Some("Episode"));
    assert_eq!(item.index_number, Some(1), "episode number");
    assert_eq!(item.parent_index_number, Some(1), "season number");
    assert_eq!(item.season_id.as_deref(), Some(SEASON_ID));
    assert_eq!(item.season_name.as_deref(), Some("Season 1"));
    assert_eq!(item.series_id.as_deref(), Some(SERIES_ID));
    assert_eq!(item.series_name.as_deref(), Some("Starlight Signals"));
    assert_eq!(item.runtime(), Some(CLIP));
    assert_eq!(
        item.production_year, None,
        "an episode row here names no year"
    );
    assert!(item.is_playback_item());
    assert_eq!(
        item.image_tag(ImageKind::Primary),
        Some("bce85a884a27f1e5b4c9f745c7b913b4")
    );
    let source = item.media_sources.first().expect("one version");
    assert_eq!(
        source
            .media_streams
            .first()
            .expect("one track")
            .codec
            .as_deref(),
        Some("hevc")
    );
}

#[test]
fn a_series_and_a_season_are_folder_rows_with_counts_and_no_length() {
    let series = detail("item_series");
    assert_eq!(series.id, SERIES_ID);
    assert_eq!(series.kind.as_deref(), Some("Series"));
    assert_eq!(series.child_count, Some(1), "one season");
    assert_eq!(series.recursive_item_count, Some(3), "three episodes");
    assert_eq!(series.runtime(), None, "a folder has no length");
    assert!(!series.is_playback_item());
    assert!(series.media_sources.is_empty(), "a folder has no version");
    assert!(series.image_tags.is_empty(), "this series has no artwork");
    assert_eq!(series.unplayed_item_count(), Some(3));
    assert_eq!(series.position(), Some(Duration::ZERO));

    let (seasons, _raw) = page("show_seasons");
    let season = seasons.items.first().expect("one season");
    assert_eq!(season.id, SEASON_ID);
    assert_eq!(season.kind.as_deref(), Some("Season"));
    assert_eq!(season.name.as_deref(), Some("Season 1"));
    assert_eq!(season.index_number, Some(1));
    assert_eq!(
        season.parent_index_number, None,
        "a season is the top level"
    );
    assert_eq!(season.series_id.as_deref(), Some(SERIES_ID));
    assert_eq!(season.series_name.as_deref(), Some("Starlight Signals"));
    assert!(!season.is_playback_item());
}

#[test]
fn a_position_is_ticks_on_the_wire_and_a_duration_in_the_ui() {
    let (next_up, raw) = page("shows_nextup");
    let episode = next_up.items.first().expect("one row");
    let on_wire = raw["Items"][0]["UserData"]["PlaybackPositionTicks"]
        .as_i64()
        .expect("ticks as a number");
    assert_eq!(episode.position(), ticks::ticks_to_duration(on_wire));
    assert_eq!(episode.position(), Some(Duration::ZERO));
    assert_eq!(
        episode.user_data.as_ref().expect("watch state").play_count,
        Some(0)
    );

    // The opposite case, and the one a continue-watching card has to handle:
    // the server knows a third of this season is watched and still answers
    // position 0 for the season itself. The position lives on the episode
    // inside, so the card opens the item instead of seeking from this row.
    let (resume, _raw) = page("user_items_resume");
    let season = resume.items.first().expect("one row");
    assert_eq!(season.kind.as_deref(), Some("Season"));
    assert_eq!(season.position(), Some(Duration::ZERO));
    assert_eq!(season.played_percentage(), Some(33.33333333333333));
    assert_eq!(season.unplayed_item_count(), Some(2));
    assert_eq!(
        resume.items[1].kind.as_deref(),
        Some("Series"),
        "resume answers folder rows, not only playable ones"
    );
    assert_eq!(
        episode.index_number,
        Some(2),
        "next up names the episode number"
    );
}

#[test]
fn episodes_of_a_season_are_numbered_and_scoped_to_the_show() {
    let (episodes, _raw) = page("show_episodes");
    let rows: Vec<(u64, u64)> = episodes
        .items
        .iter()
        .map(|episode| {
            (
                episode.index_number.expect("an episode number"),
                episode.parent_index_number.expect("a season number"),
            )
        })
        .collect();
    assert_eq!(rows, vec![(1, 1), (2, 1), (3, 1)]);
    assert!(
        episodes
            .items
            .iter()
            .all(|episode| episode.series_id.as_deref() == Some(SERIES_ID)),
        "every row of a season query belongs to that show"
    );
    let tagged: usize = episodes
        .items
        .iter()
        .filter(|episode| episode.image_tag(ImageKind::Primary).is_some())
        .count();
    assert_eq!(tagged, 3, "every episode in this answer carries a preview");
}

#[test]
fn a_search_hint_names_its_own_item_and_only_an_episode_hint_names_a_show() {
    let hints: SearchHintResult = parse("search_hints");
    assert_eq!(hints.total_record_count, Some(4));
    assert_eq!(hints.search_hints.len(), 4);

    let episode = &hints.search_hints[0];
    assert_eq!(episode.name.as_deref(), Some("Starlight Signals S01E01"));
    assert_eq!(episode.kind.as_deref(), Some("Episode"));
    // The server sends the same id twice, once as `ItemId` and once as `Id`.
    assert_eq!(episode.item_id.as_deref(), Some(EPISODE_ID));
    assert_eq!(episode.id, episode.item_id);
    assert_eq!(episode.opened_id(), Some(EPISODE_ID));
    assert_eq!(episode.runtime(), Some(CLIP));
    assert_eq!(episode.index_number, Some(1));
    assert_eq!(episode.parent_index_number, Some(1));
    assert_eq!(episode.series.as_deref(), Some("Starlight Signals"));

    let series = hints
        .search_hints
        .iter()
        .find(|hint| hint.kind.as_deref() == Some("Series"))
        .expect("a series hint");
    assert_eq!(series.opened_id(), Some(SERIES_ID));
    assert_eq!(series.index_number, None, "a series hint numbers nothing");
    assert_eq!(series.parent_index_number, None);
    assert_eq!(series.series, None, "a series hint names no show above it");
    assert_eq!(series.runtime(), None);
    // A hint's image tag is a flat field, and this hint has none — so its
    // artwork path is `None` rather than a request the server cannot answer.
    assert_eq!(series.primary_image_tag, None);
    assert_eq!(series.image_path(320), None);
    assert_eq!(
        episode.image_path(320).as_deref(),
        Some(
            "/Items/28c2ea280a096582d8661f8ac0062da3/Images/Primary?maxWidth=320&tag=bce85a884a27f1e5b4c9f745c7b913b4"
        )
    );
}

#[test]
fn playback_info_names_a_session_and_its_own_view_of_the_same_source() {
    let info: PlaybackInfo = parse("playback_info");
    assert_eq!(
        info.play_session_id.as_deref(),
        Some("50ea4d36ddc843118b7851a0ab3c4453"),
        "the session id the later progress reports answer to"
    );
    let source = info.media_sources.first().expect("one version");
    assert_eq!(source.id.as_deref(), Some(MOVIE_ID));
    assert_eq!(source.container.as_deref(), Some("mkv"));
    assert_eq!(source.size, Some(54_564_552));
    assert_eq!(source.bitrate, Some(14_550_547));
    assert_eq!(source.supports_direct_play, Some(true));
    assert_eq!(source.requires_opening, Some(false));
    assert_eq!(source.media_streams.len(), 1);

    // The contradiction the capture shows: the *same* source of the *same*
    // item answers `SupportsTranscoding: true` from `GET /Items/{id}` and
    // `false` from `POST /Items/{id}/PlaybackInfo`, because PlaybackInfo
    // answers against the requesting `DeviceProfile` (an empty one here) and
    // the item row reports the source's own properties. `GET` is the only
    // honest source of "can this be played as it sits".
    let item = detail("item_movie");
    let from_item = item.media_sources.first().expect("one version");
    assert_eq!(from_item.supports_transcoding, Some(true));
    assert_eq!(source.supports_transcoding, Some(false));
    assert_eq!(
        from_item.supports_direct_stream,
        source.supports_direct_stream
    );
    assert_eq!(from_item.container, source.container);
}

#[test]
fn the_images_answer_lists_only_the_types_the_server_can_serve() {
    let images: Vec<ImageInfo> = parse("item_images");
    assert_eq!(images.len(), 1);
    let primary = images.first().expect("one image");
    // The same `Primary` tag the item row names, which is what makes an
    // artwork URL cacheable per tag rather than per request.
    assert_eq!(primary.image_type.as_deref(), Some("Primary"));
    assert_eq!(primary.image_tag.as_deref(), Some(MOVIE_TAG));
    assert_eq!(primary.width, Some(1920));
    assert_eq!(primary.height, Some(1080));
    assert_eq!(primary.size, Some(79_627));
    // No fixture anywhere carries a `Logo`, `Thumb` or `Backdrop` image, so
    // those builders are pinned by URL shape and by a live tag only for
    // `Primary`; see `docs/JELLYFIN.md`.
    let kinds: Vec<&str> = images
        .iter()
        .filter_map(|image| image.image_type.as_deref())
        .collect();
    assert_eq!(kinds, vec!["Primary"]);
}

#[test]
fn what_no_fixture_ever_answers_stays_absent_rather_than_invented() {
    for name in [
        "items_movies",
        "items_shows",
        "show_episodes",
        "show_seasons",
        "shows_nextup",
        "user_items_resume",
        "user_views",
    ] {
        let (page, _raw) = page(name);
        for item in &page.items {
            assert!(
                item.official_rating.is_none(),
                "{name} unexpectedly answers a certification"
            );
            assert!(!item.image_tags.contains_key("Logo"), "{name} has a Logo");
            assert!(item.backdrop_image_tags.is_empty(), "{name} has a backdrop");
            assert!(item.people.is_empty(), "{name} has credits");
            assert!(item.genres.is_empty(), "{name} has genres");
            for source in &item.media_sources {
                assert!(
                    source
                        .media_streams
                        .iter()
                        .all(|stream| stream.kind.as_deref() == Some("Video")),
                    "{name} answers a non-video track"
                );
            }
        }
    }
}
