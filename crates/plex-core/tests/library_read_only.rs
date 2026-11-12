//! **library read-only** tests, run against the mock Plex server over real
//! loopback HTTP (`tests/mock/mod.rs`): section listing, item browse, search
//! and item metadata on a connected server.
//!
//! What this target proves **by execution** (every assertion below is against
//! requests the mock recorded from real loopback HTTP traffic made by
//! `plex_core::http::Http`, and against the values the product parsed out of the
//! mock's answers):
//!
//! * the whole discovery path - `GET /api/v2/resources` on a mock plex.tv with the
//!   **account** token, then probe + `GET /identity` + the four library calls on
//!   a mock PMS with **that row's own** token - really runs end to end, and
//!   `library_calls_never_carry_the_account_token` asserts the account token
//!   appears nowhere on the PMS host while the row token appears nowhere on the
//!   plex.tv host;
//! * every library call is a `GET` on the documented route with the documented
//!   path/query, the credential travels only in the `X-Plex-Token` **header**,
//!   and an unvalidated key or an absurd query produces **zero** extra requests;
//! * the mock's route table cannot drift from the product's path constants
//!   (`every_documented_route_is_routed_and_named` matches on **every**
//!   [`mock::Route`] variant), and no request other than the documented ones is
//!   made - no `POST`/`PUT`/`DELETE`/`PATCH` at all;
//! * a non-2xx answer, an unparseable answer and an answer with no item are
//!   typed errors (`HttpStatus` / `AuthRejected` / `ResponseShape`), their
//!   rendered text carries no token and no server body, and nothing is
//!   fabricated in their place;
//! * what a UI would show comes from the mock's answer
//!   (`tests/fixtures/library_sections.json`, `library_items.json`,
//!   `search_results.json`, `metadata_item.json`), and
//! * the **captured answers** in `tests/fixtures/real/` (recorded from a live
//!   Plex Media Server through this client's own transport) parse through the
//!   same product paths and report the values that server recorded.
//!
//! This is **not** a Plex integration test. The mock speaks plain HTTP on
//! loopback and replays recorded bytes, which proves the parser against what a
//! server actually sent; whether a live server answers the same way for this
//! account is checked by running the client against it. The hand-authored mock
//! shapes (`docs/CAPABILITIES.md`, `docs/PLEX_AUTH.md` §5 item 1: no
//! Plex-published spec was located) cover the routes and fault cases no
//! capture covers.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)] // test scaffolding; product code keeps these lints on

mod common;
mod mock;

use plex_core::api::{SIGNOUT_PATH, USER_PATH};
use plex_core::auth::PINS_PATH;
use plex_core::client_info::{ClientInfo, TOKEN_HEADER};
use plex_core::error::CoreError;
use plex_core::library::{
    browse_items, list_sections, metadata, search, validate_query, validate_segment, BrowseOptions,
    Page, CHILDREN_PATH, CONTAINER_SIZE_QUERY, CONTAINER_START_QUERY, HUBS_PATH, MAX_KEY_LEN,
    MAX_PAGE_SIZE, MAX_QUERY_LEN, METADATA_PATH, ON_DECK_PATH, SEARCH_HUBS_PATH, SEARCH_PATH,
    SECTIONS_PATH, SECTION_ITEMS_PATH, SECTION_SORTS_PATH,
};
use plex_core::progress::{
    mark_watched, report_timeline as report_progress, Outcome as ProgressOutcome, PlayState,
    ProgressUpdate, LIBRARY_IDENTIFIER, QUERY_DURATION, QUERY_IDENTIFIER, QUERY_KEY,
    QUERY_RATING_KEY, QUERY_STATE, QUERY_TIME, SCROBBLE_PATH, TIMELINE_PATH,
};
use plex_core::resources::{fetch_resources, parse_resources, RESOURCES_PATH};
use plex_core::secret::SecretString;
use plex_core::server::{connect, AttemptKind, Server, IDENTITY_PATH, PROBE_PATH};

use mock::{
    no_record_carries, requests_with_token_header, route_for, token_only_in_header, MockPlex,
    Recorded, Route, Script, Step, MOCK_TOKEN,
};

/// The `accessToken` of the single `resources` row these tests use. Distinct
/// from `mock::MOCK_TOKEN` (the **account** token) so "which token reached this
/// host" is observable in the mock's records.
const ROW_TOKEN: &str = "ROWTOKEN-LIB-4a1c9e7b2d6f8053";

/// The identifier `tests/fixtures/identity.json` reports for the mock host.
const MOCK_MACHINE: &str = "MOCKMACHINE-3c9e2b7a51d84f06";

/// The row's `clientIdentifier`, deliberately different from the identity this
/// test client creates in its temp dir so the row is not skipped as "our own
/// device".
const ROW_CLIENT: &str = "mock-pms-row-client-id";

/// The section key `tests/fixtures/library_sections.json` records, and the
/// rating key `library_items.json` / `metadata_item.json` record.
const SECTION_KEY: &str = "1";
const RATING_KEY: &str = "10";

/// One `connections[]` entry (verified field names only) pointing at a running
/// mock host over plain loopback HTTP.
fn local_http(base: &str) -> serde_json::Value {
    let port = base
        .rsplit_once(':')
        .and_then(|(_, port)| port.parse::<u16>().ok())
        .expect("mock base must carry a port");
    serde_json::json!({
        "address": "127.0.0.1",
        "port": port,
        "protocol": "http",
        "uri": base,
        "local": 1,
        "relay": 0,
        "IPv6": 0,
    })
}

/// A `provides: "server"`, `owned: 1` row with its **own** `accessToken`, using
/// only the verified field set (`docs/PLEX_AUTH.md` §1 item 6).
fn server_row(base: &str) -> serde_json::Value {
    serde_json::json!({
        "accessToken": ROW_TOKEN,
        "clientIdentifier": ROW_CLIENT,
        "device": "Mock-PMS",
        "name": "Mock PMS",
        "provides": "server",
        "presence": 1,
        "owned": 1,
        "httpsRequired": 0,
        "dnsRebindingProtection": 1,
        "natLoopbackSupported": 1,
        "relay": 0,
        "connections": [local_http(base)],
    })
}

/// The script a mock PMS needs to be connectable **and** to answer the four
/// library routes from the hand-authored fixtures.
fn library_script() -> Script {
    Script::new()
        .with(Route::Root, &[Step::Respond(200, r#"{"mock":"root"}"#)])
        .with(Route::Identity, &[Step::Fixture("identity.json")])
        .with(Route::Sections, &[Step::Fixture("library_sections.json")])
        .with(
            Route::SectionItems(SECTION_KEY.to_owned()),
            &[Step::Fixture("library_items.json")],
        )
        .with(Route::Search, &[Step::Fixture("search_results.json")])
        .with(
            Route::Metadata(RATING_KEY.to_owned()),
            &[Step::Fixture("metadata_item.json")],
        )
}

/// A mock plex.tv whose `resources` answer contains exactly the one row that
/// points at `pms_base` (the running mock PMS) and carries [`ROW_TOKEN`].
fn plex_tv_for(pms_base: &str) -> MockPlex {
    MockPlex::start(Script::new().with(
        Route::Resources,
        &[Step::FixtureJson(
            200,
            serde_json::to_string(&vec![server_row(pms_base)]).expect("mock row must serialize"),
        )],
    ))
}

/// Start a mock PMS and connect to it the way the product does: the account
/// token reads `resources` from a **mock plex.tv**, the row that comes back is
/// parsed (not hand-built), and `server::connect` uses **that row's** token.
///
/// Returns the plex.tv mock, the mock PMS and the connected server.
fn m3_session(tag: &str) -> (MockPlex, MockPlex, Server) {
    let dir = common::TempDir::new(tag);
    let client = common::client(&dir);
    let pms = MockPlex::start(library_script());
    let plex_tv = plex_tv_for(pms.base());

    let list = fetch_resources(
        &common::http_at(plex_tv.base(), &client),
        &SecretString::new(MOCK_TOKEN),
        "this-client-is-not-the-row",
    )
    .expect("the mock resources answer must parse");
    assert_eq!(
        list.servers().len(),
        1,
        "the scripted row is the only server"
    );
    assert_eq!(
        list.excluded().len(),
        0,
        "nothing in that row may be excluded"
    );

    let server = connect(&list.servers()[0], &client).expect("mock PMS must connect");
    assert!(server.machine_identifier().matches(MOCK_MACHINE));
    assert_eq!(server.base_url(), pms.base());
    assert!(server.token().matches(ROW_TOKEN));
    // A `connect` through a parsed row never marks the credential as delegated.
    assert!(!server.uses_delegated_token());
    (plex_tv, pms, server)
}

/// Percent-decode (`%XX` and `+`) of a recorded request's query part, so a test
/// can compare what the product asked for with what actually went on the wire.
fn decoded_query(record: &Recorded) -> String {
    let query = record
        .path_and_query
        .split_once('?')
        .map(|(_, query)| query)
        .unwrap_or("");
    let bytes = query.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or_default();
                let value = u8::from_str_radix(hex, 16).unwrap_or(b'?');
                out.push(value as char);
                i += 3;
            }
            other => {
                out.push(other as char);
                i += 1;
            }
        }
    }
    out
}

