//! The two sign-in paths against the mock: Quick Connect (the browser-style
//! default) and password fallback, plus the ticks conversion contract.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

mod mock;

use jellyfin_core::auth;
use jellyfin_core::device::DeviceInfo;
use jellyfin_core::quickconnect::{self, QuickConnect};
use jellyfin_core::{Http, JfError};

const MOCK_JF_TOKEN: &str = "mockjf-access-9f8e7d6c5b4a3210";
const MOCK_QC_SECRET: &str = "0123456789abcdef0123456789abcdef";

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

fn authenticate_answer() -> mock::Answer {
    mock::Answer::json(
        200,
        format!(
            r#"{{"AccessToken":"{MOCK_JF_TOKEN}","User":{{"Id":"user-1","Name":"Alex"}},"ServerId":"srv-uuid-1"}}"#
        ),
    )
}

#[test]
fn quick_connect_runs_initiate_poll_pending_poll_activated_exchange() {
    let mock = mock::MockJfServer::start();
    mock.route("GET /QuickConnect/Enabled", mock::Answer::json(200, "true"));
    mock.route(
        "POST /QuickConnect/Initiate",
        mock::Answer::json(
            200,
            format!(r#"{{"Secret":"{MOCK_QC_SECRET}","Code":"481516","DeviceId":"dev-1"}}"#),
        ),
    );
    // First poll: not yet activated. Second poll: activated, and the exchange
    // that follows answers with the token.
    mock.route(
        "GET /QuickConnect/Connect",
        mock::Answer::json(200, r#"{"AuthUrl":"http://host/QuickConnect?code=481516","Secret":"s","Authenticated":false}"#),
    );
    mock.route(
        "GET /QuickConnect/Connect",
        mock::Answer::json(200, r#"{"Authenticated":true}"#),
    );
    mock.route(
        "POST /Users/AuthenticateWithQuickConnect",
        authenticate_answer(),
    );

    let http = http_with(&mock.url());
    let dev = device();
    assert!(quickconnect::enabled(&http).expect("enabled"));
    let ticket = quickconnect::initiate(&http, &dev).expect("initiate");
    assert_eq!(ticket.code, "481516");
    assert!(ticket.secret.matches(MOCK_QC_SECRET));

    assert_eq!(
        quickconnect::check(&http, &dev, &ticket).expect("first poll"),
        QuickConnect::Pending
    );
    match quickconnect::check(&http, &dev, &ticket).expect("second poll") {
        QuickConnect::Authorized(auth) => {
            assert_eq!(auth.server_id.as_deref(), Some("srv-uuid-1"));
            assert!(auth.into_token().matches(MOCK_JF_TOKEN));
        }
        other => panic!("expected Authorized, got {other:?}"),
    }

    // The polling URL carried the Quick Connect secret (the server's own API
    // design), the exchange carried it in the JSON body, and no request ever
    // carried the access token in a URL.
    let requests = mock.requests();
    let poll = requests
        .iter()
        .find(|r| r.path_and_query.starts_with("/QuickConnect/Connect"))
        .expect("poll request");
    assert!(poll
        .path_and_query
        .contains(&format!("secret={MOCK_QC_SECRET}")));
    let exchange = requests
        .iter()
        .find(|r| r.path_and_query == "/Users/AuthenticateWithQuickConnect")
        .expect("exchange request");
    assert!(exchange
        .body
        .contains(&format!("\"Secret\":\"{MOCK_QC_SECRET}\"")));
    assert!(
        !exchange.body.contains(MOCK_JF_TOKEN),
        "token is an answer, never a request field"
    );
    for request in &requests {
        assert!(
            request
                .authorization
                .as_deref()
                .is_none_or(|value| !value.contains("Token=")),
            "no token before sign-in: {:?}",
            request.path_and_query
        );
    }
}

#[test]
fn an_unknown_or_aged_out_secret_expires() {
    let mock = mock::MockJfServer::start();
    mock.route(
        "POST /QuickConnect/Initiate",
        mock::Answer::json(
            200,
            format!(r#"{{"Secret":"{MOCK_QC_SECRET}","Code":"481516"}}"#),
        ),
    );
    // 404 is what the server answers for an unknown or aged-out secret.
    let http = http_with(&mock.url());
    let dev = device();
    let ticket = quickconnect::initiate(&http, &dev).expect("initiate");
    assert_eq!(
        quickconnect::check(&http, &dev, &ticket).expect("poll of unknown secret"),
        QuickConnect::Expired
    );
}

#[test]
fn a_disabled_quick_connect_is_announced_not_silently_pending() {
    let mock = mock::MockJfServer::start();
    mock.route(
        "GET /QuickConnect/Enabled",
        mock::Answer::json(200, "false"),
    );
    mock.route(
        "POST /QuickConnect/Initiate",
        mock::Answer {
            status: 401,
            content_type: "text/plain".to_owned(),
            body: Vec::new(),
        },
    );
    let http = http_with(&mock.url());
    assert!(!quickconnect::enabled(&http).expect("disabled"));
    assert_eq!(
        quickconnect::initiate(&http, &device()),
        Err(JfError::QuickConnectDisabled)
    );
}

#[test]
fn password_sign_in_posts_credentials_and_answers_with_a_token() {
    let mock = mock::MockJfServer::start();
    mock.route("POST /Users/AuthenticateByName", authenticate_answer());
    let http = http_with(&mock.url());
    let auth = auth::sign_in_password(&http, &device(), "alex", "s3cret pw").expect("sign in");
    assert_eq!(auth.user.as_ref().expect("user").id, "user-1");
    let request = &mock.requests()[0];
    assert_eq!(request.method, "POST");
    assert_eq!(request.path_and_query, "/Users/AuthenticateByName");
    assert!(request.body.contains(r#""Username":"alex""#));
    assert!(request.body.contains(r#""Pw":"s3cret pw""#));
    // The password is body-only: not in the URL, and the identity header
    // carries no Token yet.
    assert!(!request.path_and_query.contains("s3cret"));
    let authorization = request.authorization.as_deref().expect("identity header");
    assert!(!authorization.contains("Token="));
}

#[test]
fn a_wrong_password_is_an_auth_rejection_not_a_shape_error() {
    let mock = mock::MockJfServer::start();
    mock.route(
        "POST /Users/AuthenticateByName",
        mock::Answer {
            status: 401,
            content_type: "application/json".to_owned(),
            body: r#"{"type":"https://tools.ietf.org/html/rfc9110#section-15.5.2"}"#
                .as_bytes()
                .to_vec(),
        },
    );
    let error = auth::sign_in_password(&http_with(&mock.url()), &device(), "alex", "nope")
        .expect_err("401 must reject");
    assert_eq!(error, JfError::AuthRejected { status: 401 });
    // Display stays closed: the server's problem-document body never rides in.
    assert!(!format!("{error}").contains("tools.ietf.org"));
}

#[test]
fn an_empty_password_is_allowed_but_an_empty_username_is_not() {
    let mock = mock::MockJfServer::start();
    mock.route("POST /Users/AuthenticateByName", authenticate_answer());
    let http = http_with(&mock.url());
    auth::sign_in_password(&http, &device(), "alex", "").expect("passwordless user");
    assert_eq!(
        auth::sign_in_password(&http, &device(), "", ""),
        Err(JfError::InvalidInput)
    );
}

#[test]
fn ticks_convert_with_explicit_loss_below_100ns() {
    use jellyfin_core::ticks::{duration_to_ticks, ticks_to_duration, TICKS_PER_SECOND};
    assert_eq!(TICKS_PER_SECOND, 10_000_000);
    assert_eq!(
        ticks_to_duration(1_234_567_890),
        Some(std::time::Duration::from_micros(123_456_789))
    );
    assert_eq!(
        duration_to_ticks(std::time::Duration::from_millis(1_234)),
        12_340_000
    );
    // Round trip is stable up to tick precision (the sub-tick tail is lost).
    let ticks = 9_007_199_254_740_000; // ~104 days
    let duration = ticks_to_duration(ticks).expect("valid");
    assert_eq!(duration_to_ticks(duration), ticks);
    assert_eq!(ticks_to_duration(-1), None, "negative ticks are absent");
}
