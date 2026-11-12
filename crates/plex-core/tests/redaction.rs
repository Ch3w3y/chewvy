//! Redaction proof: **token strings never appear in log output or in
//! rendered error text**, asserted on *captured* log records.
//!
//! This is the executable half of the sign-in requirement that
//! `src/log.rs`, `src/tokens.rs` and `tests/token_storage.rs` point at. It
//! installs the product capture backend (`plex_core::log::init_capture`),
//! drives the real code paths that log (pin claim, poll failure, token-file
//! mode self-heal) plus the paths that hold a token (store, reload,
//! authenticated call, logout, 401/422 clearing), and then asserts on the
//! buffer: no captured record contains the mock token literal.
//!
//! Mock tests, not Plex integration tests: they prove what *our* code renders,
//! with the same distinctive token literal the fixtures carry. What plex.tv
//! itself echoes back is not yet verified against a real server.
//!
//! ## Why the assertions are non-vacuous
//!
//! * the first test *calibrates the detector*: it proves a token written to
//!   `log::warn!` really lands in the buffer, so the later "no line contains
//!   it" assertions are a real measurement and not an empty buffer;
//! * each measured test looks only at the records appended **during its own
//!   flow** (windowed by `captured().len()` before/after), and every test in
//!   this binary takes the process-wide `SERIAL` lock for its whole body, so
//!   no other test of this binary can interleave records into the window and
//!   no test can erase another one's evidence with `log::clear()`;
//! * the product flows are asserted to have *produced* captured records
//!   (claim/poll/chmod warnings), so the window is never empty.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)] // product code keeps these lints on

use std::collections::BTreeMap;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use plex_core::api::{clear_token_on_auth_rejection, fetch_account, signout, Signout};
use plex_core::auth::{create_pin, PairingSession, Phase};
use plex_core::error::{CoreError, StorageError};
// `plex_core::log` would shadow the extern `log` crate name (whose macros the
// calibration test needs), so the product capture module is imported renamed.
use plex_core::log as logging;
use plex_core::secret::{SecretString, REDACTED};
use plex_core::tokens::{Consent, FileTokenStore, TokenKey, TokenStore};

mod common;
mod mock;

use common::{client, http_at, TempDir};
use mock::{
    create_pin_fixture, pin_fixture, MockPlex, Route, Script, Step, MOCK_PIN_CODE, MOCK_TOKEN,
};

/// Tests of this binary are serialized (cargo runs them as threads of one
/// process). Without this, one test's calibration leak could land inside
/// another test's capture window and the proof would be meaningless.
static SERIAL: OnceLock<Mutex<()>> = OnceLock::new();

fn serial() -> MutexGuard<'static, ()> {
    SERIAL
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Install the product capture backend (idempotent) and refuse to let any test
/// in this file claim redaction from a buffer someone else owns.
fn require_capture() {
    let _ = logging::init_capture();
    assert!(
        logging::capture_active(),
        "redaction assertions need plex_core::log's capture backend to be the active log sink"
    );
}

/// Records appended to the capture buffer while a flow runs.
struct Window {
    start: usize,
}

fn window() -> Window {
    Window {
        start: logging::captured().len(),
    }
}

impl Window {
    /// The records this flow produced.
    fn lines(&self) -> Vec<String> {
        logging::captured().into_iter().skip(self.start).collect()
    }

    /// This flow's records that contain the token literal (must be empty).
    fn leaks(&self) -> Vec<String> {
        self.lines()
            .into_iter()
            .filter(|line| line.contains(MOCK_TOKEN))
            .collect()
    }
}

