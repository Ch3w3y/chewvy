//! Behaviour tests for `GET /api/v2/user` (`plex_core::api::fetch_account`)
//! against the **local mock Plex server**, using the recorded account shapes in
//! `tests/fixtures/user.json` and `tests/fixtures/user_2fa.json`.
//!
//! Mock tests, not Plex integration tests. What Plex actually returns for a
//! real account (and whether `rememberExpiresAt` is a hard TTL) is an M8 gate;
//! what is proven here is *our* parsing, our token-safety and our 401/422 rule.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)] // product code keeps these lints on

use plex_core::api::{fetch_account, Account, USER_PATH};
use plex_core::error::CoreError;
use plex_core::secret::{SecretString, REDACTED};

mod common;
mod mock;

use common::{client, http_at, TempDir};
use mock::{MockPlex, Route, Script, MOCK_TOKEN};

/// Start a mock whose only scripted route is `GET /api/v2/user` answering
/// `step`, plus a transport pointed at it and the token to send.
fn user_call(
    tag: &str,
    step: mock::Step,
) -> (TempDir, MockPlex, plex_core::http::Http, SecretString) {
    let dir = TempDir::new(tag);
    let client_info = client(&dir);
    let server = MockPlex::start(Script::new().with(Route::User, &[step]));
    let http = http_at(server.base(), &client_info);
    let token = SecretString::new(MOCK_TOKEN);
    (dir, server, http, token)
}

#[test]
fn the_recorded_account_shape_is_parsed_into_uuid_token_expiry_and_2fa() {
    let (_dir, server, http, token) = user_call("acct-user", mock::Step::Fixture("user.json"));
    let account = fetch_account(&http, &token).expect("the recorded user shape must parse");

    assert_eq!(account.uuid, "mock-uuid-0000-0000-000000000000");
    assert_eq!(account.username.as_deref(), Some("mockuser"));
    assert_eq!(account.remember_expires_at, Some(1_893_456_000));
    assert!(!account.two_factor_enabled);
    assert!(account.has_token(), "the fixture carries an authToken");
    assert_eq!(account.token_len(), MOCK_TOKEN.len());

    // The account token is reachable only through an explicit accessor.
    let carried = account.token().expect("Some(token)");
    assert!(carried.matches(MOCK_TOKEN));
    let taken = account
        .clone()
        .take_token()
        .expect("take_token yields the value");
    assert!(taken.matches(MOCK_TOKEN));

    // Exactly one request, header-only token, identity headers present.
    let requests = server.requests();
    assert_eq!(requests.len(), 1, "no retry, no extra calls");
    assert_eq!(requests[0].method, "GET");
    assert_eq!(requests[0].path_and_query, USER_PATH);
    assert!(
        mock::token_only_in_header(&requests[0], "X-Plex-Token"),
        "the account token must be header-only: {:?}",
        requests[0].path_and_query
    );
    assert_eq!(requests[0].header("Accept"), Some("application/json"));
}

#[test]
fn the_2fa_account_shape_is_parsed_as_two_factor_without_any_password_path() {
    let (_dir, server, http, token) = user_call("acct-2fa", mock::Step::Fixture("user_2fa.json"));
    let account = fetch_account(&http, &token).expect("the 2FA fixture must parse");
    assert!(account.two_factor_enabled, "twoFactorEnabled: true");
    assert_eq!(account.remember_expires_at, Some(0));
    assert_eq!(account.username.as_deref(), Some("mockuser-2fa"));
    assert!(account.has_token());

    // There is no 2FA code, password or credential field in the whole request
    // the mock saw: 2FA is completed in the browser sign-in page only.
    let request = &server.requests()[0];
    assert!(request.body.is_empty(), "GET /api/v2/user sends no body");
    for forbidden in ["password", "passcode", "totp", "code=", "authcode"] {
        assert!(
            !request.path_and_query.to_lowercase().contains(forbidden)
                && !request
                    .headers
                    .iter()
                    .any(|(name, value)| format!("{name} {value}")
                        .to_lowercase()
                        .contains(forbidden)),
            "the client must never send a {forbidden} field"
        );
    }
}