/// The query parameters of one recorded request as `(key, decoded value)`.
fn query_pairs(record: &Recorded) -> Vec<(String, String)> {
    decoded_query(record)
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            (key.to_owned(), value.to_owned())
        })
        .collect()
}

/// Path part only of a recorded request (query dropped).
fn path_of(record: &Recorded) -> String {
    record
        .path_and_query
        .split('?')
        .next()
        .unwrap_or_default()
        .to_owned()
}

/// Recorded requests whose path is `path`.
fn at_path(records: &[Recorded], path: &str) -> Vec<Recorded> {
    records
        .iter()
        .filter(|record| path_of(record) == path)
        .cloned()
        .collect()
}

/// Connect to a running mock PMS from a hand-built row of its own.
///
/// Used by the tests that are about **one specific library answer** (a fault
/// status, a broken shape). The `resources` -> row -> connect path is proven end
/// to end by [`library_calls_never_carry_the_account_token`]; re-running the
/// plex.tv step in every test would only repeat the same proof.
fn connect_mock_pms(pms: &MockPlex, client: &ClientInfo) -> Server {
    let list = parse_resources(
        serde_json::to_string(&vec![server_row(pms.base())])
            .expect("mock row must serialize")
            .as_bytes(),
        "this-client-is-not-the-row",
    )
    .expect("mock row must parse");
    connect(&list.servers()[0], client).expect("mock PMS must connect")
}

/// A connectable script that answers `route` with `step`.
fn script_with(route: Route, step: Step) -> Script {
    library_script().with(route, &[step])
}

/// A TCP port that was bound on loopback and then released, so nothing listens
/// there (the mock servers in that test hold the other ports).
fn closed_loopback_port() -> u16 {
    let listener =
        std::net::TcpListener::bind("127.0.0.1:0").expect("loopback port must be bindable");
    let port = listener.local_addr().expect("bound addr").port();
    drop(listener);
    port
}

/// The request line the product puts on the wire for one mock route, built from
/// the **product's own path constants** (nothing here is copied from the mock).
fn product_request_line(route: &Route) -> (&'static str, String) {
    match route {
        Route::Pins => ("POST", PINS_PATH.to_owned()),
        Route::PinPoll(id) => ("GET", format!("{PINS_PATH}/{id}")),
        Route::User => ("GET", USER_PATH.to_owned()),
        Route::Signout => ("DELETE", SIGNOUT_PATH.to_owned()),
        Route::Resources => ("GET", RESOURCES_PATH.to_owned()),
        Route::Root => ("GET", PROBE_PATH.to_owned()),
        Route::Identity => ("GET", IDENTITY_PATH.to_owned()),
        Route::Sections => ("GET", SECTIONS_PATH.to_owned()),
        Route::SectionItems(key) => ("GET", format!("{SECTION_ITEMS_PATH}/{key}/all")),
        Route::Search => ("GET", SEARCH_PATH.to_owned()),
        Route::Metadata(key) => ("GET", format!("{METADATA_PATH}/{key}")),
        Route::Hubs => ("GET", HUBS_PATH.to_owned()),
        Route::HubsSearch => ("GET", SEARCH_HUBS_PATH.to_owned()),
        Route::OnDeck => ("GET", ON_DECK_PATH.to_owned()),
        Route::Children(key) => ("GET", format!("{CHILDREN_PATH}/{key}/children")),
        Route::SectionSorts(key) => ("GET", format!("{SECTION_SORTS_PATH}/{key}/sorts")),
        // Both progress endpoints address the item in the query, so the request
        // line this table can compare is the path alone.
        Route::Timeline => ("GET", TIMELINE_PATH.to_owned()),
        Route::Scrobble => ("GET", SCROBBLE_PATH.to_owned()),
    }
}

/// Every [`Route`] variant the mock can answer, one canonical value each, with
/// the name this workspace gives it.
///
/// [`product_request_line`] matches on **every** variant, so a new `Route`
/// variant cannot be added to the mock without also naming what the product must
/// send for it: a route can neither drift from the product nor sit unused
/// without one of the two stops compiling.
fn all_routes() -> Vec<(&'static str, Route)> {
    vec![
        ("pins", Route::Pins),
        ("pin_poll", Route::PinPoll("pin-1".to_owned())),
        ("user", Route::User),
        ("signout", Route::Signout),
        ("resources", Route::Resources),
        ("root_probe", Route::Root),
        ("identity", Route::Identity),
        ("sections", Route::Sections),
        ("section_items", Route::SectionItems(SECTION_KEY.to_owned())),
        ("search", Route::Search),
        ("metadata", Route::Metadata(RATING_KEY.to_owned())),
        ("hubs", Route::Hubs),
        ("hubs_search", Route::HubsSearch),
        ("on_deck", Route::OnDeck),
        ("children", Route::Children(RATING_KEY.to_owned())),
        ("section_sorts", Route::SectionSorts(SECTION_KEY.to_owned())),
        ("timeline", Route::Timeline),
        ("scrobble", Route::Scrobble),
    ]
}

/// The routes this target exercises with real traffic. The four auth routes
/// (`pins`, `pin_poll`, `user`, `signout`) are M2's, exercised by
/// `tests/pin_flow.rs`, `tests/account.rs` and `tests/logout.rs`. `timeline` and
/// `scrobble` are the two state-changing routes, exercised by
/// [`progress_reporting_is_off_by_default_and_is_the_only_write`] (see
/// [`M6B_PROGRESS_ROUTES`]).
const M3_ROUTES: [&str; 7] = [
    "resources",
    "root_probe",
    "identity",
    "sections",
    "section_items",
    "search",
    "metadata",
];

/// M2's auth routes (see [`M3_ROUTES`]).
const M2_ROUTES: [&str; 4] = ["pins", "pin_poll", "user", "signout"];

/// M6b's two **state-changing** routes and the P1b read routes that are not
/// `GET`-shaped constants of the M3 set (see [`P1B_ROUTES`]).
///
/// Both progress calls are `GET` on the wire, so unlike every other route here
/// they are state changes by *path*, not by verb. Unlike the M2/M3 routes they
/// **are** exercised with real loopback traffic:
/// [`progress_reporting_is_off_by_default_and_is_the_only_write`] sends the
/// timeline report once, opted in, sends nothing at all by default, and sends
/// the scrobble for an explicit mark-watched.
/// `crates/plex-core/tests/mutation_surface.rs` holds the source side (which
/// call sites exist at all).
const M6B_PROGRESS_ROUTES: [&str; 2] = ["timeline", "scrobble"];

/// P1b's read routes: hubs, search hubs, on-deck, children of one item, and the
/// sorts one section offers. Each is driven with real loopback traffic (request
/// line asserted) by a named `#[test]` in
/// `crates/plex-core/tests/library_endpoints.rs`.
const P1B_ROUTES: [&str; 5] = [
    "hubs",
    "hubs_search",
    "on_deck",
    "children",
    "section_sorts",
];