#[test]
fn the_capture_backend_catches_a_token_if_one_is_ever_logged() {
    // Detector calibration: without this test the redaction assertions could
    // pass on a buffer that never sees product log calls at all.
    let _serial = serial();
    require_capture();

    let before = window();
    // Deliberate synthetic leak: exactly the shape of the mistake the
    // acceptance clause forbids.
    ::log::warn!("synthetic calibration record for the redaction detector: {MOCK_TOKEN}");
    let lines = before.lines();
    assert!(
        lines.iter().any(|line| line.contains(MOCK_TOKEN)),
        "the capture backend must see a token that is actually logged, got {lines:?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("WARN")),
        "captured records must carry the level, got {lines:?}"
    );

    // And the product's own helper for rendering foreign text scrubs it.
    let scrubbed = logging::scrub(&format!("upstream said: {MOCK_TOKEN}"), &[MOCK_TOKEN]);
    assert!(!scrubbed.contains(MOCK_TOKEN), "{scrubbed}");
    assert!(scrubbed.contains(REDACTED), "{scrubbed}");
}

/// Drive the full signed-in lifecycle while the capture backend is active:
/// create pin -> pair (pending -> claimed) -> store -> reload -> authenticated
/// account call -> logout.
#[test]
fn a_signed_in_flow_leaves_no_token_in_captured_log_output() {
    let _serial = serial();
    require_capture();
    let dir = TempDir::new("redaction-flow");
    let client_info = client(&dir);
    let pin_id = "redaction-flow-1";
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
            .with(Route::User, &[Step::Fixture("user.json")])
            .with(Route::Signout, &[Step::NoBody(204)]),
    );
    let http = http_at(server.base(), &client_info);
    let path = dir.join("token.json");

    let capture = window();

    let pin = create_pin(&http).expect("pin creation must succeed");
    let mut session = PairingSession::with_pin(&http, &client_info, pin)
        .expect("pairing must start for a fresh pin");
    // The UI-visible strings (pairing code, claim URL) are not secrets and are
    // produced here; only the token literal is under test.
    let _code = session.pairing_code().to_owned();
    let _url = session.claim_url().to_owned();
    let mut outcome = session
        .run(Duration::from_secs(8))
        .expect("the state machine must report an outcome");
    assert_eq!(outcome.phase, Phase::Claimed, "{outcome:?}");
    let token = outcome
        .take_token()
        .expect("a claimed outcome carries a token");

    let store =
        FileTokenStore::with_consent(&path, &dir.join("legacy-token.json"), Consent::granted())
            .expect("the test grants the plaintext-file consent");
    let key = TokenKey::plex("deadbeef-dead-4ead-beef-deadbeefdead").expect("valid test uuid");
    store
        .store(&key, &token)
        .expect("store must accept the claimed token");

    // Force the one product log call that touches the token *file*: a mode that
    // something outside this client widened. `read_record` logs the mode and
    // the path (never the token) and tightens it back to 0600.
    #[cfg(unix)]
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
        .expect("test must be able to loosen the file mode");
    let reloaded = store
        .load(&key)
        .expect("load must not error")
        .expect("the token must reload");
    #[cfg(unix)]
    assert_eq!(
        std::fs::metadata(&path)
            .expect("token file must be stat-able")
            .permissions()
            .mode()
            & 0o777,
        0o600,
        "the mode self-heal must have run (that is the log line under test)"
    );

    let account = fetch_account(&http, &reloaded).expect("the account call must succeed");
    assert!(account.has_token(), "the mock account carries a token");
    let signout_outcome =
        signout(&http, &reloaded, &store, Some(&key)).expect("204 signout must succeed");
    assert_eq!(signout_outcome, Signout::LoggedOut);

    let lines = capture.lines();
    assert!(
        !lines.is_empty(),
        "this flow must produce captured log records, otherwise the assertion proves nothing"
    );
    assert!(
        lines.iter().any(|line| line.contains("pin claimed")),
        "the claimed-pin log record must have been captured, got {lines:?}"
    );
    #[cfg(unix)]
    assert!(
        lines
            .iter()
            .any(|line| line.contains("token file mode was 644")),
        "the mode-tightening log record must have been captured, got {lines:?}"
    );
    assert!(
        capture.leaks().is_empty(),
        "a token literal reached captured log output: {:?}",
        capture.leaks()
    );
    // The claimed-pin record carries only a length, never the value.
    assert!(
        !lines.iter().any(|line| line.contains("token len 0")),
        "the claimed record must report a real length, got {lines:?}"
    );
    server.shutdown();
}

