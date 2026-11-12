//! Plex PIN browser-authorization state machine (core).
//!
//! Flow implemented, per docs/PLEX_AUTH.md §1 (1..3) with our own
//! failure rules (§3 "Planned (our design…)"):
//!
//! * `POST /api/v2/pins` in **classic mode** — no `strong` parameter, no JSON
//!   body, no token. Response yields `id` (internal pin id) + `code` (the
//!   short pairing code the user types or carries into the browser URL).
//! * The user is shown [`PairingSession::pairing_code`] and
//!   [`PairingSession::claim_url`] (`https://app.plex.tv/auth/#!?clientID=…&code=…`
//!   plus the `context[device][…]` hints Plex's own clients send).
//! * Poll `GET /api/v2/pins/{id}` **every 1 s inside a 120 s budget**.
//! * States: `pending` (no `authToken` yet) → `claimed` (token present) |
//!   `expired` | `cancelled` | `timeout` | `rejected` (401/422).
//! * **Any** poll failure (transport, unexpected status, unparseable body) is
//!   `expired`: this client cannot tell a dead pin from a server fault, so the
//!   pin is dropped and a **new pin** is required — the same pin id is never
//!   polled again ([`CoreError::PinReuseRefused`] guards a second attempt).
//! * `401`/`422` → `rejected`: caller must clear the stored token
//!   ([`crate::api::clear_token_on_auth_rejection`]) and start a new pin. There
//!   is no blind retry and **no password path anywhere in this crate**.
//! * In-flight cancel: polling stops (the pending 1 s tick wakes immediately)
//!   and the pin is dropped — measured at well under 1 s in
//!   `tests/pin_flow.rs`.
//!
//! Backoff: the poll tick is fixed at 1 s; a poll whose HTTP call exceeds
//! [`POLL_HTTP_TIMEOUT`] is a transport failure and therefore an expiry, which
//! keeps both the UI cancel latency and the whole pairing budget bounded.

use std::collections::HashSet;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use crate::client_info::ClientInfo;
use crate::error::CoreError;
use crate::http::Http;
use crate::secret::SecretString;

/// Pin creation endpoint (classic mode).
pub const PINS_PATH: &str = "/api/v2/pins";

/// Poll interval (docs/PLEX_AUTH.md §1.3: reference client polls every 1 s).
pub const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Whole pairing budget (docs/PLEX_AUTH.md §1.3: reference `run(timeout=120)`).
pub const PAIRING_BUDGET: Duration = Duration::from_secs(120);

/// HTTP timeout applied to each poll by the auth worker itself.
///
/// This is *our* bound, chosen so that a stuck poll cannot push cancel
/// latency past `POLL_HTTP_TIMEOUT + POLL_INTERVAL`; a poll that exceeds it is
/// treated as a transport failure, i.e. the pin expires and a new one is
/// created. Not a Plex-documented value.
pub const POLL_HTTP_TIMEOUT: Duration = Duration::from_millis(1_500);

/// TCP/TLS connect bound applied to the poll transport by
/// [`PairingSession::with_pin`] (our own bound, not a Plex-documented value).
pub const POLL_CONNECT_TIMEOUT: Duration = Duration::from_secs(1);

/// Plex's browser claim page (docs/PLEX_AUTH.md §1.2b).
pub const CLAIM_URL_BASE: &str = "https://app.plex.tv/auth/#!";

/// One created pin: `id` is polled, `code` is what the user sees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pin {
    /// Internal pin id returned by `POST /api/v2/pins`.
    pub id: String,
    /// Short pairing code shown to the user (displayable by design; it is not
    /// a bearer token and is never sent to us again).
    pub code: String,
}

/// The state machine's phases, as reported to the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Pin created, waiting for the user to complete the browser sign-in.
    Pending,
    /// The pin was claimed: the account token is now available.
    Claimed,
    /// Poll budget elapsed without a claim.
    Timeout {
        /// Whole seconds of the budget that elapsed.
        secs: u64,
    },
    /// Poll budget ended, or **any** poll step failed.
    Expired,
    /// Cancelled by the user: polling stopped, pin dropped.
    Cancelled,
    /// 401/422: the stored token was rejected; clear it and create a new pin.
    Rejected {
        /// 401 or 422.
        status: u16,
    },
}