#[test]
fn every_documented_route_is_routed_and_named() {
    let routes = all_routes();
    assert_eq!(routes.len(), 18, "every Route variant must be named");
    let mut names: Vec<&str> = routes.iter().map(|(name, ..)| *name).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), 18, "route names must be distinct");

    for (name, route) in &routes {
        let (method, line) = product_request_line(route);
        // The mock answers a scripted route by looking up `route_for(method,
        // path)`, so this is the drift check: the request line the **product**
        // builds must land on the variant a test arms, on the method the product
        // uses for it.
        let routed = route_for(method, &line)
            .unwrap_or_else(|| panic!("{name}: {method} {line} is not a known mock route"));
        assert_eq!(&routed, route, "{name}: route table must match the product");
    }

    // Nothing else is a known route: for a truncated or unknown path the mock has
    // no scripted answer at all (it answers 404, see the `Step` docs).
    for path in [
        "/library/sections/",
        "/library/metadata/",
        "/api/v2/pins/",
        "/identity/extra",
        "/library/sections/1/all/extra",
        "/library/sections/2/sorts/extra",
        "/library/metadata/1/children/extra",
        "/hubs/",
        "/hubs/search/extra",
        "/library/onDeck/extra",
        "/:/timeline/extra",
        "/:/scrobble/extra",
    ] {
        assert_eq!(route_for("GET", path), None, "{path} must not be a route");
    }

    // Exactly the M3 set is what this target drives; the rest of the table is
    // M2's and is named as such instead of silently uncovered here.
    let mut covered: Vec<&str> = routes
        .iter()
        .filter(|(name, ..)| M3_ROUTES.contains(name))
        .map(|(name, ..)| *name)
        .collect();
    covered.sort_unstable();
    let mut expected = M3_ROUTES.to_vec();
    expected.sort_unstable();
    assert_eq!(covered, expected, "every library route must be named here");

    // Nothing may sit in the mock table unowned by a milestone: every route is
    // either driven here, by the M2 auth tests, or by the single progress write
    // test. A new route that no test drives fails here.
    let mut driven: Vec<&str> = routes
        .iter()
        .map(|(name, ..)| *name)
        .filter(|name| {
            M3_ROUTES.contains(name)
                || M2_ROUTES.contains(name)
                || M6B_PROGRESS_ROUTES.contains(name)
                || P1B_ROUTES.contains(name)
        })
        .collect();
    driven.sort_unstable();
    assert_eq!(
        driven, names,
        "every mock route must be owned by a test that drives it"
    );
    // Which mock routes are sent with a non-`GET` method at all: the pin claim
    // and signout. The M6b progress calls changed this list when their real shape
    // was confirmed - they are `GET`, so **no** state-changing route of this
    // client is verb-identifiable, and a new non-`GET` route must appear here
    // rather than arriving silently.
    let mut non_get: Vec<(&str, &str)> = routes
        .iter()
        .map(|(name, route)| {
            let (method, _) = product_request_line(route);
            (*name, method)
        })
        .filter(|(.., method)| *method != "GET")
        .collect();
    non_get.sort_unstable();
    assert_eq!(
        non_get,
        vec![("pins", "POST"), ("signout", "DELETE")],
        "only the pin claim and signout are non-GET; the progress calls are GET"
    );
}

/// Drive the whole M3 read-only path over real loopback HTTP and assert which
/// credential reached which host: the **account** token only ever on the mock
/// plex.tv, the row's **own** `accessToken` only ever on the mock PMS.
#[test]
fn library_calls_never_carry_the_account_token() {
    let (plex_tv, pms, server) = m3_session("m3-token-split");

    let sections = list_sections(&server).expect("the mock must answer /library/sections");
    let section = sections[0]
        .id()
        .expect("the fixture section key must be addressable");
    let page = browse_items(&server, &section, Page::first(), &BrowseOptions::default())
        .expect("the mock must answer the section item page");
    let rating = page.items[0]
        .require_rating_key()
        .expect("the fixture item must carry a ratingKey");
    let found = search(&server, "Sample Generated").expect("the mock must answer /search");
    let item = metadata(&server, &rating).expect("the mock must answer /library/metadata");
    assert_eq!(found.items.len(), 1);
    assert_eq!(item.rating_key.as_deref(), Some(rating.as_str()));

    let tv = plex_tv.requests();
    let pms_reqs = pms.requests();

    // The account token is never seen on the PMS host - not in a header, not in
    // a request line, not in a body.
    assert!(
        no_record_carries(&pms_reqs, MOCK_TOKEN),
        "the account token must never reach a PMS host: {:?}",
        pms_reqs
            .iter()
            .map(|r| &r.path_and_query)
            .collect::<Vec<_>>()
    );
    // ...and the row token is never seen on the plex.tv host.
    assert!(
        no_record_carries(&tv, ROW_TOKEN),
        "a server row's own token must never reach plex.tv"
    );

    // The one plex.tv request presents the account token as a header only.
    assert_eq!(tv.len(), 1, "resources is one request");
    assert!(
        token_only_in_header(&tv[0], TOKEN_HEADER),
        "the account token may only travel in {}",
        TOKEN_HEADER
    );

    // The PMS host: credential-free liveness probe, then `identity` + the four
    // library calls, each with the **row** token in the header only.
    assert_eq!(
        requests_with_token_header(&pms_reqs, TOKEN_HEADER, ROW_TOKEN),
        5,
        "identity + 4 library calls carry the row token"
    );
    assert_eq!(
        pms_reqs
            .iter()
            .filter(|record| record.header(TOKEN_HEADER).is_none())
            .count(),
        1,
        "only the liveness probe is credential-free"
    );
    for record in &pms_reqs {
        assert!(
            !record.path_and_query.contains(ROW_TOKEN),
            "{}: the row token must not appear in a request line",
            record.path_and_query
        );
        assert_eq!(
            record.value_matches(ROW_TOKEN),
            usize::from(record.header(TOKEN_HEADER) == Some(ROW_TOKEN)),
            "{}: the row token appears at most once, as {}",
            record.path_and_query,
            TOKEN_HEADER
        );
    }
}

/// What a UI would list comes from the mock answer, not from the code: the
/// three fixture sections with their `key` / `title` / `type`, and the section
/// `id()` that [`browse_items`] needs.
#[test]
fn sections_come_from_the_mock_answer() {
    let (_plex_tv, pms, server) = m3_session("m3-sections");

    let sections = list_sections(&server).expect("the mock section list must parse");
    let described: Vec<(String, Option<String>, Option<String>)> = sections
        .iter()
        .map(|section| {
            (
                section.id().expect("fixture key must be addressable"),
                section.title.clone(),
                section.kind.clone(),
            )
        })
        .collect();
    assert_eq!(
        described,
        vec![
            (
                "1".to_owned(),
                Some("Sample Movies".to_owned()),
                Some("movie".to_owned())
            ),
            (
                "2".to_owned(),
                Some("Sample Shows".to_owned()),
                Some("show".to_owned())
            ),
            (
                "3".to_owned(),
                Some("Sample Photos".to_owned()),
                Some("photo".to_owned())
            ),
        ],
        "the section list must be exactly what the mock answered"
    );

    let ids: Vec<String> = sections.iter().map(|s| s.key.clone()).collect();
    assert_eq!(
        ids,
        vec![
            "/library/sections/1",
            "/library/sections/2",
            "/library/sections/3"
        ],
        "section keys are kept verbatim, not rewritten"
    );

    let requests = pms.requests();
    assert_eq!(requests.len(), 3, "probe + identity + one section list");
    assert_eq!(path_of(&requests[2]), SECTIONS_PATH);
    assert!(requests.iter().all(|record| record.method == "GET"));
}

/// `GET /library/sections/{key}/all` with this project's container window: the
/// window lands on the wire as the paging query, and the item fields shown to a
/// UI are the fixture's.
#[test]
fn browsing_items_pages_with_the_container_window() {
    let (_plex_tv, pms, server) = m3_session("m3-browse");

    let first = browse_items(
        &server,
        SECTION_KEY,
        Page::first(),
        &BrowseOptions::default(),
    )
    .expect("page 1 must parse");
    assert_eq!(first.items.len(), 2, "the fixture answers two items");
    assert_eq!(first.total, Some(2), "the server's own total is kept");
    assert_eq!(first.start, Some(0));
    assert!(
        first.identifier.is_none(),
        "the mock answered no identifier"
    );
    assert_eq!(
        first
            .items
            .iter()
            .map(|item| item.rating_key.clone())
            .collect::<Vec<_>>(),
        vec![Some("10".to_owned()), Some("11".to_owned())],
        "item order is the server's order"
    );
    assert_eq!(
        first.items[0].title.as_deref(),
        Some("Sample Generated Movie")
    );
    assert_eq!(first.items[0].kind.as_deref(), Some("movie"));
    assert_eq!(
        first.items[0].duration,
        Some(600_000),
        "duration kept verbatim"
    );
    assert_eq!(first.items[0].added_at, Some(1_700_000_000));
    assert!(first.items[0].has_media(), "the fixture item has parts");
    assert_eq!(first.items[0].parts.len(), 2);
    assert_eq!(first.items[0].parts[0].container.as_deref(), Some("mkv"));
    assert_eq!(first.items[0].parts[0].size, Some(1_048_576));
    assert!(!first.has_more(50), "2 items served for a window of 50");

    let second = browse_items(
        &server,
        SECTION_KEY,
        Page::new(25, 25).expect("legal page"),
        &BrowseOptions::default(),
    )
    .expect("page 2 request must be made the same way");
    assert_eq!(
        second.items.len(),
        2,
        "the same scripted answer, honestly reported"
    );

    let requests = pms.requests();
    let browsed: Vec<&Recorded> = requests
        .iter()
        .filter(|record| path_of(record) == format!("{SECTION_ITEMS_PATH}/{SECTION_KEY}/all"))
        .collect();
    assert_eq!(browsed.len(), 2, "exactly one request per page");
    assert_eq!(
        query_pairs(browsed[0]),
        vec![
            (CONTAINER_START_QUERY.to_owned(), "0".to_owned()),
            (CONTAINER_SIZE_QUERY.to_owned(), MAX_PAGE_SIZE.to_string()),
        ],
        "the window the product asked for is the window on the wire"
    );
    assert_eq!(
        query_pairs(browsed[1]),
        vec![
            (CONTAINER_START_QUERY.to_owned(), "25".to_owned()),
            (CONTAINER_SIZE_QUERY.to_owned(), "25".to_owned()),
        ]
    );
    for record in &browsed {
        assert!(
            !record.query_contains(TOKEN_HEADER),
            "no credential in a query"
        );
        assert_eq!(record.header(TOKEN_HEADER), Some(ROW_TOKEN));
    }
}

