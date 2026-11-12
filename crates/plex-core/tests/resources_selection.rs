//! Server-discovery behaviour tests: the verified-field guard, the
//! selection rules (`provides == "server"` + `owned`), the candidate order,
//! `httpsRequired` filtering, the accepted JSON renderings and label hygiene.
//!
//! These are **mock/unit tests of this client's own rules**: the row shapes come
//! from `tests/fixtures/` (hand-authored, see `docs/CAPABILITIES.md`). They
//! say nothing about real plex.tv behaviour, which is not yet verified against a real server.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)] // product code keeps these lints on

use std::path::PathBuf;

use plex_core::client_info::TOKEN_HEADER;
use plex_core::log::MAX_LABEL_LEN;
use plex_core::resources::{
    candidate_order, fetch_resources, parse_resources, ExclusionReason, Protocol, Reachability,
};
use plex_core::secret::SecretString;

mod common;
mod mock;

use mock::{MockPlex, Route, Script, Step, MOCK_TOKEN};

/// Names this client reads from a `resources` row/connection (the verified set,
/// `docs/PLEX_AUTH.md` §1 item 6).
const VERIFIED: [&str; 18] = [
    "accessToken",
    "clientIdentifier",
    "device",
    "name",
    "provides",
    "presence",
    "owned",
    "httpsRequired",
    "dnsRebindingProtection",
    "natLoopbackSupported",
    "relay",
    "address",
    "port",
    "protocol",
    "uri",
    "local",
    "IPv6",
    "httpuri",
];

/// Container-wrapper names accepted *only* as the JSON rendering of the row
/// list (each is **[U]**, see `docs/CAPABILITIES.md`).
const WRAPPER: [&str; 3] = ["MediaContainer", "media-server", "mediaServer"];

/// Names the retained field list records as **not existing**; the module
/// documents them but must never map them.
const FORBIDDEN: [&str; 5] = [
    "ownedBy",
    "machineIdentifier",
    "httpsEnabled",
    "reachable",
    "secure",
];

fn resources_source() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("resources.rs");
    std::fs::read_to_string(&path).expect("resources.rs must be readable for the field guard")
}

/// Every `#[serde(rename = "...")]` name in the module, i.e. the whole set of
/// wire names this client maps.
fn mapped_names(source: &str) -> Vec<String> {
    let mut names = Vec::new();
    for line in source.lines() {
        // Doc prose may *describe* an attribute; only code maps one.
        if line.trim_start().starts_with("//") {
            continue;
        }
        let mut offset = 0;
        while let Some(found) = line[offset..].find("rename = \"") {
            let at = offset + found;
            let rest = &line[at + "rename = \"".len()..];
            let end = rest.find('"').expect("rename value must be quoted");
            names.push(rest[..end].to_owned());
            offset = at + 1;
        }
    }
    names
}

#[test]
fn m3_resources_field_guard_is_the_verified_set_only() {
    let source = resources_source();
    let names = mapped_names(&source);
    assert!(!names.is_empty(), "the wire models must map named fields");
    for name in &names {
        assert!(
            VERIFIED.contains(&name.as_str()) || WRAPPER.contains(&name.as_str()),
            "resources.rs maps {name}, which is outside the verified set \
             (container wrappers {WRAPPER:?} are the only extra names allowed)"
        );
    }
    // The container names may only ever appear as wrappers, never as a row or
    // connection field.
    for forbidden in FORBIDDEN {
        let hits = source
            .lines()
            .filter(|l| {
                let l = l.trim_start();
                !l.starts_with("//") && contains_word(l, forbidden)
            })
            .collect::<Vec<_>>();
        assert!(
            hits.is_empty(),
            "{forbidden} is used as code in resources.rs ({}), but the retained field list \
             records it as absent",
            hits.join(" | ").trim()
        );
    }
}

