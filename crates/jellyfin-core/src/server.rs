//! Server identity: URL normalisation, `/System/Info/Public`, `/System/Ping`.

use serde::Deserialize;

use crate::error::JfError;
use crate::http::Http;

/// Path of the unauthenticated identity endpoint.
pub const INFO_PUBLIC_PATH: &str = "/System/Info/Public";

/// Path of the cheap liveness endpoint.
pub const PING_PATH: &str = "/System/Ping";

/// Normalise a user-typed server URL to the API base `scheme://authority`.
///
/// Accepts `host`, `host:port`, `http://…`, `https://…` and a pasted
/// `http://host:8096/web/index.html`; drops any path, query and fragment
/// (every endpoint this crate calls hangs off the root) and any trailing
/// slash. A missing scheme defaults to `http` (a typed LAN address); `https`
/// must be typed explicitly.
///
/// # Errors
///
/// [`JfError::InvalidInput`] when there is no host or the scheme is not
/// http/https.
pub fn normalize_base(raw: &str) -> Result<String, JfError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(JfError::InvalidInput);
    }
    let with_scheme = if trimmed.contains("://") {
        trimmed.to_owned()
    } else {
        format!("http://{trimmed}")
    };
    let parsed = url::Url::parse(&with_scheme).map_err(|_| JfError::InvalidInput)?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(JfError::InvalidInput);
    }
    // Rebuild from scheme + authority only: the rebuild is how the path,
    // query, fragment and any default port are dropped. `host_str` already
    // brackets IPv6 literals.
    let authority = parsed.host_str().ok_or(JfError::InvalidInput)?;
    let port = parsed
        .port()
        .map_or_else(String::new, |port| format!(":{port}"));
    Ok(format!("{}://{authority}{port}", parsed.scheme()))
}

/// The identity every Jellyfin server answers without a token.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PublicInfo {
    /// The stable server id (a system uuid) — the token-store key for this
    /// server.
    #[serde(rename = "Id")]
    pub id: String,
    /// The name the user gave the server.
    #[serde(rename = "ServerName")]
    pub server_name: String,
    /// Server version string.
    #[serde(rename = "Version")]
    pub version: String,
    /// Whether the first-run setup wizard has finished. A server that has not
    /// finished it accepts an address but cannot sign anyone in, and a server
    /// that answers without this key is treated as finished: only a `false`
    /// says otherwise.
    #[serde(rename = "StartupWizardCompleted", default)]
    pub startup_wizard_completed: Option<bool>,
}

/// Fetch the server's public identity. The `Id` is non-empty by server
/// contract; an empty one is treated as a shape error.
///
/// # Errors
///
/// Transport/status/shape errors; [`JfError::ResponseShape`] for an identity
/// without a usable `Id`.
pub fn public_info(http: &Http) -> Result<PublicInfo, JfError> {
    let response = http.get(INFO_PUBLIC_PATH, &[], None)?;
    let info: PublicInfo = response.json()?;
    if info.id.is_empty() {
        return Err(JfError::ResponseShape);
    }
    Ok(info)
}

/// Cheap liveness check: `GET /System/Ping` answers `204` with no body.
///
/// # Errors
///
/// Transport or status errors; success means the host answered, nothing more.
pub fn ping(http: &Http) -> Result<(), JfError> {
    let response = http.get(PING_PATH, &[], None)?;
    if response.is_success() {
        Ok(())
    } else {
        Err(JfError::HttpStatus {
            status: response.status,
        })
    }
}