impl Phase {
    /// Machine-readable code for the UI/CLI.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Claimed => "claimed",
            Self::Timeout { .. } => "timeout",
            Self::Expired => "expired",
            Self::Cancelled => "cancelled",
            Self::Rejected { .. } => "rejected",
        }
    }

    /// True for the states that end a pairing attempt.
    #[must_use]
    pub fn is_terminal(self) -> bool {
        !matches!(self, Self::Pending)
    }

    /// Map a terminal phase onto the typed error a caller must handle.
    ///
    /// `Claimed` yields `Ok(None)` (the token is in [`PairingSession::outcome`]).
    pub fn error(self) -> Result<Option<CoreError>, CoreError> {
        match self {
            Self::Pending | Self::Claimed => Ok(None),
            Self::Expired => Ok(Some(CoreError::PinExpired)),
            Self::Timeout { secs } => Ok(Some(CoreError::PinTimeout { secs })),
            Self::Cancelled => Ok(Some(CoreError::PinCancelled)),
            Self::Rejected { status } => Ok(Some(CoreError::AuthRejected { status })),
        }
    }
}

/// Terminal result of a pairing attempt: a phase plus the token when claimed.
///
/// `Debug` is hand-written and redacted, so an `Outcome` can be logged safely.
#[derive(Clone, PartialEq, Eq)]
pub struct Outcome {
    /// Final phase.
    pub phase: Phase,
    /// The account token, present only for [`Phase::Claimed`].
    token: Option<SecretString>,
}

impl std::fmt::Debug for Outcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Outcome")
            .field("phase", &self.phase)
            .field("token", &crate::secret::REDACTED)
            .field("has_token", &self.token.is_some())
            .finish()
    }
}

impl Outcome {
    /// Length of the claimed token without exposing it (UI indicator).
    #[must_use]
    pub fn token_len(&self) -> usize {
        self.token.as_ref().map_or(0, SecretString::len)
    }

    /// Move the token out of the outcome (the only way to obtain it).
    pub fn take_token(&mut self) -> Option<SecretString> {
        self.token.take()
    }

    /// Clone the token for storage/requests.
    #[must_use]
    pub fn token(&self) -> Option<SecretString> {
        self.token
            .as_ref()
            .map(|token| SecretString::new(token.expose()))
    }
}

/// Pins that already reached a failure state in this process.
///
/// The pin id is device-persistent-ish but not a bearer secret; keeping it here
/// is what makes "a new pin is required, the same pin is not retried" an
/// enforceable rule instead of a convention.
static FAILED_PINS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

fn failed_pins() -> &'static Mutex<HashSet<String>> {
    FAILED_PINS.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Mark a pin as spent so it can never be polled again.
pub fn retire_pin(id: &str) {
    if let Ok(mut guard) = failed_pins().lock() {
        guard.insert(id.to_owned());
    }
}

/// True when `id` already failed and must not be re-used.
#[must_use]
pub fn pin_is_retired(id: &str) -> bool {
    failed_pins().lock().is_ok_and(|guard| guard.contains(id))
}

/// A running pairing attempt: a worker thread plus the channels the UI uses.
pub struct PairingSession {
    pin: Pin,
    claim_url: String,
    cancel: Sender<()>,
    events: Receiver<Phase>,
    worker: Option<thread::JoinHandle<Outcome>>,
    outcome: Option<Outcome>,
    budget: Duration,
}

impl std::fmt::Debug for PairingSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PairingSession")
            .field("pin", &self.pin)
            .field("claim_url", &self.claim_url)
            .field("finished", &self.outcome.is_some())
            .finish()
    }
}

