//! The async control loop: reply tokens, pending-request correlation and the
//! non-blocking event pump.
//!
//! This is the heart of the player layer and it is libmpv-agnostic (it runs against
//! [`crate::mock`] in tests and against [`crate::mpv_link::FfiPlayer`] in the
//! real session). The rules it exists to enforce:
//!
//! * every async call gets a **unique, monotonically increasing** reply token
//!   ([`TokenAllocator`], token `0` is never issued - it means "no token");
//! * a `MPV_EVENT_COMMAND_REPLY` is attributed to a request only by its
//!   `reply_userdata` token, and only while that request is still outstanding
//!   ([`PendingReplies`]); a reply for an unknown/stale token is reported as
//!   [`crate::PlayerEvent::Uncorrelated`] instead of being guessed onto the newest
//!   command;
//! * progress is drained by [`PlayerSession::pump`], which reads the wakeup
//!   flag and then calls [`crate::link::PlayerLink::next_event`] - a
//!   **non-blocking** call (`mpv_wait_event(h, 0)`) - until the queue is empty,
//!   with a hard event budget. It never blocks, never sleeps and never busy-spins:
//!   a drain that finds an empty queue returns immediately.

use std::collections::VecDeque;

use crate::commands::{props, ItemSource, Named, SeekMode};
use crate::controls;
use crate::events::{EndReason, EventKind, Observed, PlayerError, PlayerEvent};
use crate::link::{PlayerLink, RawEvent, Request};
use crate::state::ObservedState;
use crate::tracks::{select_audio, select_subtitle, TrackSelection};

/// Token `0` is reserved: libmpv events that are not replies carry
/// `reply_userdata == 0`, so a real token must never be 0.
pub const NO_TOKEN: u64 = 0;

/// How many delivered events a [`PlayerSession`] keeps for inspection.
///
/// A UI loop pumps every frame, so an ever-growing event log would grow the
/// session's memory without bound. The session keeps a **bounded window** of the most recent
/// events plus cumulative totals ([`PlayerSession::delivered_total`],
/// [`PlayerSession::evicted_events`]), and the UI reads events with
/// [`PlayerSession::drain_events`] (read-and-clear) rather than re-reading a
/// growing log.
pub const MAX_TRACKED_EVENTS: usize = 256;

/// How many [`PumpReport`]s a [`PlayerSession`] keeps, for the same reason
/// ([`PlayerSession::pump_totals`] carries the cumulative numbers).
pub const MAX_TRACKED_PUMP_REPORTS: usize = 64;
/// Bounded window for correlated-but-rejected replies.
pub const MAX_TRACKED_REJECTIONS: usize = 32;
/// Bounded window for control-gate / flush fact lines.
pub const MAX_TRACKED_FLUSHES: usize = 16;

/// Reason label recorded when a command reply arrives with `reply_userdata == 0`.
///
/// libmpv uses `0` for events that are **not** replies, so such an event can
/// never be attributed to an outstanding request. Both report sites - the
/// [`PlayerEvent::Uncorrelated`] delivery and [`PendingReplies::unmatched`] -
/// carry this same label so a report cannot show two different reasons for one
/// event.
pub const UNMATCHED_NO_TOKEN: &str = "reply carried no reply_userdata token";

/// Reason label recorded when a reply's token matches no outstanding request
/// (already answered, stale, or foreign to this session).
pub const UNMATCHED_NO_REQUEST: &str = "no outstanding request with this token";

/// Allocates `reply_userdata` tokens for async calls.
///
/// Tokens are unique for the lifetime of one session: [`TokenAllocator`] never
/// reuses a token, so a late reply for an already-finished command can be
/// recognised as stale rather than mis-read as the reply to a newer command.
#[derive(Clone, Debug)]
pub struct TokenAllocator {
    next: u64,
    issued: usize,
}

impl Default for TokenAllocator {
    fn default() -> Self {
        Self::new()
    }
}

impl TokenAllocator {
    /// A fresh allocator whose first token is `1`.
    #[must_use]
    pub fn new() -> Self {
        Self { next: 1, issued: 0 }
    }

    /// Allocate the next token.
    pub fn alloc(&mut self) -> u64 {
        let token = self.next;
        self.next = self.next.saturating_add(1);
        self.issued += 1;
        token
    }

    /// How many tokens have been issued (used by the reports).
    #[must_use]
    pub fn issued(&self) -> usize {
        self.issued
    }

    /// The next token value that would be issued.
    #[must_use]
    pub fn next_token(&self) -> u64 {
        self.next
    }
}

