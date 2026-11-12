//! The scripted mock link: a [`PlayerLink`] that records calls instead of
//! calling libmpv.
//!
//! This is what the control-loop tests and `chewvy-cli --smoke-no-display` drive,
//! and it is deliberately *not* a claim about libmpv: every recorded call is
//! reported as "would have been sent". Real libmpv behaviour is established by
//! [`crate::probe`] and recorded in the `--probe-json` report.
//!
//! The mock records the exact argument list / property format it was given, so a
//! test can assert the wrapper's wire shape (`loadfile <uri> replace`,
//! `seek 30 relative`, `set volume = double`) rather than only that "a command
//! happened".

use std::collections::VecDeque;
use std::sync::mpsc::{channel, Receiver, Sender};

use crate::commands::{Named, PropertyValue};
use crate::events::{EndReason, EventKind, Observed, PlayerError};
use crate::link::{PlayerLink, RawEvent, Request, Scriptable};

/// Version string the mock reports (clearly not a libmpv version).
pub const MOCK_VERSION: &str = "mock-link-not-libmpv";

/// Header event id for a kind, when the header constants exist.
///
/// Without `libmpv-ffi` there is no header constant to use, so ids are `0` and
/// only [`RawEvent::kind`] is meaningful (the mock never relies on the id).
#[must_use]
pub fn id_for(kind: EventKind) -> u32 {
    #[cfg(feature = "libmpv-ffi")]
    {
        use crate::ffi as mpv;
        match kind {
            EventKind::CommandReply => mpv::mpv_event_id_MPV_EVENT_COMMAND_REPLY,
            EventKind::SetPropertyReply => mpv::mpv_event_id_MPV_EVENT_SET_PROPERTY_REPLY,
            EventKind::PropertyChange => mpv::mpv_event_id_MPV_EVENT_PROPERTY_CHANGE,
            EventKind::EndFile => mpv::mpv_event_id_MPV_EVENT_END_FILE,
            EventKind::StartFile => mpv::mpv_event_id_MPV_EVENT_START_FILE,
            EventKind::FileLoaded => mpv::mpv_event_id_MPV_EVENT_FILE_LOADED,
            EventKind::PlaybackRestart => mpv::mpv_event_id_MPV_EVENT_PLAYBACK_RESTART,
            EventKind::QueueOverflow => mpv::mpv_event_id_MPV_EVENT_QUEUE_OVERFLOW,
            EventKind::Other { raw } => raw,
        }
    }
    #[cfg(not(feature = "libmpv-ffi"))]
    {
        match kind {
            EventKind::Other { raw } => raw,
            _ => 0,
        }
    }
}

/// A scripted `MPV_EVENT_COMMAND_REPLY` for `token`.
#[must_use]
pub fn reply_event(token: u64, error_code: i32) -> RawEvent {
    RawEvent {
        event_id: id_for(EventKind::CommandReply),
        kind: EventKind::CommandReply,
        error: error_code,
        reply_userdata: token,
        property: None,
        end_file: None,
    }
}

/// A scripted `MPV_EVENT_SET_PROPERTY_REPLY` for `token`.
///
/// This is the reply libmpv gives `mpv_set_property_async` (`client.h:1095-1110,
/// 1266-1269`); the mock scripts it with the header id so a test can prove the
/// engine correlates id 4 with the outstanding `set ...` request instead of
/// dropping it as an unknown event.
#[must_use]
pub fn set_property_reply_event(token: u64, error_code: i32) -> RawEvent {
    RawEvent {
        event_id: id_for(EventKind::SetPropertyReply),
        kind: EventKind::SetPropertyReply,
        error: error_code,
        reply_userdata: token,
        property: None,
        end_file: None,
    }
}

/// A scripted `MPV_EVENT_PROPERTY_CHANGE` for an observed property.
#[must_use]
pub fn property_event(name: &str, value: PropertyValue) -> RawEvent {
    let observed = Observed {
        name: name.to_owned(),
        value,
        header_backed: crate::commands::lookup_property(name).map(|n| n.is_header_backed()),
    };
    RawEvent {
        event_id: id_for(EventKind::PropertyChange),
        kind: EventKind::PropertyChange,
        error: crate::mock_success_code(),
        reply_userdata: 0,
        property: Some(observed),
        end_file: None,
    }
}

/// A scripted `MPV_EVENT_END_FILE`.
#[must_use]
pub fn end_file_event(reason: EndReason, error_code: i32) -> RawEvent {
    RawEvent {
        event_id: id_for(EventKind::EndFile),
        kind: EventKind::EndFile,
        error: error_code,
        reply_userdata: 0,
        property: None,
        end_file: Some((reason, error_code)),
    }
}

