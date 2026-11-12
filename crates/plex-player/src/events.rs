//! The typed event/error surface the player layer gives the UI.
//!
//! The UI never asks libmpv for state mid-frame: it consumes these events, which
//! come from the async command replies and the observed properties only.

use thiserror::Error;

/// A libmpv error code with the header-declared name for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MpvError {
    /// Numeric `mpv_error` value.
    pub code: i32,
    /// Name from the `mpv_error_MPV_ERROR_*` constants in the installed headers.
    pub name: String,
}

impl MpvError {
    /// Build an error from a raw code, naming it from the header constants.
    #[must_use]
    pub fn from_code(code: i32) -> Self {
        Self {
            code,
            name: error_name(code).to_owned(),
        }
    }
}

/// Name of an `mpv_error` code.
///
/// With the `libmpv-ffi` feature this maps the numeric code through the
/// `mpv_error_MPV_ERROR_*` constants bindgen read out of
/// `/usr/include/mpv/client.h`, so the mapping is header-backed, not invented.
/// Without the feature no header constants exist to compare against and every
/// code is honestly reported as not header-backed.
#[must_use]
pub fn error_name(code: i32) -> &'static str {
    #[cfg(feature = "libmpv-ffi")]
    {
        use crate::ffi as mpv;
        match code {
            mpv::mpv_error_MPV_ERROR_SUCCESS => "MPV_ERROR_SUCCESS",
            mpv::mpv_error_MPV_ERROR_EVENT_QUEUE_FULL => "MPV_ERROR_EVENT_QUEUE_FULL",
            mpv::mpv_error_MPV_ERROR_NOMEM => "MPV_ERROR_NOMEM",
            mpv::mpv_error_MPV_ERROR_UNINITIALIZED => "MPV_ERROR_UNINITIALIZED",
            mpv::mpv_error_MPV_ERROR_INVALID_PARAMETER => "MPV_ERROR_INVALID_PARAMETER",
            mpv::mpv_error_MPV_ERROR_OPTION_NOT_FOUND => "MPV_ERROR_OPTION_NOT_FOUND",
            mpv::mpv_error_MPV_ERROR_OPTION_FORMAT => "MPV_ERROR_OPTION_FORMAT",
            mpv::mpv_error_MPV_ERROR_OPTION_ERROR => "MPV_ERROR_OPTION_ERROR",
            mpv::mpv_error_MPV_ERROR_PROPERTY_NOT_FOUND => "MPV_ERROR_PROPERTY_NOT_FOUND",
            mpv::mpv_error_MPV_ERROR_PROPERTY_FORMAT => "MPV_ERROR_PROPERTY_FORMAT",
            mpv::mpv_error_MPV_ERROR_PROPERTY_UNAVAILABLE => "MPV_ERROR_PROPERTY_UNAVAILABLE",
            mpv::mpv_error_MPV_ERROR_PROPERTY_ERROR => "MPV_ERROR_PROPERTY_ERROR",
            mpv::mpv_error_MPV_ERROR_COMMAND => "MPV_ERROR_COMMAND",
            mpv::mpv_error_MPV_ERROR_LOADING_FAILED => "MPV_ERROR_LOADING_FAILED",
            mpv::mpv_error_MPV_ERROR_AO_INIT_FAILED => "MPV_ERROR_AO_INIT_FAILED",
            mpv::mpv_error_MPV_ERROR_VO_INIT_FAILED => "MPV_ERROR_VO_INIT_FAILED",
            mpv::mpv_error_MPV_ERROR_NOTHING_TO_PLAY => "MPV_ERROR_NOTHING_TO_PLAY",
            mpv::mpv_error_MPV_ERROR_UNKNOWN_FORMAT => "MPV_ERROR_UNKNOWN_FORMAT",
            mpv::mpv_error_MPV_ERROR_UNSUPPORTED => "MPV_ERROR_UNSUPPORTED",
            mpv::mpv_error_MPV_ERROR_NOT_IMPLEMENTED => "MPV_ERROR_NOT_IMPLEMENTED",
            mpv::mpv_error_MPV_ERROR_GENERIC => "MPV_ERROR_GENERIC",
            _ => "MPV_ERROR_NOT_IN_HEADERS",
        }
    }
    #[cfg(not(feature = "libmpv-ffi"))]
    {
        let _ = code;
        "MPV_ERROR_NOT_IN_HEADERS"
    }
}