/// One outstanding async request.
#[derive(Clone, Debug, PartialEq)]
pub struct PendingRequest {
    /// The token we passed to libmpv.
    pub token: u64,
    /// Our label for the request (`loadfile …`, `set pause = flag`, …).
    pub label: String,
    /// The async entry point this request was issued through
    /// (`mpv_command_async` / `mpv_set_property_async`), recorded at issue time
    /// from the [`Request`] variant so a failing reply reports its real origin.
    pub call: String,
}

/// Outstanding async requests, correlated by reply token.
#[derive(Clone, Debug, Default)]
pub struct PendingReplies {
    entries: VecDeque<PendingRequest>,
    /// Replies that arrived for a token we had no outstanding request for.
    unmatched: Vec<(u64, &'static str)>,
}

impl PendingReplies {
    /// Empty set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an issued request (label and async call come from the request).
    pub fn insert(&mut self, token: u64, request: &Request) {
        self.entries.push_back(PendingRequest {
            token,
            label: request.label(),
            call: request.call_name().to_owned(),
        });
    }

    /// `true` when a token is outstanding.
    #[must_use]
    pub fn contains(&self, token: u64) -> bool {
        self.entries.iter().any(|p| p.token == token)
    }

    /// Remove and return the outstanding request for `token`, or `None` (and
    /// record the miss).
    pub fn take(&mut self, token: u64) -> Option<PendingRequest> {
        if token == NO_TOKEN {
            self.unmatched.push((token, UNMATCHED_NO_TOKEN));
            return None;
        }
        if let Some(index) = self.entries.iter().position(|p| p.token == token) {
            // VecDeque::remove is O(n) but n is the number of outstanding
            // commands for one item (single digits), and it keeps insertion
            // order for the report.
            return self.entries.remove(index);
        }
        self.unmatched.push((token, UNMATCHED_NO_REQUEST));
        None
    }

    /// Outstanding tokens, in issue order.
    #[must_use]
    pub fn tokens(&self) -> Vec<u64> {
        self.entries.iter().map(|p| p.token).collect()
    }

    /// Outstanding labels, in issue order.
    #[must_use]
    pub fn labels(&self) -> Vec<String> {
        self.entries.iter().map(|p| p.label.clone()).collect()
    }

    /// Number of requests still awaiting their reply.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// `true` when no request is awaiting a reply.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The label of an outstanding request, if still outstanding.
    #[must_use]
    pub fn label_for(&self, token: u64) -> Option<String> {
        self.entries
            .iter()
            .find(|p| p.token == token)
            .map(|p| p.label.clone())
    }

    /// The async call an outstanding token was issued through.
    ///
    /// Must be read **before** the reply is delivered (delivery removes the
    /// entry), which is what gives [`PlayerError::Command`] its `call`.
    #[must_use]
    pub fn call_for(&self, token: u64) -> Option<String> {
        self.entries
            .iter()
            .find(|p| p.token == token)
            .map(|p| p.call.clone())
    }

    /// Drop every outstanding request (teardown: nothing left to correlate).
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Replies seen for tokens we could not attribute.
    #[must_use]
    pub fn unmatched(&self) -> &[(u64, &'static str)] {
        &self.unmatched
    }
}

/// What one [`PlayerSession::pump`] call did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PumpReport {
    /// How many raw events were taken from the link.
    pub drained: usize,
    /// How many wakeup flags the link reported.
    pub wakeups: usize,
    /// How many raw events were ignored as undistinguished event kinds.
    pub other_events: usize,
    /// `true` when the drain stopped because the event budget ran out rather
    /// than because the queue was empty: the caller must pump again.
    pub budget_exhausted: bool,
}

/// Cumulative pump totals for a whole session.
///
/// [`PlayerSession::pump_reports`] is a bounded window; these totals keep the
/// session's whole history reportable without storing every report.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PumpTotals {
    /// How many pump calls the session performed.
    pub pump_calls: usize,
    /// Total raw events taken from the link.
    pub drained: usize,
    /// Total wakeup flags the link reported.
    pub wakeups: usize,
    /// Total raw events ignored as undistinguished event kinds.
    pub other_events: usize,
    /// How many pumps stopped because their budget ran out.
    pub budget_exhaustions: usize,
}

