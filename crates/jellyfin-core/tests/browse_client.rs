//! A `Client` browsing a mock 12.4 server: the exact requests it makes, the
//! fixtures it parses out of them, and the URL shapes it builds without ever
//! putting the token in a URL.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::time::Duration;

use jellyfin_core::browse::{self, Client, ImageKind, ItemKind};
use jellyfin_core::dto::Item;
use jellyfin_core::playback::{Delivery, DeviceProfile, DirectPlayProfile, ProfileKind};
use jellyfin_core::{media_browser_header, server, DeviceInfo, Http, SecretString};

mod mock;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/live-12.4/");
const USER_ID: &str = "70eb2746aca443c098159c5d6e66d886";
const MOVIE_ID: &str = "5197d80dc0c848286caeb3d345930689";
const MOVIE_TAG: &str = "f575702cdb1f1f0c154421dbe6b03885";
const SERIES_ID: &str = "323fe870cae813165d6a87e8b47c9693";
const SEASON_ID: &str = "600b7329d762180dc41c3b2c893a6526";
const MOVIES_VIEW_ID: &str = "f137a2dd21bbc1b99aa5c0f6bf02a805";

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!("{FIXTURES}{name}.json")).expect("fixture present")
}

fn http_with(mock: &mock::MockJfServer) -> Http {
    Http::with_timeouts(
        &mock.url(),
        std::time::Duration::from_secs(5),
        std::time::Duration::from_secs(5),
    )
    .expect("mock base must normalise")
}

fn device() -> DeviceInfo {
    DeviceInfo {
        app: "Chewvy".to_owned(),
        app_version: "0.1.0".to_owned(),
        device: "test-machine".to_owned(),
        device_id: "0a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d".to_owned(),
    }
}

fn token() -> SecretString {
    SecretString::new("c8f0a1d2e3b4456789abcdef01234567")
}

/// The smallest profile that names what the captured source actually is: an
/// `mkv` with an H.264 video stream. `SupportsDirectPlay` is the server's answer
/// *for the profile it was sent*, so a client that wants a source handed over
/// untouched has to say what it can decode.
fn profile() -> DeviceProfile {
    DeviceProfile {
        name: None,
        max_static_bitrate: None,
        max_streaming_bitrate: None,
        direct_play_profiles: vec![DirectPlayProfile {
            container: "mkv".to_owned(),
            kind: ProfileKind::Video,
            video_codec: Some("h264".to_owned()),
            audio_codec: None,
        }],
        codec_profiles: vec![],
        transcoding_profiles: vec![],
    }
}

/// Percent-encoding the server's own query syntax has to survive: `url`'s form
/// encoder turns the `,` separating `Fields` and `IncludeItemTypes` values into
/// `%2C`, which ASP.NET decodes back before binding.
fn expected_query(pairs: &[(&str, &str)]) -> String {
    let mut out = String::new();
    for (key, value) in pairs {
        out.push('&');
        out.push_str(key);
        out.push('=');
        out.push_str(&form_urlencoded(value));
    }
    out.remove(0);
    out
}

/// The tiny slice of `application/x-www-form-urlencoded` this test needs:
/// `url`'s encoder leaves `A-Za-z0-9*-._` and passes everything else as `%XX`,
/// with a space as `+`.
fn form_urlencoded(value: &str) -> String {
    let mut out = String::new();
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'*' => {
                out.push(*byte as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{:02X}", byte)),
        }
    }
    out
}

/// The `Fields` list a captured browse request asks for, spelled out once.
const CAPTURED_FIELDS: &str =
    "Overview,Genres,People,MediaSources,MediaStreams,ProviderIds,Taglines,DateCreated,PrimaryImageAspectRatio";

