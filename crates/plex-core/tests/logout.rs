//! Behaviour tests for logout: `DELETE /api/v2/users/signout`
//! (`plex_core::api::signout`) driven against the **local mock Plex server**,
//! paired with the local token deletion rule, plus the
//! [`plex_core::api::clear_token_on_auth_rejection`] helper.
//!
//! Mock tests, not Plex integration tests: they prove *our* local-deletion rule
//! and our no-blind-retry rule. What plex.tv really answers for a real account
//! token is not yet verified against a real server.
//!
//! Every case starts from the same state: a signed-in client with a real
//! `FileTokenStore` (consent granted in-test) that already holds
//! `mock::MOCK_TOKEN` on disk, so "deleted locally" is an observable fact
//! (`load() == None`, file gone) rather than a claim.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)] // product code keeps these lints on

use std::path::PathBuf;

use plex_core::api::{
    clear_token_on_auth_rejection, fetch_account, signout, Signout, SIGNOUT_PATH,
};
use plex_core::error::CoreError;
use plex_core::http::Http;
use plex_core::secret::SecretString;
use plex_core::tokens::{Consent, FileTokenStore, TokenKey, TokenStore};

mod common;
mod mock;

use common::{client, http_at, TempDir};
use mock::{MockPlex, Route, Script, Step, MOCK_TOKEN};