/// When a **control** request issued by [`PlayerSession`] is actually sent to
/// the link.
///
/// Why this exists: a real display-free libmpv run in the headless test environment answered
/// `MPV_ERROR_COMMAND` for every `seek` that was issued soon after the
/// `loadfile` reply / `MPV_EVENT_START_FILE`, and accepted the same controls
/// later in the same item run (the `--probe-json` report of a display-free
/// run records both). **Which** lifecycle
/// event is the earliest one
/// that accepts a control is *not* measured by this repository
/// (`docs/CAPABILITIES.md`), so the real link uses the conservative gate:
/// controls are queued until `MPV_EVENT_FILE_LOADED` has been delivered, then
/// issued in order, and any rejection is still reported by
/// [`PlayerSession::rejected_replies`] instead of being swallowed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlGate {
    /// Send every control request as soon as the caller asks for it.
    ///
    /// This is what [`crate::mock::MockLink`] uses so the mock tests can isolate
    /// request marshalling and token correlation; it is **not** what a real
    /// libmpv session uses.
    Immediate,
    /// Queue control requests until `MPV_EVENT_FILE_LOADED` has been delivered
    /// (with `MPV_EVENT_PLAYBACK_RESTART` as the fallback flush point), then
    /// issue them in issue order.
    FileLoaded,
}

impl ControlGate {
    /// Machine-readable name recorded in probe/smoke reports.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Immediate => "immediate",
            Self::FileLoaded => "file-loaded",
        }
    }
}

/// A correlated command reply that came back with an error.
///
/// This is how a rejected control stays visible: the request *did* reach libmpv
/// and *was* answered, so it is not a [`PlayerError`] from the issuing call,
/// and without recording it a rejected `seek` looks like a successful one
/// (that is how the seek defect survived earlier reviews).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RejectedReply {
    /// Reply token of the request this reply was attributed to.
    pub token: u64,
    /// Our label for that request (`seek 0.5 absolute`, `set volume = double`, …).
    pub command: String,
    /// Header error name (`MPV_ERROR_COMMAND`, …).
    pub error: String,
}

impl RejectedReply {
    /// Machine-readable fact line.
    #[must_use]
    pub fn fact(&self) -> String {
        format!(
            "token={} command={} error={}",
            self.token, self.command, self.error
        )
    }
}

/// One played item and its async control session.
///
/// One session == one `mpv_handle` (see [`crate::mpv_link`]). The UI layer owns a
/// session per item and drives it exclusively through `pump`.
pub struct PlayerSession<L: PlayerLink> {
    link: L,
    tokens: TokenAllocator,
    pending: PendingReplies,
    state: ObservedState,
    events: Vec<PlayerEvent>,
    item: Option<ItemSource>,
    pump_reports: Vec<PumpReport>,
    delivered: usize,
    evicted: usize,
    totals: PumpTotals,
    closed: bool,
    /// Last end-file error, kept so the UI can report *why* an item failed.
    item_error: Option<PlayerError>,
    /// An `MPV_EVENT_START_FILE` was delivered for this item: libmpv has begun
    /// loading it. Whether a control is accepted at this point is **not**
    /// established by this repository - see [`PlayerSession::playback_started`].
    playback_started: bool,
    /// An `MPV_EVENT_FILE_LOADED` was delivered for this item.
    file_loaded: bool,
    /// When this session sends control requests (from the link's policy).
    gate: ControlGate,
    /// Control requests waiting for the gate to open, in issue order.
    deferred: VecDeque<(u64, Request)>,
    /// Correlated replies that came back with an error.
    rejections: Vec<RejectedReply>,
    /// Machine-readable gate/flush facts (`flush requested=.. issued=..`).
    flushes: Vec<String>,
}

impl<L: PlayerLink> PlayerSession<L> {
    /// Open a session for `raw_source`, issue the `loadfile` command and observe
    /// the properties this layer reports.
    ///
    /// # Errors
    ///
    /// [`PlayerError::UnsupportedItem`] for a source this layer cannot play, or
    /// the link's own failure ([`PlayerError::Rejected`], [`PlayerError::InitFailed`]
    /// …) if libmpv refused the setup.
    pub fn open(mut link: L, raw_source: &str) -> Result<Self, PlayerError> {
        let item = ItemSource::parse(raw_source)?;
        let mut tokens = TokenAllocator::new();
        let mut pending = PendingReplies::new();

        for named in props::ALL {
            let token = tokens.alloc();
            link.observe(*named, token)?;
        }
        let token = tokens.alloc();
        let request = controls::load_request(item.load_arg())?;
        pending.insert(token, &request);
        link.request(token, &request)?;
        // The link's control policy is read *before* the link is moved into the
        // session. `loadfile` itself was issued above and is never gated.
        let gate = link.default_control_gate();

        Ok(Self {
            link,
            tokens,
            pending,
            state: ObservedState::new(),
            events: Vec::new(),
            item: Some(item),
            pump_reports: Vec::new(),
            delivered: 0,
            evicted: 0,
            totals: PumpTotals::default(),
            closed: false,
            item_error: None,
            playback_started: false,
            file_loaded: false,
            gate,
            deferred: VecDeque::new(),
            rejections: Vec::new(),
            flushes: Vec::new(),
        })
    }