/// Why the current item stopped playing (`mpv_event_end_file.reason`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndReason {
    /// `MPV_END_FILE_REASON_EOF`.
    Eof,
    /// `MPV_END_FILE_REASON_STOP` (our own `stop`/teardown).
    Stop,
    /// `MPV_END_FILE_REASON_QUIT`.
    Quit,
    /// `MPV_END_FILE_REASON_ERROR` - libmpv could not play the item.
    Error,
    /// `MPV_END_FILE_REASON_REDIRECT`.
    Redirect,
    /// A reason value the installed headers do not declare.
    Unknown {
        /// Raw `mpv_end_file_reason` value.
        raw: u32,
    },
}

impl EndReason {
    /// Map a raw header enum value.
    ///
    /// With `libmpv-ffi` off there are no header constants to compare against,
    /// so every value is reported as [`EndReason::Unknown`] rather than guessed.
    #[must_use]
    pub fn from_raw(raw: u32) -> Self {
        #[cfg(feature = "libmpv-ffi")]
        {
            use crate::ffi as mpv;
            match raw {
                mpv::mpv_end_file_reason_MPV_END_FILE_REASON_EOF => Self::Eof,
                mpv::mpv_end_file_reason_MPV_END_FILE_REASON_STOP => Self::Stop,
                mpv::mpv_end_file_reason_MPV_END_FILE_REASON_QUIT => Self::Quit,
                mpv::mpv_end_file_reason_MPV_END_FILE_REASON_ERROR => Self::Error,
                mpv::mpv_end_file_reason_MPV_END_FILE_REASON_REDIRECT => Self::Redirect,
                _ => Self::Unknown { raw },
            }
        }
        #[cfg(not(feature = "libmpv-ffi"))]
        Self::Unknown { raw }
    }

    /// Machine-readable label for reports.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Eof => "eof",
            Self::Stop => "stop",
            Self::Quit => "quit",
            Self::Error => "error",
            Self::Redirect => "redirect",
            Self::Unknown { .. } => "unknown",
        }
    }
}

/// Which `mpv_event_id` kinds the control layer distinguishes.
///
/// With `libmpv-ffi` the mapping is from the header-declared
/// `mpv_event_id_MPV_EVENT_*` constants; any other id is kept as
/// [`EventKind::Other`] with its raw value instead of being guessed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    /// `MPV_EVENT_COMMAND_REPLY` - carries `reply_userdata`.
    CommandReply,
    /// `MPV_EVENT_SET_PROPERTY_REPLY` (`client.h:1266-1269`) - the reply to
    /// `mpv_set_property_async`. It carries `reply_userdata` and `error`
    /// (`client.h:1593-1608`) but *not* `mpv_event_property`, so it is a
    /// distinct kind from [`EventKind::CommandReply`] even though the engine
    /// correlates both through the same pending-token table.
    SetPropertyReply,
    /// `MPV_EVENT_PROPERTY_CHANGE` - carries an `mpv_event_property`.
    PropertyChange,
    /// `MPV_EVENT_END_FILE` - the current item stopped.
    EndFile,
    /// `MPV_EVENT_START_FILE` - libmpv is switching to this item; the load has
    /// begun. Delivered *after* an async `loadfile` reply in a real run.
    StartFile,
    /// `MPV_EVENT_FILE_LOADED` - the item's decoder setup completed.
    FileLoaded,
    /// `MPV_EVENT_PLAYBACK_RESTART` - playback actually restarted (after the
    /// initial load, after a seek, or after unpausing).
    PlaybackRestart,
    /// `MPV_EVENT_QUEUE_OVERFLOW` - libmpv dropped events.
    QueueOverflow,
    /// Any other event id (log messages, shutdown, audio/video reconfig, ...).
    Other {
        /// Raw `mpv_event_id`.
        raw: u32,
    },
}