#[test]
fn an_account_response_without_an_auth_token_is_a_valid_account_without_a_token() {
    let (_dir, _server, http, token) = user_call(
        "acct-notoken",
        mock::Step::Respond(200, r#"{"uuid":"uuid-no-token","username":"no-token"}"#),
    );
    let account = fetch_account(&http, &token).expect("a uuid-only user is still an account");
    assert_eq!(account.uuid, "uuid-no-token");
    assert!(!account.has_token());
    assert_eq!(account.token_len(), 0);
    assert!(account.token().is_none());
    assert!(
        !account.two_factor_enabled,
        "absent twoFactorEnabled means false"
    );
    assert_eq!(account.remember_expires_at, None);
    assert_eq!(account.username.as_deref(), Some("no-token"));
    // `email` has no public accessor in M2 (api.rs keeps it private and renders
    // it as `<hidden>` in Debug), so a UI layer cannot render it by accident.
    let debug = format!("{account:?}");
    assert!(debug.contains("email: \"<hidden>\""), "{debug}");
}

#[test]
fn a_missing_or_empty_uuid_and_unparseable_bodies_are_shape_errors() {
    let cases = [
        mock::Step::Respond(200, r#"{"username":"no-uuid"}"#),
        mock::Step::Respond(200, r#"{"uuid":""}"#),
        mock::Step::Respond(200, "this is not JSON"),
        mock::Step::Respond(200, "[]"),
    ];
    for (index, step) in cases.into_iter().enumerate() {
        let (_dir, _server, http, token) = user_call(&format!("acct-shape-{index}"), step);
        let error = fetch_account(&http, &token)
            .expect_err("a malformed or uuid-less account body must be a shape error");
        assert_eq!(
            error,
            CoreError::ResponseShape,
            "case {index}: got {error:?}"
        );
        assert!(
            !format!("{error:?}").contains(MOCK_TOKEN),
            "case {index}: the error must not echo the body or the token: {error:?}"
        );
    }
}

#[test]
fn http_401_and_422_on_the_account_call_are_auth_rejections_not_retries() {
    for status in [401u16, 422u16] {
        let (_dir, server, http, token) = user_call(
            &format!("acct-auth-{status}"),
            mock::Step::Respond(
                status,
                r#"{"errors":[{"code":1}],"message":"invalid token"}"#,
            ),
        );
        let error =
            fetch_account(&http, &token).expect_err(&format!("{status} must be an auth rejection"));
        assert_eq!(error, CoreError::AuthRejected { status });
        assert_eq!(error.code(), "auth-rejected");
        assert_eq!(
            server.count_path(USER_PATH),
            1,
            "{status}: the client must not retry the account call blindly"
        );
        assert!(!error.to_string().contains(MOCK_TOKEN));
    }
}

#[test]
fn a_server_fault_on_the_account_call_is_not_reported_as_an_auth_failure() {
    for status in [429u16, 500u16, 503u16] {
        let (_dir, _server, http, token) =
            user_call(&format!("acct-status-{status}"), mock::Step::NoBody(status));
        let error = fetch_account(&http, &token)
            .expect_err(&format!("{status} must surface as a status error"));
        assert_eq!(error, CoreError::HttpStatus { status });
        assert_ne!(error, CoreError::AuthRejected { status });
    }
}

#[test]
fn account_debug_output_is_redacted_and_never_prints_the_token() {
    let (_dir, _server, http, token) = user_call("acct-debug", mock::Step::Fixture("user.json"));
    let account = fetch_account(&http, &token).expect("account must parse");
    let debug = format!("{account:?}");
    assert!(
        !debug.contains(MOCK_TOKEN),
        "Account Debug leaked the token: {debug}"
    );
    assert!(
        debug.contains(REDACTED),
        "the token slot must render as redacted"
    );
    assert!(
        debug.contains("uuid: \"mock-uuid-0000-0000-000000000000\""),
        "{debug}"
    );
    assert!(debug.contains("<hidden>"), "the e-mail slot must be hidden");
    assert!(
        !debug.contains("mock@example.invalid"),
        "e-mail must not be printed"
    );
    assert!(debug.contains("has_token: true"));

    // The token-bearing accessor's own Debug is redacted too.
    let secret = account.token().expect("Some(token)");
    assert!(!format!("{secret:?}").contains(MOCK_TOKEN));
    assert!(!secret.to_string().contains(MOCK_TOKEN));
}

/// `Account` is `Clone + PartialEq`; equality must not become a printing path.
#[test]
fn two_accounts_compared_and_kept_still_never_render_the_token() {
    let (_dir, _server, http, token) = user_call("acct-eq", mock::Step::Fixture("user.json"));
    let a = fetch_account(&http, &token).expect("account A");
    let b = a.clone();
    assert_eq!(a, b);
    let rendered = format!("{a:?} {b:?}");
    assert!(!rendered.contains(MOCK_TOKEN));
    let collected: Vec<Account> = vec![a.clone(), b.clone()];
    assert!(
        !collected
            .iter()
            .any(|acct| format!("{acct:?}").contains(MOCK_TOKEN)),
        "a collection of accounts must still be token-free to print"
    );
}