/// Word-boundary-ish match so `Reachability` is not read as `reachable`.
fn contains_word(line: &str, word: &str) -> bool {
    let boundary = |c: char| !c.is_ascii_alphanumeric() && c != '_';
    let mut from = 0;
    while let Some(found) = line[from..].find(word) {
        let at = from + found;
        let before_ok = at == 0 || boundary(line.as_bytes()[at - 1] as char);
        let after = at + word.len();
        let after_ok = after >= line.len() || boundary(line.as_bytes()[after] as char);
        if before_ok && after_ok {
            return true;
        }
        from = at + 1;
    }
    false
}

fn row(client_id: &str, provides: &str, owned: bool, token: &str) -> serde_json::Value {
    serde_json::json!({
        "accessToken": token,
        "clientIdentifier": client_id,
        "device": format!("{client_id}-device"),
        "name": format!("{client_id} name"),
        "provides": provides,
        "presence": 1,
        "owned": if owned { 1 } else { 0 },
        "httpsRequired": 0,
        "dnsRebindingProtection": 1,
        "natLoopbackSupported": 1,
        "relay": 0,
        "connections": [{
            "address": "10.0.0.1",
            "port": 32400,
            "protocol": "https",
            "uri": "https://10.0.0.1:32400",
            "local": 1, "relay": 0, "IPv6": 0,
            "httpuri": "http://10.0.0.1:32400"
        }]
    })
}

fn rows_text(rows: Vec<serde_json::Value>) -> String {
    serde_json::to_string(&rows).expect("test rows must serialize")
}

#[test]
fn selects_owned_and_shared_server_rows_with_their_own_token() {
    let body = rows_text(vec![
        row("srv-owned", "server", true, "ROWTOKEN-OWNED-1111"),
        row("srv-shared", "server", false, "ROWTOKEN-SHARED-2222"),
        row("srv-notoken", "server", true, ""),
        row("cli-1", "client", true, "ROWTOKEN-CLIENT-3333"),
        row("box-1", "player", true, "ROWTOKEN-PLAYER-4444"),
        row("ours", "server", true, "ROWTOKEN-OURS-5555"),
    ]);
    let list = parse_resources(body.as_bytes(), "ours").expect("rows must parse");

    assert_eq!(
        list.servers()
            .iter()
            .map(|s| s.client_identifier())
            .collect::<Vec<_>>(),
        vec!["srv-owned", "srv-shared"],
        "a provides==server row with its own token is selectable, owned or shared"
    );
    let reasons: Vec<(String, ExclusionReason)> = list
        .excluded()
        .iter()
        .map(|e| (e.client_identifier.clone().unwrap_or_default(), e.reason))
        .collect();
    assert!(reasons.contains(&(
        "srv-notoken".to_owned(),
        ExclusionReason::MissingAccessToken
    )));
    assert!(reasons.contains(&("cli-1".to_owned(), ExclusionReason::NotAServer)));
    assert!(reasons.contains(&("box-1".to_owned(), ExclusionReason::NotAServer)));
    assert!(reasons.contains(&("ours".to_owned(), ExclusionReason::OwnDeviceRow)));

    // A row with no addressable connection is reported, not silently dropped.
    let mut broken = row("srv-noconn", "server", true, "ROWTOKEN-BROKEN-6666");
    broken["connections"] = serde_json::json!([{"address": "10.0.0.2", "port": 32400}]);
    let list = parse_resources(rows_text(vec![broken]).as_bytes(), "ours").expect("must parse");
    assert!(list.servers().is_empty());
    assert_eq!(
        list.excluded()[0].reason,
        ExclusionReason::NoUsableConnection
    );
}

#[test]
fn a_shared_row_is_selectable_with_its_own_token() {
    let list = parse_resources(
        rows_text(vec![row(
            "srv-shared",
            "server",
            false,
            "ROWTOKEN-SHARED-2222",
        )])
        .as_bytes(),
        "ours",
    )
    .expect("rows must parse");
    assert_eq!(list.servers().len(), 1, "a shared server can be browsed");
    let shared = &list.servers()[0];
    assert!(!shared.is_owned(), "owned == 0 must not be read as owned");
    assert_eq!(shared.token_len(), "ROWTOKEN-SHARED-2222".len());
    assert!(!shared.candidates().is_empty());
}

