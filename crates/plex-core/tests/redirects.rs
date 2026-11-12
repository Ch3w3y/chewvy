//! **No-redirect / 3xx transport behaviour**, run against the mock Plex
//! server over real loopback HTTP (`tests/mock/mod.rs`, `Step::Redirect`).
//!
//! Why this target exists: `plex_core::http::Http` builds its ureq agent with
//! `max_redirects(0)` and turns every `300..400` answer into
//! `CoreError::HttpStatus` **before** reading the body (`src/http.rs`). The
//! reason given there is a token-safety reason: ureq-proto's own redirect
//! handling keeps custom headers, so following a redirect would re-send
//! `X-Plex-Token` to the redirect target - a different host, possibly over
//! plain http. That claim was previously carried only by a code comment, which
//! is not evidence. Every assertion below is against requests a **second mock
//! host recorded from its own listener**, i.e. what actually reached the
//! redirect target, plus the request log of the host that answered 3xx.
//!
//! What is proven here by execution:
//!
//! * a 3xx answer is returned to the caller as that status, and a library call
//!   or a `resources` call produces **exactly one** request: no second request
//!   is made to the `Location` target, and no retry follows the 3xx;
//! * the `Location` target - a real, running mock host with its own request
//!   log - records **zero** requests, so no credential of any kind is ever
//!   re-presented to it;
//! * a 3xx on `GET /identity` is a failed connect attempt
//!   ([`AttemptKind::HttpStatus`]), not a machine identifier and not a
//!   silently "reachable" host; a 3xx on the credential-free probe does not by
//!   itself end the attempt either;
//! * a 3xx answer never becomes data: no sections, no rows and no item are
//!   fabricated from a redirect body, and the typed error's rendered text
//!   carries no token.
//!
//! What is **not** settled by this target (recorded in
//! `docs/CAPABILITIES.md`, "Transport behaviour (3xx) and 429 policy"):
//! whether a real Plex Media Server or plex.tv ever answers these routes with
//! a redirect, and what the client *should* do when it does. This target pins
//! our own refusal rule; the real behaviour is not yet verified against a real server. 429 is asserted
//! here only as "surfaced once, no invented retry".

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)] // test scaffolding; product code keeps these lints on

mod common;
mod mock;

use plex_core::client_info::{ClientInfo, TOKEN_HEADER};
use plex_core::error::CoreError;
use plex_core::library::{list_sections, SECTIONS_PATH};
use plex_core::resources::{fetch_resources, parse_resources, RESOURCES_PATH};
use plex_core::secret::SecretString;
use plex_core::server::{connect, AttemptKind, Server, IDENTITY_PATH, PROBE_PATH};

use mock::{MockPlex, Route, Script, Step, MOCK_TOKEN};

/// The `accessToken` of the row used for the PMS-side tests; distinct from
/// `mock::MOCK_TOKEN` (the **account** token) so "which token reached which
/// host" is observable in each mock's own log.
const ROW_TOKEN: &str = "ROWTOKEN-REDIR-2b8f6a0c4d1e93a7";

/// The row's `clientIdentifier`, different from this test client's own identity
/// so the row is not skipped as "our own device".
const ROW_CLIENT: &str = "mock-pms-redirect-row-client";

/// A `connections[]` entry (verified field names only) at a running mock host.
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

/// A `provides: "server"`, `owned: 1` row whose connections point at `base`.
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

/// Parse (never hand-build) the row that points at `pms` and connect to it the
/// way the product does: probe `GET /` with no credential, then `GET /identity`
/// with **that row's own** token.
fn connect_parsed_row(pms: &MockPlex, client: &ClientInfo) -> Server {
    let list = parse_resources(
        serde_json::to_string(&vec![server_row(pms.base())])
            .expect("mock row must serialize")
            .as_bytes(),
        "this-client-is-not-the-row",
    )
    .expect("mock row must parse");
    connect(&list.servers()[0], client).expect("mock PMS must connect")
}

/// A mock host that **would** answer normally, armed only as the `Location`
/// target of a redirect. If the client ever followed a redirect, this host's
/// own log would show it.
fn decoy(script: Script) -> MockPlex {
    MockPlex::start(script)
}