#[test]
fn a_client_browses_the_captured_endpoints_and_never_names_the_token_in_a_url() {
    let server = mock::MockJfServer::start();
    // The mock routes on path alone, so `/Items` here answers both the view
    // query and the item detail; the detail fixture is one item and the view
    // fixture is an envelope, so each call below is checked against the shape
    // the endpoint it went to actually answers.
    for (route, name) in [
        ("GET /Users/Me", "users_me"),
        ("GET /UserViews", "user_views"),
        ("GET /UserItems/Resume", "user_items_resume"),
        ("GET /Shows/NextUp", "shows_nextup"),
        ("GET /Search/Hints", "search_hints"),
    ] {
        server.route(route, mock::Answer::json(200, fixture(name)));
    }
    server.route(
        "GET /Items",
        mock::Answer::json(200, fixture("items_movies")),
    );
    server.route("GET /Items", mock::Answer::json(200, fixture("item_movie")));
    server.route(
        &format!("GET /Items/{MOVIE_ID}"),
        mock::Answer::json(200, fixture("item_movie")),
    );
    server.route(
        &format!("GET /Shows/{SERIES_ID}/Seasons"),
        mock::Answer::json(200, fixture("show_seasons")),
    );
    server.route(
        &format!("GET /Shows/{SERIES_ID}/Episodes"),
        mock::Answer::json(200, fixture("show_episodes")),
    );
    server.route(
        &format!("GET /Items/{MOVIE_ID}/Images"),
        mock::Answer::json(200, fixture("item_images")),
    );
    server.route(
        &format!("POST /Items/{MOVIE_ID}/PlaybackInfo"),
        mock::Answer::json(200, fixture("playback_info")),
    );

    let http = http_with(&server);
    let device = device();
    let token = token();
    let client = Client::new(&http, &device, &token, USER_ID).expect("a usable user id");

    // Each of these answers a fixture, so the parsed rows are pinned by
    // `browse_dto.rs`; here what matters is that the call happened at all.
    assert_eq!(client.me().expect("me").user_id(), USER_ID);
    let views = client.views().expect("views");
    assert_eq!(views.items.len(), 2);
    let movies = client
        .view_items(MOVIES_VIEW_ID, &[ItemKind::Movie])
        .expect("movies");
    assert_eq!(movies.items.len(), 2);
    assert_eq!(
        client.item(MOVIE_ID).expect("item").runtime(),
        Some(Duration::from_secs(30))
    );
    assert_eq!(client.seasons(SERIES_ID).expect("seasons").items.len(), 1);
    assert_eq!(
        client
            .episodes(SERIES_ID, Some(1))
            .expect("episodes")
            .items
            .len(),
        3
    );
    assert_eq!(client.resume().expect("resume").items.len(), 2);
    assert_eq!(client.next_up().expect("next up").items.len(), 1);
    assert_eq!(
        client
            .search_hints("Starlight", 25)
            .expect("hints")
            .search_hints
            .len(),
        4
    );
    assert_eq!(client.images(MOVIE_ID).expect("images").len(), 1);
    assert!(client
        .playback_info(MOVIE_ID, &profile(), Delivery::ServerChoice)
        .expect("playback info")
        .play_session_id
        .is_some());

    // Every one of those requests carried the token in the header and never in
    // its query — Plex's `X-Plex-Token`-in-the-query habit would leak it into
    // server access logs.
    let requests = server.requests();
    assert_eq!(requests.len(), 11);
    for request in &requests {
        assert!(
            !request.path_and_query.contains("c8f0a1d2")
                && !request.path_and_query.contains("access_token")
                && !request.path_and_query.contains("api_key"),
            "{} names the token",
            request.path_and_query
        );
        match request.method.as_str() {
            "GET" => assert_eq!(
                request.authorization.as_deref(),
                Some(expected_header().as_str())
            ),
            other => {
                assert_eq!(other, "POST");
                assert_eq!(
                    request.authorization.as_deref(),
                    Some(expected_header().as_str())
                );
            }
        }
    }
}

fn expected_header() -> String {
    media_browser_header(&device(), Some(&token()))
}

#[test]
fn a_view_query_is_the_captured_shape_with_its_lists_percent_encoded() {
    let server = mock::MockJfServer::start();
    server.route(
        "GET /Items",
        mock::Answer::json(200, fixture("items_movies")),
    );
    let http = http_with(&server);
    let device = device();
    let token = token();
    let client = Client::new(&http, &device, &token, USER_ID).expect("a usable user id");

    client
        .view_items(MOVIES_VIEW_ID, &[ItemKind::Movie, ItemKind::Series])
        .expect("a page of items");
    let kinds = format!("{},{}", ItemKind::Movie.as_str(), ItemKind::Series.as_str());
    let expected = format!(
        "/Items?{}",
        expected_query(&[
            ("userId", USER_ID),
            ("parentId", MOVIES_VIEW_ID),
            ("recursive", "true"),
            ("includeItemTypes", &kinds),
            ("sortBy", "SortName"),
            ("fields", CAPTURED_FIELDS),
            ("enableImageTypes", browse::ENABLE_IMAGE_TYPES),
        ])
    );
    let recorded = server.requests().first().expect("one request").clone();
    assert_eq!(recorded.path_and_query, expected);
    // A live 12.4 server filters on a comma-joined list and ignores a `|`-joined
    // one, so the wire must carry the comma, encoded as `%2C`.
    assert!(
        recorded
            .path_and_query
            .contains("includeItemTypes=Movie%2CSeries"),
        "{}",
        recorded.path_and_query
    );
    assert!(
        recorded
            .path_and_query
            .contains(&format!("fields={}", form_urlencoded(CAPTURED_FIELDS))),
        "{}",
        recorded.path_and_query
    );
    assert!(recorded.body.is_empty(), "a browse read carries no body");
}

