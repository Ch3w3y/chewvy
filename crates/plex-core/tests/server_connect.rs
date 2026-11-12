//! Connect/credential tests, run against the mock Plex server over real
//! loopback HTTP (`tests/mock/mod.rs`): the `resources` call, the credential-free
//! probe, per-server token separation, candidate ordering/attempts and
//! identifier capture.
//!
//! What this target is: real HTTP requests made by the product transport
//! (`plex_core::http::Http`) to mock servers that answer the hand-authored
//! shapes in `tests/fixtures/`. It is **not** a real Plex integration: the mock
//! speaks plain HTTP on loopback, so an `https` candidate can only ever fail as
//! [`plex_core::server::AttemptKind::Unreachable`] here, and `https` > `http`
//! ranking is proven by `tests/resources_selection.rs::candidate_order_*`, not
//! by a live TLS probe. Real plex.tv/PMS behaviour is not yet verified against a real server.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)] // product code keeps these lints on

mod common;
mod mock;

use std::net::{TcpListener, TcpStream};

use plex_core::resources::{fetch_resources, ExclusionReason, Protocol, Reachability};
use plex_core::secret::SecretString;
use plex_core::server::{
    connect, connect_shared, AttemptKind, ConnectFailure, IDENTITY_PATH, PROBE_PATH,
};

use mock::{
    no_record_carries, requests_with_token_header, token_only_in_header, MockPlex, Route, Script,
    Step, MOCK_TOKEN,
};

/// The only header this client may put a token in.
const TOKEN_HEADER: &str = "X-Plex-Token";

/// The **account** token (the credential that reads `api/v2/resources`):
/// `mock::MOCK_TOKEN`, used everywhere below so a test can assert it never
/// appears at a mock PMS host. Distinct from, and not a substring of,
/// [`ROW_A`]/[`ROW_B`].
const ROW_A: &str = "ROWTOKEN-A-2b7c1d9e4f6a83c5";
const ROW_B: &str = "ROWTOKEN-B-7f2a90c4e6b18d35";

/// Machine identifiers each mock PMS reports about itself in `GET /identity`.
const MACHINE_A: &str = "MACHINE-A-1a2b3c4d5e6f7081";
const MACHINE_B: &str = "MACHINE-B-807f6e5d4c3b2a19";

/// An identifier planted **in a row**. The verified `resources` field set has no
/// such attribute (docs/PLEX_AUTH.md §1 item 6), so reading it here can only
/// come from this test, and the client must never use it.
const DECOY_ROW_IDENTIFIER: &str = "ROW-DECOY-9a8b7c6d5e4f3021";

// ------------------------------------------------------------- mock builders

/// A `connections[]` entry in the verified field shape
/// (`address`,`port`,`protocol`,`uri`,`local`,`relay`,`IPv6`) pointing at a
/// running mock server over plain loopback HTTP.
fn local_http(base: &str) -> serde_json::Value {
    let port = base
        .rsplit_once(':')
        .and_then(|(_, port)| port.parse::<u16>().ok())
        .expect("mock server base must carry a port");
    connection(base, "http", true, false, false, "127.0.0.1", port, None)
}

/// One `connections[]` entry, with the ordering knobs (`local`, `relay`,
/// `IPv6`) set explicitly for the candidate-order tests.
#[allow(clippy::too_many_arguments, clippy::fn_params_excessive_bools)]
fn connection(
    uri: &str,
    protocol: &str,
    local: bool,
    relay: bool,
    ip_v6: bool,
    address: &str,
    port: u16,
    httpuri: Option<&str>,
) -> serde_json::Value {
    let mut entry = serde_json::json!({
        "address": address,
        "port": port,
        "protocol": protocol,
        "uri": uri,
        "local": i64::from(local),
        "relay": i64::from(relay),
        "IPv6": i64::from(ip_v6),
    });
    if let Some(httpuri) = httpuri {
        entry["httpuri"] = serde_json::json!(httpuri);
    }
    entry
}

/// A `provides: "server"`, `owned: 1` row with its **own** `accessToken` and the
/// given connections.
fn server_row(
    client_identifier: &str,
    access_token: &str,
    connections: Vec<serde_json::Value>,
) -> serde_json::Value {
    serde_json::json!({
        "accessToken": access_token,
        "clientIdentifier": client_identifier,
        "device": client_identifier,
        "name": format!("{client_identifier} name"),
        "provides": "server",
        "presence": 1,
        "owned": 1,
        "httpsRequired": 0,
        "dnsRebindingProtection": 1,
        "natLoopbackSupported": 1,
        "relay": 0,
        "connections": connections,
    })
}