impl PairingSession {
    /// Create a pin (classic mode) and start polling it on a worker thread.
    ///
    /// The caller's [`Http`] is used only for pin creation; the poll transport
    /// is re-configured by the worker with [`POLL_HTTP_TIMEOUT`] (see
    /// [`Http::with_poll_timeouts`]).
    pub fn start(http: &Http, client: &ClientInfo) -> Result<Self, CoreError> {
        let pin = create_pin(http)?;
        Self::with_pin(http, client, pin)
    }

    /// Start polling an existing pin, refusing a pin that already failed.
    ///
    /// The only way to continue after `expired`/`cancelled`/`timeout`/`rejected`
    /// is a **new** pin via [`PairingSession::start`]; re-calling this with the
    /// same pin yields [`CoreError::PinReuseRefused`] (no blind retry).
    pub fn with_pin(http: &Http, client: &ClientInfo, pin: Pin) -> Result<Self, CoreError> {
        Self::with_pin_budget(http, client, pin, PAIRING_BUDGET)
    }

    /// The same attempt with an explicit whole pairing budget.
    ///
    /// Product code always gets [`PAIRING_BUDGET`] (via [`PairingSession::start`]
    /// / [`PairingSession::with_pin`]); this variant exists so the timeout
    /// branch and the 1 s tick can be observed in tests in a fraction of 120 s
    /// and so a UI can bound an attempt tighter than the Plex pin lifetime.
    /// A budget longer than [`PAIRING_BUDGET`] is refused: Plex's classic pin is
    /// not valid past that window, so polling longer would only hide a dead pin.
    pub fn with_pin_budget(
        http: &Http,
        client: &ClientInfo,
        pin: Pin,
        budget: Duration,
    ) -> Result<Self, CoreError> {
        if budget.is_zero() || budget > PAIRING_BUDGET {
            return Err(CoreError::InvalidInput);
        }
        if pin.id.is_empty() || pin.code.is_empty() {
            return Err(CoreError::ResponseShape);
        }
        if pin_is_retired(&pin.id) {
            return Err(CoreError::PinReuseRefused);
        }
        let poll_http = http.with_poll_timeouts(client, POLL_CONNECT_TIMEOUT, POLL_HTTP_TIMEOUT)?;
        let claim_url = build_claim_url(client, &pin)?;
        let (cancel_tx, cancel_rx) = mpsc::channel::<()>();
        let (event_tx, event_rx) = mpsc::channel::<Phase>();
        let worker_pin = pin.clone();
        let worker = thread::spawn(move || {
            run_poll_loop(poll_http, worker_pin, cancel_rx, event_tx, budget)
        });
        Ok(Self {
            pin,
            claim_url,
            cancel: cancel_tx,
            events: event_rx,
            worker: Some(worker),
            outcome: None,
            budget,
        })
    }

    /// The whole pairing budget this attempt enforces.
    #[must_use]
    pub fn budget(&self) -> Duration {
        self.budget
    }

    /// The pin this session polls.
    #[must_use]
    pub fn pin(&self) -> &Pin {
        &self.pin
    }

    /// The pairing code to show/type at `https://plex.tv/link` (classic mode).
    #[must_use]
    pub fn pairing_code(&self) -> &str {
        &self.pin.code
    }

    /// The browser URL to open for the user (carries the pairing code, never a
    /// token).
    #[must_use]
    pub fn claim_url(&self) -> &str {
        &self.claim_url
    }

    /// Ask the worker to stop polling and drop the pin.
    ///
    /// Returns once the worker has exited, so "cancel stopped the polling" is
    /// observable to the caller, not optimistic. The pin is retired.
    pub fn cancel(&mut self) -> Result<Outcome, CoreError> {
        let _ = self.cancel.send(());
        self.finish()
    }

    /// Wait for the next phase change (bounded), for UI event loops.
    ///
    /// Returns `None` when `timeout` elapsed with no new phase.
    pub fn next_phase(&mut self, wait: Duration) -> Result<Option<Phase>, CoreError> {
        if let Some(outcome) = &self.outcome {
            return Ok(Some(outcome.phase));
        }
        match self.events.recv_timeout(wait) {
            Ok(phase) => Ok(Some(phase)),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => self.finish().map(|outcome| Some(outcome.phase)),
        }
    }