/// The failure states (poll fault -> expired, 401/422 -> cleared, 500 ->
/// retryable logout, consent-declined store) while the token is live in the
/// store: their log records and `Display`/`Debug` text must stay token-free.
#[test]
fn failure_paths_log_and_render_only_error_codes_never_the_token() {
    let _serial = serial();
    require_capture();
    let dir = TempDir::new("redaction-failures");
    let client_info = client(&dir);
    let pin_id = "redaction-failures-1";
    let server = MockPlex::start(
        Script::new()
            .with(Route::Pins, &[create_pin_fixture(pin_id, MOCK_PIN_CODE)])
            .with(
                Route::PinPoll(pin_id.to_owned()),
                &[Step::Respond(500, r#"{"errors":["mock: server fault"]}"#)],
            )
            .with(
                Route::User,
                &[Step::Respond(401, r#"{"errors":["mock: rejected"]}"#)],
            )
            .with(Route::Signout, &[Step::NoBody(500)]),
    );
    let http = http_at(server.base(), &client_info);
    let path = dir.join("token.json");
    let store =
        FileTokenStore::with_consent(&path, &dir.join("legacy-token.json"), Consent::granted())
            .expect("the test grants the plaintext-file consent");
    // Seed the store with the very literal the assertion looks for, so a leak
    // has something real to leak and "cleared" is an observable fact.
    let key = TokenKey::plex("deadbeef-dead-4ead-beef-deadbeefdead").expect("valid test uuid");
    let seeded = SecretString::new(MOCK_TOKEN);
    store
        .store(&key, &seeded)
        .expect("the seed token must be storable");

    let capture = window();

    // 1. A faulting poll expires the pin (new pin required, never retried).
    let pin = create_pin(&http).expect("pin creation must succeed");
    let mut session = PairingSession::with_pin(&http, &client_info, pin)
        .expect("pairing must start for a fresh pin");
    let outcome = session
        .run(Duration::from_secs(6))
        .expect("the state machine must report an outcome");
    assert_eq!(outcome.phase, Phase::Expired, "{outcome:?}");
    let expired = outcome
        .phase
        .error()
        .expect("phase error")
        .expect("expired must map to an error");
    assert_eq!(expired, CoreError::PinExpired);

    // 2. A 500 logout keeps the local token (retryable) and errors.
    let signout_error =
        signout(&http, &seeded, &store, Some(&key)).expect_err("500 logout must error");
    assert_eq!(signout_error, CoreError::HttpStatus { status: 500 });
    assert!(
        store
            .load(&key)
            .expect("load after a failed logout")
            .is_some_and(|t| t.matches(MOCK_TOKEN)),
        "a failed logout must keep the local token"
    );

    // 3. A 401 on an authenticated call clears the local token, no blind retry.
    let account_error = fetch_account(&http, &seeded).expect_err("401 must be an error");
    let cleared = clear_token_on_auth_rejection(&store, Some(&key), account_error)
        .expect("clearing returns the error");
    assert_eq!(cleared, CoreError::AuthRejected { status: 401 });
    assert!(
        store.load(&key).expect("load after clearing").is_none(),
        "401 must clear the local token"
    );
    assert!(!path.exists(), "clearing must delete the token file");

    // 4. No consent -> no file store, and the refusal text is token-free.
    let declined_path = dir.join("declined.json");
    let consent_error = FileTokenStore::with_consent(
        &declined_path,
        &dir.join("declined-legacy.json"),
        Consent::DECLINED,
    )
    .expect_err("the file store requires explicit consent");
    assert_eq!(consent_error, CoreError::ConsentRequired);
    assert!(
        !declined_path.exists(),
        "no consent means nothing is written"
    );

    let lines = capture.lines();
    assert!(
        !lines.is_empty(),
        "the failure flows must produce captured log records"
    );
    assert!(
        lines.iter().any(|line| line.contains("pin poll failed")),
        "the poll-failure log record must have been captured, got {lines:?}"
    );
    assert!(
        capture.leaks().is_empty(),
        "a token literal reached captured log output on a failure path: {:?}",
        capture.leaks()
    );
    server.shutdown();
}

/// The other half of the clause: **error formatting**. Every `CoreError`
/// variant, the redacted structs a UI would print, and the scrubbing helper
/// that renders foreign error text (ureq/io `Display`, which this client does
/// not control) are all asserted against the same token literal.
#[test]
fn rendered_error_and_secret_text_never_shows_the_token() {
    let _serial = serial();
    let dir = TempDir::new("redaction-render");
    let client_info = client(&dir);
    let server = MockPlex::start(Script::new().with(Route::User, &[Step::Fixture("user.json")]));
    let http = http_at(server.base(), &client_info);
    let token = SecretString::new(MOCK_TOKEN);
    let account = fetch_account(&http, &token).expect("the account call must succeed");
    let path = dir.join("token.json");
    let store =
        FileTokenStore::with_consent(&path, &dir.join("legacy-token.json"), Consent::granted())
            .expect("the test grants the plaintext-file consent");
    store
        .store(
            &TokenKey::plex("deadbeef-dead-4ead-beef-deadbeefdead").expect("valid test uuid"),
            &token,
        )
        .expect("the token must be storable");

    // Every CoreError variant the client can produce, rendered both ways.
    let errors = [
        CoreError::Identity,
        CoreError::Transport,
        CoreError::HttpStatus { status: 500 },
        CoreError::ResponseShape,
        CoreError::PinExpired,
        CoreError::PinCancelled,
        CoreError::AuthRejected { status: 401 },
        CoreError::AuthRejected { status: 422 },
        CoreError::PinTimeout { secs: 120 },
        CoreError::PinReuseRefused,
        CoreError::Storage(StorageError::Dbus),
        CoreError::Storage(StorageError::Locked),
        CoreError::Storage(StorageError::NotFound),
        CoreError::Storage(StorageError::Io),
        CoreError::Storage(StorageError::Payload),
        CoreError::Storage(StorageError::NoBackend),
        CoreError::Storage(StorageError::NoConfigHome),
        CoreError::Storage(StorageError::UnsafeDirectory),
        CoreError::ConsentRequired,
        CoreError::SecretServiceUnavailable,
        CoreError::InvalidInput,
    ];
    let mut rendered: Vec<(String, String)> = Vec::new();
    for error in &errors {
        rendered.push((format!("Display {error:?}"), error.to_string()));
        rendered.push((format!("Debug {error:?}"), format!("{error:?}")));
        // The helper a UI/CLI would use to show the error chain.
        rendered.push((format!("describe {error:?}"), logging::describe(error, &[])));
    }
    for (label, value) in [
        ("SecretString Debug", format!("{token:?}")),
        ("SecretString Display", format!("{token}")),
        ("Account Debug", format!("{account:?}")),
        ("FileTokenStore Debug", format!("{store:?}")),
        ("Account::token_len", format!("{}", account.token_len())),
        ("SecretString len", format!("{}", token.len())),
    ] {
        rendered.push((label.to_owned(), value));
    }
    for (label, value) in &rendered {
        assert!(
            !value.contains(MOCK_TOKEN),
            "{label} rendered the token: {value}"
        );
    }
    // The redacting renderings really redact (not just "no token, no marker"):
    // the token slot shows the fixed marker, never the value.
    assert_eq!(format!("{token:?}"), REDACTED);
    assert_eq!(format!("{token}"), REDACTED);
    assert!(format!("{account:?}").contains(REDACTED));

    // A foreign error whose own text *does* contain a token (what ureq or an
    // io error could surface from an upstream body/URL): unscrubbed it leaks,
    // `describe` must not. Asserting the leak proves the scrubbing ran.
    let inner = std::io::Error::other(format!("upstream said: {MOCK_TOKEN}"));
    let outer = std::io::Error::new(std::io::ErrorKind::ConnectionReset, inner);
    assert!(
        format!("{outer}").contains(MOCK_TOKEN),
        "the fixture error must actually carry the token, otherwise the assertion below is vacuous"
    );
    let shown = logging::describe(&outer, &[MOCK_TOKEN]);
    assert!(!shown.contains(MOCK_TOKEN), "describe leaked: {shown}");
    assert!(
        shown.contains(REDACTED),
        "describe must mark the scrubbed secret: {shown}"
    );
    server.shutdown();
}

/// The "never in argv" half, as an auditable source rule.
///
/// `plex-core` spawns no process at all, so there is no argv that could carry a
/// token. This test locks that fact down and enumerates every place a
/// `SecretString` is converted back into plain text (`.expose()`), so a new
/// one cannot be added without review. Log-call-site behaviour is proven
/// dynamically by the tests above; this test only guards *coupling*: no `log::`
/// call site in `src/` may interpolate an `.expose()` value within the same
/// statement window.
#[test]
fn no_product_source_builds_an_argv_and_exposes_secrets_only_at_audited_sites() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&src)
        .expect("src dir must exist")
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
        .collect();
    files.sort();
    assert!(
        files.len() >= 5,
        "expected the plex-core source files, got {files:?}"
    );

    let spawn_needles = [
        "process::Command",
        "Command::new",
        "env::args",
        "env::args_os",
        "std::process",
    ];
    let mut argv_sites: Vec<(String, &'static str)> = Vec::new();
    let mut expose_sites: BTreeMap<String, usize> = BTreeMap::new();
    let mut coupling: Vec<(String, String)> = Vec::new();

    for path in &files {
        let name = path
            .file_name()
            .expect("file name")
            .to_string_lossy()
            .into_owned();
        let text = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("{} must be readable: {e}", path.display()));
        for needle in spawn_needles {
            if text.contains(needle) {
                argv_sites.push((name.clone(), needle));
            }
        }
        let count = text.matches(".expose()").count();
        if count > 0 {
            expose_sites.insert(name.clone(), count);
        }
        for (index, _) in text.match_indices("log::") {
            let from = index.saturating_sub(240);
            let to = text.len().min(index + 240);
            let statement = &text[from..to];
            if statement.contains(".expose()") {
                coupling.push((name.clone(), statement.replace('\n', " ⏎ ")));
            }
        }
    }

    assert!(
        argv_sites.is_empty(),
        "plex-core must not build an argv/spawn a process (a token could leak via process args): {argv_sites:?}"
    );
    assert!(
        coupling.is_empty(),
        "a log:: call site in src/ sits within an .expose() window: {coupling:?}"
    );

    // Audited exposure sites. Each one has a stated reason; adding a new one
    // (or more than one in a file) must be justified here, not silently.
    let expected: BTreeMap<String, usize> = [
        // Re-reading a stored token into the account model / re-cloning it for
        // a request: the value moves into another SecretString, not into text.
        ("api.rs".to_owned(), 1),
        ("auth.rs".to_owned(), 1),
        // The single transport hand-off: X-Plex-Token header value.
        ("client_info.rs".to_owned(), 1),
        // Serializing the keyed record and the legacy record to the 0600 file.
        ("tokens.rs".to_owned(), 2),
        // The secret-service payload bytes.
        ("secret_service.rs".to_owned(), 1),
        // Unit tests inside secret.rs that assert the value round-trips.
        ("secret.rs".to_owned(), 2),
    ]
    .into_iter()
    .collect();
    assert_eq!(
        expose_sites, expected,
        "token exposure sites changed; review each one (header/store/model only, never logs or UI text)"
    );
}