/// Parse mock rows without going through plex.tv (used where the test is about
/// the connect path, not the `resources` call).
fn rows_resource(rows: Vec<serde_json::Value>) -> plex_core::resources::ServerList {
    plex_core::resources::parse_resources(
        serde_json::to_string(&rows)
            .expect("mock rows must serialize")
            .as_bytes(),
        "this-client-is-not-a-row",
    )
    .expect("mock rows must parse")
}

/// A `GET /` answer (the probe reads no body, only that something answers).
fn ok_probe() -> Step {
    Step::Respond(200, r#"{"mock":"root answer"}"#)
}

/// The `GET /identity` answer of one mock PMS, naming **its own** identifier.
fn identity_of(machine: &str) -> Step {
    Step::FixtureJson(
        200,
        serde_json::to_string(&serde_json::json!({
            "MediaContainer": {"machineIdentifier": machine, "friendlyName": "Mock PMS"}
        }))
        .expect("identity shape must serialize"),
    )
}

/// The `GET /identity` answer taken verbatim from `tests/fixtures/identity.json`
/// (`MOCKMACHINE-3c9e2b7a51d84f06`).
fn identity_fixture() -> Step {
    Step::Fixture("identity.json")
}

/// A mock PMS that answers the probe and reports one identifier.
fn pms_script(probe: Step, identity: Step) -> Script {
    Script::new()
        .with(Route::Root, &[probe])
        .with(Route::Identity, &[identity])
}

/// A TCP port that was bound on loopback and then released, so nothing listens
/// there: `Unreachable` candidates in the attempt-record tests.
///
/// Caveat: another process could in principle claim the port in between. The
/// mock servers used by those tests are already listening, so they hold other
/// ports and this address is only ever probed, never served.
fn closed_loopback_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("loopback port must be bindable");
    let port = listener.local_addr().expect("bound addr").port();
    drop(listener);
    port
}

/// Project the recorded attempts to what a test compares (base URL, transport,
/// local/remote/relay class, fallback flag, outcome) in tried order.
fn shapes(failure: &ConnectFailure) -> Vec<(String, Protocol, Reachability, bool, AttemptKind)> {
    failure
        .attempts()
        .iter()
        .map(|attempt| {
            (
                attempt.base_url.clone(),
                attempt.protocol,
                attempt.reachability,
                attempt.fallback,
                attempt.kind,
            )
        })
        .collect()
}

// ------------------------------------------------------------------ plex.tv

#[test]
fn the_resources_call_presents_the_account_token_only_as_a_header() {
    let dir = common::TempDir::new("m3-sc-tv");
    let client = common::client(&dir);
    let plex_tv = MockPlex::start(
        Script::new().with(Route::Resources, &[Step::Fixture("resources_rows.json")]),
    );

    let list = fetch_resources(
        &common::http_at(plex_tv.base(), &client),
        &SecretString::new(MOCK_TOKEN),
        "this-client-is-not-a-row-in-that-fixture",
    )
    .expect("the mock resources answer must parse");

    let requests = plex_tv.requests();
    assert_eq!(requests.len(), 1, "one resources call, no retry");
    let call = &requests[0];
    assert_eq!(call.method, "GET");
    assert_eq!(
        call.path_and_query, "/api/v2/resources?includeHttps=1&includeRelay=1&includeIPv6=1",
        "the endpoint and the three include flags, exactly as planned"
    );
    // Header-only: the token is the `X-Plex-Token` header value and appears
    // nowhere in the request line (no `X-Plex-Token` name, no token value).
    assert!(token_only_in_header(call, TOKEN_HEADER));
    assert!(!call.path_and_query.contains("X-Plex-Token"));
    assert_eq!(
        requests_with_token_header(&requests, TOKEN_HEADER, MOCK_TOKEN),
        1
    );
    assert!(!list.servers().is_empty(), "the fixture rows must parse");
    // The row tokens of that fixture are never sent by *this* call.
    assert!(no_record_carries(&requests, ROW_A));
    plex_tv.shutdown();
}

