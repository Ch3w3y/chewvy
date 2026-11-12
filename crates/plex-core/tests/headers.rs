//! Behaviour tests for the `X-Plex-*` header builder and the
//! header-only `X-Plex-Token` rule.
//!
//! Two shapes of assertion live here:
//!
//! * the assembled header set (`plex_core::http::headers_for`) — names, order,
//!   and that a token appears **exactly once** and only as `X-Plex-Token`;
//! * what a **local mock Plex server** actually received on the wire, which is
//!   the only way to prove the identity headers really are sent and that the
//!   token never rides in a query string.
//!
//! Mock tests, not Plex integration tests: real-account acceptance is
//! real-server verification.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)] // product code keeps these lints on

use plex_core::auth::create_pin;
use plex_core::client_info::{ClientInfo, Identity};
use plex_core::error::CoreError;
use plex_core::http::headers_for;
use plex_core::secret::SecretString;

mod common;
mod mock;

use common::{client, http_at, TempDir};
use mock::{create_pin_fixture, MockPlex, Route, Script, MOCK_PIN_CODE, MOCK_TOKEN};

/// The nine identity headers plus `Accept` that every request must carry, in
/// the order `ClientInfo::headers` builds them.
const IDENTITY_HEADER_NAMES: [&str; 10] = [
    "X-Plex-Product",
    "X-Plex-Client-Identifier",
    "X-Plex-Version",
    "X-Plex-Platform",
    "X-Plex-Platform-Version",
    "X-Plex-Device",
    "X-Plex-Device-Name",
    "X-Plex-Provides",
    "X-Plex-Language",
    "Accept",
];

#[test]
fn the_identity_header_set_is_complete_ordered_and_token_free() {
    let dir = TempDir::new("headers-set");
    let client_info = client(&dir);
    let headers = headers_for(&client_info, None);

    let names: Vec<&str> = headers.iter().map(|h| h.name.as_str()).collect();
    assert_eq!(
        names, IDENTITY_HEADER_NAMES,
        "the header builder must send exactly the identity set plus Accept"
    );
    assert!(
        !names.iter().any(|n| n.eq_ignore_ascii_case("X-Plex-Token")),
        "headers_for(None) must not carry a token"
    );
    assert_eq!(
        client_info.header_names(),
        IDENTITY_HEADER_NAMES
            .iter()
            .map(|n| (*n).to_owned())
            .collect::<Vec<String>>(),
        "ClientInfo::header_names is the same 9 `X-Plex-*` identity headers plus Accept"
    );
    assert_eq!(
        client_info
            .header_names()
            .iter()
            .filter(|n| n.starts_with("X-Plex-"))
            .count(),
        9,
        "exactly nine X-Plex-* identity headers, Accept is separate"
    );
    for header in &headers {
        assert!(
            !header.value.is_empty(),
            "{} must have a value",
            header.name
        );
        assert!(
            header.value.is_ascii(),
            "{} must be ASCII to be a valid header value",
            header.name
        );
    }
    assert_eq!(headers[9].value, "application/json");
    assert_eq!(headers[0].value, plex_core::client_info::PRODUCT);
    assert_eq!(headers[3].value, plex_core::client_info::PLATFORM);
    assert_eq!(headers[8].value, client_info.language);
}

#[test]
fn a_token_is_added_exactly_once_and_only_as_the_token_header() {
    let dir = TempDir::new("headers-token");
    let client_info = client(&dir);
    let token = SecretString::new(MOCK_TOKEN);

    let headers = headers_for(&client_info, Some(&token));
    let matches: Vec<&String> = headers
        .iter()
        .filter(|h| h.name == plex_core::client_info::TOKEN_HEADER)
        .map(|h| &h.value)
        .collect();
    assert_eq!(matches.len(), 1, "X-Plex-Token must appear exactly once");
    assert_eq!(matches[0], MOCK_TOKEN);
    assert_eq!(
        headers.len(),
        IDENTITY_HEADER_NAMES.len() + 1,
        "the token header is appended to the identity set, not swapped in"
    );
    // Nothing else in the set carries the token value.
    assert_eq!(
        headers
            .iter()
            .filter(|h| h.value == MOCK_TOKEN)
            .filter(|h| h.name != plex_core::client_info::TOKEN_HEADER)
            .count(),
        0,
        "no other header value may hold the token"
    );
    // The unauthenticated set is unchanged by this call.
    assert_eq!(headers_for(&client_info, None).len(), 10);
}

