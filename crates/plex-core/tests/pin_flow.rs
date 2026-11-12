//! Behaviour tests for the PIN browser sign-in state machine, driven
//! against the **local mock Plex server** (`tests/mock/mod.rs`) whose answers
//! come from the hand-authored recordings in `tests/fixtures/`.
//!
//! These are mock tests, not Plex integration tests: they prove *our* state
//! machine (states, timings, no-blind-retry, header rules) and say nothing
//! about real plex.tv behaviour. Real-account acceptance is not yet verified against a real server.
//!
//! Pin ids are unique per test because the used-pin registry in
//! `plex_core::auth` is process-global and `cargo test` runs these tests as
//! threads of one process.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)] // product code keeps these lints on

use std::time::{Duration, Instant};

use plex_core::api::{fetch_account, signout, Signout, USER_PATH};
use plex_core::auth::{
    create_pin, poll_once, PairingSession, Phase, PAIRING_BUDGET, POLL_HTTP_TIMEOUT, POLL_INTERVAL,
};
use plex_core::client_info::ClientInfo;
use plex_core::error::CoreError;
use plex_core::http::Http;
use plex_core::secret::{SecretString, REDACTED};
use plex_core::tokens::{Consent, FileTokenStore, TokenKey, TokenStore};

mod common;
mod mock;

use common::{client, http_at, TempDir};
use mock::{
    create_pin_fixture, pending_forever, pin_fixture, MockPlex, Route, Script, MOCK_PIN_CODE,
    MOCK_TOKEN,
};

/// Arm the mock so `pin_id` answers **pending** on every poll: the state
/// machine must end the attempt via claim (the test arms a later step),
/// cancel or budget, never via a server answer.
fn arm_pending(server: &MockPlex, pin_id: &str) {
    server.set_script(Script::new().with(
        Route::PinPoll(pin_id.to_owned()),
        &[pending_forever(pin_id)],
    ));
}

fn pairing(
    server: &MockPlex,
    http: &Http,
    client_info: &ClientInfo,
    pin_id: &str,
    budget: Duration,
) -> PairingSession {
    arm_pending(server, pin_id);
    session_on(http, client_info, pin_id, budget)
}

/// A session whose poll answers are **whatever the test already scripted** on
/// its mock (the failure cases: 500 / broken JSON / a cut connection / 401 /
/// 422 / no answer / slow answer). Re-arming here would replace the scripted
/// failure with a pending answer, so this helper deliberately does not.
fn pairing_scripted(
    http: &Http,
    client_info: &ClientInfo,
    pin_id: &str,
    budget: Duration,
) -> PairingSession {
    session_on(http, client_info, pin_id, budget)
}

fn session_on(
    http: &Http,
    client_info: &ClientInfo,
    pin_id: &str,
    budget: Duration,
) -> PairingSession {
    PairingSession::with_pin_budget(
        http,
        client_info,
        plex_core::auth::Pin {
            id: pin_id.to_owned(),
            code: MOCK_PIN_CODE.to_owned(),
        },
        budget,
    )
    .unwrap_or_else(|e| panic!("pairing session for {pin_id} must start: {e}"))
}

fn pin_scripted(http: &Http, client_info: &ClientInfo, pin_id: &str) -> PairingSession {
    pairing_scripted(http, client_info, pin_id, PAIRING_BUDGET)
}

