//! Typed core errors.
//!
//! Every variant is a *closed* set of `&'static str` / scalars: no variant can
//! carry a caller-supplied message, so no Plex token, pairing secret or URL
//! path containing one can reach `Display`/`Debug`. Transport details that
//! `ureq` reports are mapped onto a variant + a bounded status code here rather
//! than chained through (`source()` still exposes the underlying transport
//! error, which ureq itself renders without any request header values).

use thiserror::Error;

/// HTTP statuses that mean "your token is not accepted" (docs/PLEX_AUTH.md
/// §3: clear the token, require a new pin, never retry blindly).
pub const AUTH_REJECTED_STATUSES: [u16; 2] = [401, 422];

/// True for the statuses that trigger token clearing + new pin.
pub fn is_auth_rejected(status: u16) -> bool {
    AUTH_REJECTED_STATUSES.contains(&status)
}

/// Everything `plex-core` can fail with, token-free by construction.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CoreError {
    /// The identity/config could not be read or written.
    #[error("client identity could not be loaded or saved")]
    Identity,

    /// The transport failed (DNS, connect, TLS, read timeout). Modelled as a
    /// poll failure: the pin is expired, never retried with the same pin.
    #[error("network transport failure")]
    Transport,

    /// The server answered with a status we can classify.
    #[error("http status {status}")]
    HttpStatus {
        /// Response status code.
        status: u16,
    },

    /// A response body was not the JSON shape we expect. Never contains body
    /// bytes.
    #[error("unexpected api response shape")]
    ResponseShape,

    /// The pin's lifetime ended or any poll step failed: a **new** pin is
    /// required, the same pin id is never re-used.
    #[error("pairing pin expired, a new pin is required")]
    PinExpired,

    /// The pin flow was cancelled by the user; polling stopped and the pin was
    /// dropped.
    #[error("pairing cancelled")]
    PinCancelled,

    /// 401/422: the stored token was rejected. The local token was cleared and
    /// a new pin is required; there is no blind retry and no password path.
    #[error("authentication rejected (http {status}), local token cleared, new pin required")]
    AuthRejected {
        /// 401 or 422.
        status: u16,
    },

    /// The 120 s pairing budget elapsed without a claim.
    #[error("pairing timed out after {secs} s, a new pin is required")]
    PinTimeout {
        /// Budget in whole seconds.
        secs: u64,
    },

    /// The pairing pin flow was asked to run twice on the same pin after a
    /// failure state (programming error, guards the "no blind retry" rule).
    #[error("pin already used: a new pin must be created")]
    PinReuseRefused,

    /// Token storage failed for a backend-specific reason.
    #[error("token storage failure: {}", .0.as_str())]
    Storage(StorageError),

    /// The file fallback store was used without the recorded in-app consent.
    #[error("plaintext file token store requires explicit user consent")]
    ConsentRequired,

    /// The XDG Secret Service is not reachable on this session (no bus, no
    /// secrets daemon, or the collection is locked).
    #[error("xdg secret service unavailable")]
    SecretServiceUnavailable,

    /// An input value (pin id, URL, header value) failed validation. Never
    /// echoes the rejected value.
    #[error("invalid input")]
    InvalidInput,
}

impl CoreError {
    /// Machine-readable error code for UI/CLI consumers and tests.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Identity => "identity",
            Self::Transport => "transport",
            Self::HttpStatus { .. } => "http-status",
            Self::ResponseShape => "response-shape",
            Self::PinExpired => "pin-expired",
            Self::PinCancelled => "pin-cancelled",
            Self::AuthRejected { .. } => "auth-rejected",
            Self::PinTimeout { .. } => "pin-timeout",
            Self::PinReuseRefused => "pin-reuse-refused",
            Self::Storage(_) => "storage",
            Self::ConsentRequired => "consent-required",
            Self::SecretServiceUnavailable => "secret-service-unavailable",
            Self::InvalidInput => "invalid-input",
        }
    }

    /// Map an HTTP status onto the auth semantics.
    ///
    /// 401/422 -> [`CoreError::AuthRejected`] (caller must clear the token).
    /// Any other status -> [`CoreError::HttpStatus`], and for the pin poll path
    /// the caller turns that into [`CoreError::PinExpired`] (docs/PLEX_AUTH.md
    /// §1.3: the client cannot distinguish a dead pin from a server fault).
    pub fn from_status(status: u16) -> Self {
        if is_auth_rejected(status) {
            Self::AuthRejected { status }
        } else {
            Self::HttpStatus { status }
        }
    }
}

/// Storage failure kinds (fixed strings, so the rendered error cannot carry a
/// token, a path from a response body, or a secret value).
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum StorageError {
    /// D-Bus/session-bus failure.
    #[error("dbus")]
    Dbus,
    /// The secrets daemon answered with a prompt (locked collection) or a
    /// rejected session algorithm.
    #[error("prompt-or-locked")]
    Locked,
    /// The item was not found in the store.
    #[error("not-found")]
    NotFound,
    /// Filesystem error while reading/writing the fallback store.
    #[error("io")]
    Io,
    /// The stored payload did not parse.
    #[error("bad-payload")]
    Payload,
    /// No backend available (checked before any request is attempted).
    #[error("no-backend")]
    NoBackend,
    /// `HOME`/`$XDG_CONFIG_HOME` did not resolve, so there is no private place
    /// to store a token. There is deliberately no current-working-directory
    /// fallback: nothing is written.
    #[error("no-config-home")]
    NoConfigHome,
    /// The would-be storage directory is writable by group or other, or the
    /// directory only exists because we created it (we never chmod a directory
    /// we did not create).
    #[error("unsafe-directory")]
    UnsafeDirectory,
}

impl StorageError {
    /// Machine-readable storage failure code. A fixed set of `&'static str`, so
    /// `CoreError::Storage(StorageError)` cannot render anything but these.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dbus => "dbus",
            Self::Locked => "prompt-or-locked",
            Self::NotFound => "not-found",
            Self::Io => "io",
            Self::Payload => "bad-payload",
            Self::NoBackend => "no-backend",
            Self::NoConfigHome => "no-config-home",
            Self::UnsafeDirectory => "unsafe-directory",
        }
    }
}