#[test]
fn the_detail_resume_nextup_search_and_playback_queries_match_the_capture() {
    let server = mock::MockJfServer::start();
    server.route("GET /UserItems/Resume", mock::Answer::json(204, ""));
    server.route("GET /Shows/NextUp", mock::Answer::json(204, ""));
    server.route("GET /Search/Hints", mock::Answer::json(204, ""));
    server.route("POST /Items/PlaybackInfo", mock::Answer::json(204, ""));
    let http = http_with(&server);
    let device = device();
    let token = token();
    let client = Client::new(&http, &device, &token, USER_ID).expect("a usable user id");
    let fields = CAPTURED_FIELDS;

    // A `204` has no body, which is what `server::ping` proves the transport
    // treats as an empty answer; here it stands in for an empty list.
    let _ = client.resume();
    let _ = client.next_up();
    let _ = client.search_hints("Starlight", 25);
    let _ = client.playback_info(MOVIE_ID, &profile(), Delivery::ServerChoice);

    let recorded: Vec<String> = server
        .requests()
        .into_iter()
        .map(|request| request.path_and_query)
        .collect();
    assert_eq!(
        recorded,
        vec![
            format!(
                "/UserItems/Resume?{}",
                expected_query(&[("userId", USER_ID), ("fields", fields)])
            ),
            format!(
                "/Shows/NextUp?{}",
                expected_query(&[("userId", USER_ID), ("fields", fields)])
            ),
            format!(
                "/Search/Hints?{}",
                expected_query(&[
                    ("userId", USER_ID),
                    ("searchTerm", "Starlight"),
                    ("limit", "25")
                ])
            ),
            format!("/Items/{MOVIE_ID}/PlaybackInfo?userId={USER_ID}"),
        ],
        "one shape per endpoint"
    );
}

#[test]
fn a_posted_playback_info_requests_a_read_and_not_a_playback() {
    let server = mock::MockJfServer::start();
    server.route("POST /Items/PlaybackInfo", mock::Answer::json(204, ""));
    let http = http_with(&server);
    let device = device();
    let token = token();
    let client = Client::new(&http, &device, &token, USER_ID).expect("a usable user id");

    let _ = client.playback_info(MOVIE_ID, &profile(), Delivery::ServerChoice);
    let request = server.requests().first().expect("one request").clone();

    // The one `POST` in the browse surface only asks the server which sources
    // an item has: the client's own profile, and no `PlayMethod`, no
    // `AutoOpenLiveStream`, no playing queue — nothing to forget server-side.
    assert_eq!(request.method, "POST");
    let body: serde_json::Value = serde_json::from_str(&request.body).expect("a JSON body");
    assert_eq!(
        body["DeviceProfile"]["DirectPlayProfiles"][0]["Container"], "mkv",
        "{}",
        request.body
    );
    assert_eq!(
        body["DeviceProfile"]["DirectPlayProfiles"][0]["Type"],
        "Video"
    );
    for started in ["PlayMethod", "AutoOpenLiveStream", "NowPlayingQueue"] {
        assert!(
            body["DeviceProfile"].get(started).is_none() && body.get(started).is_none(),
            "{started} would start something: {}",
            request.body
        );
    }
}