/// A scripted `MPV_EVENT_QUEUE_OVERFLOW`.
#[must_use]
pub fn queue_overflow_event() -> RawEvent {
    RawEvent {
        event_id: id_for(EventKind::QueueOverflow),
        kind: EventKind::QueueOverflow,
        error: crate::mock_success_code(),
        reply_userdata: 0,
        property: None,
        end_file: None,
    }
}

/// A scripted item-lifecycle event (`START_FILE` / `FILE_LOADED` /
/// `PLAYBACK_RESTART`).
///
/// These events are what the control gate reacts to, so the gate tests script
/// them explicitly instead of assuming a link emits them. Like every other
/// scripted event here, the event id is the header id when the `libmpv-ffi`
/// feature is on and `0` otherwise - it is a mock, not libmpv evidence.
#[must_use]
pub fn lifecycle_event(kind: EventKind) -> RawEvent {
    RawEvent {
        event_id: id_for(kind),
        kind,
        error: crate::mock_success_code(),
        reply_userdata: 0,
        property: None,
        end_file: None,
    }
}

/// A scripted `MPV_EVENT_START_FILE`.
#[must_use]
pub fn start_file_event() -> RawEvent {
    lifecycle_event(EventKind::StartFile)
}

/// A scripted `MPV_EVENT_FILE_LOADED` (the point the conservative gate opens at).
#[must_use]
pub fn file_loaded_event() -> RawEvent {
    lifecycle_event(EventKind::FileLoaded)
}

/// A scripted `MPV_EVENT_PLAYBACK_RESTART` (the gate's fallback flush point).
#[must_use]
pub fn playback_restart_event() -> RawEvent {
    lifecycle_event(EventKind::PlaybackRestart)
}

/// A scripted event of a kind this layer does not distinguish.
#[must_use]
pub fn other_event(event_id: u32) -> RawEvent {
    RawEvent {
        event_id,
        kind: EventKind::Other { raw: event_id },
        error: crate::mock_success_code(),
        reply_userdata: 0,
        property: None,
        end_file: None,
    }
}

/// A [`PlayerLink`] that does no I/O: it records what it was asked to do and
/// replays a scripted event queue.
#[derive(Default)]
pub struct MockLink {
    /// `Some` when an implementation is pretending to be real libmpv (used by
    /// the probe-shaped smoke report to show the version it *would* have used).
    version: Option<String>,
    options: Vec<(String, String)>,
    /// Machine-readable log of every call, in call order.
    calls: Vec<String>,
    /// `observe` requests: name, requested format, token.
    observations: Vec<(String, String, u64)>,
    queue: VecDeque<RawEvent>,
    wakeup_pending: bool,
    initialized: bool,
    closed: bool,
    /// When set, the next `request` fails with this code instead of queuing.
    reject: Option<i32>,
    /// When armed, `next_event` **blocks** until someone sends on the channel.
    /// This deliberately violates the [`PlayerLink`] non-blocking contract and
    /// only exists so a test can prove the engine's pump is what keeps the real
    /// link's non-blocking behaviour usable from a UI thread.
    park: Option<Receiver<()>>,
    /// How many `next_event` calls were made (evidence a drain loop terminated).
    drains: usize,
}

impl MockLink {
    /// A fresh mock with the honest [`MOCK_VERSION`] label.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A mock that reports `version` as its libmpv version (only for smoke
    /// reports that want to show the build-time pkg-config fact alongside the
    /// scripted session; the session itself is still mock).
    #[must_use]
    pub fn with_version(version: &str) -> Self {
        Self {
            version: Some(version.to_owned()),
            ..Self::default()
        }
    }

    /// Script a queue of events, replayed in order by `next_event`.
    #[must_use]
    pub fn with_script(events: Vec<RawEvent>) -> Self {
        Self {
            queue: events.into_iter().collect(),
            ..Self::default()
        }
    }

    /// Append one scripted event.
    pub fn push_event(&mut self, event: RawEvent) {
        self.queue.push_back(event);
    }

    /// Make the next [`PlayerLink::request`] fail with `code` (simulates libmpv
    /// rejecting a command before it was queued).
    pub fn reject_next(&mut self, code: i32) {
        self.reject = Some(code);
    }

    /// Arm the blocking mode and hand back the sender that releases it.
    ///
    /// Only used by the negative "the wrapper never blocks" test.
    pub fn arm_blocking(&mut self) -> Sender<()> {
        let (tx, rx) = channel();
        self.park = Some(rx);
        tx
    }

    /// Stop the blocking mode.
    pub fn disarm_blocking(&mut self) {
        self.park = None;
    }

    /// Pretend the wakeup callback fired.
    pub fn simulate_wakeup(&mut self) {
        self.wakeup_pending = true;
    }