#[test]
fn claimed_flow_goes_pending_then_claimed_and_carries_the_token() {
    let dir = TempDir::new("pin-claim");
    let client_info = client(&dir);
    let pin_id = "pin-claim-1";
    let server = MockPlex::start(
        Script::new().with(Route::Pins, &[create_pin_fixture(pin_id, MOCK_PIN_CODE)]),
    );
    let http = http_at(server.base(), &client_info);

    // Step 1: `POST /api/v2/pins` in classic mode (real product call).
    let pin = create_pin(&http).expect("classic-mode pin creation must succeed");
    assert_eq!(pin.id, pin_id);
    assert_eq!(pin.code, MOCK_PIN_CODE);
    let created = server
        .requests()
        .iter()
        .find(|r| r.path_and_query == "/api/v2/pins")
        .expect("the mock must have seen the pin POST")
        .clone();
    assert_eq!(created.method, "POST");
    assert!(
        created.body.is_empty(),
        "classic mode sends no body: no strong pin, no JSON"
    );
    assert!(
        created.header("X-Plex-Token").is_none(),
        "pin creation happens before any token exists"
    );
    assert!(
        !created.query_contains("strong"),
        "classic mode only: this client never asks for a strong pin"
    );
    assert_eq!(
        created.header("Accept"),
        Some("application/json"),
        "identity headers must be on the pin POST too"
    );

    // Step 2: one pending poll, then the recorded claim answer.
    server.set_script(
        Script::new()
            .with(Route::Pins, &[create_pin_fixture(pin_id, MOCK_PIN_CODE)])
            .with(
                Route::PinPoll(pin_id.to_owned()),
                &[
                    pin_fixture(200, "pin_pending.json", pin_id),
                    pin_fixture(200, "pin_claimed.json", pin_id),
                ],
            ),
    );
    let mut session = PairingSession::with_pin(&http, &client_info, pin.clone())
        .expect("pairing must start for a fresh pin");
    assert_eq!(
        session.budget(),
        PAIRING_BUDGET,
        "the pairing budget is 120 s"
    );
    assert_eq!(session.pin(), &pin);
    assert_eq!(session.pairing_code(), MOCK_PIN_CODE);

    let url = session.claim_url().to_owned();
    assert!(url.starts_with("https://app.plex.tv/auth/#!?"), "{url}");
    assert!(
        url.contains(&format!("clientID={}", client_info.client_identifier)),
        "the claim URL must carry the client identifier: {url}"
    );
    assert!(url.contains(&format!("code={MOCK_PIN_CODE}")), "{url}");
    assert!(
        !url.contains(MOCK_TOKEN),
        "a token must never enter the claim URL"
    );

    let mut outcome = session
        .run(Duration::from_secs(8))
        .expect("the state machine must report an outcome");
    assert_eq!(outcome.phase, Phase::Claimed, "{outcome:?}");
    assert!(outcome.phase.is_terminal());
    assert_eq!(
        server.poll_count(pin_id),
        2,
        "one pending poll, then the poll that answered the claim"
    );
    assert_eq!(
        server.count_path("/api/v2/pins"),
        1,
        "this flow must not create extra pins"
    );
    assert_eq!(outcome.token_len(), MOCK_TOKEN.len());
    let token = outcome
        .take_token()
        .expect("a claimed outcome carries a token");
    assert_eq!(token.expose(), MOCK_TOKEN);
    assert_eq!(format!("{token:?}"), REDACTED);
    assert!(
        !format!("{outcome:?}").contains(MOCK_TOKEN),
        "Debug leaked the token"
    );
    assert!(
        server
            .requests()
            .iter()
            .all(|r| r.header("X-Plex-Token").is_none()),
        "pairing itself never sends a token header"
    );
    server.shutdown();
}

