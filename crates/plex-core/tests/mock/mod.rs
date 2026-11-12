//! A **local fake Plex server** used by the M2 (auth) and M3 (server
//! discovery/connect, library read-only) and transport behaviour tests.
//!
//! Everything this server answers comes from hand-authored JSON in
//! `tests/fixtures/` plus literal statuses built by the tests. It is a mock:
//! it proves our state machine, header and token-store behaviour against a
//! fake plex.tv, **not** against Plex. Real-account acceptance is not yet verified against a real server and no test in this directory substitutes for it.
//!
//! The server listens on `127.0.0.1:0` (ephemeral port, loopback only) over
//! plain HTTP. `tiny_http` 0.12.0 has no `force_close()`/`close()` method, so
//! shutdown is: clear the `running` flag (the handler loop condition is
//! `while running.load()`, so clearing it makes the loop exit), then drop the
//! only strong `Arc<Server>` so `Server::drop` closes the listener and frees
//! the port. The handler thread deliberately holds only a `Weak<Server>`, so
//! "the port is closed" is something a test can actually observe.
//!
//! What each "broken" step really does, checked against the pinned
//! dependency source rather than asserted:
//!
//! * `tiny_http` 0.12.0 has **no** way to close a connection from a handler:
//!   `Request` has no `close()`/`force_close()`, and `impl Drop for Request`
//!   (`tiny_http-0.12.0/src/request.rs`, `impl Drop for Request`, which
//!   responds `Response::empty(500)`) means **dropping a request answers HTTP
//!   500**. That is why this mock has no "drop the request = connection
//!   closed" step: it would be an HTTP 500 in disguise, and the failure-matrix
//!   tests would silently run the same case twice.
//! * `Step::NoReply` parks the `tiny_http::Request` in the mock's pending
//!   list, so nothing is written and the connection stays open. The client's
//!   own `timeout_global` fires (`ureq::Error::Timeout`) and our transport maps
//!   it to `CoreError::Transport`. `tiny_http` dispatches one task per
//!   connection, so parking a request does not block the other routes.
//! * `Step::CutConnection` takes the raw connection writer
//!   (`Request::into_writer`) and writes bytes that are not a parseable HTTP
//!   answer. `ureq` 3.3.0 reports that as `Error::Protocol`, which
//!   `plex_core::http::map_ureq_error` maps to `CoreError::Transport`. The
//!   asserted behaviour in `tests/logout.rs` and `tests/pin_flow.rs` is what
//!   proves that claim, not this comment.
//! * After [`MockPlex::shutdown`] the listener is closed, so a new connection
//!   is refused (`ureq::Error::Io` -> `CoreError::Transport`).

// Each test binary compiles this whole module and uses only a subset of it, so
// `dead_code` would fire per target. Unused scaffolding is not a free pass
// though: `tests/library_read_only.rs::every_documented_route_is_routed_and_named`
// matches on **every** `Route` variant and ties each one to the path constant
// the product module actually requests, so a route can neither drift from the
// product nor sit unused without that exhaustive match noticing.
#![allow(dead_code)]
#![allow(clippy::unwrap_used, clippy::expect_used)] // mock scaffolding may panic loudly; product code keeps these lints on

use std::collections::HashMap;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::thread;
use std::time::Duration;

use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