/// `GET /search?query=…`: the query travels as a query **parameter**, the
/// credential as a header, and the matches are the mock's.
#[test]
fn search_sends_the_query_as_a_parameter_and_the_token_as_a_header() {
    let (_plex_tv, pms, server) = m3_session("m3-search");

    let results =
        search(&server, "Sample Generated Movie").expect("the mock search answer must parse");
    assert_eq!(results.items.len(), 1);
    assert_eq!(results.total, Some(1));
    assert_eq!(results.items[0].rating_key.as_deref(), Some("10"));
    assert_eq!(
        results.items[0].title.as_deref(),
        Some("Sample Generated Movie")
    );
    assert!(
        !results.items[0].has_media(),
        "the search fixture carries no parts"
    );

    let searches = at_path(&pms.requests(), SEARCH_PATH);
    assert_eq!(searches.len(), 1, "one search per call");
    assert_eq!(
        query_pairs(&searches[0]),
        vec![("query".to_owned(), "Sample Generated Movie".to_owned())],
        "what the mock saw decoded is exactly the query the product passed"
    );
    assert!(!searches[0].query_contains(ROW_TOKEN));
    assert_eq!(searches[0].header(TOKEN_HEADER), Some(ROW_TOKEN));
}

/// `GET /library/metadata/{ratingKey}`: one item, with the media parts the
/// answer nests under `Media`.
#[test]
fn metadata_reads_one_item_with_its_media_part() {
    let (_plex_tv, pms, server) = m3_session("m3-metadata");

    let item = metadata(&server, RATING_KEY).expect("the mock metadata answer must parse");
    assert_eq!(item.rating_key.as_deref(), Some(RATING_KEY));
    assert_eq!(item.key.as_deref(), Some("/library/metadata/10"));
    assert_eq!(item.kind.as_deref(), Some("movie"));
    assert_eq!(item.title.as_deref(), Some("Sample Generated Movie"));
    assert_eq!(item.index, Some(1));
    assert_eq!(
        item.parent_title.as_deref(),
        Some("Sample Generated Collection")
    );
    assert_eq!(item.duration, Some(600_000));
    assert_eq!(item.added_at, Some(1_700_000_000));
    assert_eq!(item.parts.len(), 1, "the parts under Media are read");
    assert_eq!(
        item.parts[0].key.as_deref(),
        Some("/library/parts/10/1/file.mkv")
    );
    assert_eq!(item.parts[0].protocol.as_deref(), Some("http"));
    assert!(item.has_media());

    let calls = at_path(&pms.requests(), &format!("{METADATA_PATH}/{RATING_KEY}"));
    assert_eq!(calls.len(), 1);
    assert!(!calls[0].query_contains(TOKEN_HEADER));
    assert_eq!(calls[0].header(TOKEN_HEADER), Some(ROW_TOKEN));
}

/// An unvalidated key, an absurd query or an oversized page is refused **before
/// a request is built**: the mock records zero additional traffic.
#[test]
fn invalid_keys_and_queries_produce_no_request_at_all() {
    let (_plex_tv, pms, server) = m3_session("m3-invalid");

    list_sections(&server).expect("one baseline request must have been made");
    let baseline = pms.requests().len();

    let rejected = [
        browse_items(&server, "1/all", Page::first(), &BrowseOptions::default()).err(),
        browse_items(&server, "../2", Page::first(), &BrowseOptions::default()).err(),
        browse_items(&server, "1?a=2", Page::first(), &BrowseOptions::default()).err(),
        browse_items(&server, "", Page::first(), &BrowseOptions::default()).err(),
        metadata(&server, "10/11").err(),
        metadata(&server, "..").err(),
        metadata(&server, &"9".repeat(MAX_KEY_LEN + 1)).err(),
        search(&server, "").err(),
        search(&server, "movie\nanything").err(),
        search(&server, &"q".repeat(MAX_QUERY_LEN + 1)).err(),
        Page::new(0, 0).err(),
        Page::new(0, MAX_PAGE_SIZE + 1).err(),
        validate_segment("a/b").err(),
        validate_segment("has space").err(),
        validate_query("tab\there").err(),
    ];
    for error in rejected.iter().flatten() {
        assert_eq!(error, &CoreError::InvalidInput, "must be invalid-input");
        assert_eq!(error.code(), "invalid-input");
    }
    assert_eq!(rejected.iter().filter(|error| error.is_some()).count(), 15);

    // Acceptable values really are accepted, so the guard above is not just "all
    // inputs are refused".
    // `validate_segment`/`validate_query` hand back the accepted value as
    // `Ok(String)`, so the payload is taken out of the `Result` first.
    assert_eq!(validate_segment(SECTION_KEY).ok().as_deref(), Some("1"));
    assert_eq!(
        validate_segment("show-1_2.3").ok().as_deref(),
        Some("show-1_2.3")
    );
    assert_eq!(
        validate_query("Sample Generated Movie").ok().as_deref(),
        Some("Sample Generated Movie")
    );
    assert_eq!(
        Page::first(),
        Page {
            start: 0,
            size: MAX_PAGE_SIZE
        }
    );

    assert_eq!(
        pms.requests().len(),
        baseline,
        "a rejected call must make no request"
    );
}

/// Run the library call that belongs to `route`, so a fault can be armed on
/// exactly the route that is then exercised.
fn call_for(route: &Route, server: &Server) -> Result<(), CoreError> {
    match route {
        Route::Sections => list_sections(server).map(|sections| assert_eq!(sections.len(), 3)),
        Route::SectionItems(key) => {
            browse_items(server, key, Page::first(), &BrowseOptions::default()).map(|_| ())
        }
        Route::Search => search(server, "Sample").map(|_| ()),
        Route::Metadata(key) => metadata(server, key).map(|_| ()),
        other => panic!("no library call for {other:?}"),
    }
}

/// Every request this milestone makes is a `GET` on a documented route, with the
/// row credential in the header only. Nothing here can change a server.
#[test]
fn library_traffic_is_read_only_and_only_uses_documented_routes() {
    let (plex_tv, pms, server) = m3_session("m3-read-only");

    let sections = list_sections(&server).expect("sections must list");
    let key = sections[0].id().expect("fixture key must be addressable");
    browse_items(&server, &key, Page::first(), &BrowseOptions::default())
        .expect("items must browse");
    search(&server, "Sample").expect("search must answer");
    metadata(&server, RATING_KEY).expect("metadata must answer");

    let pms_reqs = pms.requests();
    let tv = plex_tv.requests();
    let mut lines: Vec<(String, String)> = pms_reqs
        .iter()
        .chain(tv.iter())
        .map(|record| (record.method.clone(), path_of(record)))
        .collect();
    lines.sort();
    let mut expected: Vec<(String, String)> = vec![
        ("GET".to_owned(), PROBE_PATH.to_owned()),
        ("GET".to_owned(), IDENTITY_PATH.to_owned()),
        ("GET".to_owned(), SECTIONS_PATH.to_owned()),
        (
            "GET".to_owned(),
            format!("{SECTION_ITEMS_PATH}/{SECTION_KEY}/all"),
        ),
        ("GET".to_owned(), SEARCH_PATH.to_owned()),
        ("GET".to_owned(), format!("{METADATA_PATH}/{RATING_KEY}")),
        ("GET".to_owned(), RESOURCES_PATH.to_owned()),
    ];
    expected.sort();
    assert_eq!(
        lines, expected,
        "browsing makes exactly these requests and nothing else (no POST/PUT/DELETE/PATCH)"
    );

    assert_eq!(
        requests_with_token_header(&pms_reqs, TOKEN_HEADER, ROW_TOKEN),
        pms_reqs.len() - 1,
        "every credentialed PMS request presents the row token as a header"
    );
    assert!(
        no_record_carries(&pms_reqs, MOCK_TOKEN),
        "no account token on the PMS"
    );
}