/// The identity header set every request must carry (asserted on the wire).
const WIRE_HEADER_NAMES: [&str; 10] = [
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

/// The Plex slot these tests exercise; any well-formed account uuid will do.
fn test_key() -> TokenKey {
    TokenKey::plex("deadbeef-dead-4ead-beef-deadbeefdead").expect("valid test uuid")
}

/// A signed-in fixture: mock Plex answering `SIGNOUT_PATH` with `steps`, the
/// transport, a `FileTokenStore` that already holds the mock token, and the
/// private temp dir that keeps it (declared last so it outlives the store).
struct SignedIn {
    server: MockPlex,
    http: Http,
    store: FileTokenStore,
    token: SecretString,
    key: TokenKey,
    path: PathBuf,
    /// Never read: declared last so the private temp dir outlives the store
    /// that writes into it. The `_` prefix states that it is an owner, not a
    /// value a test forgot to use.
    _dir: TempDir,
}

fn signed_in(tag: &str, steps: &[Step]) -> SignedIn {
    signed_in_scripted(tag, Script::new().with(Route::Signout, steps))
}

fn signed_in_scripted(tag: &str, script: Script) -> SignedIn {
    let dir = TempDir::new(tag);
    let client_info = client(&dir);
    let server = MockPlex::start(script);
    let http = http_at(server.base(), &client_info);
    let path = dir.join("tokens.json");
    let store = FileTokenStore::with_consent(&path, &dir.join("token.json"), Consent::granted())
        .expect("the test grants the plaintext-file consent");
    let key = test_key();
    store
        .store(&key, &SecretString::new(MOCK_TOKEN))
        .expect("fixture must start signed in");
    SignedIn {
        server,
        http,
        store,
        token: SecretString::new(MOCK_TOKEN),
        key,
        path,
        _dir: dir,
    }
}

impl SignedIn {
    /// Is a token still stored locally, and is the file there? Local state a
    /// test can actually observe instead of trusting the API's own words.
    fn local_token(&self) -> Option<String> {
        self.store
            .load(&self.key)
            .expect("load must not error")
            .map(|token| token.expose().to_owned())
    }

    fn expect_signed_out(&self, why: &str) {
        assert!(self.local_token().is_none(), "{why}: token must be gone");
        assert!(!self.path.exists(), "{why}: token file must be removed");
    }

    fn expect_still_signed_in(&self, why: &str) {
        let stored = self
            .local_token()
            .unwrap_or_else(|| panic!("{why}: the local token must be kept"));
        assert_eq!(stored, MOCK_TOKEN, "{why}: kept token must be unchanged");
        assert!(self.path.exists(), "{why}: token file must still exist");
    }

    /// Failures must not be an excuse to leak the value into a message.
    ///
    /// Takes `impl AsRef<str>` so callers can hand it a formatted `String` (a
    /// rendered error) without a separate borrow; the guard is unchanged.
    fn assert_token_free(message: impl AsRef<str>) {
        let message = message.as_ref();
        assert!(
            !message.contains(MOCK_TOKEN),
            "error text leaked: {message}"
        );
    }
}

#[test]
fn a_204_signout_deletes_the_local_token_and_reports_logged_out() {
    let f = signed_in("logout-204", &[Step::NoBody(204)]);
    let outcome =
        signout(&f.http, &f.token, &f.store, Some(&f.key)).expect("204 signout must succeed");
    assert_eq!(outcome, Signout::LoggedOut);

    f.expect_signed_out("204 logout");
    assert!(f.store.load(&f.key).expect("load after logout").is_none());
    assert!(f
        .store
        .stored_at(&f.key)
        .expect("stored_at after logout")
        .is_none());

    // Exactly one request: no retry, no extra endpoints.
    let requests = f.server.requests();
    assert_eq!(requests.len(), 1, "signout must not be retried");
    assert_eq!(requests[0].method, "DELETE");
    assert_eq!(requests[0].path_and_query, SIGNOUT_PATH);
    assert!(
        mock::token_only_in_header(&requests[0], "X-Plex-Token"),
        "the token must ride only in the header: {:?}",
        requests[0].path_and_query
    );
    assert_eq!(requests[0].value_matches(MOCK_TOKEN), 1);
    assert!(requests[0].body.is_empty(), "signout sends no body");
}

#[test]
fn a_401_or_422_signout_reports_already_signed_out_and_still_deletes_the_token() {
    for status in [401u16, 422u16] {
        let f = signed_in(
            &format!("logout-{status}"),
            &[Step::Respond(
                status,
                r#"{"errors":[{"code":1}],"message":"invalid token"}"#,
            )],
        );
        let outcome = signout(&f.http, &f.token, &f.store, Some(&f.key))
            .unwrap_or_else(|e| panic!("{status} signout must be a success path, got {e:?}"));
        assert_eq!(
            outcome,
            Signout::AlreadySignedOut { status },
            "{status}: reported as already signed out, not as an error"
        );
        f.expect_signed_out(&format!("{status} logout"));
        assert_eq!(
            f.server.count_path(SIGNOUT_PATH),
            1,
            "{status}: signout must not be retried blindly"
        );
        assert_eq!(f.server.requests().len(), 1, "{status}: no other calls");
    }
}

#[test]
fn a_server_or_client_error_on_signout_keeps_the_local_token_so_logout_can_be_retried() {
    // Statuses that do NOT mean "your token is rejected": the local token is
    // deliberately kept so the user can try again instead of being silently
    // signed out here while still signed in at Plex.
    for status in [400u16, 403u16, 429u16, 500u16, 503u16] {
        let f = signed_in(&format!("logout-keep-{status}"), &[Step::NoBody(status)]);
        let error = signout(&f.http, &f.token, &f.store, Some(&f.key))
            .expect_err(&format!("{status} signout must surface an error"));
        assert_eq!(
            error,
            CoreError::HttpStatus { status },
            "{status}: {error:?}"
        );
        assert_ne!(error, CoreError::AuthRejected { status });
        assert_eq!(error.code(), "http-status");
        SignedIn::assert_token_free(format!("{error:?} {error}"));
        f.expect_still_signed_in(&format!("{status} logout failure"));
        assert_eq!(
            f.server.count_path(SIGNOUT_PATH),
            1,
            "{status}: no logout retry loop"
        );
    }
}

#[test]
fn a_cut_connection_on_signout_keeps_the_token_and_is_not_an_auth_event() {
    // The mock answers with bytes that are not a parseable HTTP answer, so the
    // client sees the connection cut mid-answer. That is a transport failure,
    // not "signed out", and not a status code.
    //
    // This is deliberately not `drop(request)` in the mock: `tiny_http` 0.12.0
    // answers a dropped request with HTTP 500 (see `tests/mock/mod.rs`), so
    // that step would be a second copy of the 500 case below.
    let f = signed_in("logout-cut", &[Step::CutConnection]);
    let error = signout(&f.http, &f.token, &f.store, Some(&f.key))
        .expect_err("a cut connection must not look like a logout");
    assert_eq!(error, CoreError::Transport, "got {error:?}");
    assert_eq!(error.code(), "transport");
    SignedIn::assert_token_free(format!("{error:?} {error}"));
    f.expect_still_signed_in("cut connection");
    assert_eq!(f.server.count_path(SIGNOUT_PATH), 1, "no logout retry loop");
}

#[test]
fn a_refused_connection_on_signout_keeps_the_token_too() {
    // Nothing listening at all (loopback port 1, never used by the mock which
    // binds an ephemeral port): the request never reaches Plex, so the local
    // token must survive for a later retry.
    let dir = TempDir::new("logout-refused");
    let client_info = client(&dir);
    let store = FileTokenStore::with_consent(
        &dir.join("tokens.json"),
        &dir.join("token.json"),
        Consent::granted(),
    )
    .expect("consent granted by the test");
    let key = test_key();
    let token = SecretString::new(MOCK_TOKEN);
    store.store(&key, &token).expect("fixture starts signed in");
    let http = http_at("http://127.0.0.1:1", &client_info);

    let error =
        signout(&http, &token, &store, Some(&key)).expect_err("a refused connection must fail");
    assert_eq!(error, CoreError::Transport, "got {error:?}");
    assert_eq!(error.code(), "transport");
    SignedIn::assert_token_free(format!("{error:?} {error}"));
    assert!(
        store
            .load(&key)
            .expect("load")
            .is_some_and(|t| t.matches(MOCK_TOKEN)),
        "a refused connection must keep the local token"
    );
    assert!(dir.join("tokens.json").exists(), "token file must survive");
}

#[test]
fn signout_is_a_bodyless_delete_carrying_identity_headers_and_the_token_once() {
    let f = signed_in("logout-shape", &[Step::NoBody(204)]);
    signout(&f.http, &f.token, &f.store, Some(&f.key)).expect("signout must succeed");

    let request = &f.server.requests()[0];
    assert_eq!(request.method, "DELETE");
    for name in WIRE_HEADER_NAMES {
        assert!(
            request.header(name).is_some(),
            "{name} must be sent on the signout request too"
        );
    }
    assert_eq!(request.header("Accept"), Some("application/json"));
    assert!(!request.query_contains(MOCK_TOKEN), "no token in the query");
    assert!(
        !request.path_and_query.contains('?'),
        "signout sends no query at all: {}",
        request.path_and_query
    );

    // And no credential-shaped field anywhere on the wire.
    for forbidden in ["password", "passcode", "totp", "authcode", "pin="] {
        assert!(
            !request.path_and_query.to_lowercase().contains(forbidden)
                && !request
                    .headers
                    .iter()
                    .any(|(n, v)| format!("{n} {v}").to_lowercase().contains(forbidden)),
            "the client must never send a {forbidden} field"
        );
    }
}

#[test]
fn signing_out_when_nothing_is_stored_locally_still_reports_logged_out() {
    // A store with no token (app restarted after a partial logout): deleting an
    // absent token is not an error, and logout still reports LoggedOut.
    let dir = TempDir::new("logout-empty-store");
    let client_info = client(&dir);
    let server = MockPlex::start(Script::new().with(Route::Signout, &[Step::NoBody(204)]));
    let http = http_at(server.base(), &client_info);
    let store = FileTokenStore::with_consent(
        &dir.join("tokens.json"),
        &dir.join("token.json"),
        Consent::granted(),
    )
    .expect("consent granted by the test");
    assert!(store
        .load(&test_key())
        .expect("empty store loads")
        .is_none());

    let outcome = signout(
        &http,
        &SecretString::new(MOCK_TOKEN),
        &store,
        Some(&test_key()),
    )
    .expect("an already-empty local store must not break logout");
    assert_eq!(outcome, Signout::LoggedOut);
    assert!(!dir.join("tokens.json").exists());
}

#[test]
fn clear_token_on_auth_rejection_clears_only_for_auth_rejections() {
    // 401/422: the local token is deleted and the same error is handed back for
    // the UI ("signed out, new pin required"), never rewritten into something.
    for status in [401u16, 422u16] {
        let dir = TempDir::new(&format!("clear-{status}"));
        let path = dir.join("tokens.json");
        let store =
            FileTokenStore::with_consent(&path, &dir.join("token.json"), Consent::granted())
                .expect("consent granted");
        let key = test_key();
        store
            .store(&key, &SecretString::new(MOCK_TOKEN))
            .expect("fixture starts signed in");

        let returned =
            clear_token_on_auth_rejection(&store, Some(&key), CoreError::AuthRejected { status })
                .expect("passthrough must yield an error to surface");
        assert_eq!(returned, CoreError::AuthRejected { status });
        assert!(
            store.load(&key).expect("load").is_none(),
            "{status}: AuthRejected must delete the local token"
        );
        assert!(!path.exists(), "{status}: token file must be removed");
    }

    // Any other error is passed through untouched and the token is kept: a
    // network failure must not masquerade as an auth failure.
    for error in [
        CoreError::Transport,
        CoreError::HttpStatus { status: 500 },
        CoreError::ResponseShape,
        CoreError::PinExpired,
        CoreError::InvalidInput,
    ] {
        let dir = TempDir::new(&format!("clear-passthrough-{}", error.code()));
        let path = dir.join("tokens.json");
        let store =
            FileTokenStore::with_consent(&path, &dir.join("token.json"), Consent::granted())
                .expect("consent granted");
        let key = test_key();
        store
            .store(&key, &SecretString::new(MOCK_TOKEN))
            .expect("fixture starts signed in");

        let returned = clear_token_on_auth_rejection(&store, Some(&key), error.clone())
            .expect("passthrough result");
        assert_eq!(returned, error, "the error must be handed back unchanged");
        assert!(
            store
                .load(&key)
                .expect("load")
                .is_some_and(|t| t.matches(MOCK_TOKEN)),
            "{}: a non-auth error must keep the local token",
            error.code()
        );
        assert!(path.exists(), "{}: token file must survive", error.code());
    }
}

#[test]
fn a_stored_token_survives_a_failed_logout_and_is_still_usable_afterwards() {
    // 500 at logout -> the user is still signed in locally and the same token
    // still authenticates the next call (mock answers the account GET).
    let script = Script::new()
        .with(Route::Signout, &[Step::NoBody(500)])
        .with(Route::User, &[Step::Fixture("user.json")]);
    let f = signed_in_scripted("logout-survive", script);

    let error =
        signout(&f.http, &f.token, &f.store, Some(&f.key)).expect_err("500 logout must fail");
    assert_eq!(error, CoreError::HttpStatus { status: 500 });

    let reloaded = f
        .store
        .load(&f.key)
        .expect("the token must still be stored")
        .expect("Some(token)");
    let account = fetch_account(&f.http, &reloaded)
        .expect("the kept token must still work for an account call");
    assert!(account.has_token());
    assert!(account.token().is_some_and(|t| t.matches(MOCK_TOKEN)));
    assert_eq!(f.server.count_path(plex_core::api::USER_PATH), 1);
    assert_eq!(f.server.count_path(SIGNOUT_PATH), 1, "no logout retry loop");
}