// ------------------------------------------------------- token separation

#[test]
fn the_account_token_never_reaches_an_owned_server_host() {
    let dir = common::TempDir::new("m3-sc-sep");
    let client = common::client(&dir);

    // Two independent mock PMS hosts, each reporting its own identifier.
    let pms_a = MockPlex::start(pms_script(ok_probe(), identity_of(MACHINE_A)));
    let pms_b = MockPlex::start(pms_script(ok_probe(), identity_of(MACHINE_B)));

    // A mock plex.tv whose rows point at those hosts, each with its OWN token.
    // Row A also carries a `machineIdentifier` the client must never read
    // (the retained field list records no such attribute on a row).
    let mut row_a = server_row("pms-a", ROW_A, vec![local_http(pms_a.base())]);
    row_a["machineIdentifier"] = serde_json::json!(DECOY_ROW_IDENTIFIER);
    let row_b = server_row("pms-b", ROW_B, vec![local_http(pms_b.base())]);
    let body = serde_json::to_string(&vec![row_a, row_b]).expect("test rows must serialize");

    let plex_tv =
        MockPlex::start(Script::new().with(Route::Resources, &[Step::FixtureJson(200, body)]));
    let list = fetch_resources(
        &common::http_at(plex_tv.base(), &client),
        &SecretString::new(MOCK_TOKEN),
        "this-client-is-not-a-row",
    )
    .expect("both rows must parse");
    assert_eq!(list.servers().len(), 2);

    let server_a = connect(&list.servers()[0], &client).expect("mock host A must answer");
    let server_b = connect(&list.servers()[1], &client).expect("mock host B must answer");

    let seen_a = pms_a.requests();
    let seen_b = pms_b.requests();
    assert!(!seen_a.is_empty() && !seen_b.is_empty());

    // 1. The **account** token never reaches either PMS host - not as a header
    //    value, not in a request line, not in a body.
    assert!(
        no_record_carries(&seen_a, MOCK_TOKEN),
        "an owned-server connect sent the account token to host A"
    );
    assert!(
        no_record_carries(&seen_b, MOCK_TOKEN),
        "an owned-server connect sent the account token to host B"
    );

    // 2. Per-resource separation: host A only ever saw row A's token and host B
    //    only ever saw row B's.
    assert!(
        no_record_carries(&seen_a, ROW_B),
        "host A saw row B's token"
    );
    assert!(
        no_record_carries(&seen_b, ROW_A),
        "host B saw row A's token"
    );
    assert_eq!(requests_with_token_header(&seen_a, TOKEN_HEADER, ROW_A), 1);
    assert_eq!(requests_with_token_header(&seen_b, TOKEN_HEADER, ROW_B), 1);

    // 3. Header-only transport on the PMS calls too: no recorded request line
    //    carries a token name or a token value.
    for record in seen_a.iter().chain(seen_b.iter()) {
        assert!(
            !record.path_and_query.contains("X-Plex-Token"),
            "a token name appeared in a request line: {}",
            record.path_and_query
        );
        assert!(
            !record.path_and_query.contains(ROW_A) && !record.path_and_query.contains(ROW_B),
            "a token value appeared in a request line: {}",
            record.path_and_query
        );
    }

    // 4. Each connected server holds its own row token, not the account token,
    //    and reports it as **not** delegated.
    for (server, row_token) in [(server_a.clone(), ROW_A), (server_b.clone(), ROW_B)] {
        assert!(!server.uses_delegated_token());
        assert!(server.token().matches(row_token));
        assert!(!server.token().matches(MOCK_TOKEN));
        assert_eq!(server.token_len(), row_token.len());
    }

    // 5. The identifier came from the host that answered, not from the row:
    //    row A's `machineIdentifier` decoy is not used, and host A's identifier
    //    is not host B's.
    assert!(server_a.machine_identifier().matches(MACHINE_A));
    assert!(!server_a.machine_identifier().matches(DECOY_ROW_IDENTIFIER));
    assert!(!server_a.machine_identifier().matches(MACHINE_B));
    assert!(server_b.machine_identifier().matches(MACHINE_B));

    plex_tv.shutdown();
    pms_a.shutdown();
    pms_b.shutdown();
}

// ------------------------------------------------ probe / identity behaviour