    /// The link behind this session (read-only; for reports).
    #[must_use]
    pub fn link(&self) -> &L {
        &self.link
    }

    /// The link behind this session, mutably.
    ///
    /// This exists for the display-free smoke session and the control-loop
    /// tests, which queue scripted events on a live session's mock link, so it
    /// is only available for links that advertise scriptability
    /// ([`crate::link::Scriptable`]: [`crate::MockLink`]). Real libmpv
    /// ([`crate::mpv_link::FfiPlayer`]) implements no such marker, so a session
    /// over a live FFI link cannot hand out mutable link access at all.
    #[must_use]
    pub fn link_mut(&mut self) -> &mut L
    where
        L: crate::link::Scriptable,
    {
        &mut self.link
    }

    /// The played item, until the session is closed.
    #[must_use]
    pub fn item(&self) -> Option<&ItemSource> {
        self.item.as_ref()
    }

    /// State built only from observed properties.
    #[must_use]
    pub fn state(&self) -> &ObservedState {
        &self.state
    }

    /// Events delivered so far, in delivery order.
    ///
    /// Bounded window: at most [`MAX_TRACKED_EVENTS`] of the most recent events
    /// are kept, so the index space shifts when older events are evicted. Use
    /// [`PlayerSession::delivered_total`] / [`PlayerSession::evicted_events`] for
    /// the cumulative view and [`PlayerSession::drain_events`] to take the
    /// events a UI loop has not seen yet.
    #[must_use]
    pub fn events(&self) -> &[PlayerEvent] {
        &self.events
    }

    /// How many events this session has delivered in total (including evicted).
    #[must_use]
    pub fn delivered_total(&self) -> usize {
        self.delivered
    }

    /// How many delivered events have already been evicted from the window.
    #[must_use]
    pub fn evicted_events(&self) -> usize {
        self.evicted
    }

    /// Take every currently tracked event and clear the window.
    ///
    /// This is the accessor the UI loop is designed for: it hands over the
    /// events produced since the last drain instead of leaving the session to
    /// accumulate them.
    pub fn drain_events(&mut self) -> Vec<PlayerEvent> {
        std::mem::take(&mut self.events)
    }

    /// Every pump report so far (bounded window of the most recent
    /// [`MAX_TRACKED_PUMP_REPORTS`] pumps; [`PlayerSession::pump_totals`] keeps
    /// the cumulative numbers).
    #[must_use]
    pub fn pump_reports(&self) -> &[PumpReport] {
        &self.pump_reports
    }

    /// Cumulative pump totals over the whole session.
    #[must_use]
    pub fn pump_totals(&self) -> PumpTotals {
        self.totals
    }

    /// Outstanding request tokens, in issue order.
    #[must_use]
    pub fn pending_tokens(&self) -> Vec<u64> {
        self.pending.tokens()
    }