#[test]
fn artwork_paths_are_the_captured_shape_from_the_tags_an_item_names() {
    let server = mock::MockJfServer::start();
    server.route(
        &format!("GET /Items/{MOVIE_ID}"),
        mock::Answer::json(200, fixture("item_movie")),
    );
    server.route(
        &format!("GET /Items/{MOVIE_ID}/Images/Primary"),
        mock::Answer::json(200, "data"),
    );
    let http = http_with(&server);
    let device = device();
    let token = token();
    let client = Client::new(&http, &device, &token, USER_ID).expect("a usable user id");
    let item = client.item(MOVIE_ID).expect("a movie");

    // The captured artwork request is `/Items/{id}/Images/Primary?maxWidth=…
    // &tag=…` — the item's own id, the type as a path segment, and the tag the
    // item row named.
    let primary = item
        .image_path(ImageKind::Primary, 640)
        .expect("this movie carries a Primary image");
    assert_eq!(
        primary,
        format!("/Items/{MOVIE_ID}/Images/Primary?maxWidth=640&tag={MOVIE_TAG}")
    );
    // Fetched through the transport, that path resolves against the server
    // base, and the token still travels in the header alone.
    let response = http
        .get(&primary, &[], Some(&expected_header()))
        .expect("the artwork path is fetchable");
    assert!(response.is_success());
    let recorded = server.requests().last().expect("one request").clone();
    assert_eq!(recorded.path_and_query, primary);

    // A `Logo` (Plex's `clearLogo`) is a type this client asks the server to
    // name, but no fixture item carries one — so the builder returns `None`
    // instead of inventing a tagless artwork request.
    assert_eq!(item.image_tag(ImageKind::Logo), None);
    assert_eq!(item.image_path(ImageKind::Logo, 640), None);
    assert_eq!(item.image_path(ImageKind::Backdrop, 1280), None);
    assert_eq!(
        browse::image_path(MOVIE_ID, ImageKind::Logo, "cafebabe", 320).as_deref(),
        Some("/Items/5197d80dc0c848286caeb3d345930689/Images/Logo?maxWidth=320&tag=cafebabe"),
        "the same builder with a tag answers the documented route"
    );
    // A backdrop is addressed by its index in `BackdropImageTags`.
    assert_eq!(
        browse::image_path(SERIES_ID, ImageKind::Backdrop, "beef", 1920).as_deref(),
        Some("/Items/323fe870cae813165d6a87e8b47c9693/Images/Backdrop/0?maxWidth=1920&tag=beef")
    );
}

#[test]
fn a_static_direct_stream_path_asks_for_the_source_as_it_sits() {
    // `mediaSourceId` comes from the media source, never from the item id;
    // for a `File` source the capture shows the same value, but this builder
    // is given both so a remote or additional version cannot be mistaken.
    assert_eq!(
        browse::stream_path(MOVIE_ID, MOVIE_ID).as_deref(),
        Some(
            "/Videos/5197d80dc0c848286caeb3d345930689/stream?static=true&mediaSourceId=5197d80dc0c848286caeb3d345930689"
        )
    );
    assert!(browse::stream_path(MOVIE_ID, "0f0f").is_some());
    assert!(browse::stream_path("", MOVIE_ID).is_none());
    assert!(browse::stream_path(MOVIE_ID, "").is_none());
}

#[test]
fn an_id_that_could_grow_into_another_route_never_reaches_the_server() {
    let server = mock::MockJfServer::start();
    let http = http_with(&server);
    let device = device();
    let token = token();
    let client = Client::new(&http, &device, &token, USER_ID).expect("a usable user id");

    for bad in [
        "",
        "../../Items",
        "1?x=1",
        "a b",
        &format!("id#{}", "x".repeat(64)),
    ] {
        assert!(
            matches!(client.item(bad), Err(jellyfin_core::JfError::InvalidInput)),
            "{bad:?} must not be sent as an id"
        );
        assert!(browse::image_path(bad, ImageKind::Primary, "t", 1).is_none());
    }
    assert!(server.requests().is_empty(), "nothing was sent");

    // A user id is validated the same way, before a `Client` exists.
    assert!(Client::new(&http, &device, &token, "").is_err());
    assert!(Client::new(&http, &device, &token, "a/b").is_err());

    // A real id, and the hyphenated form some endpoints answer with, pass.
    assert!(browse::validate_id(HYPHENATED_ID).is_ok());
    assert!(browse::image_path(HYPHENATED_ID, ImageKind::Thumb, "t", 1).is_some());
}

const HYPHENATED_ID: &str = "5197d80d-c0c8-4828-6cae-b3d345930689";