#[test]
fn the_probe_carries_no_credential_and_identity_carries_that_rows_own_token() {
    let dir = common::TempDir::new("m3-sc-probe");
    let client = common::client(&dir);
    let pms = MockPlex::start(pms_script(ok_probe(), identity_fixture()));

    let row = server_row("pms-probe", ROW_A, vec![local_http(pms.base())]);
    let list = plex_core::resources::parse_resources(
        serde_json::to_string(&vec![row]).expect("rows").as_bytes(),
        "not-a-row",
    )
    .expect("the row must parse");
    let server = connect(&list.servers()[0], &client).expect("the mock host must answer");

    let seen = pms.requests();
    assert_eq!(seen.len(), 2, "exactly one probe + one identifier call");
    let probe = &seen[0];
    assert_eq!(probe.path_and_query, PROBE_PATH);
    assert_eq!(probe.method, "GET");
    assert!(
        probe.header(TOKEN_HEADER).is_none(),
        "the liveness probe must not present any credential"
    );
    assert!(
        no_record_carries(std::slice::from_ref(probe), ROW_A),
        "the row token rode on the probe"
    );
    assert!(probe.body.is_empty());

    let identity = &seen[1];
    assert_eq!(identity.path_and_query, IDENTITY_PATH);
    assert_eq!(identity.header(TOKEN_HEADER), Some(ROW_A));
    assert!(!identity.path_and_query.contains(ROW_A));
    assert_eq!(
        identity.body, "",
        "the identifier call is a GET with no body"
    );

    // The identifier really came from the fixture answer.
    assert!(server
        .machine_identifier()
        .matches("MOCKMACHINE-3c9e2b7a51d84f06"));

    // `Server::probe` (the UI's "still reachable?" check) is also credential-free.
    let status = server.probe().expect("the mock host still answers");
    assert_eq!(status, 200);
    let after = pms.requests();
    assert_eq!(after.len(), 3);
    assert_eq!(after[2].path_and_query, PROBE_PATH);
    assert!(after[2].header(TOKEN_HEADER).is_none());
    assert_eq!(
        requests_with_token_header(&after, TOKEN_HEADER, ROW_A),
        1,
        "only the identifier call ever carried the row token"
    );

    pms.shutdown();
}

