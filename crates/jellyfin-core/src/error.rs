//! Closed error type for the Jellyfin core.
//!
//! Every variant carries at most a status code: no server message, URL or
//! body ever rides along, so no secret (and no foreign text that could carry
//! one) can reach `Display`/`Debug`. This mirrors `plex_core::error::CoreError`
//! and reuses its storage error, which is already closed and token-free.

use plex_core::error::StorageError;

/// Everything that can go wrong talking to a Jellyfin server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum JfError {
    /// The connection failed or was cut; not an auth event.
    #[error("network transport failure")]
    Transport,
    /// The server answered with an unexpected HTTP status.
    #[error("HTTP status {status}")]
    HttpStatus {
        /// The status code, the only fact kept from the answer.
        status: u16,
    },
    /// The answer was not the JSON shape the endpoint promises. Never echoes
    /// the payload.
    #[error("unexpected response shape")]
    ResponseShape,
    /// A caller handed in something this crate refuses (bad URL, empty token,
    /// non-UTF8 datagram).
    #[error("invalid input")]
    InvalidInput,
    /// The device identity could not be read or created.
    #[error("device identity failure")]
    Identity,
    /// Token storage failed (same closed taxonomy as plex-core).
    #[error("token storage failure: {0:?}")]
    Storage(StorageError),
    /// The server rejected the presented token (401/403).
    #[error("authentication rejected (HTTP {status})")]
    AuthRejected {
        /// 401 or 403, as reported by the server.
        status: u16,
    },
    /// Quick Connect is switched off in the server settings.
    #[error("Quick Connect is disabled on this server")]
    QuickConnectDisabled,
    /// A local socket/io failure (LAN discovery). `ErrorKind` only: it carries
    /// no address text.
    #[error("local io failure")]
    Io,
}

impl JfError {
    /// Map an HTTP status to the right variant; 2xx yields `HttpStatus` too,
    /// call sites that treat success specially check `Response::is_success`
    /// first.
    #[must_use]
    pub fn from_status(status: u16) -> Self {
        match status {
            401 | 403 => Self::AuthRejected { status },
            _ => Self::HttpStatus { status },
        }
    }

    /// Stable machine-readable code for logs and the UI, like
    /// `CoreError::code`.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Transport => "transport",
            Self::HttpStatus { .. } => "http-status",
            Self::ResponseShape => "response-shape",
            Self::InvalidInput => "invalid-input",
            Self::Identity => "identity",
            Self::Storage(_) => "storage",
            Self::AuthRejected { .. } => "auth-rejected",
            Self::QuickConnectDisabled => "quick-connect-disabled",
            Self::Io => "io",
        }
    }
}