    /// Poll the pin synchronously until a terminal phase or `budget` elapses
    /// from now. Convenience wrapper for the CLI/tests; `cancel` still works
    /// from another thread because it only sends on a channel.
    pub fn run(&mut self, budget: Duration) -> Result<Outcome, CoreError> {
        let deadline = Instant::now() + budget;
        loop {
            if Instant::now() >= deadline {
                return self.cancel();
            }
            match self.next_phase(POLL_INTERVAL)? {
                Some(phase) if phase.is_terminal() => return self.finish(),
                Some(_) => {}
                None => {}
            }
        }
    }

    /// The terminal outcome, joining the worker (idempotent).
    pub fn outcome(&mut self) -> Result<Outcome, CoreError> {
        if let Some(outcome) = &self.outcome {
            return Ok(outcome.clone());
        }
        self.finish()
    }

    fn finish(&mut self) -> Result<Outcome, CoreError> {
        if let Some(outcome) = &self.outcome {
            return Ok(outcome.clone());
        }
        // Drain any phase the UI has not consumed yet, then join.
        while let Ok(phase) = self.events.try_recv() {
            let _ = phase;
        }
        let handle = self.worker.take();
        let outcome = match handle {
            Some(handle) => match handle.join() {
                Ok(outcome) => outcome,
                Err(_) => Outcome {
                    phase: Phase::Expired,
                    token: None,
                },
            },
            None => return Err(CoreError::ResponseShape),
        };
        retire_pin(&self.pin.id);
        let result = outcome.clone();
        self.outcome = Some(outcome);
        Ok(result)
    }
}

impl Drop for PairingSession {
    fn drop(&mut self) {
        // Never leave a poller running after its session is gone: ask it to
        // stop and retire the pin. `Drop` cannot return an error, so the
        // request is best-effort by design.
        let _ = self.cancel.send(());
        retire_pin(&self.pin.id);
        if let Some(handle) = self.worker.take() {
            handle.thread().unpark();
            // `unpark` cannot interrupt a thread blocked in a channel wait, so
            // the cancel message above is what wakes the 1 s tick: join here is
            // bounded by that tick, which is why "cancel stopped the polling"
            // is observable rather than optimistic.
            let _ = handle.join();
        }
    }
}

/// `POST /api/v2/pins` in classic mode: no `strong`, no body, no token.
pub fn create_pin(http: &Http) -> Result<Pin, CoreError> {
    let response = http.post(PINS_PATH, &[], None, None)?;
    if !response.is_success() {
        return Err(CoreError::from_status(response.status));
    }
    // plex.tv sends `id` as a JSON number; a string is accepted too.
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum IdWire {
        Num(u64),
        Str(String),
    }
    #[derive(serde::Deserialize)]
    struct PinWire {
        id: Option<IdWire>,
        code: Option<String>,
    }
    let wire: PinWire = response.json().map_err(|_| CoreError::ResponseShape)?;
    let id = match wire.id {
        Some(IdWire::Num(n)) => n.to_string(),
        Some(IdWire::Str(s)) if !s.is_empty() => s,
        _ => return Err(CoreError::ResponseShape),
    };
    let code = wire
        .code
        .filter(|v| !v.is_empty())
        .ok_or(CoreError::ResponseShape)?;
    Ok(Pin { id, code })
}