impl EventKind {
    /// Map a raw event id (header constants when the FFI feature is on).
    #[must_use]
    pub fn from_raw(raw: u32) -> Self {
        #[cfg(feature = "libmpv-ffi")]
        {
            use crate::ffi as mpv;
            match raw {
                mpv::mpv_event_id_MPV_EVENT_COMMAND_REPLY => Self::CommandReply,
                mpv::mpv_event_id_MPV_EVENT_SET_PROPERTY_REPLY => Self::SetPropertyReply,
                mpv::mpv_event_id_MPV_EVENT_PROPERTY_CHANGE => Self::PropertyChange,
                mpv::mpv_event_id_MPV_EVENT_END_FILE => Self::EndFile,
                mpv::mpv_event_id_MPV_EVENT_START_FILE => Self::StartFile,
                mpv::mpv_event_id_MPV_EVENT_FILE_LOADED => Self::FileLoaded,
                mpv::mpv_event_id_MPV_EVENT_PLAYBACK_RESTART => Self::PlaybackRestart,
                mpv::mpv_event_id_MPV_EVENT_QUEUE_OVERFLOW => Self::QueueOverflow,
                _ => Self::Other { raw },
            }
        }
        #[cfg(not(feature = "libmpv-ffi"))]
        Self::Other { raw }
    }

    /// Machine-readable label.
    #[must_use]
    pub fn label(self) -> String {
        match self {
            Self::CommandReply => "command-reply".to_owned(),
            Self::SetPropertyReply => "set-property-reply".to_owned(),
            Self::PropertyChange => "property-change".to_owned(),
            Self::EndFile => "end-file".to_owned(),
            Self::StartFile => "start-file".to_owned(),
            Self::FileLoaded => "file-loaded".to_owned(),
            Self::PlaybackRestart => "playback-restart".to_owned(),
            Self::QueueOverflow => "queue-overflow".to_owned(),
            Self::Other { raw } => format!("other#{raw}"),
        }
    }
}

/// A value delivered by a property observation or property-set reply.
///
/// `labelled_unverified` is true when the property name is not backed by the
/// installed headers ([`crate::commands::SourceLabel::Unverified`]): the value is
/// then evidence from *this run*, not a compile-time guarantee.
#[derive(Clone, Debug, PartialEq)]
pub struct Observed {
    /// Property name as delivered by libmpv.
    pub name: String,
    /// Value (`MPV_FORMAT_*` mapped onto [`crate::commands::PropertyValue`]).
    pub value: crate::commands::PropertyValue,
    /// `Some(true)` if the name is header-backed, `Some(false)` if it is an
    /// unverified name we observed at run time, `None` when the name did not
    /// come from our own table (e.g. a property someone else observed).
    pub header_backed: Option<bool>,
}

impl Observed {
    /// Value as a short displayable string for reports and logs.
    #[must_use]
    pub fn value_text(&self) -> String {
        use crate::commands::PropertyValue as V;
        match &self.value {
            V::Flag(b) => (if *b { "true" } else { "false" }).to_owned(),
            V::Int64(i) => i.to_string(),
            V::Double(d) => format!("{d:.3}"),
            V::Str(s) => s.clone(),
            V::None => "none".to_owned(),
        }
    }
}