    /// Replies that arrived for tokens we never issued (or that had already been
    /// attributed).
    #[must_use]
    pub fn unmatched_replies(&self) -> &[(u64, &'static str)] {
        self.pending.unmatched()
    }

    /// The end-file failure for this item, if libmpv reported one.
    #[must_use]
    pub fn item_error(&self) -> Option<&PlayerError> {
        self.item_error.as_ref()
    }

    /// `true` once an `MPV_EVENT_START_FILE` has been delivered for this item.
    ///
    /// This is the session-level "libmpv has begun loading the item" signal, and
    /// nothing more. In the test environment's runs the async `loadfile` **reply** was
    /// observed arriving before `START_FILE`, and `seek` requests issued in that
    /// window were answered `MPV_ERROR_COMMAND` (observed in those runs: not a
    /// general libmpv rule). It is **not established** that
    /// `START_FILE` is the point after which controls are accepted, so this layer
    /// does not gate controls on it: see [`PlayerSession::control_gate`] and
    /// `docs/CAPABILITIES.md`.
    #[must_use]
    pub fn playback_started(&self) -> bool {
        self.playback_started
    }

    /// `true` once an `MPV_EVENT_FILE_LOADED` has been delivered.
    ///
    /// Header-declared event id; whether the test environment's libmpv delivers it was
    /// observed in the display-free run (`file-loaded` appears in the event
    /// order), not assumed. This is the point the conservative [`ControlGate`]
    /// releases queued controls at - chosen as a *safe* point, not as a measured
    /// earliest-accepting point.
    #[must_use]
    pub fn file_loaded(&self) -> bool {
        self.file_loaded
    }

    /// `true` after [`PlayerSession::close`] (no further request or pump works).
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// When this session sends control requests.
    #[must_use]
    pub fn control_gate(&self) -> ControlGate {
        self.gate
    }

    /// Change the control gate. Switching to [`ControlGate::Immediate`] sends
    /// everything that is already queued, in issue order.
    pub fn set_control_gate(&mut self, gate: ControlGate) {
        self.gate = gate;
        if gate == ControlGate::Immediate {
            self.flush_controls();
        }
    }

    /// Control requests still waiting for the gate to open, in issue order.
    #[must_use]
    pub fn deferred_controls(&self) -> Vec<String> {
        self.deferred.iter().map(|(_, r)| r.label()).collect()
    }

    /// Machine-readable gate facts ([`ControlGate::label`] plus one line per
    /// flush: requested / sent / link-rejected / gate / `file_loaded`).
    #[must_use]
    pub fn control_flushes(&self) -> &[String] {
        &self.flushes
    }

    /// Correlated replies that came back with an error, in reply order.
    ///
    /// A UI must treat these as playback-control failures: the request reached
    /// libmpv and libmpv said no.
    #[must_use]
    pub fn rejected_replies(&self) -> &[RejectedReply] {
        &self.rejections
    }

    /// Send every queued control request now, in issue order.
    ///
    /// Called automatically when the gate opens ([`ControlGate::FileLoaded`] and
    /// `MPV_EVENT_FILE_LOADED`, or `MPV_EVENT_PLAYBACK_RESTART` as the fallback
    /// if `FILE_LOADED` is never delivered). Exposed so a caller that has its own
    /// evidence that controls are safe can release them early.
    ///
    /// Returns how many queued requests reached the link.
    pub fn flush_controls(&mut self) -> usize {
        if self.deferred.is_empty() || self.closed {
            return 0;
        }
        let queued: Vec<(u64, Request)> = self.deferred.drain(..).collect();
        let mut sent = 0usize;
        let mut failed = 0usize;
        for (token, request) in &queued {
            match self.send(*token, request.clone()) {
                Ok(()) => sent += 1,
                Err(err) => {
                    failed += 1;
                    self.record_flush_error(*token, &request.label(), &err);
                }
            }
        }
        if self.flushes.len() < MAX_TRACKED_FLUSHES {
            self.flushes.push(format!(
                "flush requested={} sent={} link-rejected={} gate={} file_loaded={} playback_started={}",
                queued.len(),
                sent,
                failed,
                self.gate.label(),
                self.file_loaded,
                self.playback_started
            ));
        }
        sent
    }

    /// Issue one control request, honouring the gate.
    ///
    /// A deferred request still gets its reply token immediately, so a caller can
    /// correlate the reply later: the token is only registered as outstanding
    /// when the request is really sent, which keeps [`PlayerEvent::Uncorrelated`]
    /// meaningful (nothing can reply to a request libmpv never saw).
    fn issue_control(&mut self, request: Request) -> Result<u64, PlayerError> {
        if self.closed {
            return Err(PlayerError::Closed);
        }
        let token = self.tokens.alloc();
        if self.gate == ControlGate::FileLoaded && !self.file_loaded {
            self.deferred.push_back((token, request));
            return Ok(token);
        }
        self.send(token, request)?;
        Ok(token)
    }

    /// Send an already-tokenised request to the link and register it.
    fn send(&mut self, token: u64, request: Request) -> Result<(), PlayerError> {
        self.pending.insert(token, &request);
        match self.link.request(token, &request) {
            Ok(()) => Ok(()),
            Err(err) => {
                self.pending.take(token);
                Err(err)
            }
        }
    }

    fn record_flush_error(&mut self, token: u64, label: &str, err: &PlayerError) {
        if self.flushes.len() < MAX_TRACKED_FLUSHES {
            self.flushes.push(format!(
                "flush-error token={} command={} code={}",
                token,
                label,
                err.code()
            ));
        }
    }

    /// Issue a request **immediately**, bypassing the control gate.
    ///
    /// This is the low-level primitive: `loadfile` (which must not be queued) and
    /// the control-loop tests use it. A UI issuing a *control* should use the
    /// control helpers below, which honour [`PlayerSession::control_gate`].
    ///
    /// Returns the reply token the caller can pass to [`PlayerSession::pump_until_reply`].
    ///
    /// # Errors
    ///
    /// [`PlayerError::Closed`] after teardown, or the link's rejection.
    pub fn issue(&mut self, request: Request) -> Result<u64, PlayerError> {
        if self.closed {
            return Err(PlayerError::Closed);
        }
        let token = self.tokens.alloc();
        self.pending.insert(token, &request);
        match self.link.request(token, &request) {
            Ok(()) => Ok(token),
            Err(err) => {
                // The request never became outstanding: drop it from the pending
                // set so a stale reply can never be attributed to it.
                self.pending.take(token);
                Err(err)
            }
        }
    }

    /// Pause / resume the current item (gated control request).
    pub fn set_pause(&mut self, paused: bool) -> Result<u64, PlayerError> {
        self.issue_control(controls::pause_request(paused))
    }

    /// Seek to `target` seconds (`SeekMode::Relative` for an offset).
    ///
    /// Gated: with the conservative gate the request is queued until
    /// `MPV_EVENT_FILE_LOADED`, so a UI may call this as soon as the user asks
    /// and the layer sends it when libmpv is in a state this layer assumes
    /// accepts it. If libmpv still rejects it, the reply is recorded in
    /// [`PlayerSession::rejected_replies`].
    pub fn seek(&mut self, target: f64, mode: SeekMode) -> Result<u64, PlayerError> {
        self.issue_control(controls::seek_request(target, mode)?)
    }

    /// Set output volume (`0.0..=controls::MAX_VOLUME`).
    pub fn set_volume(&mut self, percent: f64) -> Result<u64, PlayerError> {
        self.issue_control(controls::volume_request(percent)?)
    }

    /// Select an audio track (`TrackSelection::None` mutes).
    pub fn select_audio(&mut self, selection: &TrackSelection) -> Result<u64, PlayerError> {
        self.issue_control(select_audio(selection)?)
    }

    /// Select a subtitle track (`TrackSelection::None` hides subtitles).
    pub fn select_subtitle(&mut self, selection: &TrackSelection) -> Result<u64, PlayerError> {
        self.issue_control(select_subtitle(selection)?)
    }

    /// Stop playback of the current item.
    ///
    /// Never queued: a stop is issued even if the item never loaded, so a stuck
    /// session is always closable.
    pub fn stop(&mut self) -> Result<u64, PlayerError> {
        self.issue(controls::stop_request())
    }

    /// Observe one extra property (for code that later decodes node values).
    pub fn observe(&mut self, property: Named) -> Result<u64, PlayerError> {
        if self.closed {
            return Err(PlayerError::Closed);
        }
        let token = self.tokens.alloc();
        self.link.observe(property, token)?;
        Ok(token)
    }

    /// Drain the link's event queue: non-blocking, bounded, no busy-wait.
    ///
    /// `budget` caps how many raw events one call may take. `budget == 0` still
    /// reads the wakeup flag (so a UI loop can observe wakeups without taking
    /// events).
    pub fn pump(&mut self, budget: usize) -> PumpReport {
        let mut report = PumpReport::default();
        if self.closed {
            self.record_pump(&report);
            return report;
        }
        if self.link.took_wakeup() {
            report.wakeups += 1;
            self.push_event(PlayerEvent::Woken);
        }
        let mut taken = 0usize;
        while taken < budget {
            match self.link.next_event() {
                None => break,
                Some(raw) => {
                    taken += 1;
                    report.drained += 1;
                    self.deliver(raw, &mut report);
                }
            }
        }
        // Conservative: the loop used its whole budget, so the caller must pump
        // again to be sure the queue is empty. (Peeking would consume an event.)
        report.budget_exhausted = budget > 0 && taken == budget;
        self.record_pump(&report);
        report
    }

    /// Drain until the reply for `token` arrives, or the drain budget runs out.
    ///
    /// Bounded on purpose: a UI loop must be able to call this every frame
    /// without risking an unbounded wait. An unacked command is an error, not an
    /// infinite wait.
    ///
    /// # Errors
    ///
    /// [`PlayerError::NoReply`] when `max_events` drains produced no reply for
    /// `token`, [`PlayerError::Command`] when the reply carries a libmpv error,
    /// [`PlayerError::Closed`] after teardown.
    pub fn pump_until_reply(
        &mut self,
        token: u64,
        max_events: usize,
    ) -> Result<PlayerEvent, PlayerError> {
        if self.closed {
            return Err(PlayerError::Closed);
        }
        let mut report = PumpReport::default();
        let label = self
            .pending
            .label_for(token)
            .unwrap_or_else(|| "no outstanding request with this token".to_owned());
        let mut failure: Option<(String, String, i32, String)> = None;
        let mut matched: Option<PlayerEvent> = None;
        for _ in 0..max_events {
            match self.link.next_event() {
                Some(raw) => {
                    // Read the async call **before** delivery removes the entry:
                    // this is what makes `PlayerError::Command.call` a fact
                    // rather than a guess about which call failed.
                    let issued_call = self.pending.call_for(raw.reply_userdata);
                    report.drained += 1;
                    self.deliver(raw, &mut report);
                    if let Some(event) = self.events.last().cloned() {
                        let reply = match &event {
                            PlayerEvent::CommandReply {
                                token: got,
                                command,
                                ok,
                                error,
                            } if *got == token => Some((*ok, command.clone(), error.clone())),
                            _ => None,
                        };
                        if let Some((ok, command, error)) = reply {
                            if ok {
                                matched = Some(event);
                                break;
                            }
                            let (code, name) = match &error {
                                Some(err) => (err.code, err.name.clone()),
                                None => {
                                    let code = crate::mock_error_code("MPV_ERROR_COMMAND")
                                        .unwrap_or(crate::mock_success_code());
                                    (code, crate::events::error_name(code).to_owned())
                                }
                            };
                            let call = issued_call.unwrap_or_else(|| {
                                // The correlated entry was already gone; say so
                                // rather than inventing which call it was.
                                "unknown-async-call".to_owned()
                            });
                            failure = Some((call, command, code, name));
                            break;
                        }
                    }
                }
                None => {
                    // Nothing queued: a wakeup is the only reason to keep going
                    // (never a sleep, never a spin on an empty queue).
                    if self.link.took_wakeup() {
                        report.wakeups += 1;
                        self.push_event(PlayerEvent::Woken);
                    } else {
                        break;
                    }
                }
            }
        }
        let drains = report.drained;
        self.record_pump(&report);
        if let Some(event) = matched {
            return Ok(event);
        }
        if let Some((call, command, code, name)) = failure {
            return Err(PlayerError::Command {
                call,
                command,
                code,
                name,
            });
        }
        Err(PlayerError::NoReply {
            command: label,
            token,
            drains,
        })
    }

    /// Tear the session down: `stop` is issued best-effort, then no further
    /// request or pump is allowed.
    ///
    /// # Errors
    ///
    /// [`PlayerError::Closed`] when the session was already closed.
    pub fn close(&mut self) -> Result<(), PlayerError> {
        if self.closed {
            return Err(PlayerError::Closed);
        }
        let stop = controls::stop_request();
        let token = self.tokens.alloc();
        // Best effort: a failure here is recorded by dropping the pending entry
        // rather than masking the teardown the caller asked for.
        self.pending.insert(token, &stop);
        if self.link.request(token, &stop).is_err() {
            self.pending.take(token);
        }
        self.closed = true;
        self.pending.clear();
        if !self.deferred.is_empty() && self.flushes.len() < MAX_TRACKED_FLUSHES {
            self.flushes.push(format!(
                "flush dropped-deferred={} reason=close",
                self.deferred.len()
            ));
        }
        self.deferred.clear();
        // One session == one played item: after teardown there is no item left
        // to report, and the `mpv_handle` it was played with is gone.
        self.item = None;
        self.push_event(PlayerEvent::Closed);
        Ok(())
    }

    /// Deliver one event into the bounded window, keeping cumulative totals.
    fn push_event(&mut self, event: PlayerEvent) {
        self.delivered += 1;
        // Bounded window: evict the oldest event (n <= MAX_TRACKED_EVENTS, so
        // the O(n) shift is bounded by design) rather than growing without cap.
        if self.events.len() >= MAX_TRACKED_EVENTS {
            self.events.remove(0);
            self.evicted += 1;
        }
        self.events.push(event);
    }

    /// Record one pump report in the bounded window and add it to the totals.
    fn record_pump(&mut self, report: &PumpReport) {
        self.totals.pump_calls += 1;
        self.totals.drained += report.drained;
        self.totals.wakeups += report.wakeups;
        self.totals.other_events += report.other_events;
        if report.budget_exhausted {
            self.totals.budget_exhaustions += 1;
        }
        if self.pump_reports.len() >= MAX_TRACKED_PUMP_REPORTS {
            self.pump_reports.remove(0);
        }
        self.pump_reports.push(report.clone());
    }

    /// Translate one raw event into a [`PlayerEvent`] and apply its state effect.
    fn deliver(&mut self, raw: RawEvent, report: &mut PumpReport) {
        match raw.kind {
            // libmpv answers `mpv_command_async` and `mpv_set_property_async`
            // with different event ids (`client.h:1262-1272`); both carry
            // `reply_userdata`, so both must go through the same correlation or
            // a rejected `set ...` control would be invisible to the UI.
            EventKind::CommandReply | EventKind::SetPropertyReply => self.deliver_reply(raw),
            EventKind::PropertyChange => {
                if let Some(observed) = raw.property {
                    self.apply_property(&observed);
                    self.push_event(PlayerEvent::PropertyChange { observed });
                } else {
                    report.other_events += 1;
                }
            }
            EventKind::EndFile => {
                // The item is over: a control queued for it can never be useful
                // and must not leak into the next item's session window.
                if !self.deferred.is_empty() && self.flushes.len() < MAX_TRACKED_FLUSHES {
                    self.flushes.push(format!(
                        "flush dropped-deferred={} reason=end-file",
                        self.deferred.len()
                    ));
                }
                self.deferred.clear();
                let (reason, code) = raw.end_file.unwrap_or((EndReason::Unknown { raw: 0 }, 0));
                let error = if crate::events::is_success(code) {
                    None
                } else {
                    Some(crate::events::MpvError::from_code(code))
                };
                if reason == EndReason::Error {
                    self.item_error = Some(PlayerError::ItemFailed {
                        item: self
                            .item
                            .as_ref()
                            .map(ItemSource::load_arg)
                            .unwrap_or("<none>")
                            .to_owned(),
                        reason: reason.label().to_owned(),
                        code,
                        name: crate::events::error_name(code).to_owned(),
                    });
                }
                self.push_event(PlayerEvent::EndFile { reason, error });
            }
            EventKind::StartFile => {
                self.playback_started = true;
                self.push_event(PlayerEvent::ItemStarted);
            }
            EventKind::FileLoaded => {
                self.file_loaded = true;
                self.push_event(PlayerEvent::FileLoaded);
                // The conservative gate opens here: everything the UI asked for
                // while the item was still loading goes out now, in issue order.
                self.flush_controls();
            }
            EventKind::PlaybackRestart => {
                self.push_event(PlayerEvent::PlaybackRestarted);
                // Fallback for a stream that restarts playback without libmpv
                // ever delivering FILE_LOADED in the session window: controls
                // must not stay queued forever.
                self.flush_controls();
            }
            EventKind::QueueOverflow => self.push_event(PlayerEvent::QueueOverflow),
            EventKind::Other { .. } => report.other_events += 1,
        }
    }

    /// Correlate one async reply (`MPV_EVENT_COMMAND_REPLY` or
    /// `MPV_EVENT_SET_PROPERTY_REPLY`) with the outstanding request that carries
    /// its `reply_userdata`.
    fn deliver_reply(&mut self, raw: RawEvent) {
        let correlated = self.pending.take(raw.reply_userdata);
        let ok = crate::events::is_success(raw.error);
        let error = if ok {
            None
        } else {
            Some(crate::events::MpvError::from_code(raw.error))
        };
        let event = match correlated {
            Some(pending) => {
                if !ok && self.rejections.len() < MAX_TRACKED_REJECTIONS {
                    self.rejections.push(RejectedReply {
                        token: pending.token,
                        command: pending.label.clone(),
                        error: error
                            .as_ref()
                            .map(|e| e.name.clone())
                            .unwrap_or_else(|| "<unnamed>".to_owned()),
                    });
                }
                PlayerEvent::CommandReply {
                    token: raw.reply_userdata,
                    command: pending.label,
                    ok,
                    error,
                }
            }
            None => PlayerEvent::Uncorrelated {
                token: raw.reply_userdata,
                reason: if raw.reply_userdata == NO_TOKEN {
                    UNMATCHED_NO_TOKEN
                } else {
                    UNMATCHED_NO_REQUEST
                },
            },
        };
        self.push_event(event);
    }

    /// Property observations are the *only* source of state.
    fn apply_property(&mut self, observed: &Observed) {
        self.state.apply(observed);
    }
}