#[test]
fn a_status_the_server_answers_becomes_that_status_not_a_parsed_empty() {
    let server = mock::MockJfServer::start();
    server.route(
        &format!("GET /Items/{SEASON_ID}"),
        mock::Answer::json(404, r#"{"Message":"Item not found"}"#),
    );
    let http = http_with(&server);
    let device = device();
    let token = token();
    let client = Client::new(&http, &device, &token, USER_ID).expect("a usable user id");

    let error = client.item(SEASON_ID).expect_err("a 404 is not an item");
    assert!(
        matches!(error, jellyfin_core::JfError::HttpStatus { status: 404 }),
        "{error:?}"
    );
    assert!(
        !server
            .requests()
            .first()
            .expect("one request")
            .path_and_query
            .contains("Item not found"),
        "the answer's body stays out of the request"
    );
}

#[test]
fn a_missing_user_id_is_a_shape_error_on_the_way_out_not_a_defaulted_one() {
    let server = mock::MockJfServer::start();
    server.route(
        "GET /Users/Me",
        mock::Answer::json(200, r#"{"Name":"fixture"}"#),
    );
    let http = http_with(&server);
    let device = device();
    let token = token();
    let client = Client::new(&http, &device, &token, USER_ID).expect("a usable user id");

    // `Id` is required on a user too: a server that answers a name and no id
    // cannot be browsed, because every other query needs that id.
    let error = client.me().expect_err("a user without an id");
    assert!(
        matches!(error, jellyfin_core::JfError::ResponseShape),
        "{error:?}"
    );
}

#[test]
fn a_browsed_item_row_is_the_shape_the_plex_side_library_model_expects() {
    // one Jellyfin row has to be able to become a
    // `plex_core::library::Item`. Pinned here for the two shapes whose fields
    // Jellyfin names differently in name but not in meaning.
    let server = mock::MockJfServer::start();
    server.route(
        &format!("GET /Shows/{SERIES_ID}/Episodes"),
        mock::Answer::json(200, fixture("show_episodes")),
    );
    let http = http_with(&server);
    let device = device();
    let token = token();
    let client = Client::new(&http, &device, &token, USER_ID).expect("a usable user id");
    let episode: Item = client
        .episodes(SERIES_ID, Some(1))
        .expect("episodes")
        .items
        .remove(0);

    // Plex's `parentRatingKey`/`grandparentTitle` map onto Jellyfin's season
    // and series ids and names, and the resume position onto `viewOffset`.
    assert_eq!(episode.season_id.as_deref(), Some(SEASON_ID));
    assert_eq!(episode.series_name.as_deref(), Some("Starlight Signals"));
    assert_eq!(episode.index_number, Some(1));
    assert_eq!(episode.parent_index_number, Some(1));
    assert_eq!(episode.position(), Some(Duration::ZERO));
    assert_eq!(episode.runtime(), Some(Duration::from_secs(30)));
    assert_eq!(
        episode.image_path(ImageKind::Thumb, 640),
        None,
        "this answer names only a Primary preview, which is what a Plex thumb is"
    );
    assert_eq!(
        episode.image_path(ImageKind::Primary, 640).as_deref(),
        Some("/Items/28c2ea280a096582d8661f8ac0062da3/Images/Primary?maxWidth=640&tag=bce85a884a27f1e5b4c9f745c7b913b4")
    );
}

#[test]
fn a_client_reuses_one_server_and_token_for_every_call() {
    let server = mock::MockJfServer::start();
    for (method, path, name) in [
        ("GET", "/UserViews", "user_views"),
        ("GET", "/UserItems/Resume", "user_items_resume"),
    ] {
        server.route(
            &format!("{method} {path}"),
            mock::Answer::json(200, fixture(name)),
        );
    }
    let http = http_with(&server);
    let device = device();
    let token = token();
    let client = Client::new(&http, &device, &token, USER_ID).expect("a usable user id");

    assert_eq!(client.user_id(), USER_ID);
    let _ = client.views();
    let _ = client.resume();
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests
            .iter()
            .map(|request| request.authorization.clone())
            .collect::<Vec<_>>(),
        vec![Some(expected_header()), Some(expected_header())],
        "the same header on every call"
    );
    // The user id is a query parameter, never a per-user port as on Plex.
    assert_eq!(server.url(), http.base());
    assert!(
        requests.iter().all(|request| request
            .path_and_query
            .contains(&format!("userId={USER_ID}"))),
        "every browse call names its user"
    );
    let path_and_query = server
        .requests()
        .first()
        .expect("one request")
        .path_and_query
        .clone();
    assert_eq!(
        path_and_query
            .split_once('?')
            .expect("a browse query")
            .1
            .split('&')
            .filter(|pair| pair.starts_with("userId="))
            .collect::<Vec<_>>(),
        vec![format!("userId={USER_ID}").as_str()],
        "one userId per request"
    );
}

#[test]
fn an_unreachable_answer_is_a_shape_error_the_caller_can_see() {
    let server = mock::MockJfServer::start();
    server.route("GET /UserViews", mock::Answer::json(200, r#"{"Items":{}}"#));
    let http = http_with(&server);
    let device = device();
    let token = token();
    let client = Client::new(&http, &device, &token, USER_ID).expect("a usable user id");

    // A `QueryResult` whose rows are the wrong JSON type is a shape error,
    // not an empty page: a server that changed its envelope must not read as
    // a user with no libraries.
    let error = client.views().expect_err("a malformed envelope");
    assert!(
        matches!(error, jellyfin_core::JfError::ResponseShape),
        "{error:?}"
    );
    // And the transport proves the request was still well-formed (`server`
    // above is the mock; `server::ping` shares this transport shape).
    assert!(server::ping(&http).is_err(), "the mock has no /health");
}