/// The redirect target is never contacted, so no credential travels to it: the
/// strongest form of "we do not re-present a token to a redirect target" that a
/// mock can express.
fn assert_decoy_untouched(decoy: &MockPlex, context: &str) {
    let seen = decoy.requests();
    assert_eq!(
        seen.len(),
        0,
        "{context}: the Location target was contacted"
    );
}

/// The `GET /` + `GET /identity` a successful connect already made, as a prefix
/// of a PMS mock's request log.
fn connected_prefix(pms: &MockPlex) -> usize {
    let reqs = pms.requests();
    assert_eq!(
        reqs.len(),
        2,
        "connect made exactly the probe and identity calls"
    );
    assert_eq!(reqs[0].method, "GET");
    assert_eq!(reqs[0].path_and_query, PROBE_PATH);
    assert_eq!(
        reqs[0].header(TOKEN_HEADER),
        None,
        "the probe carries no credential"
    );
    assert_eq!(reqs[1].path_and_query, IDENTITY_PATH);
    assert_eq!(
        reqs[1].header(TOKEN_HEADER),
        Some(ROW_TOKEN),
        "identity uses this row's own token"
    );
    reqs.len()
}

/// A 3xx answer is refused **in place**: the caller gets that status, the host
/// that answered saw exactly one request, and the running `Location` target saw
/// none - so nothing that looks like a follow-up request (or a token replay)
/// exists on the wire.
#[test]
fn a_redirect_answer_is_refused_without_a_second_request() {
    let dir = common::TempDir::new("redirect-library");
    let client = common::client(&dir);

    let decoy =
        decoy(Script::new().with(Route::Sections, &[Step::Fixture("library_sections.json")]));
    let pms = MockPlex::start(
        Script::new()
            .with(Route::Root, &[Step::Respond(200, r#"{"mock":"root"}"#)])
            .with(Route::Identity, &[Step::Fixture("identity.json")])
            .with(
                Route::Sections,
                &[Step::Redirect(302, decoy.base().to_owned())],
            ),
    );
    let server = connect_parsed_row(&pms, &client);
    let connected = connected_prefix(&pms);

    let error =
        list_sections(&server).expect_err("a redirect answer must not be reported as sections");
    assert_eq!(
        error,
        CoreError::HttpStatus { status: 302 },
        "refused as its own status"
    );
    assert_eq!(error.code(), "http-status", "stable error code");

    let reqs = pms.requests();
    assert_eq!(
        reqs.len(),
        connected + 1,
        "exactly one library request: no follow-up to the Location, no retry"
    );
    let call = reqs.last().expect("the refused library call is recorded");
    assert_eq!(call.method, "GET");
    assert_eq!(call.path_and_query, SECTIONS_PATH);
    assert_eq!(
        call.header(TOKEN_HEADER),
        Some(ROW_TOKEN),
        "token in the header only"
    );
    assert!(!call.query_contains(ROW_TOKEN), "no token in the query");
    assert!(
        !call.path_and_query.contains(ROW_TOKEN),
        "no token in the request line"
    );

    assert_decoy_untouched(&decoy, "a 302 on /library/sections");

    // The redirect body is never read, so it cannot be mistaken for data, and
    // the typed error renders without the credential.
    for text in [format!("{error}"), format!("{error:?}")] {
        assert!(
            !text.contains(ROW_TOKEN),
            "the error text carries the row token"
        );
        assert!(
            !text.contains(MOCK_TOKEN),
            "the error text carries the account token"
        );
        assert!(
            !text.contains("redirect"),
            "the error text is not the server body: {text}"
        );
    }
    // The connected server keeps its own credential for a later, separate call.
    assert!(
        server.token().matches(ROW_TOKEN),
        "the row token is not rotated by a refused redirect"
    );
}

/// Every 3xx the mock can answer with is reported as **that** status. None of
/// them is followed, retried or turned into an empty answer.
#[test]
fn every_3xx_answer_is_reported_as_that_status_and_never_followed() {
    let dir = common::TempDir::new("redirect-statuses");
    let client = common::client(&dir);

    for status in [301u16, 302, 303, 307, 308] {
        let decoy =
            decoy(Script::new().with(Route::Sections, &[Step::Fixture("library_sections.json")]));
        let pms = MockPlex::start(
            Script::new()
                .with(Route::Root, &[Step::Respond(200, r#"{"mock":"root"}"#)])
                .with(Route::Identity, &[Step::Fixture("identity.json")])
                .with(
                    Route::Sections,
                    &[Step::Redirect(status, decoy.base().to_owned())],
                ),
        );
        let server = connect_parsed_row(&pms, &client);
        let connected = connected_prefix(&pms);

        let error = list_sections(&server)
            .err()
            .unwrap_or_else(|| panic!("{status}: a redirect answer must not become sections"));
        assert_eq!(
            error,
            CoreError::HttpStatus { status },
            "{status}: kept as itself"
        );
        assert_eq!(
            pms.requests().len(),
            connected + 1,
            "{status}: one request only"
        );
        assert_decoy_untouched(&decoy, &format!("a {status} on /library/sections"));
    }
}

/// A 3xx on the credentialed `GET /identity` step is a **failed attempt** on
/// the candidate it belongs to: no machine identifier is captured, the attempt
/// is recorded with the status, and the redirect target records nothing.
#[test]
fn a_redirect_on_identity_is_a_failed_attempt_not_a_machine_identifier() {
    let dir = common::TempDir::new("redirect-identity");
    let client = common::client(&dir);

    let decoy = decoy(Script::new().with(Route::Identity, &[Step::Fixture("identity.json")]));
    let pms = MockPlex::start(
        Script::new()
            .with(Route::Root, &[Step::Respond(200, r#"{"mock":"root"}"#)])
            .with(
                Route::Identity,
                &[Step::Redirect(302, decoy.base().to_owned())],
            ),
    );

    let list = parse_resources(
        serde_json::to_string(&vec![server_row(pms.base())])
            .expect("mock row must serialize")
            .as_bytes(),
        "this-client-is-not-the-row",
    )
    .expect("mock row must parse");
    let failure = connect(&list.servers()[0], &client)
        .expect_err("a redirecting /identity must not produce a connected server");

    let attempts = failure.attempts();
    assert_eq!(
        attempts.len(),
        1,
        "one candidate, one recorded attempt, no retry"
    );
    assert_eq!(
        attempts[0].kind,
        AttemptKind::HttpStatus(302),
        "the status is the reason"
    );
    assert_eq!(
        attempts[0].kind.message(),
        "the server answered with an error status"
    );
    assert!(
        !failure.message().contains(ROW_TOKEN),
        "the connect summary is token-free"
    );

    let reqs = pms.requests();
    assert_eq!(
        reqs.len(),
        2,
        "the probe and one identity request, nothing more"
    );
    assert_eq!(reqs[0].path_and_query, PROBE_PATH);
    assert_eq!(reqs[1].path_and_query, IDENTITY_PATH);
    assert_eq!(reqs[1].header(TOKEN_HEADER), Some(ROW_TOKEN));
    assert_decoy_untouched(&decoy, "a 302 on /identity");
}

/// A 3xx on the credential-free probe is an HTTP answer, so the connect moves
/// on to the credentialed step rather than treating the host as offline - and
/// when **that** step is redirected too, the attempt fails with the status and
/// still never contacts the target.
#[test]
fn a_redirect_on_the_probe_does_not_make_the_host_look_offline() {
    let dir = common::TempDir::new("redirect-probe");
    let client = common::client(&dir);

    let decoy =
        decoy(Script::new().with(Route::Root, &[Step::Respond(200, r#"{"mock":"decoy"}"#)]));

    // (a) identity answers normally: the row still connects, and the probe
    // redirect was never followed.
    let pms = MockPlex::start(
        Script::new()
            .with(Route::Root, &[Step::Redirect(301, decoy.base().to_owned())])
            .with(Route::Identity, &[Step::Fixture("identity.json")]),
    );
    let server = connect_parsed_row(&pms, &client);
    assert_eq!(
        server.base_url(),
        pms.base(),
        "the connected base is the advertised one"
    );
    let reqs = pms.requests();
    assert_eq!(reqs.len(), 2, "probe + identity, no third request");
    assert_eq!(reqs[0].path_and_query, PROBE_PATH);
    assert_decoy_untouched(&decoy, "a 301 on the probe");

    // (b) identity is redirected as well: the attempt fails with 302 and the
    // probe target still records nothing.
    let pms2 = MockPlex::start(
        Script::new()
            .with(Route::Root, &[Step::Redirect(301, decoy.base().to_owned())])
            .with(
                Route::Identity,
                &[Step::Redirect(302, decoy.base().to_owned())],
            ),
    );
    let list = parse_resources(
        serde_json::to_string(&vec![server_row(pms2.base())])
            .expect("mock row must serialize")
            .as_bytes(),
        "this-client-is-not-the-row",
    )
    .expect("mock row must parse");
    let failure = connect(&list.servers()[0], &client)
        .expect_err("two redirecting steps must not produce a connected server");
    assert_eq!(
        failure.attempts()[0].kind,
        AttemptKind::HttpStatus(302),
        "the credentialed step is the recorded reason"
    );
    assert_eq!(
        pms2.requests().len(),
        2,
        "one probe, one identity, no follow-up"
    );
    assert_decoy_untouched(&decoy, "a 301 probe + 302 identity");
}

/// The **account** token path (`GET /api/v2/resources` on a mock plex.tv) is
/// refused the same way: one request, token in the header only, and the
/// redirect target never contacted - which is the exact leak `max_redirects(0)`
/// exists to prevent.
#[test]
fn a_redirect_on_resources_never_sends_the_account_token_onward() {
    let dir = common::TempDir::new("redirect-resources");
    let client = common::client(&dir);

    let decoy = decoy(Script::new().with(
        Route::Resources,
        &[Step::FixtureJson(200, r#"[]"#.to_owned())],
    ));
    let tv = MockPlex::start(Script::new().with(
        Route::Resources,
        &[Step::Redirect(302, decoy.base().to_owned())],
    ));

    let error = fetch_resources(
        &common::http_at(tv.base(), &client),
        &SecretString::new(MOCK_TOKEN),
        "this-client-is-not-the-row",
    )
    .expect_err("a redirect answer must not be reported as server rows");
    assert_eq!(
        error,
        CoreError::HttpStatus { status: 302 },
        "refused as its own status"
    );

    let reqs = tv.requests();
    assert_eq!(
        reqs.len(),
        1,
        "one resources request, no follow-up, no retry"
    );
    let call = reqs.first().expect("the resources request is recorded");
    assert_eq!(call.method, "GET");
    assert_eq!(
        call.header(TOKEN_HEADER),
        Some(MOCK_TOKEN),
        "the account token travels as the header"
    );
    assert_eq!(call.value_matches(MOCK_TOKEN), 1, "present exactly once");
    assert_eq!(
        call.path_and_query,
        format!("{RESOURCES_PATH}?includeHttps=1&includeRelay=1&includeIPv6=1"),
        "the documented path + query, and no credential in it"
    );
    assert!(
        !call.query_contains(MOCK_TOKEN),
        "no account token in the query"
    );
    assert!(
        !call.path_and_query.contains(MOCK_TOKEN),
        "no account token in the request line"
    );
    assert_decoy_untouched(&decoy, "a 302 on /api/v2/resources");

    for text in [format!("{error}"), format!("{error:?}")] {
        assert!(
            !text.contains(MOCK_TOKEN),
            "the error text carries the account token"
        );
        assert!(
            !text.contains(ROW_TOKEN),
            "the error text carries a row token"
        );
    }
}

/// 429 on a library route is surfaced once, as `HttpStatus { 429 }`: this
/// milestone invents no backoff, no retry and no "retry-after" interpretation
/// (`docs/CAPABILITIES.md`, "Transport behaviour (3xx) and 429 policy").
#[test]
fn a_429_answer_is_surfaced_once_as_that_status() {
    let dir = common::TempDir::new("rate-limit-library");
    let client = common::client(&dir);

    let pms = MockPlex::start(
        Script::new()
            .with(Route::Root, &[Step::Respond(200, r#"{"mock":"root"}"#)])
            .with(Route::Identity, &[Step::Fixture("identity.json")])
            .with(
                Route::Sections,
                &[Step::Respond(429, r#"{"errors":["mock: rate limited"]}"#)],
            ),
    );
    let server = connect_parsed_row(&pms, &client);
    let connected = connected_prefix(&pms);

    let error = list_sections(&server).expect_err("429 must not be reported as an empty library");
    assert_eq!(error, CoreError::HttpStatus { status: 429 }, "kept as 429");
    assert_eq!(
        pms.requests().len(),
        connected + 1,
        "429 is surfaced once: no retry loop is invented"
    );
    assert!(
        !format!("{error}").contains(ROW_TOKEN),
        "429 renders without the row token"
    );
}