#[test]
fn a_claimed_pin_token_is_stored_reloaded_used_and_logged_out() {
    // The whole acceptance clause in one test: the **claimed** token (taken
    // from the state machine, not handed to the store as a fixture string) is
    // stored, reloaded from disk by a later process shape, used for an
    // authenticated account call, and finally deleted by logout.
    let dir = TempDir::new("pin-store");
    let client_info = client(&dir);
    let pin_id = "pin-store-1";
    let server = MockPlex::start(
        Script::new()
            .with(Route::Pins, &[create_pin_fixture(pin_id, MOCK_PIN_CODE)])
            .with(
                Route::PinPoll(pin_id.to_owned()),
                &[
                    pin_fixture(200, "pin_pending.json", pin_id),
                    pin_fixture(200, "pin_claimed.json", pin_id),
                ],
            )
            .with(Route::User, &[mock::Step::Fixture("user.json")])
            .with(Route::Signout, &[mock::Step::NoBody(204)]),
    );
    let http = http_at(server.base(), &client_info);

    // 1. claim it through the real state machine.
    let pin = create_pin(&http).expect("classic-mode pin creation must succeed");
    assert_eq!(pin.id, pin_id);
    let mut session = PairingSession::with_pin(&http, &client_info, pin.clone())
        .expect("pairing must start for a fresh pin");
    let mut outcome = session
        .run(Duration::from_secs(8))
        .expect("the state machine must report an outcome");
    assert_eq!(outcome.phase, Phase::Claimed, "{outcome:?}");
    let claimed = outcome
        .take_token()
        .expect("a claimed outcome carries the token");
    assert_eq!(claimed.expose(), MOCK_TOKEN);

    // 2. hand that secret to the real store (in-test consent, plaintext file).
    let path = dir.join("tokens.json");
    let store = FileTokenStore::with_consent(&path, &dir.join("token.json"), Consent::granted())
        .expect("the test grants the plaintext-file consent");
    let key = TokenKey::plex("deadbeef-dead-4ead-beef-deadbeefdead").expect("valid test uuid");
    store
        .store(&key, &claimed)
        .expect("the claimed token must be storable");
    assert!(path.exists(), "the claimed token must be on disk");

    // 3. reload it: the file is the only memory a restarted app has.
    let reloaded = store
        .load(&key)
        .expect("load must not error")
        .expect("the stored token must reload");
    assert_eq!(reloaded.expose(), MOCK_TOKEN, "reloaded != claimed");
    assert_eq!(format!("{reloaded:?}"), REDACTED);

    // 4. the reloaded token authenticates a real call, header-only.
    let account =
        fetch_account(&http, &reloaded).expect("the reloaded token must work for the account call");
    assert!(account.has_token());
    assert!(account.token().is_some_and(|t| t.matches(MOCK_TOKEN)));
    assert_eq!(server.count_path(USER_PATH), 1);
    let pairing_requests = server.requests();
    let account_request = pairing_requests
        .iter()
        .find(|r| r.path_and_query == USER_PATH)
        .expect("the mock must have seen the account call");
    assert_eq!(account_request.header("X-Plex-Token"), Some(MOCK_TOKEN));
    assert!(
        !account_request.query_contains(MOCK_TOKEN),
        "no token in query"
    );

    // 5. logout closes the loop: the same token is gone locally.
    let signout_outcome =
        signout(&http, &reloaded, &store, Some(&key)).expect("204 signout must succeed");
    assert_eq!(signout_outcome, Signout::LoggedOut);
    assert!(!path.exists(), "logout must delete the claimed token file");
    assert!(store.load(&key).expect("load after logout").is_none());
    assert!(store
        .stored_at(&key)
        .expect("stored_at after logout")
        .is_none());

    // A fresh store built on the same path sees nothing either: the deletion
    // is on disk, not just in this process.
    let reopened = FileTokenStore::with_consent(&path, &dir.join("token.json"), Consent::granted())
        .expect("consent still granted");
    assert!(reopened
        .load(&key)
        .expect("reopened store must load")
        .is_none());
    let final_requests = server.requests();
    assert!(
        final_requests
            .iter()
            .all(|r| r.header("X-Plex-Token").is_none()
                || r.path_and_query == USER_PATH
                || r.path_and_query == plex_core::api::SIGNOUT_PATH),
        "pairing must not have sent a token"
    );
    let _: &SecretString = &reloaded; // keeps the secret alive to the last assert
    server.shutdown();
}