    /// Every recorded call, in order, machine-readable.
    #[must_use]
    pub fn calls(&self) -> &[String] {
        &self.calls
    }

    /// Recorded `observe` requests (name, requested format, token).
    #[must_use]
    pub fn observations(&self) -> &[(String, String, u64)] {
        &self.observations
    }

    /// Recorded options (name, value).
    #[must_use]
    pub fn options(&self) -> &[(String, String)] {
        &self.options
    }

    /// How many `next_event` calls happened.
    #[must_use]
    pub fn drains(&self) -> usize {
        self.drains
    }

    /// Events still queued (a pump that leaves events behind is a bug worth
    /// asserting on).
    #[must_use]
    pub fn queued(&self) -> usize {
        self.queue.len()
    }

    /// `true` when a call with this exact machine-readable text was recorded.
    #[must_use]
    pub fn called(&self, call: &str) -> bool {
        self.calls.iter().any(|c| c == call)
    }

    fn record(&mut self, call: String) {
        self.calls.push(call);
    }
}

/// The mock is the only link in this crate that advertises scriptability, so
/// it is the only link a [`crate::engine::PlayerSession`] will hand out mutably.
/// Real libmpv ([`crate::mpv_link::FfiPlayer`]) deliberately implements nothing
/// here.
impl Scriptable for MockLink {}

impl PlayerLink for MockLink {
    fn set_option(&mut self, name: &str, value: &str) -> Result<(), PlayerError> {
        if self.initialized {
            return Err(PlayerError::Rejected {
                call: "mpv_set_option_string".to_owned(),
                command: name.to_owned(),
                code: crate::mock_error_code("MPV_ERROR_OPTION_NOT_FOUND").unwrap_or(0),
                name: crate::events::error_name(
                    crate::mock_error_code("MPV_ERROR_OPTION_NOT_FOUND").unwrap_or(0),
                )
                .to_owned(),
            });
        }
        self.options.push((name.to_owned(), value.to_owned()));
        self.record(format!("option {name}={value}"));
        Ok(())
    }

    fn initialize(&mut self) -> Result<(), PlayerError> {
        if self.closed {
            return Err(PlayerError::Closed);
        }
        self.initialized = true;
        self.record("initialize".to_owned());
        Ok(())
    }

    fn request(&mut self, token: u64, request: &Request) -> Result<(), PlayerError> {
        if self.closed {
            return Err(PlayerError::Closed);
        }
        if let Some(code) = self.reject.take() {
            return Err(PlayerError::Rejected {
                // Same entry-point name [`crate::mpv_link`] reports for a real
                // rejection: the async call this request maps to (`Request::call_name`).
                call: request.call_name().to_owned(),
                command: request.label(),
                code,
                name: crate::events::error_name(code).to_owned(),
            });
        }
        match request {
            Request::Command { name, args } => {
                let mut line = format!("command token={token} {}", name.name);
                for arg in args {
                    line.push(' ');
                    line.push_str(arg);
                }
                self.record(line);
            }
            Request::SetProperty { name, value } => {
                self.record(format!(
                    "set-property token={token} {} {} {}",
                    name.name,
                    crate::link::value_format(value),
                    match value {
                        PropertyValue::Flag(b) => (if *b { "true" } else { "false" }).to_owned(),
                        PropertyValue::Int64(i) => i.to_string(),
                        PropertyValue::Double(d) => d.to_string(),
                        PropertyValue::Str(s) => s.clone(),
                        PropertyValue::None => "none".to_owned(),
                    }
                ));
            }
        }
        Ok(())
    }

    fn observe(&mut self, property: Named, token: u64) -> Result<(), PlayerError> {
        if self.closed {
            return Err(PlayerError::Closed);
        }
        let format = crate::state::observed_format(property);
        self.observations
            .push((property.name.to_owned(), format.to_string(), token));
        self.record(format!(
            "observe {} {} token={token}",
            property.name, format
        ));
        Ok(())
    }

    fn next_event(&mut self) -> Option<RawEvent> {
        self.drains += 1;
        if let Some(rx) = &self.park {
            // Deliberate contract violation, armed only by the negative test:
            // `recv()` blocks. A real link must never do this (`mpv_wait_event(h, 0)`).
            return rx.recv().ok().map(|()| other_event(u32::MAX));
        }
        self.queue.pop_front()
    }

    fn took_wakeup(&mut self) -> bool {
        let woke = self.wakeup_pending;
        self.wakeup_pending = false;
        woke
    }

    fn libmpv_version(&self) -> String {
        self.version
            .clone()
            .unwrap_or_else(|| MOCK_VERSION.to_owned())
    }

    fn is_closed(&self) -> bool {
        self.closed
    }
}