#[test]
fn every_identity_header_reaches_the_mock_on_the_pin_post() {
    let dir = TempDir::new("headers-wire");
    let client_info = client(&dir);
    let pin_id = "headers-pin-1";
    let server = MockPlex::start(
        Script::new().with(Route::Pins, &[create_pin_fixture(pin_id, MOCK_PIN_CODE)]),
    );
    let http = http_at(server.base(), &client_info);

    let pin = create_pin(&http).expect("pin creation must succeed");
    assert_eq!(pin.id, pin_id);

    let requests = server.requests();
    let post = requests
        .iter()
        .find(|r| r.path_and_query == "/api/v2/pins")
        .expect("the mock must have seen the pin POST");
    for name in IDENTITY_HEADER_NAMES {
        let value = post
            .header(name)
            .unwrap_or_else(|| panic!("{name} must be on the wire request"));
        assert!(!value.is_empty(), "{name} must not be empty on the wire");
    }
    assert_eq!(post.header("Accept"), Some("application/json"));
    assert_eq!(
        post.header("X-Plex-Product"),
        Some(plex_core::client_info::PRODUCT)
    );
    assert_eq!(
        post.header("X-Plex-Client-Identifier"),
        Some(client_info.client_identifier.as_str())
    );
    assert!(
        post.header("X-Plex-Token").is_none(),
        "a pin request is unauthenticated: it carries no token"
    );
    assert!(
        !post.path_and_query.contains("strong"),
        "classic-mode pin creation sends no `strong` query key"
    );
    assert_eq!(server.requests().len(), 1);
}

#[test]
fn a_token_on_an_authenticated_request_is_header_only_on_the_wire() {
    let dir = TempDir::new("headers-auth");
    let client_info = client(&dir);
    let server =
        MockPlex::start(Script::new().with(Route::User, &[mock::Step::Fixture("user.json")]));
    let http = http_at(server.base(), &client_info);
    let token = SecretString::new(MOCK_TOKEN);

    let response = http
        .get(plex_core::api::USER_PATH, &[], Some(&token))
        .expect("authenticated GET must reach the mock");
    assert!(response.is_success());

    let recorded = server
        .requests()
        .into_iter()
        .next()
        .expect("the mock must have seen the GET");
    assert!(
        mock::token_only_in_header(&recorded, "X-Plex-Token"),
        "the token must appear once, as X-Plex-Token, never in the query: {:?}",
        recorded.path_and_query
    );
    assert_eq!(recorded.value_matches(MOCK_TOKEN), 1);
    assert!(!recorded.query_contains(MOCK_TOKEN));
    // The identity headers are still there on an authenticated request.
    for name in IDENTITY_HEADER_NAMES {
        assert!(
            recorded.header(name).is_some(),
            "{name} must be sent with an authenticated request too"
        );
    }
}

#[test]
fn a_query_key_named_x_plex_token_is_refused_before_any_request() {
    let dir = TempDir::new("headers-query-key");
    let client_info = client(&dir);
    let server = MockPlex::start(Script::new());
    let http = http_at(server.base(), &client_info);
    let token = SecretString::new(MOCK_TOKEN);

    let error = http
        .send(
            "GET",
            plex_core::api::USER_PATH,
            &[("X-Plex-Token", MOCK_TOKEN)],
            Some(&token),
            None,
        )
        .expect_err("a token placed in the query map must be refused");
    assert_eq!(error, CoreError::InvalidInput);
    assert_eq!(error.code(), "invalid-input");
    assert!(
        !error.to_string().contains(MOCK_TOKEN),
        "the refusal must not echo the rejected value: {error}"
    );
    assert!(
        server.requests().is_empty(),
        "the refusal must happen before the request leaves the process"
    );
}

#[test]
fn a_token_already_embedded_in_the_request_url_is_refused_too() {
    let dir = TempDir::new("headers-query-url");
    let client_info = client(&dir);
    let server = MockPlex::start(Script::new());
    let http = http_at(server.base(), &client_info);
    let token = SecretString::new(MOCK_TOKEN);

    let error = http
        .get(
            &format!("{}?X-Plex-Token={}", plex_core::api::USER_PATH, MOCK_TOKEN),
            &[],
            Some(&token),
        )
        .expect_err("a URL that already carries the token must be refused");
    assert_eq!(error, CoreError::InvalidInput);
    assert!(!format!("{error:?}").contains(MOCK_TOKEN));
    assert!(
        server.requests().is_empty(),
        "the URL guard must reject before connecting"
    );
}

#[test]
fn client_identifier_is_a_stable_per_install_uuid_on_a_0600_identity_file() {
    let dir = TempDir::new("headers-identity");
    let path = dir.join("identity.json");
    let first = ClientInfo::load_or_create(&path, common::TEST_VERSION)
        .expect("identity must be created on first use");
    let second = ClientInfo::load_or_create(&path, common::TEST_VERSION)
        .expect("identity must reload, not be regenerated");

    assert!(
        plex_core::client_info::is_uuid(&first.client_identifier),
        "Client-Identifier must be a canonical UUID v4"
    );
    assert_eq!(
        first.client_identifier, second.client_identifier,
        "the identifier must be stable per install, not per request"
    );
    assert_eq!(first.headers().len(), 10);
    assert_eq!(
        headers_for(&first, None)
            .iter()
            .find(|h| h.name == "X-Plex-Client-Identifier")
            .expect("the built header set must carry the identifier")
            .value,
        first.client_identifier
    );

    let mode = dir.mode_of(&path);
    assert_eq!(mode & 0o777, 0o600, "the identity file must be 0600");

    // The identity file holds the identifier and nothing secret.
    let text = std::fs::read_to_string(&path).expect("identity file must be readable");
    let parsed: Identity =
        serde_json::from_str(&text).expect("identity file must parse as Identity");
    assert_eq!(parsed.client_identifier, first.client_identifier);
    assert!(
        !text.contains(MOCK_TOKEN),
        "an identity file must never contain an account token"
    );
}