/// A non-2xx answer, an unparseable answer and an answer with no item are typed
/// errors, surfaced once (no invented retry), and their rendered text carries no
/// token and no server body.
#[test]
fn library_faults_are_typed_and_render_token_free() {
    let dir = common::TempDir::new("m3-faults");
    let client = common::client(&dir);

    // Each fault is armed on a route/section key nothing else in this file
    // scripts, so the fault is the only possible answer for that call.
    let cases: [(&str, Route, Step, CoreError); 4] = [
        (
            "429",
            Route::SectionItems("2".to_owned()),
            Step::Respond(429, r#"{"errors":["mock: rate limited"]}"#),
            CoreError::HttpStatus { status: 429 },
        ),
        (
            "401",
            Route::SectionItems("3".to_owned()),
            Step::Respond(401, r#"{"errors":["mock: rejected token"]}"#),
            CoreError::AuthRejected { status: 401 },
        ),
        (
            "not-json",
            Route::SectionItems("4".to_owned()),
            Step::Respond(200, "this is not JSON at all"),
            CoreError::ResponseShape,
        ),
        (
            "no-item",
            Route::Metadata("77".to_owned()),
            Step::Respond(200, r#"{"MediaContainer":{"size":0,"Directory":[]}}"#),
            CoreError::ResponseShape,
        ),
    ];

    for (name, route, step, want) in &cases {
        let pms = MockPlex::start(script_with(route.clone(), step.clone()));
        let server = connect_mock_pms(&pms, &client);
        let before = pms.requests().len();

        let error = call_for(route, &server)
            .err()
            .unwrap_or_else(|| panic!("{name}: the armed fault must be an error"));
        assert_eq!(&error, want, "{name}: fault must map to this variant");
        assert_eq!(error.code(), want.code(), "{name}: stable error code");

        let rendered = format!("{error}");
        let debugged = format!("{error:?}");
        for text in [rendered.as_str(), debugged.as_str()] {
            assert!(
                !text.contains(ROW_TOKEN),
                "{name}: error text carries the token"
            );
            assert!(
                !text.contains(MOCK_TOKEN),
                "{name}: error text carries the account token"
            );
            assert!(
                !text.contains("Sample"),
                "{name}: error text carries a server body"
            );
        }

        // Surfaced once: this client invents no retry/backoff for 429 or any
        // other status (`docs/CAPABILITIES.md`, 429 rules = not implemented).
        assert_eq!(
            pms.requests().len(),
            before + 1,
            "{name}: exactly one library request, no retry"
        );
    }
}

/// An offline server is reported as an unreachable candidate with the address a
/// user can actually fix, and a server that stops answering mid-session is a
/// transport failure - never an empty library and never a fabricated item.
#[test]
fn an_offline_server_is_reported_with_an_actionable_failure() {
    let dir = common::TempDir::new("m3-offline");
    let client = common::client(&dir);
    let port = closed_loopback_port();
    let base = format!("http://127.0.0.1:{port}");

    let list = parse_resources(
        serde_json::to_string(&vec![server_row(&base)])
            .expect("mock row must serialize")
            .as_bytes(),
        "this-client-is-not-the-row",
    )
    .expect("mock row must parse");
    let failure = match connect(&list.servers()[0], &client) {
        Ok(server) => panic!(
            "nothing listens at {port}, so connect must fail: {}",
            server.base_url()
        ),
        Err(failure) => failure,
    };

    assert_eq!(
        failure.attempts().len(),
        1,
        "one candidate, one recorded attempt"
    );
    let attempt = &failure.attempts()[0];
    assert_eq!(attempt.kind, AttemptKind::Unreachable);
    assert!(
        attempt.base_url.contains(&port.to_string()),
        "the attempt names the port"
    );
    let line = attempt.message();
    assert!(
        line.contains("offline"),
        "the attempt line must say why: {line}"
    );
    assert!(
        !line.contains(ROW_TOKEN),
        "an attempt line is not a credential dump"
    );
    assert!(
        !failure.message().contains(ROW_TOKEN),
        "the summary is token-free"
    );
    assert!(
        !failure.requires_https(),
        "this server published no httpsRequired"
    );

    // A server that was connected and then stops answering is a transport
    // failure on the library call itself, not an empty result. The fault is
    // armed while the mock still listens, so "one request, no invented retry"
    // is observable in its request log.
    let pms = MockPlex::start(library_script());
    let server = connect_mock_pms(&pms, &client);
    list_sections(&server).expect("the mock answers while it is up");
    let seen = pms.requests().len();

    pms.set_script(Script::new().with(Route::Sections, &[Step::CutConnection]));
    let error = match list_sections(&server) {
        Ok(sections) => panic!(
            "a mock that cuts the connection must not answer, got {} sections",
            sections.len()
        ),
        Err(error) => error,
    };
    assert_eq!(
        error,
        CoreError::Transport,
        "a connection cut mid-answer is transport"
    );
    assert_eq!(error.code(), "transport");
    assert_eq!(
        pms.requests().len(),
        seen + 1,
        "the cut connection is one more request, not a retry loop"
    );

    // Last: a host that is gone altogether. `MockPlex::shutdown` consumes the
    // mock and closes the listener, so nothing can be recorded after this
    // point; what is asserted here is only that no sections are fabricated
    // for a host that refuses the connection.
    pms.shutdown();
    let error = match list_sections(&server) {
        Ok(sections) => panic!(
            "a shut-down mock must not answer, got {} sections",
            sections.len()
        ),
        Err(error) => error,
    };
    assert_eq!(
        error,
        CoreError::Transport,
        "a refused connection is transport"
    );
    assert_eq!(error.code(), "transport");
}

// ---------------------------------------------------------------------------
// M6b: the one opt-in write, measured as request records
// ---------------------------------------------------------------------------

/// **M6b mutation guard, mock-execution half.** Playback position is opt-in, off
/// by default, and the position report plus the explicit mark-watched call are
/// the only requests this client can make that change what a server remembers -
/// measured against request records the mock took from real loopback HTTP
/// traffic, not against source text (the source half is `tests/mutation_surface.rs`).
///
/// In order:
///
/// 1. `ProgressUpdate::new` is **off**, and `progress::report_timeline` on it is
///    `Outcome::NotEnabled` with **zero** new requests on either mock host and no
///    `POST`/`PUT`/`PATCH`/`DELETE` anywhere: the default never reaches a
///    connection.
/// 2. After the explicit [`ProgressUpdate::opt_in`], `report_timeline` yields
///    `Reported { status: 200 }` and exactly **one** new request exists:
///    `GET /:/timeline` carrying `ratingKey`, `identifier`, `state`, `time` and
///    `duration` in **milliseconds**, with the row credential **only** as
///    `X-Plex-Token`, while the mock plex.tv host sees no new request at all.
/// 3. `progress::mark_watched` sends `GET /:/scrobble?key={RATING_KEY}` with the
///    same `identifier`, once - and it is **not** gated by the opt-in, because a
///    person asked for it rather than playback producing it.
///
/// The mock answers these two paths because this project wrote the mock, so what
/// is proven here is this client's own default-off / one-request behaviour, not
/// that a real Plex Media Server accepts the shape (that stays a user-verified
/// gate, `docs/CAPABILITIES.md`).
#[test]
fn progress_reporting_is_off_by_default_and_is_the_only_write() {
    let (plex_tv, pms, server) = m3_session("m6b-progress-report");
    pms.set_script(
        library_script()
            .with(
                Route::Timeline,
                &[Step::Respond(200, r#"{"mock":"timeline-accepted"}"#)],
            )
            .with(
                Route::Scrobble,
                &[Step::Respond(200, r#"{"mock":"scrobble-accepted"}"#)],
            ),
    );
    let baseline_pms = pms.requests().len();
    let baseline_tv = plex_tv.requests().len();

    // 1. Default state: off, and off costs no transport at all.
    let off = ProgressUpdate::new(RATING_KEY, PlayState::Playing)
        .with_position_ms(Some(41_000), Some(300_000));
    assert!(!off.enabled, "a freshly built report is off");
    assert_eq!(
        off.refusal(),
        Some("progress reporting is off (opt-in required)"),
        "the opt-in is the first thing refusal() checks"
    );
    let outcome =
        report_progress(&server, &off).expect("a refused report is an Outcome, not an error");
    assert_eq!(
        outcome,
        ProgressOutcome::NotEnabled {
            reason: "progress reporting is off (opt-in required)",
        },
        "the default is answered locally"
    );
    assert_eq!(outcome.label(), "not-enabled-no-request");
    assert!(!outcome.made_a_request());
    assert_eq!(
        pms.requests().len(),
        baseline_pms,
        "a disabled report added no request to the mock PMS"
    );
    assert_eq!(
        plex_tv.requests().len(),
        baseline_tv,
        "a disabled report added no request to plex.tv either"
    );
    for verb in ["POST", "PUT", "PATCH", "DELETE"] {
        assert_eq!(
            pms.count_method(verb),
            0,
            "the mock PMS saw no {verb} before any opt-in: {:?}",
            pms.requests()
        );
        assert_eq!(
            plex_tv.count_method(verb),
            0,
            "the mock plex.tv saw no {verb} before any opt-in"
        );
    }

    // 2. Opt-in: exactly one timeline report, on the PMS only.
    let opted = off.clone().opt_in();
    assert!(
        opted.enabled,
        "opt_in is the only way this becomes reportable"
    );
    assert_eq!(opted.refusal(), None, "the scripted report is valid");
    let reported =
        report_progress(&server, &opted).expect("the mock answers the scripted timeline route");
    assert_eq!(reported, ProgressOutcome::Reported { status: 200 });
    assert!(reported.made_a_request());

    let pms_reqs = pms.requests();
    let tv_reqs = plex_tv.requests();
    assert_eq!(
        pms_reqs.len(),
        baseline_pms + 1,
        "exactly one request was added to the mock PMS: {pms_reqs:?}"
    );
    assert_eq!(tv_reqs.len(), baseline_tv, "plex.tv saw nothing new");
    let non_gets: Vec<(String, String)> = pms_reqs
        .iter()
        .filter(|record| record.method != "GET")
        .map(|record| (record.method.clone(), path_of(record)))
        .collect();
    assert!(
        non_gets.is_empty(),
        "every PMS request this client makes is a GET, the progress reports included: {non_gets:?}"
    );
    assert_eq!(
        tv_reqs
            .iter()
            .filter(|record| record.method != "GET")
            .count(),
        0,
        "the account host is read-only for this client"
    );

    let reports = at_path(&pms_reqs, TIMELINE_PATH);
    let record = reports
        .first()
        .expect("the timeline report must be recorded");
    assert_eq!(record.method, "GET");
    assert_eq!(record.body, "", "the timeline report is query-only");
    let mut pairs = query_pairs(record);
    pairs.sort();
    assert_eq!(
        pairs,
        vec![
            (QUERY_DURATION.to_owned(), "300000".to_owned()),
            (QUERY_IDENTIFIER.to_owned(), LIBRARY_IDENTIFIER.to_owned()),
            (QUERY_RATING_KEY.to_owned(), RATING_KEY.to_owned()),
            (QUERY_STATE.to_owned(), "playing".to_owned()),
            (QUERY_TIME.to_owned(), "41000".to_owned()),
        ],
        "the timeline query names the item, the library plugin, and the position in milliseconds"
    );
    assert!(
        !pairs.iter().any(|(name, _)| name == QUERY_KEY),
        "a report without an item path omits the optional `key` rather than guessing one"
    );
    assert_eq!(
        requests_with_token_header(&reports, TOKEN_HEADER, ROW_TOKEN),
        1,
        "the row credential is presented as a header on the report"
    );
    assert_eq!(
        record.value_matches(ROW_TOKEN),
        1,
        "the row credential appears exactly once in the request"
    );
    assert!(!record.query_contains(ROW_TOKEN), "never in the query");
    assert!(
        !record.path_and_query.contains(ROW_TOKEN),
        "never in the request line"
    );
    assert!(
        no_record_carries(&pms_reqs, MOCK_TOKEN),
        "the account token never reaches the PMS, not even on its progress report"
    );

    // 3. The explicit mark-watched is its own call, and its own request.
    let before_scrobble = pms.requests().len();
    let scrobbled =
        mark_watched(&server, RATING_KEY).expect("the mock answers the scripted scrobble route");
    assert_eq!(scrobbled, 200, "the scrobble's whole answer is its status");
    let scrobble_reqs = pms.requests();
    assert_eq!(
        scrobble_reqs.len(),
        before_scrobble + 1,
        "exactly one request was added by marking watched: {scrobble_reqs:?}"
    );
    assert_eq!(
        plex_tv.requests().len(),
        baseline_tv,
        "marking watched touched the PMS, not the account host"
    );
    let scrobbles = at_path(&scrobble_reqs, SCROBBLE_PATH);
    let record = scrobbles.first().expect("the scrobble must be recorded");
    assert_eq!(record.method, "GET");
    assert_eq!(record.body, "", "the scrobble is query-only");
    let mut pairs = query_pairs(record);
    pairs.sort();
    assert_eq!(
        pairs,
        vec![
            (QUERY_IDENTIFIER.to_owned(), LIBRARY_IDENTIFIER.to_owned()),
            ("key".to_owned(), RATING_KEY.to_owned()),
        ],
        "/:/scrobble addresses the item by `key` (not `ratingKey`) and names the library plugin"
    );
    assert_eq!(
        requests_with_token_header(&scrobbles, TOKEN_HEADER, ROW_TOKEN),
        1,
        "the row credential is presented as a header on the scrobble"
    );
    assert!(
        no_record_carries(&scrobble_reqs, MOCK_TOKEN),
        "marking watched never presents the account token to the PMS"
    );
}

// ----------------------------------------------------------------- captures
//
// The tests below replay `tests/fixtures/real/*.json` (answers a live Plex
// Media Server sent to this client's own transport) through the same product
// paths as the hand-authored fixtures, so a model that names a field the
// server does not send, or misses one it does, fails here.

/// Rating keys and a section key those captures were taken for.
const CAPTURED_MOVIE: &str = "30874";
const CAPTURED_EPISODE: &str = "36100";
const CAPTURED_SHOW: &str = "35773";
const CAPTURED_SECTION: &str = "2";

fn captured_metadata(key: &str, fixture: &'static str, tag: &str) -> (MockPlex, Server) {
    let (_plex_tv, pms, server) = m3_session(tag);
    pms.set_script(script_with(
        Route::Metadata(key.to_owned()),
        Step::Fixture(fixture),
    ));
    (pms, server)
}

/// `GET /library/metadata/{ratingKey}` over the captured movie answer.
#[test]
fn the_shape_of_a_real_movie_answer() {
    let (pms, server) =
        captured_metadata(CAPTURED_MOVIE, "real/movie_metadata.json", "m3-cap-movie");
    let item = metadata(&server, CAPTURED_MOVIE).expect("the captured movie answer must parse");

    assert_eq!(item.rating_key.as_deref(), Some(CAPTURED_MOVIE));
    assert_eq!(item.key.as_deref(), Some("/library/metadata/30874"));
    assert_eq!(item.title.as_deref(), Some("The 'Brook"));
    assert_eq!(item.kind.as_deref(), Some("movie"));
    assert_eq!(item.year, Some(1989));
    assert_eq!(item.content_rating.as_deref(), Some("gb/PG"));
    assert_eq!(item.audience_rating, Some(7.1));
    assert_eq!(item.rating, Some(5.7));
    assert!(item.summary.is_some(), "the answer carries a summary");
    assert_eq!(
        item.tagline.as_deref(),
        Some("Every map leaves something out.")
    );
    assert_eq!(
        item.genres,
        vec![
            "Comedy".to_owned(),
            "Thriller".to_owned(),
            "Mystery".to_owned()
        ],
        "Genre[].tag in the answer's order"
    );
    assert_eq!(item.directors, vec!["Noor Sallow".to_owned()]);
    assert_eq!(item.roles.len(), 3);
    assert_eq!(item.roles[0].tag.as_deref(), Some("Ida Rosswood"));
    assert_eq!(item.roles[0].role.as_deref(), Some("Theo O'Dell"));
    assert!(
        item.roles[0]
            .thumb
            .as_deref()
            .unwrap_or_default()
            .starts_with("https://metadata-static.plex.tv/"),
        "a credited Role carries a remote Role thumb"
    );
    assert_eq!(item.images.len(), 3);
    assert_eq!(item.images[0].alt.as_deref(), Some("The 'Brook"));
    assert_eq!(
        item.image_url("clearLogo"),
        Some("/library/metadata/30874/clearLogo/1789186896"),
        "the clearLogo Image is read from Image[]"
    );
    assert_eq!(
        item.thumb.as_deref(),
        Some("/library/metadata/30874/thumb/1789186896")
    );
    assert_eq!(
        item.art.as_deref(),
        Some("/library/metadata/30874/art/1789186896")
    );
    assert_eq!(item.library_section_id, Some(2));
    assert_eq!(item.duration, Some(6_081_194));
    assert_eq!(item.added_at, Some(1_762_085_654));
    assert_eq!(
        (
            item.index,
            item.parent_index,
            item.parent_rating_key.as_deref(),
            item.grandparent_rating_key.as_deref(),
            item.grandparent_title.as_deref(),
            item.grandparent_thumb.as_deref(),
            item.grandparent_art.as_deref(),
        ),
        (None, None, None, None, None, None, None),
        "a movie row records no season/show parentage"
    );
    assert_eq!(
        (
            item.view_offset,
            item.view_count,
            item.leaf_count,
            item.viewed_leaf_count,
            item.child_count,
        ),
        (None, None, None, None, None),
        "this row records no play or leaf counts"
    );

    assert_eq!(item.media.len(), 1);
    let media = &item.media[0];
    assert_eq!(media.id, Some(52844), "the version id, not the item key");
    assert_eq!(media.duration, Some(6_081_194));
    assert_eq!(media.bitrate, Some(7777));
    assert_eq!(media.video_resolution.as_deref(), Some("1080"));
    assert_eq!(media.video_codec.as_deref(), Some("h264"));
    assert_eq!(media.video_profile.as_deref(), Some("high"));
    assert_eq!(media.audio_codec.as_deref(), Some("aac"));
    assert_eq!(media.audio_channels, Some(2));
    assert_eq!(media.container.as_deref(), Some("mkv"));
    assert_eq!(media.parts.len(), 1);

    let part = &item.parts[0];
    assert_eq!(item.parts.len(), 1, "the Media part is the item's part");
    assert_eq!(
        part.key.as_deref(),
        Some("/library/parts/52844/1695796561/file.mkv")
    );
    assert_eq!(part.container.as_deref(), Some("mkv"));
    assert_eq!(part.size, Some(5_911_954_320));
    assert_eq!(part.duration, Some(6_081_194));
    assert_eq!(
        part.protocol, None,
        "no captured answer names a Part protocol"
    );
    assert_eq!(
        part.streams.len(),
        3,
        "video, audio and subtitle in one order"
    );

    let video = &part.streams[0];
    assert_eq!(video.stream_type, Some(1));
    assert_eq!(video.codec.as_deref(), Some("h264"));
    assert_eq!(video.display_title.as_deref(), Some("1080p"));
    assert!(video.default, "the answer marks the video stream default");
    assert!(
        !video.selected,
        "only the chosen audio stream is marked selected"
    );
    assert_eq!(video.channels, None);
    let audio = &part.streams[1];
    assert_eq!(audio.stream_type, Some(2));
    assert_eq!(audio.codec.as_deref(), Some("aac"));
    assert_eq!(audio.display_title.as_deref(), Some("English (AAC Stereo)"));
    assert_eq!(audio.language.as_deref(), Some("English"));
    assert_eq!(audio.language_tag.as_deref(), Some("en"));
    assert_eq!(audio.channels, Some(2));
    assert!(
        audio.selected,
        "the answer marks this audio stream selected"
    );
    let subtitle = &part.streams[2];
    assert_eq!(subtitle.stream_type, Some(3));
    assert_eq!(subtitle.codec.as_deref(), Some("eia_608"));
    assert!(!subtitle.selected);

    assert_eq!(item.markers.len(), 1, "Marker[] is read from the item row");
    let marker = &item.markers[0];
    assert_eq!(marker.marker_type.as_deref(), Some("credits"));
    assert_eq!(marker.start_offset_ms, Some(5_845_074));
    assert_eq!(
        marker.end_offset_ms,
        Some(6_081_194),
        "credits run to the end"
    );
    assert!(item.chapters.is_empty(), "this answer names no chapters");
    assert!(item.has_media());

    let calls = at_path(
        &pms.requests(),
        &format!("{METADATA_PATH}/{CAPTURED_MOVIE}"),
    );
    assert_eq!(calls.len(), 1);
    assert!(!calls[0].query_contains(TOKEN_HEADER));
    assert_eq!(calls[0].header(TOKEN_HEADER), Some(ROW_TOKEN));
}

/// `GET /library/metadata/{ratingKey}` over the captured episode answer: the
/// parentage chain, the credits marker and the chapters.
#[test]
fn the_shape_of_a_real_episode_answer() {
    let (_pms, server) = captured_metadata(
        CAPTURED_EPISODE,
        "real/episode_metadata.json",
        "m3-cap-episode",
    );
    let item = metadata(&server, CAPTURED_EPISODE).expect("the captured episode answer must parse");

    assert_eq!(item.title.as_deref(), Some("Pilot"));
    assert_eq!(item.kind.as_deref(), Some("episode"));
    assert_eq!(item.year, Some(2014));
    assert_eq!(item.content_rating.as_deref(), Some("TV-14"));
    assert_eq!(item.audience_rating, Some(6.7));
    assert_eq!(item.rating, None, "this row names no critic rating");
    assert!(item.tagline.is_none(), "no tagline on this row");
    assert!(item.summary.is_some());
    assert!(item.genres.is_empty(), "an episode row names no Genre[]");
    assert_eq!(item.directors, vec!["Dag Halloran".to_owned()]);
    assert_eq!(item.roles.len(), 3);
    assert_eq!(item.roles[0].tag.as_deref(), Some("Milo Larkspur"));
    assert_eq!(item.index, Some(1), "episode index within the season");
    assert_eq!(item.parent_index, Some(1), "season index");
    assert_eq!(item.parent_title.as_deref(), Some("Season 1"));
    assert_eq!(item.parent_rating_key.as_deref(), Some("36099"));
    assert_eq!(item.grandparent_title.as_deref(), Some("The 404"));
    assert_eq!(item.grandparent_rating_key.as_deref(), Some(CAPTURED_SHOW));
    assert_eq!(
        item.grandparent_thumb.as_deref(),
        Some("/library/metadata/35773/thumb/1787107952")
    );
    assert_eq!(
        item.parent_thumb.as_deref(),
        Some("/library/metadata/36099/thumb/1774015269"),
        "the season's own thumb, the poster a season card draws"
    );
    assert_eq!(
        item.grandparent_art.as_deref(),
        Some("/library/metadata/35773/art/1787107952")
    );
    assert_eq!(
        item.thumb.as_deref(),
        Some("/library/metadata/36100/thumb/1774055077")
    );
    assert_eq!(
        item.art.as_deref(),
        Some("/library/metadata/35773/art/1787107952"),
        "the art is the show's"
    );
    assert_eq!(item.images.len(), 3);
    assert_eq!(
        item.image_url("snapshot"),
        Some("/library/metadata/36100/thumb/1774055077"),
        "the episode still is the snapshot Image"
    );
    assert_eq!(item.library_section_id, Some(1));
    assert_eq!(item.duration, Some(2_476_480));

    assert_eq!(item.markers.len(), 1);
    assert_eq!(item.markers[0].marker_type.as_deref(), Some("credits"));
    assert_eq!(item.markers[0].start_offset_ms, Some(2_450_832));
    assert_eq!(item.markers[0].end_offset_ms, Some(2_476_480));

    let chapters = &item.chapters;
    assert_eq!(chapters.len(), 3, "Chapter[] is read from the item row");
    assert_eq!(
        chapters
            .iter()
            .map(|chapter| (
                chapter.index,
                chapter.start_offset_ms,
                chapter.end_offset_ms
            ))
            .collect::<Vec<_>>(),
        vec![
            (Some(1), Some(0), Some(162_662)),
            (Some(2), Some(162_662), Some(609_108)),
            (Some(3), Some(609_108), Some(1_067_320)),
        ],
        "chapter index and ranges, in the answer's order"
    );
    assert_eq!(
        chapters[0].thumb.as_deref(),
        Some("/library/media/62023/chapterImages/1")
    );

    let media = &item.media[0];
    assert_eq!(media.id, Some(62023));
    assert_eq!(media.bitrate, Some(19_506));
    assert_eq!(media.video_resolution.as_deref(), Some("1080"));
    assert_eq!(media.video_codec.as_deref(), Some("h264"));
    assert_eq!(media.video_profile.as_deref(), Some("high"));
    assert_eq!(media.audio_codec.as_deref(), Some("dca-ma"));
    assert_eq!(media.audio_channels, Some(6));
    assert_eq!(media.container.as_deref(), Some("mkv"));

    let part = &item.parts[0];
    assert_eq!(item.parts.len(), 1);
    assert_eq!(
        part.key.as_deref(),
        Some("/library/parts/62023/1644270344/file.mkv")
    );
    assert_eq!(part.size, Some(6_038_236_088));
    let audio = &part.streams[1];
    assert_eq!(audio.stream_type, Some(2));
    assert_eq!(audio.codec.as_deref(), Some("dca"));
    assert_eq!(
        audio.display_title.as_deref(),
        Some("English (DTS-HD MA 5.1)")
    );
    assert_eq!(audio.channels, Some(6));
    assert!(audio.selected);
    assert_eq!(audio.language.as_deref(), Some("English"));
    assert_eq!(audio.language_tag.as_deref(), Some("en"));
    assert!(item.has_media());
}

/// The markers and chapters a captured item answer reports are asked for on
/// the wire: both query keys travel with the request, the token stays in the
/// header.
#[test]
fn metadata_asks_for_markers_and_chapters_on_the_wire() {
    let (pms, server) = captured_metadata(
        CAPTURED_EPISODE,
        "real/episode_metadata.json",
        "m3-cap-wire",
    );
    metadata(&server, CAPTURED_EPISODE).expect("the captured episode answer must parse");

    let calls = at_path(
        &pms.requests(),
        &format!("{METADATA_PATH}/{CAPTURED_EPISODE}"),
    );
    assert_eq!(calls.len(), 1);
    let pairs = query_pairs(&calls[0]);
    assert!(
        pairs.contains(&("includeMarkers".to_owned(), "1".to_owned())),
        "the request names the markers query key with the documented value"
    );
    assert!(
        pairs.contains(&("includeChapters".to_owned(), "1".to_owned())),
        "the request names the chapters query key with the documented value"
    );
    assert!(
        !calls[0].query_contains(TOKEN_HEADER),
        "no credential in a query"
    );
    assert_eq!(calls[0].header(TOKEN_HEADER), Some(ROW_TOKEN));
}

/// `GET /library/sections/{key}/all` over a captured section answer: the page
/// is reported from the container's own `totalSize` / `offset` / `identifier`.
#[test]
fn a_real_section_answer_pages_with_the_container_window() {
    let (_plex_tv, pms, server) = m3_session("m3-cap-section");
    pms.set_script(script_with(
        Route::SectionItems(CAPTURED_SECTION.to_owned()),
        Step::Fixture("real/films_all.json"),
    ));

    let page = browse_items(
        &server,
        CAPTURED_SECTION,
        Page::first(),
        &BrowseOptions::default(),
    )
    .expect("the captured section answer must parse");
    assert_eq!(page.items.len(), 3, "the capture holds three rows");
    assert_eq!(page.total, Some(1654), "the container's own totalSize");
    assert_eq!(page.start, Some(0), "the container's own offset");
    assert_eq!(
        page.identifier.as_deref(),
        Some("com.plexapp.plugins.library"),
        "the container's own identifier"
    );
    assert!(
        page.has_more(MAX_PAGE_SIZE),
        "3 of 1654 items served, so more remain"
    );
    assert_eq!(page.items[0].rating_key.as_deref(), Some(CAPTURED_MOVIE));
    assert_eq!(page.items[0].title.as_deref(), Some("The 'Brook"));
    assert_eq!(page.items[1].title.as_deref(), Some("#Adrift"));
    assert_eq!(
        page.items[1].thumb.as_deref(),
        Some("/library/metadata/41709/thumb/1787107941")
    );
    assert!(
        page.items
            .iter()
            .all(|item| item.library_section_id.is_none()),
        "a section listing row does not repeat librarySectionID"
    );

    let calls = at_path(
        &pms.requests(),
        &format!("{SECTION_ITEMS_PATH}/{CAPTURED_SECTION}/all"),
    );
    assert_eq!(calls.len(), 1);
    assert_eq!(
        query_pairs(&calls[0]),
        vec![
            (CONTAINER_START_QUERY.to_owned(), "0".to_owned()),
            (CONTAINER_SIZE_QUERY.to_owned(), MAX_PAGE_SIZE.to_string()),
        ]
    );
    assert!(!calls[0].query_contains(TOKEN_HEADER));
    assert_eq!(calls[0].header(TOKEN_HEADER), Some(ROW_TOKEN));
}

/// A show row is where `leafCount` / `viewedLeafCount` / `childCount` are
/// recorded; an episode row is where the parentage is. Both are read.
#[test]
fn a_captured_show_row_counts_its_leaves_and_seasons() {
    let (_pms, server) = captured_metadata(CAPTURED_SHOW, "real/show_metadata.json", "m3-cap-show");
    let item = metadata(&server, CAPTURED_SHOW).expect("the captured show answer must parse");

    assert_eq!(item.kind.as_deref(), Some("show"));
    assert_eq!(item.title.as_deref(), Some("The 404"));
    assert_eq!(item.leaf_count, Some(100), "episodes in this show");
    assert_eq!(item.viewed_leaf_count, Some(0));
    assert_eq!(item.child_count, Some(7), "seasons under this show");
    assert_eq!(item.index, Some(1), "the show's own index");
    assert_eq!(item.parent_index, None);
    assert_eq!(item.grandparent_rating_key.as_deref(), None);
    assert_eq!(
        item.duration,
        Some(2_520_000),
        "per-episode runtime as recorded"
    );
    assert_eq!(item.added_at, Some(1_773_779_564));
    assert_eq!(item.audience_rating, Some(7.9));
    assert_eq!(item.content_rating.as_deref(), Some("TV-14"));
    assert!(
        item.tagline
            .as_deref()
            .unwrap_or_default()
            .starts_with("The ledger is never wrong."),
        "this row records a season tagline"
    );
    assert!(item.markers.is_empty(), "a show row names no Marker[]");
    assert!(item.chapters.is_empty());
    assert!(item.media.is_empty(), "a show row names no Media version");
    assert!(!item.has_media(), "only a leaf plays");
}

/// A rating key no other script in this file answers, so the answer below is
/// the only one this route can serve.
const TWICE_NAMED_KEY: &str = "51227";

/// A row that names the same part both directly and under `Media[]` is not
/// listed twice: callers address a part by position.
#[test]
fn one_part_is_not_listed_twice_when_a_row_names_it_in_both_places() {
    let json = serde_json::json!({
        "MediaContainer": { "Metadata": [{
            "ratingKey": TWICE_NAMED_KEY,
            "type": "movie",
            "title": "Twice-named part",
            "Part": [{ "key": "/library/parts/51227/1/file.mkv", "container": "mkv", "size": 1024 }],
            "Media": [{ "id": 1, "Part": [{
                "key": "/library/parts/51227/1/file.mkv", "container": "mkv", "size": 1024,
                "protocol": "http", "Stream": [{ "id": 2, "streamType": 1, "codec": "h264" }]
            }] }]
        }], "totalSize": 1, "size": 1 }
    });
    let (_plex_tv, pms, server) = m3_session("m3-part-dedup");
    pms.set_script(script_with(
        Route::Metadata(TWICE_NAMED_KEY.to_owned()),
        Step::FixtureJson(200, json.to_string()),
    ));

    let item = metadata(&server, TWICE_NAMED_KEY).expect("the answer must parse");
    assert_eq!(item.parts.len(), 1, "the same part key is kept once");
    assert_eq!(
        item.parts[0].key.as_deref(),
        Some("/library/parts/51227/1/file.mkv")
    );
    assert_eq!(item.parts[0].protocol.as_deref(), Some("http"));
    assert_eq!(
        item.parts[0].streams.len(),
        1,
        "the kept part keeps its streams"
    );
    assert_eq!(item.media.len(), 1);
    assert_eq!(
        item.media[0].parts.len(),
        1,
        "the version still reports its own part"
    );
}