#[test]
fn candidate_order_is_local_then_remote_then_relay_then_https_then_http_then_ipv4() {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/resources_rows.json"
    ))
    .expect("resources_rows.json fixture must be readable");
    let list = parse_resources(text.as_bytes(), "this-install-id").expect("fixture must parse");
    let server = list.servers()[0].clone();
    let order: Vec<(Reachability, Protocol, bool, String)> = server
        .candidates()
        .iter()
        .map(|c| (c.reachability, c.protocol, c.fallback, c.base_url.clone()))
        .collect();
    assert_eq!(
        order,
        vec![
            (
                Reachability::Local,
                Protocol::Https,
                false,
                "https://192.168.1.10:32400".to_owned()
            ),
            (
                Reachability::Local,
                Protocol::Https,
                false,
                "https://[2001:db8:1::10]:32400".to_owned()
            ),
            (
                Reachability::Local,
                Protocol::Http,
                true,
                "http://192.168.1.10:32400".to_owned()
            ),
            (
                Reachability::Local,
                Protocol::Http,
                true,
                "http://[2001:db8:1::10]:32400".to_owned()
            ),
            (
                Reachability::Remote,
                Protocol::Https,
                false,
                "https://203.0.113.10:32400".to_owned()
            ),
            (
                Reachability::Remote,
                Protocol::Http,
                true,
                "http://203.0.113.10:32400".to_owned()
            ),
            (
                Reachability::Relay,
                Protocol::Https,
                false,
                "https://relay.example.invalid:443".to_owned()
            ),
        ],
        "local > remote > relay, then https > http, then ipv4 > ipv6, httpuri last and marked"
    );
    assert!(server
        .candidates()
        .iter()
        .all(|c| c.protocol == Protocol::Https || c.fallback));
}

#[test]
fn a_https_required_server_never_gets_an_http_candidate() {
    let mut r = row("srv-tls", "server", true, "ROWTOKEN-TLS-7777");
    r["httpsRequired"] = serde_json::json!(1);
    let list = parse_resources(rows_text(vec![r]).as_bytes(), "ours").expect("must parse");
    let server = &list.servers()[0];
    assert!(server.requires_https());
    assert!(
        server
            .candidates()
            .iter()
            .all(|c| c.protocol == Protocol::Https),
        "httpsRequired must drop every http/httpuri candidate"
    );
    assert_eq!(server.candidates().len(), 1);
    // The same rule through `candidate_order` directly.
    let ordered = candidate_order(server.connections(), true);
    assert!(ordered.iter().all(|c| c.protocol == Protocol::Https));
}