/// One hand-authored mock answer.
///
/// The `*Json` variants carry a payload **derived from** a fixture file (same
/// JSON shape, this test's own pin id): `tests/fixtures/` fixes the shape, the
/// per-test id keeps the crate-wide used-pin registry from making unrelated
/// tests interfere with each other.
#[derive(Clone, Debug)]
pub enum Step {
    /// Answer 200 + `application/json` with the contents of
    /// `tests/fixtures/<name>` (a recorded mock shape, see the fixture files).
    Fixture(&'static str),
    /// Answer `status` with a literal body (for statuses/JSON the fixtures do
    /// not cover, e.g. 401/422/500).
    Respond(u16, &'static str),
    /// Answer `status` with no body at all (e.g. 204 signout, 204 pin delete).
    NoBody(u16),
    /// Answer `status` with an exact JSON body built from a fixture shape by
    /// [`create_pin_fixture`] / [`pin_fixture`] (see the module docs).
    FixtureJson(u16, String),
    /// Answer 200 forever with the shape of `pin_pending.json` for one test's
    /// pin id: the pin stays pending until the client's own budget/cancel fires.
    PendingJson(String),
    /// Sleep `millis`, then answer 200 + the fixture. Models a slow server so
    /// the poll's own HTTP bound is the thing that ends the pairing attempt.
    Slow(u64, &'static str),
    /// Never answer, keeping the connection open (see the module docs on how
    /// this is implemented): the client's own timeout ends the attempt as a
    /// transport failure.
    NoReply,
    /// Answer with bytes that are not a parseable HTTP answer, so the client
    /// sees a connection cut mid-answer instead of a status code. Not
    /// reproducible by dropping the request: `tiny_http` 0.12.0 turns that into
    /// an HTTP 500 (module docs).
    CutConnection,
    /// Answer `status` (a 3xx) with a `Location` header pointing at `location`
    /// plus a body. This is the only way this mock can express a redirect
    /// **target**, which is what makes "this client never follows it and never
    /// re-presents a token to it" a testable claim. The step that arms it is
    /// `tests/redirects.rs::a_redirect_answer_is_refused_without_a_second_request`
    /// (`docs/CAPABILITIES.md`, "Transport behaviour").
    Redirect(u16, String),
}

/// One request as the mock saw it (used to assert headers/query behaviour).
#[derive(Debug, Clone)]
pub struct Recorded {
    /// Upper-cased method name, e.g. `POST`.
    pub method: String,
    /// Path + query exactly as the client put on the wire.
    pub path_and_query: String,
    /// Request headers, as `(name, value)` in wire order.
    pub headers: Vec<(String, String)>,
    /// Request body (empty for the classic-mode pin POST, which is the point).
    pub body: String,
}

impl Recorded {
    /// Value of header `name` (case-insensitive), if present.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    /// True when `needle` appears in the request line's query part (a token in
    /// a query string must never happen).
    pub fn query_contains(&self, needle: &str) -> bool {
        self.path_and_query
            .split_once('?')
            .is_some_and(|(_, query)| query.contains(needle))
    }

    /// Count of headers whose value contains `needle` (a token must appear
    /// exactly once, as `X-Plex-Token`).
    pub fn value_matches(&self, needle: &str) -> usize {
        self.headers
            .iter()
            .filter(|(_, value)| value.as_str() == needle)
            .count()
    }
}

/// Routes the fake Plex server answers.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Route {
    /// `POST /api/v2/pins`
    Pins,
    /// `GET /api/v2/pins/{id}` (matched by the id, per pin)
    PinPoll(String),
    /// `GET /api/v2/user`
    User,
    /// `DELETE /api/v2/users/signout`
    Signout,
    /// `GET /api/v2/resources` (the account-token call)
    Resources,
    /// `GET /` (the credential-free liveness probe of `crate::server`)
    Root,
    /// `GET /identity` (machine identifier capture)
    Identity,
    /// `GET /library/sections`
    Sections,
    /// `GET /library/sections/{key}/all`
    SectionItems(String),
    /// `GET /search`
    Search,
    /// `GET /library/metadata/{ratingKey}`
    Metadata(String),
    /// `GET /hubs`
    Hubs,
    /// `GET /hubs/search`
    HubsSearch,
    /// `GET /library/onDeck`
    OnDeck,
    /// `GET /library/metadata/{ratingKey}/children`
    Children(String),
    /// `GET /library/sections/{key}/sorts`
    SectionSorts(String),
    /// `GET /:/timeline` - the opt-in playback-position report of
    /// `plex_core::progress::report_timeline`. The item is addressed by the
    /// `ratingKey` query parameter rather than in the path, so there is one
    /// route, not one per item.
    Timeline,
    /// `GET /:/scrobble` - the explicit "mark watched" call of
    /// `plex_core::progress::mark_watched`. Also query-addressed (`key`).
    Scrobble,
}

/// Scripted answers per route. Steps are consumed in order; the last step is
/// repeated once the list is exhausted (so a "pending" poll keeps answering
/// pending).
#[derive(Default, Debug)]
pub struct Script {
    answers: HashMap<Route, Vec<Step>>,
}

impl Script {
    /// An empty script (every request answers 404 `{"errors":[]}`).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Append answers for a route (order matters).
    #[must_use]
    pub fn with(mut self, route: Route, steps: &[Step]) -> Self {
        self.answers
            .entry(route.clone())
            .or_default()
            .extend(steps.iter().cloned());
        self
    }

    fn take(&mut self, route: &Route) -> Step {
        let Some(steps) = self.answers.get_mut(route) else {
            return Step::Respond(404, r#"{"errors":["mock: no scripted answer"]}"#);
        };
        if steps.len() > 1 {
            steps.remove(0)
        } else if let Some(step) = steps.first() {
            // Repeat the final answer: a pin that stays pending keeps being
            // pending, which is what the cancellation/timeout tests need.
            step.clone()
        } else {
            Step::Respond(404, r#"{"errors":["mock: empty script"]}"#)
        }
    }
}

/// The running mock Plex server.
pub struct MockPlex {
    server: Option<Arc<Server>>,
    /// `true` while the worker loop may answer requests; `shutdown`/`Drop`
    /// set it to `false`, which is what makes the loop exit.
    running: Arc<AtomicBool>,
    base: String,
    script: Arc<Mutex<Script>>,
    pending: Arc<Mutex<Vec<Request>>>,
    recorded: Arc<Mutex<Vec<Recorded>>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl MockPlex {
    /// Start the server on an ephemeral loopback port and answer `script`.
    #[must_use]
    pub fn start(script: Script) -> Self {
        let server =
            Arc::new(Server::http("127.0.0.1:0").expect("mock Plex server must bind 127.0.0.1:0"));
        let port = match server.server_addr() {
            tiny_http::ListenAddr::IP(addr) => addr.port(),
            #[cfg(unix)]
            tiny_http::ListenAddr::Unix(_) => panic!("mock Plex server must be TCP"),
        };
        let base = format!("http://127.0.0.1:{port}");
        let running = Arc::new(AtomicBool::new(true));
        let recorded = Arc::new(Mutex::new(Vec::new()));
        let pending = Arc::new(Mutex::new(Vec::new()));
        let script = Arc::new(Mutex::new(script));

        let strong = server.clone();
        let weak: Weak<Server> = Arc::downgrade(&strong);
        let worker_running = running.clone();
        let worker_script = script.clone();
        let worker_recorded = recorded.clone();
        let worker_pending = pending.clone();

        // `Server` is `Sync`, so the loop can call `try_recv` through the Weak.
        let worker = thread::spawn(move || {
            while worker_running.load(Ordering::SeqCst) {
                let Some(server) = weak.upgrade() else {
                    break;
                };
                match server.try_recv() {
                    Ok(Some(request)) => handle(
                        request,
                        &worker_script,
                        &worker_recorded,
                        &worker_pending,
                        &worker_running,
                    ),
                    Ok(None) => thread::sleep(Duration::from_millis(5)),
                    Err(_) => break,
                }
            }
        });

        Self {
            // The worker only holds a Weak, so this is the only strong count.
            server: Some(strong),
            running,
            base,
            script,
            pending,
            recorded,
            worker: Some(worker),
        }
    }

    /// Base URL for `Http::new` (plain HTTP on loopback, ephemeral port).
    #[must_use]
    pub fn base(&self) -> &str {
        &self.base
    }

    /// Replace the script while the server runs (used to arm a later phase,
    /// e.g. the claim fixture after the pin has been created).
    pub fn set_script(&self, script: Script) {
        let mut guard = self
            .script
            .lock()
            .expect("mock script lock must not be poisoned");
        *guard = script;
    }

    /// Requests seen so far, in arrival order.
    #[must_use]
    pub fn requests(&self) -> Vec<Recorded> {
        self.recorded
            .lock()
            .expect("mock request log lock must not be poisoned")
            .clone()
    }

    /// Number of times route path `path` was requested (exact match).
    #[must_use]
    pub fn count_path(&self, path: &str) -> usize {
        self.requests()
            .iter()
            .filter(|r| r.path_and_query == path)
            .count()
    }

    /// Poll requests for `pin_id` (`GET /api/v2/pins/{id}`).
    #[must_use]
    pub fn poll_count(&self, pin_id: &str) -> usize {
        self.count_path(&format!("/api/v2/pins/{pin_id}"))
    }

    /// Stop answering, close the listener and release the port.
    ///
    /// After this returns, the port is free: a connection to `base()` is
    /// refused, which is how the transport-failure test proves the poll path
    /// really fails rather than hanging.
    pub fn shutdown(mut self) {
        self.running.store(false, Ordering::SeqCst);
        // Parking requests closed? Drop them first so parked (no-reply)
        // connections close once the listener is gone.
        if let Ok(mut guard) = self.pending.lock() {
            guard.clear();
        }
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
        // Last strong Arc: `Server::drop` closes the listener.
        self.server = None;
    }
}

impl MockPlex {
    /// Requests recorded for a method name (upper-case), any path.
    #[must_use]
    pub fn count_method(&self, method: &str) -> usize {
        self.requests()
            .iter()
            .filter(|r| r.method == method)
            .count()
    }
}

impl Drop for MockPlex {
    fn drop(&mut self) {
        // Same shutdown path as the explicit call, so a test that forgets
        // `shutdown` still releases the port (the loop is bounded by 5 ms).
        self.running.store(false, Ordering::SeqCst);
        if let Ok(mut guard) = self.pending.lock() {
            guard.clear();
        }
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
        self.server = None;
    }
}

fn handle(
    mut request: Request,
    script: &Arc<Mutex<Script>>,
    recorded: &Arc<Mutex<Vec<Recorded>>>,
    pending: &Arc<Mutex<Vec<Request>>>,
    running: &Arc<AtomicBool>,
) {
    let path_and_query = request.url().to_owned();
    let path = path_and_query.split('?').next().unwrap_or("").to_owned();
    let method = request.method().to_string().to_uppercase();
    let mut body = String::new();
    if request.body_length().unwrap_or(0) > 0 {
        let mut bytes = Vec::new();
        let _ = request.as_reader().read_to_end(&mut bytes);
        body = String::from_utf8_lossy(&bytes).into_owned();
    }
    let headers: Vec<(String, String)> = request
        .headers()
        .iter()
        .map(|h| (h.field.as_str().to_string(), h.value.as_str().to_string()))
        .collect();

    if let Ok(mut guard) = recorded.lock() {
        guard.push(Recorded {
            method: method.clone(),
            path_and_query: path_and_query.clone(),
            headers,
            body,
        });
    }

    // No further answers while shutting down: park the request so the client
    // sees a transport failure instead of a 500.
    if !running.load(Ordering::SeqCst) {
        drop(request);
        return;
    }

    // A path the mock does not know gets an explicit 404 (a visible failure),
    // never a scripted answer and never a hang.
    let Some(route) = route_for(&method, &path) else {
        respond(
            request,
            404,
            r#"{"errors":["mock: unknown route"]}"#.to_owned(),
        );
        return;
    };
    let step = script
        .lock()
        .expect("mock script lock must not be poisoned")
        .take(&route);

    match step {
        Step::Fixture(name) => respond(request, 200, fixture(name)),
        Step::FixtureJson(status, json) => respond(request, status, json),
        Step::PendingJson(json) => respond(request, 200, json),
        Step::Respond(status, text) => respond(request, status, text.to_owned()),
        Step::NoBody(status) => {
            let _ = request.respond(Response::new_empty(StatusCode(status)));
        }
        Step::Slow(millis, name) => {
            thread::sleep(Duration::from_millis(millis));
            if !running.load(Ordering::SeqCst) {
                drop(request);
                return;
            }
            respond(request, 200, fixture(name));
        }
        Step::NoReply => {
            // Keep the connection open with no answer: the client's own
            // timeout is what ends the attempt.
            if let Ok(mut guard) = pending.lock() {
                guard.push(request);
            }
        }
        Step::CutConnection => {
            // Raw writer, unparsable answer, then close our side of the write
            // half: this is what makes the client see `CoreError::Transport`
            // (see the module docs for the dependency-level reason).
            let mut writer = request.into_writer();
            let _ = writer.write_all(b"MOCK CUT CONNECTION\r\n\r\n");
        }
        Step::Redirect(status, location) => {
            let location = Header::from_bytes(&b"Location"[..], location.as_bytes())
                .expect("mock Location header must be ASCII");
            let content_type = Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..])
                .expect("mock content-type header must be ASCII");
            let response =
                Response::from_data(r#"{"errors":["mock: redirect answer"]}"#.to_owned())
                    .with_status_code(StatusCode(status))
                    .with_header(location)
                    .with_header(content_type);
            let _ = request.respond(response);
        }
    }
}

fn respond(request: Request, status: u16, body: String) {
    let content_type = Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..])
        .expect("mock content-type header must be ASCII");
    let response = Response::from_data(body)
        .with_status_code(StatusCode(status))
        .with_header(content_type);
    let _ = request.respond(response);
}

/// Map a request line to the scripted route (module-private callers: the
/// worker loop; `tests/library_read_only.rs` checks this table against the
/// product path constants so the mock cannot drift away from what we send).
pub fn route_for(method: &str, path: &str) -> Option<Route> {
    match (method, path) {
        ("POST", "/api/v2/pins") => Some(Route::Pins),
        ("DELETE", "/api/v2/users/signout") => Some(Route::Signout),
        ("GET", "/api/v2/user") => Some(Route::User),
        ("GET", "/api/v2/resources") => Some(Route::Resources),
        ("GET", "/") => Some(Route::Root),
        ("GET", "/identity") => Some(Route::Identity),
        ("GET", "/library/sections") => Some(Route::Sections),
        ("GET", "/search") => Some(Route::Search),
        ("GET", "/hubs") => Some(Route::Hubs),
        ("GET", "/hubs/search") => Some(Route::HubsSearch),
        ("GET", "/library/onDeck") => Some(Route::OnDeck),
        ("GET", "/:/timeline") => Some(Route::Timeline),
        ("GET", "/:/scrobble") => Some(Route::Scrobble),
        ("GET", other) => {
            if let Some(id) = other
                .strip_prefix("/api/v2/pins/")
                .filter(|id| !id.is_empty())
            {
                return Some(Route::PinPoll(id.to_owned()));
            }
            if let Some(rest) = other.strip_prefix("/library/sections/") {
                // `{key}/sorts` is its own scripted route (the sorts one section
                // offers), with the same one-segment rule as below.
                if let Some(key) = rest.strip_suffix("/sorts") {
                    return (!key.is_empty() && !key.contains('/'))
                        .then(|| Route::SectionSorts(key.to_owned()));
                }
                // `{key}/all` and a bare `{key}` are the same scripted route, so
                // a test can script one section without naming the verb. A key
                // is exactly **one** path segment: anything with an extra
                // segment (`/library/sections/1/all/extra`) is not a route this
                // client sends, so it stays unscripted (404) instead of being
                // folded into a section that exists.
                let key = rest.strip_suffix("/all").unwrap_or(rest);
                if !key.is_empty() && !key.contains('/') {
                    return Some(Route::SectionItems(key.to_owned()));
                }
                return None;
            }
            if let Some(rest) = other.strip_prefix("/library/metadata/") {
                if let Some(id) = rest.strip_suffix("/children") {
                    // `{ratingKey}/children` is its own scripted route: one
                    // segment, or the mock has no answer.
                    return (!id.is_empty() && !id.contains('/'))
                        .then(|| Route::Children(id.to_owned()));
                }
                // Same rule as above: one segment, or the mock has no answer.
                return (!rest.is_empty() && !rest.contains('/'))
                    .then(|| Route::Metadata(rest.to_owned()));
            }
            None
        }
        _ => None,
    }
}

/// True when no recorded request carries `needle` anywhere - in a header value,
/// in the request line or in a request body.
///
/// The tests that actually run it are
/// `tests/server_connect.rs::the_account_token_never_reaches_an_owned_server_host`
/// and `tests/library_read_only.rs::library_calls_never_carry_the_account_token`,
/// both of which assert against requests the mock **recorded from real loopback
/// HTTP traffic**, not against source text.
#[must_use]
pub fn no_record_carries(records: &[Recorded], needle: &str) -> bool {
    !records.iter().any(|record| {
        record
            .headers
            .iter()
            .any(|(_, value)| value.as_str() == needle)
            || record.path_and_query.contains(needle)
            || record.body.contains(needle)
    })
}

/// Count of requests that carry `needle` as the value of header `name`.
#[must_use]
pub fn requests_with_token_header(records: &[Recorded], name: &str, needle: &str) -> usize {
    records
        .iter()
        .filter(|record| record.header(name) == Some(needle))
        .count()
}

/// `tests/fixtures/<name>` as text.
///
/// These fixtures are **hand-authored mock recordings** written by this
/// workspace from the field names documented in `docs/PLEX_AUTH.md`
/// (`id`, `code`, `authToken`, `uuid`, `rememberExpiresAt`,
/// `twoFactorEnabled`). They are not captures of a real Plex server.
pub fn fixture(name: &str) -> String {
    let path = fixture_path(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("mock fixture {} must be readable: {e}", path.display()))
}

/// Absolute path of a mock fixture (tests may read it directly too).
#[must_use]
pub fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

/// Rewrite the JSON shape recorded in `tests/fixtures/<name>` with this test's
/// pin id (and code, when non-empty), keeping every other field/value of the
/// recording untouched. `authToken` is only rewritten when the recording has
/// one, so a *pending* fixture stays a *pending* answer.
#[must_use]
fn pin_shape(name: &str, pin_id: &str, pin_code: &str) -> String {
    let text = fixture(name);
    let mut value: serde_json::Value = serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("mock fixture {name} must be JSON: {e}"));
    let object = value
        .as_object_mut()
        .unwrap_or_else(|| panic!("mock fixture {name} must be a JSON object"));
    // plex.tv sends a numeric id; numeric test ids keep that shape.
    let id = pin_id.parse::<u64>().map_or_else(
        |_| serde_json::Value::String(pin_id.to_owned()),
        serde_json::Value::from,
    );
    object.insert("id".to_owned(), id);
    if !pin_code.is_empty() {
        object.insert(
            "code".to_owned(),
            serde_json::Value::String(pin_code.to_owned()),
        );
    }
    if object.contains_key("authToken") {
        object.insert(
            "authToken".to_owned(),
            serde_json::Value::String(MOCK_TOKEN.to_owned()),
        );
    }
    serde_json::to_string(&value).expect("mock fixture must re-serialize")
}

/// `POST /api/v2/pins` answer: the recorded `pin_created.json` shape, but with
/// `pin_id` so that pairing tests cannot retire each other's pin (the used-pin
/// registry in `plex_core::auth` is process-global).
#[must_use]
pub fn create_pin_fixture(pin_id: &str, pin_code: &str) -> Step {
    Step::FixtureJson(200, pin_shape("pin_created.json", pin_id, pin_code))
}

/// `GET /api/v2/pins/{id}` answer from a fixture (status `status`), with the
/// fixture's id rewritten to `pin_id` so the answer matches the pin being
/// polled. Used for `pending` (200, no token) and `claimed` (200 + token).
#[must_use]
pub fn pin_fixture(status: u16, name: &str, pin_id: &str) -> Step {
    Step::FixtureJson(status, pin_shape(name, pin_id, ""))
}

/// A pin poll that keeps answering with the recorded **pending** shape forever
/// (no `authToken`): the state machine must end it by cancel or budget, not by
/// a server answer.
#[must_use]
pub fn pending_forever(pin_id: &str) -> Step {
    Step::PendingJson(pin_shape("pin_pending.json", pin_id, ""))
}

/// The distinctive token literal used by the mock fixtures, so a redaction
/// test can assert on the exact string that would leak.
pub const MOCK_TOKEN: &str = "MOCKTOKEN-9f4c1b7ae2d84c0f";

/// The mock pin id/code pair, for tests that need to name them.
pub const MOCK_PIN_ID: &str = "mock-pin-1";
/// Mock pairing code (not a secret: the user types it).
pub const MOCK_PIN_CODE: &str = "XPGK";

/// True when a recorded request carries `MOCK_TOKEN` only as header `name`
/// and never in the query.
#[must_use]
pub fn token_only_in_header(recorded: &Recorded, name: &str) -> bool {
    recorded.value_matches(MOCK_TOKEN) == 1
        && recorded
            .header(name)
            .is_some_and(|value| value == MOCK_TOKEN)
        && !recorded.query_contains(MOCK_TOKEN)
        && !recorded.path_and_query.contains(MOCK_TOKEN)
}

/// Helper for `Method` comparisons in tests (kept here to avoid importing
/// `tiny_http` in every test file).
#[must_use]
pub fn method_name(method: &Method) -> String {
    method.to_string().to_uppercase()
}