/// One event produced by the control loop.
#[derive(Clone, Debug, PartialEq)]
pub enum PlayerEvent {
    /// A correlated async reply: `MPV_EVENT_COMMAND_REPLY` (from
    /// `mpv_command_async`) or `MPV_EVENT_SET_PROPERTY_REPLY` (from
    /// `mpv_set_property_async`). Both are attributed through the same
    /// pending-token table; the `command` field is our own request label, so
    /// the caller can still tell a `set ...` control from a command.
    CommandReply {
        /// Reply token we assigned.
        token: u64,
        /// Our label for the request.
        command: String,
        /// `false` when libmpv reported an error for this command.
        ok: bool,
        /// libmpv error, when `ok` is false.
        error: Option<MpvError>,
    },
    /// `MPV_EVENT_PROPERTY_CHANGE` for an observed property.
    PropertyChange {
        /// Observed property.
        observed: Observed,
    },
    /// `MPV_EVENT_END_FILE`: the item stopped, with a typed reason.
    EndFile {
        /// Typed reason.
        reason: EndReason,
        /// libmpv error code, when `reason` is [`EndReason::Error`].
        error: Option<MpvError>,
    },
    /// `MPV_EVENT_START_FILE`: libmpv began switching to this item. The load is
    /// not finished yet; `FILE_LOADED` and the first `time-pos` come later.
    ItemStarted,
    /// `MPV_EVENT_FILE_LOADED`: the item's demuxer/decoder setup completed.
    FileLoaded,
    /// `MPV_EVENT_PLAYBACK_RESTART`: playback restarted (initial start, after a
    /// seek, or after unpausing).
    PlaybackRestarted,
    /// `MPV_EVENT_QUEUE_OVERFLOW` - our pump was too slow; events were dropped.
    QueueOverflow,
    /// The wakeup callback fired (evidence that control completion is driven by
    /// libmpv waking us rather than by blocking or polling).
    Woken,
    /// A reply that could not be correlated with a live request: recorded rather
    /// than mis-attributed to a newer command.
    Uncorrelated {
        /// Token carried by the reply.
        token: u64,
        /// Why it was not attributed.
        reason: &'static str,
    },
    /// The session was torn down; no further events are expected.
    Closed,
}

impl PlayerEvent {
    /// Machine-readable event label (used by tests, logs and the smoke report).
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::CommandReply {
                token,
                command,
                ok,
                error,
            } => format!(
                "reply token={token} command={command} ok={ok}{}",
                error
                    .as_ref()
                    .map(|e| format!(" error={}", e.name))
                    .unwrap_or_default()
            ),
            Self::PropertyChange { observed } => {
                format!(
                    "property name={} value={} {}",
                    observed.name,
                    observed.value_text(),
                    match observed.header_backed {
                        Some(true) => "source=header",
                        Some(false) => "source=unverified-observed",
                        None => "source=foreign",
                    }
                )
            }
            Self::EndFile { reason, error } => format!(
                "end-file reason={}{}",
                reason.label(),
                error
                    .as_ref()
                    .map(|e| format!(" error={}", e.name))
                    .unwrap_or_default()
            ),
            Self::ItemStarted => "start-file".to_owned(),
            Self::FileLoaded => "file-loaded".to_owned(),
            Self::PlaybackRestarted => "playback-restart".to_owned(),
            Self::QueueOverflow => "queue-overflow".to_owned(),
            Self::Woken => "wakeup".to_owned(),
            Self::Uncorrelated { token, reason } => {
                format!("uncorrelated token={token} reason={reason}")
            }
            Self::Closed => "closed".to_owned(),
        }
    }
}