#[test]
fn accepted_renderings_parse_and_anything_else_is_a_shape_error() {
    let rows = vec![row("srv-a", "server", true, "ROWTOKEN-A-8888")];
    let bare = rows_text(rows.clone());
    let kebab =
        serde_json::to_string(&serde_json::json!({"MediaContainer": {"media-server": rows}}))
            .expect("kebab wrapper");
    let camel =
        serde_json::to_string(&serde_json::json!({"MediaContainer": {"mediaServer": rows}}))
            .expect("camel wrapper");
    for body in [&bare, &kebab, &camel] {
        let list = parse_resources(body.as_bytes(), "ours").expect("rendering must parse");
        assert_eq!(
            list.servers().len(),
            1,
            "each accepted rendering must yield the row"
        );
    }
    for bad in [
        r#"{"nonsense":1}"#,
        "null",
        r#"[{"clientIdentifier":"x","provides":"server","owned":1,"accessToken":"t","connections":"oops"}]"#,
    ] {
        let error = parse_resources(bad.as_bytes(), "ours").expect_err("shape must be refused");
        assert!(
            matches!(error, plex_core::error::CoreError::ResponseShape),
            "unexpected shape must be ResponseShape, not {error:?}"
        );
    }
    // Tolerant parsing, recorded honestly: a `MediaContainer` whose inner keys
    // are unknown yields **no rows** rather than a shape error, so a server that
    // names things differently shows up as an empty list rather than a guess
    // (docs/CAPABILITIES.md: the `resources` JSON rendering is [U]).
    let empty = parse_resources(r#"{"MediaContainer":{"unexpected":[]}}"#.as_bytes(), "ours")
        .expect("unknown container keys are tolerated, not refused");
    assert!(empty.servers().is_empty() && empty.excluded().is_empty());
}

#[test]
fn server_supplied_labels_are_bounded_and_one_line() {
    let mut hostile = row("srv-hostile", "player", true, "ROWTOKEN-H-9999");
    hostile["name"] = serde_json::json!(format!(
        "evil\nsecond line\u{1b}[31mred\u{1b}[0m {}",
        "x".repeat(10_000)
    ));
    let list = parse_resources(rows_text(vec![hostile]).as_bytes(), "ours").expect("must parse");
    let excluded = &list.excluded()[0];
    assert!(!excluded.label.contains('\n'), "a label must stay one line");
    assert!(
        !excluded.label.contains('\u{1b}'),
        "an ANSI escape must be inert"
    );
    assert!(excluded.label.chars().count() <= MAX_LABEL_LEN + 1);
    let message = excluded.reason.message(&excluded.label);
    assert_eq!(
        message.matches('\n').count(),
        0,
        "a UI message must stay one line"
    );
    assert!(message.chars().count() < MAX_LABEL_LEN * 4);

    // `name()` still hands back what the server sent, for a safe widget.
    let owned = parse_resources(
        rows_text(vec![row("srv-ok", "server", true, "ROWTOKEN-OK-0000")]).as_bytes(),
        "ours",
    )
    .expect("must parse");
    assert_eq!(owned.servers()[0].display_name(), "srv-ok name");
}

#[test]
fn fetch_resources_uses_the_account_token_in_the_header_only_path() {
    // (1) The endpoint and query the call is built from are the documented ones.
    assert_eq!(
        plex_core::resources::RESOURCES_QUERY
            .map(|(k, v)| format!("{k}={v}"))
            .join("&"),
        "includeHttps=1&includeRelay=1&includeIPv6=1"
    );
    assert_eq!(plex_core::resources::RESOURCES_PATH, "/api/v2/resources");

    // (2) What actually goes on the wire: `fetch_resources` is run against a
    // mock plex.tv over real loopback HTTP and the assertion is against the
    // request that mock recorded from that request - not against constants.
    let dir = common::TempDir::new("resources-header-only");
    let client_info = common::client(&dir);
    let row_token = "ROWTOKEN-HEADER-ONLY-6c2f8ab4";
    let plex_tv = MockPlex::start(Script::new().with(
        Route::Resources,
        &[Step::FixtureJson(
            200,
            rows_text(vec![row("srv-header-only", "server", true, row_token)]),
        )],
    ));

    let list = fetch_resources(
        &common::http_at(plex_tv.base(), &client_info),
        &SecretString::new(MOCK_TOKEN),
        "this-client-is-not-the-row",
    )
    .expect("the mock resources answer must parse");

    let reqs = plex_tv.requests();
    assert_eq!(reqs.len(), 1, "one resources request, no retry");
    let call = reqs.first().expect("the mock recorded the request");
    assert_eq!(call.method, "GET");
    assert_eq!(
        call.path_and_query,
        format!(
            "{}?{}",
            plex_core::resources::RESOURCES_PATH,
            plex_core::resources::RESOURCES_QUERY
                .map(|(k, v)| format!("{k}={v}"))
                .join("&")
        ),
        "the request line is the documented path + query and nothing else"
    );
    // Header-only transport: the credential is present exactly once, as the
    // header, and the request URL contains no token of any kind.
    assert!(
        mock::token_only_in_header(call, TOKEN_HEADER),
        "the account token must appear only as {TOKEN_HEADER}"
    );
    assert!(
        !call.path_and_query.contains(TOKEN_HEADER),
        "no X-Plex-Token appears anywhere in the request URL"
    );
    assert!(
        !call.path_and_query.contains(MOCK_TOKEN),
        "the account token value is not in the URL"
    );
    assert!(
        !call.path_and_query.contains(row_token)
            && !call.headers.iter().any(|(_, value)| value == row_token),
        "a server row's own accessToken is never sent to plex.tv"
    );
    assert_eq!(list.servers().len(), 1, "the row parsed from that answer");
}