/// Build the browser claim URL: `clientID` = `X-Plex-Client-Identifier`,
/// `code` = the pairing code, plus the `context[device][…]` device hints.
///
/// The parameters are appended **after** the `#!` fragment, i.e. the URL is
/// `https://app.plex.tv/auth/#!?clientID=…&code=…` (the form the reference
/// clients build; see docs/PLEX_AUTH.md §1.2b). Appending them with
/// `Url::query_pairs_mut` would move the fragment *after* the query and make
/// the pairing code a server-visible query parameter, so the fragment form is
/// built here with `form_urlencoded` instead.
///
/// No token is ever put in a URL (docs/PLEX_AUTH.md §3).
pub fn build_claim_url(client: &ClientInfo, pin: &Pin) -> Result<String, CoreError> {
    // Parsed only to prove the base we ship is a valid URL; the returned string
    // is assembled manually so the `?…` stays inside the `#!` fragment.
    let base = CLAIM_URL_BASE
        .parse::<url::Url>()
        .map_err(|_| CoreError::InvalidInput)?;
    if base.fragment() != Some("!") || base.query().is_some() {
        return Err(CoreError::InvalidInput);
    }
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("clientID", &client.client_identifier)
        .append_pair("code", &pin.code)
        .append_pair("context[device][product]", &client.product)
        .append_pair("context[device][version]", &client.version)
        .append_pair("context[device][platform]", &client.platform)
        .append_pair("context[device][platformVersion]", &client.platform_version)
        .append_pair("context[device][device]", &client.device)
        .append_pair("context[device][deviceName]", &client.device_name)
        .finish();
    Ok(format!("{CLAIM_URL_BASE}?{query}"))
}

/// One poll step. `Ok(None)` = still pending, `Some(SecretString)` = claimed.
///
/// Any transport/status/shape problem returns an `Err`, which the loop turns
/// into [`Phase::Expired`]. 401/422 are reported as `Err(AuthRejected)` and the
/// loop turns them into [`Phase::Rejected`] (so the caller clears the token).
pub fn poll_once(http: &Http, pin: &Pin) -> Result<Option<SecretString>, CoreError> {
    let path = format!("{PINS_PATH}/{}", pin.id);
    let response = http.get(&path, &[], None)?;
    if !response.is_success() {
        return Err(CoreError::from_status(response.status));
    }
    if response.body.is_empty() {
        return Ok(None);
    }
    #[derive(serde::Deserialize)]
    struct PollWire {
        #[serde(rename = "authToken")]
        auth_token: Option<String>,
    }
    let wire: PollWire = response.json().map_err(|_| CoreError::ResponseShape)?;
    Ok(wire
        .auth_token
        .filter(|token| !token.is_empty())
        .map(SecretString::new))
}

fn run_poll_loop(
    http: Http,
    pin: Pin,
    cancel: Receiver<()>,
    events: Sender<Phase>,
    budget: Duration,
) -> Outcome {
    let deadline = Instant::now() + budget;
    loop {
        if cancel_ready(&cancel) {
            return Outcome {
                phase: Phase::Cancelled,
                token: None,
            };
        }
        if Instant::now() >= deadline {
            return Outcome {
                phase: Phase::Timeout {
                    secs: budget.as_secs(),
                },
                token: None,
            };
        }
        let phase = match poll_once(&http, &pin) {
            Ok(Some(token)) => {
                log::info!("pin claimed (pin id hidden, token len {})", token.len());
                let outcome = Outcome {
                    phase: Phase::Claimed,
                    token: Some(token),
                };
                let _ = events.send(outcome.phase);
                return outcome;
            }
            Ok(None) => Phase::Pending,
            // 401/422 are auth rejections, not expiries: the caller must clear
            // the token. Anything else is indistinguishable from a dead pin.
            Err(CoreError::AuthRejected { status }) => Phase::Rejected { status },
            Err(error) => {
                log::warn!("pin poll failed ({}), pin expired", error.code());
                Phase::Expired
            }
        };
        let _ = events.send(phase);
        if phase.is_terminal() {
            return Outcome { phase, token: None };
        }
        // 1 s tick that wakes immediately on cancel, so cancel latency is
        // bounded by the in-flight request, not by the remaining budget.
        match cancel.recv_timeout(POLL_INTERVAL) {
            Ok(()) => {
                return Outcome {
                    phase: Phase::Cancelled,
                    token: None,
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                return Outcome {
                    phase: Phase::Cancelled,
                    token: None,
                }
            }
        }
    }
}

fn cancel_ready(cancel: &Receiver<()>) -> bool {
    matches!(cancel.try_recv(), Ok(()))
}