/// Player-layer failures.
///
/// No token or credential ever appears in these messages: the player layer gets
/// URLs from the app, and only the *scheme/path* is quoted here.
#[derive(Clone, Debug, Error, PartialEq)]
pub enum PlayerError {
    /// The item cannot be played by this layer (unsupported scheme/source shape).
    ///
    /// The field is deliberately **not** named `source`: thiserror treats a field
    /// called `source` as the boxed error this error wraps, which would make
    /// `UnsupportedItem` fail to compile (a plain `String` is not an error).
    #[error("unsupported item {item}: {reason}")]
    UnsupportedItem {
        /// The rejected source.
        item: String,
        /// Why it was rejected.
        reason: String,
    },
    /// libmpv could not be loaded or the handle could not be created.
    #[error("libmpv unavailable ({version}): {reason}")]
    Unavailable {
        /// libmpv version reported by the build-time guard.
        version: String,
        /// Failure detail.
        reason: String,
    },
    /// `mpv_initialize` failed.
    #[error("mpv_initialize failed with {code} ({name})")]
    InitFailed {
        /// Raw `mpv_error` code.
        code: i32,
        /// Header name for the code.
        name: String,
    },
    /// The queueing call itself (`mpv_command_async` / `mpv_set_property_async`)
    /// rejected the request before it became an async request.
    #[error("{call} rejected {command}: {code} ({name})")]
    Rejected {
        /// libmpv function that rejected it.
        call: String,
        /// Our request label.
        command: String,
        /// Raw `mpv_error` code.
        code: i32,
        /// Header name for the code.
        name: String,
    },
    /// An async command completed with an error (from its reply).
    ///
    /// `call` is the async entry point the request was issued through
    /// ([`crate::Request::call_name`], recorded in
    /// [`crate::PendingRequest`] at issue time): it says whether the failing
    /// reply came back from `mpv_command_async` or `mpv_set_property_async`
    /// instead of leaving the caller to guess.
    #[error("{call} reply failed for {command}: {code} ({name})")]
    Command {
        /// The libmpv async call that produced this reply.
        call: String,
        /// Our request label.
        command: String,
        /// Raw `mpv_error` code from the reply event.
        code: i32,
        /// Header name for the code.
        name: String,
    },
    /// The item could not be played at all (END_FILE with reason=error).
    #[error("item {item} could not be played: end-file reason={reason} code={code} ({name})")]
    ItemFailed {
        /// The source we loaded.
        item: String,
        /// `mpv_end_file_reason` label.
        reason: String,
        /// Raw error code, when reported.
        code: i32,
        /// Header name for the code.
        name: String,
    },
    /// A request never got its reply within the event drains we performed.
    #[error("no reply for {command} (token {token}) after draining {drains} events")]
    NoReply {
        /// Our request label.
        command: String,
        /// Reply token we assigned.
        token: u64,
        /// How many events were drained while waiting.
        drains: usize,
    },
    /// The session was already closed by the caller.
    #[error("player session is closed")]
    Closed,
}

impl PlayerError {
    /// Machine-readable error code for reports (`unsupported_item`, `command`, ...).
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::UnsupportedItem { .. } => "unsupported_item",
            Self::Unavailable { .. } => "libmpv_unavailable",
            Self::InitFailed { .. } => "init_failed",
            Self::Rejected { .. } => "rejected",
            Self::Command { .. } => "command_failed",
            Self::ItemFailed { .. } => "item_failed",
            Self::NoReply { .. } => "no_reply",
            Self::Closed => "closed",
        }
    }
}

/// Is `code` the installed libmpv's success code?
///
/// With `libmpv-ffi` this compares against the header constant
/// `mpv_error_MPV_ERROR_SUCCESS` (bindgen-read from `client.h`), so a reply is
/// judged by the same constant real libmpv uses. Without the feature there is
/// no header constant to compare with and this honestly answers `false`: the
/// crate does not guess that `0` means success when it cannot prove it.
#[must_use]
pub fn is_success(code: i32) -> bool {
    #[cfg(feature = "libmpv-ffi")]
    {
        crate::error_code_of("MPV_ERROR_SUCCESS") == Some(code)
    }
    #[cfg(not(feature = "libmpv-ffi"))]
    {
        let _ = code;
        false
    }
}
