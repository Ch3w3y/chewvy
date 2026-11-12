//! The backend boundary of the player layer.
//!
//! [`PlayerLink`] is what [`crate::engine`] talks to; nothing in the engine,
//! commands, state, controls or tracks module knows whether it is driving real
//! libmpv ([`crate::mpv_link`]) or the scripted mock ([`crate::mock`]). That is
//! what makes the async control loop testable without libmpv *and* what lets the
//! `--smoke-no-display` report show the exact calls that would have been sent.
//!
//! This module is deliberately feature-free: it compiles with or without the
//! `libmpv-ffi` feature, so the control layer is testable on a machine that has
//! no libmpv at all (the `PlayerLink` surface is *claimed* to match real libmpv
//! here; whether a real libmpv 2.5.0 accepted our names/calls is only a fact
//! once a display-free probe run has actually been captured - see
//! [`crate::probe`, which exists to produce that record]).

use crate::commands::{Named, PropertyValue};
use crate::events::PlayerError;
use crate::events::{EndReason, EventKind, Observed};

/// A raw event read out of an `mpv_event`, decoded into safe Rust.
///
/// The fields mirror `mpv_event` (`event_id`, `error`, `reply_userdata`, and the
/// `data` payload decoded for the two event kinds this layer cares about). No
/// libmpv pointer survives this struct: everything is copied into owned Rust
/// before the event is returned.
#[derive(Clone, Debug, PartialEq)]
pub struct RawEvent {
    /// `mpv_event_id` value.
    pub event_id: u32,
    /// What kind of event this is (mapped from the header event-id constants).
    pub kind: EventKind,
    /// `mpv_event.error` (`MPV_ERROR_SUCCESS` when nothing failed).
    pub error: i32,
    /// `mpv_event.reply_userdata` - the token we passed to the async call.
    pub reply_userdata: u64,
    /// Decoded `mpv_event_property` for `MPV_EVENT_PROPERTY_CHANGE`.
    pub property: Option<Observed>,
    /// Decoded `mpv_event_end_file` for `MPV_EVENT_END_FILE` (reason + error code).
    pub end_file: Option<(EndReason, i32)>,
}

/// What the engine needs from a player backend.
///
/// Implemented by [`crate::mpv_link::FfiPlayer`] (real libmpv) and by
/// [`crate::mock::MockLink`] (display-free tests and the smoke session).
///
/// # Contract every implementation must satisfy
///
/// * [`PlayerLink::next_event`] **must not block** (libmpv: `mpv_wait_event(h, 0)`).
///   The engine drains it in a loop on the caller's thread; a blocking
///   implementation would stall the UI thread and is a contract violation
///   (proven for the real link by the probe run, and asserted for the mock).
/// * [`PlayerLink::took_wakeup`] is edge-triggered: `true` exactly once per
///   wakeup-callback invocation since the previous call, so a drain loop can
///   report wakeups without polling on a timer.
pub trait PlayerLink {
    /// Set a libmpv option before [`PlayerLink::initialize`].
    fn set_option(&mut self, name: &str, value: &str) -> Result<(), PlayerError>;
    /// `mpv_initialize` (or the mock's equivalent state change).
    fn initialize(&mut self) -> Result<(), PlayerError>;
    /// Issue `request` asynchronously, tagging the reply with `token`.
    fn request(&mut self, token: u64, request: &Request) -> Result<(), PlayerError>;
    /// `mpv_observe_property(name, token)`.
    fn observe(&mut self, property: Named, token: u64) -> Result<(), PlayerError>;
    /// Pop one queued event, or `None` when the queue is empty (non-blocking).
    fn next_event(&mut self) -> Option<RawEvent>;
    /// `true` exactly once for each wakeup-callback invocation since the last call.
    fn took_wakeup(&mut self) -> bool;
    /// libmpv version this link is talking to (build-time pkg-config fact).
    fn libmpv_version(&self) -> String;
    /// `true` after teardown, when the handle is gone.
    fn is_closed(&self) -> bool;
    /// How a [`crate::engine::PlayerSession`] on this link should order its
    /// control requests.
    ///
    /// The default is [`crate::engine::ControlGate::Immediate`] (send when
    /// asked), which is what a scripted mock does. The real libmpv link overrides
    /// this to [`crate::engine::ControlGate::FileLoaded`]: this repository
    /// measured that controls issued early in an item run were rejected
    /// (`MPV_ERROR_COMMAND`) without establishing the earliest event that makes
    /// them acceptable, so a real session must queue them instead of sending a
    /// control it has no evidence libmpv will accept.
    fn default_control_gate(&self) -> crate::engine::ControlGate {
        crate::engine::ControlGate::Immediate
    }
}

/// Re-export so implementations can name the request type without importing
/// `commands` twice (and so `mock`/`engine`/`probe` agree on one type).
pub use crate::commands::Request;

/// Marker for links that can be **scripted** (events injected, wakeup simulated
/// or a blocking mode armed).
///
/// Only such links may be handed out mutably by
/// [`crate::engine::PlayerSession::link_mut`]. [`crate::MockLink`] advertises
/// it; real libmpv ([`crate::mpv_link::FfiPlayer`]) does not and therefore
/// cannot give a caller mutable access to a live handle session.
pub trait Scriptable {}

/// The property format a set-property request would use (reported by the mock
/// and the probe so a run can show what *was* sent, not what was intended).
#[must_use]
pub fn value_format(value: &PropertyValue) -> crate::commands::ValueKind {
    value.kind()
}
