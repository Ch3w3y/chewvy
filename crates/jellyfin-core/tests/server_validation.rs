//! Server validation and identity against the mock: `Authorization` header
//! shape, `/System/Info/Public`, `/System/Ping`, URL normalisation and the
//! redaction contract for the token.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

mod mock;

use jellyfin_core::device::DeviceInfo;
use jellyfin_core::header::{media_browser_header, AUTHORIZATION_HEADER};
use jellyfin_core::server;
use jellyfin_core::{Http, JfError};

const MOCK_JF_TOKEN: &str = "mockjf-token-0f9e8d7c6b5a4321";

fn device() -> DeviceInfo {
    DeviceInfo {
        app: "Chewvy".to_owned(),
        app_version: "0.1.0".to_owned(),
        device: "test-machine".to_owned(),
        device_id: "0a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d".to_owned(),
    }
}

fn http_with(base: &str) -> Http {
    Http::with_timeouts(
        base,
        std::time::Duration::from_secs(5),
        std::time::Duration::from_secs(5),
    )
    .expect("mock base must normalise")
}

#[test]
fn a_server_is_validated_by_its_public_identity_and_ping() {
    let mock = mock::MockJfServer::start();
    mock.route(
        "GET /System/Info/Public",
        mock::Answer::json(
            200,
            r#"{"Id":"srv-uuid-1","ServerName":"Living Room","Version":"10.10.4"}"#,
        ),
    );
    mock.route(
        "GET /System/Ping",
        mock::Answer {
            status: 204,
            content_type: "text/plain".to_owned(),
            body: Vec::new(),
        },
    );
    let http = http_with(&mock.url());
    let info = server::public_info(&http).expect("public info");
    assert_eq!(info.id, "srv-uuid-1");
    assert_eq!(info.server_name, "Living Room");
    server::ping(&http).expect("ping");
    let requests = mock.requests();
    assert_eq!(requests.len(), 2, "exactly the two validation calls");
    // Public endpoints need no authorization header at all, and keep any
    // bearer parameter out of the URL.
    for request in &requests {
        assert!(
            request.authorization.is_none(),
            "public call sent a header: {:?}",
            request.path_and_query
        );
        assert!(!request.path_and_query.contains("api_key"));
        assert!(!request.path_and_query.contains("access_token"));
    }
}

#[test]
fn a_status_error_carries_only_its_code() {
    let mock = mock::MockJfServer::start();
    mock.route("GET /System/Info/Public", mock::Answer::json(500, "{}"));
    let error = server::public_info(&http_with(&mock.url())).expect_err("500 must fail");
    assert_eq!(error, JfError::HttpStatus { status: 500 });
    assert_eq!(error.code(), "http-status");
    // Display is a closed code plus a number: no body, no token.
    assert!(!format!("{error}").contains("Living"));
}

#[test]
fn urls_normalise_to_scheme_and_authority() {
    let cases = [
        ("http://host:8096", "http://host:8096"),
        ("http://host:8096/", "http://host:8096"),
        ("host:8096", "http://host:8096"),
        ("192.168.1.5", "http://192.168.1.5"),
        ("  https://jf.example.com  ", "https://jf.example.com"),
        ("http://host:8096/web/index.html", "http://host:8096"),
        ("https://host/web/#/home", "https://host"),
    ];
    for (input, expected) in cases {
        assert_eq!(
            server::normalize_base(input).as_deref(),
            Ok(expected),
            "{input}"
        );
    }
    for bad in ["", "   ", "ftp://host:21", "http://"] {
        assert_eq!(
            server::normalize_base(bad),
            Err(JfError::InvalidInput),
            "{bad:?} must be refused"
        );
    }
}

#[test]
fn the_authorization_header_quotes_percent_encodes_and_hides_the_token() {
    let value = media_browser_header(
        &device(),
        Some(&jellyfin_core::SecretString::new(MOCK_JF_TOKEN)),
    );
    // Quoted pairs, token last.
    assert!(value.starts_with("MediaBrowser Client=\"Chewvy\", "));
    assert!(value.ends_with(&format!("Token=\"{MOCK_JF_TOKEN}\"")));
    assert_eq!(AUTHORIZATION_HEADER, "Authorization");
    // A device label that would break the comma/quote parsing gets percent
    // encoded, so the server's split-on-comma still sees four fields.
    let mut tricky = device();
    tricky.device = "Kitchen, \"TV\" #1".to_owned();
    let value = media_browser_header(&tricky, None);
    let pairs = value.strip_prefix("MediaBrowser ").expect("scheme prefix");
    let commas = pairs.matches(',').count();
    assert_eq!(commas, 3, "escaped value must not add commas: {pairs}");
    assert!(pairs.contains("Device=\"Kitchen%2C%20%22TV%22%20%231\""));
}

#[test]
fn a_token_never_travels_in_a_url_or_the_error_output() {
    let mock = mock::MockJfServer::start();
    mock.route(
        "POST /Sessions/Logout",
        mock::Answer {
            status: 204,
            content_type: "text/plain".to_owned(),
            body: Vec::new(),
        },
    );
    let http = http_with(&mock.url());
    let token = jellyfin_core::SecretString::new(MOCK_JF_TOKEN);
    jellyfin_core::auth::sign_out(&http, &device(), &token).expect("sign out");
    let requests = mock.requests();
    let request = &requests[0];
    assert_eq!(request.method, "POST");
    assert_eq!(request.path_and_query, "/Sessions/Logout");
    assert!(request.body.is_empty(), "sign-out is a bodyless POST");
    let authorization = request.authorization.as_deref().expect("token header");
    assert!(authorization.contains(&format!("Token=\"{MOCK_JF_TOKEN}\"")));
    // Debug of the transport and of the secret both stay redacting.
    let rendered = format!("{http:?} {token:?}");
    assert!(!rendered.contains(MOCK_JF_TOKEN), "{rendered}");
}

#[test]
fn the_legacy_in_url_bearer_parameter_is_refused() {
    let http = http_with("http://127.0.0.1:1");
    let error = http
        .get("/System/Ping", &[("api_key", "anything")], None)
        .expect_err("api_key query must be refused");
    assert_eq!(error, JfError::InvalidInput);
}