#[test]
fn any_poll_failure_expires_the_pin_and_a_new_pin_is_required() {
    for (case, pin_id, step) in [
        (
            "http-500",
            "pin-expire-500",
            mock::Step::Respond(500, r#"{"errors":["mock: server fault"]}"#),
        ),
        (
            "malformed-json",
            "pin-expire-shape",
            mock::Step::Respond(200, "not json at all"),
        ),
        (
            "cut-connection",
            "pin-expire-cut",
            mock::Step::CutConnection,
        ),
    ] {
        let dir = TempDir::new("pin-expire");
        let client_info = client(&dir);
        let server = MockPlex::start(
            Script::new()
                .with(Route::Pins, &[create_pin_fixture(pin_id, MOCK_PIN_CODE)])
                .with(Route::PinPoll(pin_id.to_owned()), &[step]),
        );
        let http = http_at(server.base(), &client_info);
        let mut session = pin_scripted(&http, &client_info, pin_id);
        let outcome = session
            .run(Duration::from_secs(8))
            .expect("a poll failure must produce an outcome");
        assert_eq!(outcome.phase, Phase::Expired, "{case}: {outcome:?}");
        assert_eq!(
            outcome.token_len(),
            0,
            "{case}: an expired pin has no token"
        );
        assert!(
            matches!(outcome.phase.error(), Ok(Some(CoreError::PinExpired))),
            "{case}: the typed error must be PinExpired"
        );
        assert_eq!(
            server.poll_count(pin_id),
            1,
            "{case}: a failed poll must not be retried on the same pin"
        );

        // The spent pin cannot be polled again: continuing requires a new pin.
        let reused = PairingSession::with_pin(
            &http,
            &client_info,
            plex_core::auth::Pin {
                id: pin_id.to_owned(),
                code: MOCK_PIN_CODE.to_owned(),
            },
        );
        assert!(
            matches!(reused, Err(CoreError::PinReuseRefused)),
            "{case}: re-using a failed pin must be refused, got {reused:?}"
        );
        server.shutdown();
    }
}

#[test]
fn a_new_pin_after_expiry_claims_with_the_recorded_answer() {
    let dir = TempDir::new("pin-newpin");
    let client_info = client(&dir);
    let first_id = "pin-new-1";
    let server = MockPlex::start(
        Script::new()
            .with(Route::Pins, &[create_pin_fixture(first_id, MOCK_PIN_CODE)])
            .with(
                Route::PinPoll(first_id.to_owned()),
                &[mock::Step::Respond(
                    500,
                    r#"{"errors":["mock: server fault"]}"#,
                )],
            ),
    );
    let http = http_at(server.base(), &client_info);
    let mut first = pin_scripted(&http, &client_info, first_id);
    assert_eq!(
        first.run(Duration::from_secs(8)).expect("outcome").phase,
        Phase::Expired
    );

    // A new pin (recorded second-pin answers) does work.
    let second_id = "pin-new-2";
    server.set_script(
        Script::new()
            .with(Route::Pins, &[create_pin_fixture(second_id, "QM2L")])
            .with(
                Route::PinPoll(second_id.to_owned()),
                &[pin_fixture(200, "pin_claimed.json", second_id)],
            ),
    );
    let created = create_pin(&http).expect("a new pin must be creatable");
    assert_eq!(created.id, second_id);
    let mut again = PairingSession::with_pin(&http, &client_info, created)
        .expect("a fresh pin must be pollable");
    let mut outcome = again.run(Duration::from_secs(8)).expect("outcome");
    assert_eq!(outcome.phase, Phase::Claimed, "{outcome:?}");
    assert_eq!(
        outcome.take_token().expect("token").expose(),
        MOCK_TOKEN,
        "the new pin's token is the recorded one"
    );
    assert_eq!(server.poll_count(second_id), 1);
    server.shutdown();
}

#[test]
fn http_401_and_422_are_auth_rejections_with_no_blind_retry() {
    for (status, pin_id) in [(401u16, "pin-rej-401"), (422u16, "pin-rej-422")] {
        let dir = TempDir::new("pin-reject");
        let client_info = client(&dir);
        let body = if status == 401 {
            r#"{"errors":["mock: invalid credentials"]}"#
        } else {
            r#"{"errors":["mock: already claimed"]}"#
        };
        let server = MockPlex::start(
            Script::new()
                .with(Route::Pins, &[create_pin_fixture(pin_id, MOCK_PIN_CODE)])
                .with(
                    Route::PinPoll(pin_id.to_owned()),
                    &[mock::Step::Respond(status, body)],
                ),
        );
        let http = http_at(server.base(), &client_info);
        let mut session = pin_scripted(&http, &client_info, pin_id);
        let outcome = session.run(Duration::from_secs(8)).expect("outcome");
        assert_eq!(outcome.phase, Phase::Rejected { status }, "{outcome:?}");
        assert_eq!(outcome.phase.as_str(), "rejected");
        assert!(
            matches!(outcome.phase.error(), Ok(Some(CoreError::AuthRejected { status: seen })) if seen == status),
            "the typed error must be AuthRejected({status})"
        );
        assert_eq!(
            server.poll_count(pin_id),
            1,
            "an auth rejection must not be retried blindly on the same pin"
        );
        assert_eq!(server.count_path(&format!("/api/v2/pins/{pin_id}")), 1);
        server.shutdown();
    }
}

#[test]
fn cancel_stops_polling_within_one_second_and_drops_the_pin() {
    let dir = TempDir::new("pin-cancel");
    let client_info = client(&dir);
    let pin_id = "pin-cancel-1";
    let server = MockPlex::start(Script::new());
    let http = http_at(server.base(), &client_info);
    let mut session = pairing(&server, &http, &client_info, pin_id, PAIRING_BUDGET);

    // Reach the pending phase first, so cancellation is a real in-flight state.
    let phase = session
        .next_phase(Duration::from_secs(6))
        .expect("phase events must be delivered")
        .expect("the pending phase must be reported");
    assert_eq!(phase, Phase::Pending);
    assert!(
        server.poll_count(pin_id) >= 1,
        "the pin must actually be polled"
    );
    assert!(!plex_core::auth::pin_is_retired(pin_id));

    let started = Instant::now();
    let outcome = session.cancel().expect("cancel must return an outcome");
    let elapsed = started.elapsed();
    assert_eq!(outcome.phase, Phase::Cancelled, "{outcome:?}");
    assert!(
        elapsed < POLL_INTERVAL,
        "cancel took {elapsed:?}, must be < 1 s"
    );
    assert!(
        matches!(outcome.phase.error(), Ok(Some(CoreError::PinCancelled))),
        "the typed error must be PinCancelled"
    );
    let polls_at_cancel = server.poll_count(pin_id);

    // Polling really stopped: a further 1 s tick adds no requests.
    std::thread::sleep(POLL_INTERVAL + Duration::from_millis(700));
    assert_eq!(
        server.poll_count(pin_id),
        polls_at_cancel,
        "cancel must stop the poll loop, not merely hide its results"
    );
    assert!(
        plex_core::auth::pin_is_retired(pin_id),
        "cancel must drop the pin"
    );
    server.shutdown();
}

#[test]
fn dropping_a_session_stops_polling_and_retires_the_pin() {
    let dir = TempDir::new("pin-drop");
    let client_info = client(&dir);
    let pin_id = "pin-drop-1";
    let server = MockPlex::start(Script::new());
    let http = http_at(server.base(), &client_info);
    {
        let session = pairing(&server, &http, &client_info, pin_id, PAIRING_BUDGET);
        let _ = session;
    }
    let polls = server.poll_count(pin_id);
    std::thread::sleep(POLL_INTERVAL + Duration::from_millis(700));
    assert_eq!(
        server.poll_count(pin_id),
        polls,
        "no poller may outlive its session"
    );
    assert!(plex_core::auth::pin_is_retired(pin_id));
    server.shutdown();
}

#[test]
fn elapsed_budget_reports_timeout_after_the_1s_tick() {
    let dir = TempDir::new("pin-timeout");
    let client_info = client(&dir);
    let pin_id = "pin-timeout-1";
    let secs = 3u64;
    let server = MockPlex::start(Script::new());
    let http = http_at(server.base(), &client_info);
    let mut session = pairing(
        &server,
        &http,
        &client_info,
        pin_id,
        Duration::from_secs(secs),
    );
    assert_eq!(session.budget(), Duration::from_secs(secs));
    let started = Instant::now();
    let outcome = session
        .run(Duration::from_secs(secs + 6))
        .expect("a pending pin must end on its budget");
    let elapsed = started.elapsed();
    assert_eq!(outcome.phase, Phase::Timeout { secs }, "{outcome:?}");
    assert_eq!(outcome.phase.as_str(), "timeout");
    assert_eq!(outcome.token_len(), 0);
    assert!(
        matches!(
            outcome.phase.error(),
            Ok(Some(CoreError::PinTimeout { secs: seen })) if seen == secs
        ),
        "the typed error must carry the elapsed budget"
    );
    assert!(
        elapsed >= Duration::from_secs(secs),
        "budget fired early: {elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_secs(secs) + POLL_INTERVAL + Duration::from_millis(700),
        "budget overshoot must stay inside the 1 s tick: {elapsed:?}"
    );
    // A 1 s tick over `secs` whole seconds means `secs` polls, +/- the tick.
    let polls = server.poll_count(pin_id);
    assert!(
        (secs as usize..=secs as usize + 1).contains(&polls),
        "poll count {polls} must show a 1 s tick over a {secs} s budget"
    );
    server.shutdown();
}

#[test]
fn budgets_outside_the_classic_pin_window_are_refused_before_any_request() {
    let dir = TempDir::new("pin-budget");
    let client_info = client(&dir);
    let server = MockPlex::start(Script::new());
    let http = http_at(server.base(), &client_info);
    let empty_pin = |id: &str| plex_core::auth::Pin {
        id: id.to_owned(),
        code: MOCK_PIN_CODE.to_owned(),
    };
    let zero = PairingSession::with_pin_budget(
        &http,
        &client_info,
        empty_pin("pin-budget-zero"),
        Duration::ZERO,
    );
    assert!(
        matches!(zero, Err(CoreError::InvalidInput)),
        "a zero budget must be refused, got {zero:?}"
    );
    let too_long = PairingSession::with_pin_budget(
        &http,
        &client_info,
        empty_pin("pin-budget-long"),
        PAIRING_BUDGET + Duration::from_millis(1),
    );
    assert!(
        matches!(too_long, Err(CoreError::InvalidInput)),
        "polling past the pin lifetime must be refused, got {too_long:?}"
    );
    let no_id = PairingSession::with_pin(
        &http,
        &client_info,
        plex_core::auth::Pin {
            id: String::new(),
            code: MOCK_PIN_CODE.to_owned(),
        },
    );
    assert!(
        matches!(no_id, Err(CoreError::ResponseShape)),
        "a pin without an id is not a pin, got {no_id:?}"
    );
    assert!(
        server.requests().is_empty(),
        "a refused attempt must not touch the server"
    );
    server.shutdown();
}

#[test]
fn a_poll_that_never_answers_expires_on_the_poll_http_bound() {
    let dir = TempDir::new("pin-noreply");
    let client_info = client(&dir);
    let pin_id = "pin-noreply-1";
    let server = MockPlex::start(
        Script::new()
            .with(Route::Pins, &[create_pin_fixture(pin_id, MOCK_PIN_CODE)])
            .with(Route::PinPoll(pin_id.to_owned()), &[mock::Step::NoReply]),
    );
    let http = http_at(server.base(), &client_info);
    let mut session = pin_scripted(&http, &client_info, pin_id);
    let started = Instant::now();
    let outcome = session
        .run(Duration::from_secs(20))
        .expect("a stuck poll must end as a failure, not hang the session");
    let elapsed = started.elapsed();
    assert_eq!(outcome.phase, Phase::Expired, "{outcome:?}");
    assert!(
        elapsed < POLL_HTTP_TIMEOUT + Duration::from_millis(900),
        "the poll's own HTTP bound ({} ms) must end it, took {elapsed:?}",
        POLL_HTTP_TIMEOUT.as_millis()
    );
    assert_eq!(server.poll_count(pin_id), 1);
    server.shutdown();
}

#[test]
fn a_slow_answer_is_a_poll_failure_not_a_silent_wait() {
    let dir = TempDir::new("pin-slow");
    let client_info = client(&dir);
    let pin_id = "pin-slow-1";
    let server = MockPlex::start(
        Script::new()
            .with(Route::Pins, &[create_pin_fixture(pin_id, MOCK_PIN_CODE)])
            .with(
                Route::PinPoll(pin_id.to_owned()),
                &[mock::Step::Slow(2_000, "pin_pending.json")],
            ),
    );
    let http = http_at(server.base(), &client_info);
    let mut session = pin_scripted(&http, &client_info, pin_id);
    let outcome = session
        .run(Duration::from_secs(20))
        .expect("a slow answer must still end the attempt");
    assert_eq!(outcome.phase, Phase::Expired, "{outcome:?}");
    assert_eq!(
        server.poll_count(pin_id),
        1,
        "a poll that timed out must not be retried on the same pin"
    );
    server.shutdown();
}

#[test]
fn poll_once_reports_typed_errors_and_only_a_secret_token() {
    let dir = TempDir::new("pin-poll");
    let client_info = client(&dir);
    let pin_id = "pin-poll-1";
    let pin = plex_core::auth::Pin {
        id: pin_id.to_owned(),
        code: MOCK_PIN_CODE.to_owned(),
    };
    let server = MockPlex::start(Script::new());
    let http = http_at(server.base(), &client_info);

    // A pending answer (no authToken) is Ok(None): nothing to leak.
    server.set_script(Script::new().with(
        Route::PinPoll(pin_id.to_owned()),
        &[pin_fixture(200, "pin_pending.json", pin_id)],
    ));
    assert!(poll_once(&http, &pin).expect("pending poll").is_none());

    // A 500 answer is a typed error that carries no body and no token, even
    // though the same mock would have handed out a token on a claim answer.
    server.set_script(Script::new().with(
        Route::PinPoll(pin_id.to_owned()),
        &[pin_fixture(500, "pin_claimed.json", pin_id)],
    ));
    let error = poll_once(&http, &pin).expect_err("a 500 poll must fail");
    assert!(
        matches!(error, CoreError::HttpStatus { status: 500 }),
        "{error:?}"
    );
    assert!(
        !error.to_string().contains(MOCK_TOKEN),
        "Display leaked a token"
    );
    assert!(
        !format!("{error:?}").contains(MOCK_TOKEN),
        "Debug leaked a token"
    );

    // A 200 poll body that is not JSON at all is a shape error, which the
    // state machine turns into `expired` (see the failure cases above).
    server.set_script(Script::new().with(
        Route::PinPoll(pin_id.to_owned()),
        &[mock::Step::Respond(200, "not json at all")],
    ));
    let malformed = poll_once(&http, &pin).expect_err("a 200 body that is not JSON must fail");
    assert!(
        matches!(malformed, CoreError::ResponseShape),
        "{malformed:?}"
    );

    // A 200 answer that **is** valid JSON but carries no `authToken` is a
    // pending pin, not a poll failure: the recorded `pin_pending.json` shape is
    // exactly this, and polling it is what the 1 s tick is for.
    server.set_script(Script::new().with(
        Route::PinPoll(pin_id.to_owned()),
        &[mock::Step::Respond(
            200,
            r#"{"id":"pin-poll-1","code":"XPGK"}"#,
        )],
    ));
    assert!(
        poll_once(&http, &pin)
            .expect("valid JSON without authToken")
            .is_none(),
        "a poll answer with no authToken is pending, not an error"
    );

    // A claim answer yields the token, and printing it does not.
    server.set_script(Script::new().with(
        Route::PinPoll(pin_id.to_owned()),
        &[pin_fixture(200, "pin_claimed.json", pin_id)],
    ));
    let token = poll_once(&http, &pin)
        .expect("claimed poll")
        .expect("a claimed poll carries a token");
    assert_eq!(token.expose(), MOCK_TOKEN);
    assert_eq!(format!("{token:?}"), REDACTED);
    assert!(!format!("{pin:?}").contains(MOCK_TOKEN));
    server.shutdown();
}

#[test]
fn poll_and_budget_constants_match_the_documented_pairing_rules() {
    assert_eq!(POLL_INTERVAL, Duration::from_secs(1));
    assert_eq!(PAIRING_BUDGET, Duration::from_secs(120));
    assert!(POLL_HTTP_TIMEOUT < POLL_INTERVAL * 3);
    assert_eq!(plex_core::auth::PINS_PATH, "/api/v2/pins");
    assert_eq!(
        plex_core::auth::CLAIM_URL_BASE,
        "https://app.plex.tv/auth/#!"
    );
    assert_eq!(Phase::Pending.as_str(), "pending");
    assert_eq!(Phase::Claimed.as_str(), "claimed");
    assert!(!Phase::Pending.is_terminal());
    for phase in [
        Phase::Expired,
        Phase::Cancelled,
        Phase::Timeout { secs: 1 },
        Phase::Rejected { status: 401 },
    ] {
        assert!(phase.is_terminal(), "{}", phase.as_str());
        assert!(phase.error().map(|e| e.is_some()).unwrap_or(false));
    }
    assert!(Phase::Claimed.error().map(|e| e.is_none()).unwrap_or(false));
}

#[test]
fn numeric_pin_id_from_plex_tv_is_accepted() {
    // plex.tv answers `{"id": 1723818697, "code": "GBDB", ...}`: a JSON number.
    let dir = TempDir::new("pin-numeric");
    let client_info = client(&dir);
    let server = MockPlex::start(Script::new().with(
        Route::Pins,
        &[create_pin_fixture("1723818690", MOCK_PIN_CODE)],
    ));
    let http = http_at(server.base(), &client_info);
    let pin = create_pin(&http).expect("a numeric pin id must parse");
    assert_eq!(pin.id, "1723818690");
    assert_eq!(pin.code, MOCK_PIN_CODE);
}
