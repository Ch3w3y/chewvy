//! Shared endpoint-test scaffolding: a mock PMS a real
//! `plex_core::server::connect` can connect to, plus the request-line readers
//! the endpoint tests use to assert what went on the wire.
//!
//! Compiled into both P1b test binaries (`tests/library_endpoints.rs` for each
//! new call's captured answer, `tests/library_browse_query.rs` for the query a
//! browse option builds), so not every helper is used by both.

#![allow(dead_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use plex_core::client_info::TOKEN_HEADER;
use plex_core::resources::parse_resources;
use plex_core::server::{connect, Server};

use crate::common;
use crate::mock::{no_record_carries, MockPlex, Recorded, Route, Script, Step, MOCK_TOKEN};

/// The `accessToken` of the row these tests connect with.
pub const ROW_TOKEN: &str = "ROWTOKEN-P1B-9d41b7c60e5a2f83";

/// The `clientIdentifier` of that row, deliberately different from this test
/// client's own so the row is not skipped as "our own install".
pub const ROW_CLIENT: &str = "row-client-p1b";

/// Rating keys and a section key the captures were taken for.
pub const CAPTURED_SHOW: &str = "35773";
pub const CAPTURED_SEASON: &str = "36099";
pub const CAPTURED_SECTION: &str = "2";

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

/// A `provides: "server"`, `owned: 1` row pointing at `base`, carrying
/// [`ROW_TOKEN`] as its own `accessToken`.
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

/// A mock PMS that can be connected to (it answers the probe and `GET /
/// identity`), plus the server the product builds from a **parsed** row.
///
/// The caller re-arms the script for the route under test: `connect` has
/// already run, so only that route needs an answer afterwards.
pub fn p1b_session(tag: &str) -> (MockPlex, Server) {
    let dir = common::TempDir::new(tag);
    let client = common::client(&dir);
    let pms = MockPlex::start(
        Script::new()
            .with(Route::Root, &[Step::Respond(200, r#"{"mock":"root"}"#)])
            .with(Route::Identity, &[Step::Fixture("identity.json")]),
    );
    let list = parse_resources(
        serde_json::to_string(&vec![server_row(pms.base())])
            .expect("mock row must serialize")
            .as_bytes(),
        "this-client-is-not-the-row",
    )
    .expect("mock row must parse");
    assert_eq!(
        list.servers().len(),
        1,
        "the scripted row is the only server"
    );
    let server = connect(&list.servers()[0], &client).expect("mock PMS must connect");
    assert_eq!(server.base_url(), pms.base());
    assert!(server.token().matches(ROW_TOKEN));
    (pms, server)
}

/// Arm one route with one captured fixture.
pub fn capture_script(route: Route, fixture: &'static str) -> Script {
    Script::new().with(route, &[Step::Fixture(fixture)])
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

/// Percent-decode (`%XX`, `+`) of a recorded request's query part.
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
pub fn query_pairs(record: &Recorded) -> Vec<(String, String)> {
    decoded_query(record)
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            (key.to_owned(), value.to_owned())
        })
        .collect()
}

/// Every request the mock recorded for `path`, each proven to be a `GET` on
/// that exact path with the row token in the **header** only and nowhere in a
/// query. Returns the records so a test can read the query it asked with.
fn one_get(path: &str, records: &[Recorded]) -> Vec<Recorded> {
    let calls: Vec<Recorded> = records
        .iter()
        .filter(|record| path_of(record) == path)
        .cloned()
        .collect();
    assert_eq!(calls.len(), 1, "{path} must be requested exactly once");
    for record in &calls {
        assert_eq!(record.method, "GET", "{path} must be a GET");
        assert_eq!(
            record.header(TOKEN_HEADER),
            Some(ROW_TOKEN),
            "{path} must carry the row token in its header"
        );
        assert!(
            !record.query_contains(TOKEN_HEADER),
            "{path} must not carry a credential in its query"
        );
        assert!(
            !record.query_contains(ROW_TOKEN),
            "{path} must not carry the row token in its query"
        );
    }
    // No other request (no write, no second host call) may have been made.
    assert_eq!(
        records.len(),
        calls.len(),
        "only {path} may be requested after connect"
    );
    assert!(
        records.iter().all(|record| record.method == "GET"),
        "every recorded request must be a GET"
    );
    for record in records {
        assert_eq!(
            record.value_matches(ROW_TOKEN),
            1,
            "the row token appears exactly once, as the {TOKEN_HEADER} header"
        );
        assert!(
            !record.path_and_query.contains(ROW_TOKEN) && !record.body.contains(ROW_TOKEN),
            "the row token never reaches a request line or body"
        );
    }
    assert!(
        no_record_carries(records, MOCK_TOKEN),
        "no account token reaches the PMS"
    );
    calls
}

/// The requests the mock recorded **after** `connect` (which probes `/` and asks
/// `/identity`), driven through [`one_get`] so a test checks the whole exchange
/// for the route under test and nothing else.
pub fn driven(pms: &MockPlex, baseline: usize, path: &str) -> Vec<Recorded> {
    let all = pms.requests();
    assert!(
        all.len() >= baseline,
        "connect's own requests are already recorded"
    );
    one_get(path, &all[baseline..])
}

/// A test's own `query_pairs` row (the pair types are `(String, String)`).
pub fn pair(key: &str, value: &str) -> (String, String) {
    (key.to_owned(), value.to_owned())
}