#[test]
fn the_first_candidate_that_answers_ends_the_attempt_loop() {
    let dir = common::TempDir::new("m3-sc-first");
    let client = common::client(&dir);
    // Candidate 1 gets a rejected identifier, candidate 2 a good one, and
    // candidate 3 must never be contacted at all.
    let pms = MockPlex::start(Script::new().with(Route::Root, &[ok_probe()]).with(
        Route::Identity,
        &[
            Step::Respond(401, r#"{"errors":["mock: identifier call rejected"]}"#),
            identity_fixture(),
        ],
    ));

    let row = server_row(
        "pms-loop",
        ROW_A,
        vec![
            local_http(pms.base()),
            local_http(pms.base()),
            local_http(pms.base()),
        ],
    );
    let list = plex_core::resources::parse_resources(
        serde_json::to_string(&vec![row]).expect("rows").as_bytes(),
        "not-a-row",
    )
    .expect("the row must parse");
    assert_eq!(list.servers()[0].candidates().len(), 3);

    let server = connect(&list.servers()[0], &client).expect("candidate 2 must succeed");
    assert!(server
        .machine_identifier()
        .matches("MOCKMACHINE-3c9e2b7a51d84f06"));
    assert_eq!(server.base_url(), pms.base());

    let seen = pms.requests();
    assert_eq!(
        pms.count_path(PROBE_PATH),
        2,
        "candidate 3 must not be probed after a success"
    );
    assert_eq!(
        pms.count_path(IDENTITY_PATH),
        2,
        "the identifier is read once per candidate actually tried"
    );
    assert_eq!(seen.len(), 4, "probe+identity for candidates 1 and 2 only");
    // Two identity calls were actually made (candidate 1 was rejected, candidate
    // 2 accepted), so the row token rode on exactly those two - and on neither
    // probe.
    assert_eq!(requests_with_token_header(&seen, TOKEN_HEADER, ROW_A), 2);
    assert_eq!(
        seen.iter()
            .filter(|record| record.path_and_query == PROBE_PATH)
            .filter(|record| record.header(TOKEN_HEADER).is_some())
            .count(),
        0,
        "no probe carried a credential"
    );

    pms.shutdown();
}

// --------------------------------------------------------- attempt records

#[test]
fn every_failed_candidate_is_recorded_once_in_the_order_it_was_tried() {
    let dir = common::TempDir::new("m3-sc-attempts");
    let client = common::client(&dir);
    // A healthy mock host exists but is **not** addressed by any candidate: the
    // closed port below is. `requests()` being empty proves the failure came
    // from these candidates and not from a wrong address.
    let pms = MockPlex::start(pms_script(ok_probe(), identity_fixture()));
    let closed = closed_loopback_port();
    let closed_url = |scheme: &str| format!("{scheme}://127.0.0.1:{closed}");

    let row = server_row(
        "pms-dead",
        ROW_A,
        vec![
            connection(
                &closed_url("https"),
                "https",
                true,
                false,
                false,
                "127.0.0.1",
                32400,
                None,
            ),
            connection(
                &closed_url("http"),
                "http",
                true,
                false,
                false,
                "127.0.0.1",
                32400,
                None,
            ),
            connection(
                &closed_url("http"),
                "http",
                false,
                false,
                false,
                "127.0.0.1",
                32400,
                None,
            ),
            connection(
                &closed_url("http"),
                "http",
                false,
                true,
                false,
                "127.0.0.1",
                32400,
                None,
            ),
        ],
    );
    let list = plex_core::resources::parse_resources(
        serde_json::to_string(&vec![row]).expect("rows").as_bytes(),
        "not-a-row",
    )
    .expect("the row must parse");

    let failure = connect(&list.servers()[0], &client).expect_err("nothing listens there");
    assert_eq!(failure.client_identifier(), "pms-dead");
    assert!(!failure.requires_https());
    assert_eq!(
        shapes(&failure),
        vec![
            (
                closed_url("https"),
                Protocol::Https,
                Reachability::Local,
                false,
                AttemptKind::Unreachable
            ),
            (
                closed_url("http"),
                Protocol::Http,
                Reachability::Local,
                false,
                AttemptKind::Unreachable
            ),
            (
                closed_url("http"),
                Protocol::Http,
                Reachability::Remote,
                false,
                AttemptKind::Unreachable
            ),
            (
                closed_url("http"),
                Protocol::Http,
                Reachability::Relay,
                false,
                AttemptKind::Unreachable
            ),
        ],
        "one attempt per candidate, in local > remote > relay / https > http order"
    );
    let message = failure.message();
    // The summary is one header line, one bullet per candidate actually tried
    // and the offline hint. Every newline in it comes from this client's own
    // fixed formatting; the label and the candidate URLs inside it went through
    // `log::sanitize`, so no server text can forge an extra line.
    assert!(message.contains("4 tried"), "{message}");
    assert_eq!(
        message.lines().count(),
        1 + failure.attempts().len() + 1,
        "header + one bullet per tried candidate + the offline hint"
    );
    for attempt in failure.attempts() {
        assert_eq!(attempt.message().lines().count(), 1, "one line per attempt");
        assert!(
            !attempt.message().contains(ROW_A),
            "an attempt line leaked the token"
        );
    }
    let described = plex_core::log::describe(&failure, &[ROW_A]);
    assert!(!described.contains(ROW_A));
    assert_eq!(
        described.lines().count(),
        message.lines().count(),
        "describe must not add or forge lines"
    );

    assert!(
        pms.requests().is_empty(),
        "no candidate addressed the mock host"
    );
    pms.shutdown();
}

#[test]
fn a_401_on_the_identifier_step_is_recorded_as_a_rejected_token() {
    let dir = common::TempDir::new("m3-sc-401");
    let client = common::client(&dir);
    let pms = MockPlex::start(Script::new().with(Route::Root, &[ok_probe()]).with(
        Route::Identity,
        &[Step::Respond(401, r#"{"errors":["bad token"]}"#)],
    ));

    let row = server_row("pms-401", ROW_A, vec![local_http(pms.base())]);
    let list = plex_core::resources::parse_resources(
        serde_json::to_string(&vec![row]).expect("rows").as_bytes(),
        "not-a-row",
    )
    .expect("the row must parse");

    let failure: ConnectFailure = connect(&list.servers()[0], &client)
        .expect_err("a rejected identifier call must not look like a connection");
    assert_eq!(
        failure.attempts().len(),
        1,
        "an attempt is recorded for the candidate that failed"
    );
    assert_eq!(
        failure.attempts()[0].kind,
        AttemptKind::TokenRejected(401),
        "401 is 'this server's own token was not accepted', not 'unreachable'"
    );
    assert!(
        failure.message().contains("access token"),
        "{}",
        failure.message()
    );

    let seen = pms.requests();
    assert_eq!(seen.len(), 2);
    assert!(seen[0].header(TOKEN_HEADER).is_none());
    assert_eq!(seen[1].header(TOKEN_HEADER), Some(ROW_A));
    assert_eq!(requests_with_token_header(&seen, TOKEN_HEADER, ROW_A), 1);

    pms.shutdown();
}

#[test]
fn a_bad_identifier_answer_is_recorded_as_a_shape_failure_and_moves_on() {
    let dir = common::TempDir::new("m3-sc-shape");
    let client = common::client(&dir);
    let pms = MockPlex::start(Script::new().with(Route::Root, &[ok_probe()]).with(
        Route::Identity,
        &[
            Step::Respond(
                200,
                r#"{"MediaContainer":{"friendlyName":"no identifier here"}}"#,
            ),
            identity_of("MACHINE-NEXT-6b4d0f9ca7e24185"),
        ],
    ));

    let row = server_row(
        "pms-shape",
        ROW_A,
        vec![local_http(pms.base()), local_http(pms.base())],
    );
    let list = plex_core::resources::parse_resources(
        serde_json::to_string(&vec![row]).expect("rows").as_bytes(),
        "not-a-row",
    )
    .expect("the row must parse");

    let server = connect(&list.servers()[0], &client).expect("the second candidate must succeed");
    assert!(server
        .machine_identifier()
        .matches("MACHINE-NEXT-6b4d0f9ca7e24185"));
    assert_eq!(pms.count_path(IDENTITY_PATH), 2);
    assert_eq!(pms.count_path(PROBE_PATH), 2);
    // A 200 that names no identifier is never filled in with a guess.
    assert!(!server
        .machine_identifier()
        .matches("MOCKMACHINE-3c9e2b7a51d84f06"));

    pms.shutdown();
}

// ------------------------------------------------------- the failure premise

/// The offline/unreachable tests below only mean something if the port they
/// address really is closed, so that premise is checked by execution here.
#[test]
fn the_closed_loopback_port_in_the_failure_tests_really_refuses_a_connection() {
    let port = closed_loopback_port();
    let error =
        TcpStream::connect(("127.0.0.1", port)).expect_err("a released loopback port is closed");
    assert_eq!(error.kind(), std::io::ErrorKind::ConnectionRefused);
    assert!(
        plex_core::log::describe(&error, &[ROW_A, MOCK_TOKEN])
            .lines()
            .count()
            <= 2,
        "a transport failure renders as a short, secret-free line"
    );
}

// ---------------------------------------------------- httpsRequired at connect

#[test]
fn an_https_required_server_is_never_probed_over_http() {
    let dir = common::TempDir::new("m3-sc-https-required");
    let client = common::client(&dir);
    // The mock speaks plain HTTP only, which makes "no http attempt was made"
    // observable: if the client had fallen back to http, the host would have
    // recorded requests.
    let pms = MockPlex::start(pms_script(ok_probe(), identity_fixture()));
    let port = mock_port(pms.base());
    let https_uri = format!("https://127.0.0.1:{port}");
    let http_uri = format!("http://127.0.0.1:{port}");

    let mut row = server_row(
        "pms-https",
        ROW_A,
        vec![
            connection(
                &https_uri,
                "https",
                true,
                false,
                false,
                "127.0.0.1",
                port,
                Some(&http_uri),
            ),
            connection(
                &http_uri,
                "http",
                false,
                false,
                false,
                "127.0.0.1",
                port,
                None,
            ),
        ],
    );
    row["httpsRequired"] = serde_json::json!(1);

    let list = rows_resource(vec![row]);
    let resource = &list.servers()[0];
    let candidates = resource.candidates();
    assert_eq!(
        candidates.len(),
        1,
        "only the https uri survives: its recorded httpuri fallback and the http connection are both dropped"
    );
    assert!(candidates.iter().all(|c| c.protocol == Protocol::Https));

    let failure = connect(resource, &client)
        .expect_err("an https candidate cannot reach a plain-HTTP mock host");
    assert!(failure.requires_https());
    assert_eq!(
        failure.attempts().len(),
        1,
        "exactly the https candidate was tried"
    );
    assert_eq!(failure.attempts()[0].protocol, Protocol::Https);
    assert_eq!(failure.attempts()[0].kind, AttemptKind::Unreachable);
    assert!(
        failure.message().contains("httpsRequired"),
        "the message must say why nothing else was tried: {}",
        failure.message()
    );
    assert!(
        pms.requests().is_empty(),
        "an httpsRequired server must never be contacted over http"
    );
    pms.shutdown();
}

#[test]
fn an_https_required_server_that_published_only_http_addresses_is_never_contacted() {
    let pms = MockPlex::start(pms_script(ok_probe(), identity_fixture()));

    let mut row = server_row("pms-http-only", ROW_A, vec![local_http(pms.base())]);
    row["httpsRequired"] = serde_json::json!(1);
    let list = rows_resource(vec![row]);
    // Not a selectable server at all: an httpsRequired row whose only published
    // addresses are `http` has nothing this client may try, so it is reported as
    // excluded instead of being probed over http.
    assert!(
        list.servers().is_empty(),
        "nothing may be selected or tried"
    );
    assert_eq!(list.excluded().len(), 1);
    let excluded = &list.excluded()[0];
    assert_eq!(excluded.client_identifier.as_deref(), Some("pms-http-only"));
    assert_eq!(excluded.reason, ExclusionReason::NoUsableConnection);
    assert!(excluded.https_required, "the reason the user is given");
    assert!(
        excluded.candidates().is_empty(),
        "httpsRequired dropped every http candidate"
    );
    assert!(
        !excluded.connections().is_empty(),
        "what plex.tv published for the row stays visible, including its addresses"
    );
    assert!(
        excluded.token().is_some(),
        "the row token stays available for an explicitly delegated connect"
    );
    assert!(
        pms.requests().is_empty(),
        "the published http address must not have been contacted anyway"
    );
    pms.shutdown();
}

// ------------------------------------------------------- delegated credential

/// `connect_shared` is the **only** path here that presents a
/// credential it did not read from that server's own row, and only when the
/// caller hands one over explicitly. Whether Plex accepts an account token on a
/// shared server is **[U]** (`docs/CAPABILITIES.md`), so this test proves this
/// client's own rule: it is labelled as delegated, it is header-only, and the
/// owned path never does it (see the token-separation test above).
#[test]
fn connect_shared_labels_the_credential_as_delegated_and_sends_it_header_only() {
    let dir = common::TempDir::new("m3-sc-delegated");
    let client = common::client(&dir);
    let pms = MockPlex::start(pms_script(ok_probe(), identity_fixture()));
    let typed = format!("127.0.0.1:{}", mock_port(pms.base()));

    let resource = plex_core::resources::shared_resource_from_host(&typed)
        .expect("a typed loopback host must be accepted");
    assert!(!resource.is_owned(), "a typed host has no proven ownership");
    assert!(
        !resource.has_access_token(),
        "a typed host has no row token"
    );
    assert_eq!(resource.token_len(), 0);
    assert_eq!(
        connect(&resource, &client)
            .expect_err("no row token, so the owned path must not invent one")
            .attempts()
            .len(),
        0,
        "connect() without a credential of its own tries nothing"
    );

    let server = connect_shared(&resource, &client, &SecretString::new(MOCK_TOKEN))
        .expect("the http candidate of the typed host must answer");
    assert!(
        server.uses_delegated_token(),
        "the caller supplied this credential"
    );
    assert!(server.token().matches(MOCK_TOKEN));
    assert!(!server.token().matches(ROW_A));
    assert_eq!(
        server.base_url(),
        format!("http://127.0.0.1:{}", mock_port(pms.base()))
    );
    assert!(
        !server.using_http_fallback(),
        "this is the host's own http address, not a recorded httpuri fallback"
    );
    assert!(server
        .machine_identifier()
        .matches("MOCKMACHINE-3c9e2b7a51d84f06"));

    let seen = pms.requests();
    assert_eq!(
        seen.len(),
        2,
        "one credential-free probe + one identifier call"
    );
    assert!(seen[0].header(TOKEN_HEADER).is_none());
    assert_eq!(seen[1].header(TOKEN_HEADER), Some(MOCK_TOKEN));
    assert_eq!(
        requests_with_token_header(&seen, TOKEN_HEADER, MOCK_TOKEN),
        1
    );
    for record in &seen {
        assert!(!record.path_and_query.contains("X-Plex-Token"));
        assert!(!record.path_and_query.contains(MOCK_TOKEN));
    }
    pms.shutdown();
}

// --------------------------------------------------------- typed host forms

/// A host the user typed has no `port` attribute from Plex: this client's own
/// default is 32400 (`docs/PLEX_AUTH.md` §1 item 7). Including a bracketed
/// IPv6 literal with no port, which must not be read as `…:[0-9]+` port text.
#[test]
fn a_typed_host_with_no_port_defaults_to_the_plex_port_in_every_accepted_form() {
    let accepted: [(&str, &str, u16); 5] = [
        ("192.168.1.10", "192.168.1.10", 32400),
        ("192.168.1.10:32401", "192.168.1.10", 32401),
        ("[2001:db8::1]", "2001:db8::1", 32400),
        ("[2001:db8::1]:32401", "2001:db8::1", 32401),
        ("::1:32400", "::1", 32400),
    ];
    for (input, address, port) in accepted {
        let resource = plex_core::resources::shared_resource_from_host(input)
            .unwrap_or_else(|error| panic!("{input} must be accepted: {error:?}"));
        let expected_host = bracketed(address);
        let connections = resource.connections();
        assert_eq!(
            connections.len(),
            2,
            "{input}: one http + one https candidate pair"
        );
        for connection in connections {
            // An IPv6 literal is stored bracketed: it is only a valid URL host that way.
            assert_eq!(connection.address, expected_host, "{input} address form");
            assert_eq!(connection.port, port, "{input} port default/explicit");
            let scheme = if connection.protocol == Protocol::Https {
                "https"
            } else {
                "http"
            };
            assert_eq!(
                connection.uri,
                format!("{scheme}://{expected_host}:{port}"),
                "{input} uri"
            );
        }
        assert_eq!(
            resource
                .candidates()
                .iter()
                .map(|c| c.protocol)
                .collect::<Vec<_>>(),
            vec![Protocol::Https, Protocol::Http],
            "{input}: https before http"
        );
    }
}

/// What the client refuses rather than guess at: these are rejected **before**
/// any request could be built, so no credential can be sent to a malformed host.
#[test]
fn a_typed_host_that_is_not_addressable_is_refused_with_no_candidates() {
    for rejected in [
        "",
        "   ",
        "host/path",
        "host with space",
        "user@host",
        "host:0",
        "host:notaport",
        "[bad::hex]",
        "[]",
        "[::1",
    ] {
        let outcome = plex_core::resources::shared_resource_from_host(rejected);
        assert!(
            matches!(outcome, Err(plex_core::error::CoreError::InvalidInput)),
            "{rejected:?} must be refused, not guessed at"
        );
    }
}

fn mock_port(base: &str) -> u16 {
    base.rsplit_once(':')
        .and_then(|(_, port)| port.parse::<u16>().ok())
        .expect("mock base must carry a port")
}

fn bracketed(address: &str) -> String {
    if address.contains(':') {
        format!("[{address}]")
    } else {
        address.to_owned()
    }
}

#[test]
fn a_401_on_the_token_less_probe_still_counts_as_a_live_server() {
    // A real PMS answers `GET /` without a token with 401.
    let dir = common::TempDir::new("m3-sc-probe401");
    let client = common::client(&dir);
    let pms = MockPlex::start(pms_script(
        Step::Respond(401, "<html>Unauthorized</html>"),
        identity_fixture(),
    ));
    let row = server_row("pms-401", ROW_A, vec![local_http(pms.base())]);
    let list = plex_core::resources::parse_resources(
        serde_json::to_string(&vec![row]).expect("rows").as_bytes(),
        "not-a-row",
    )
    .expect("the row must parse");
    let server = connect(&list.servers()[0], &client).expect("a 401 probe is a live host");
    assert_eq!(server.base_url(), pms.base());
    pms.shutdown();
}
