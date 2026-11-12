//! Quick Connect: Jellyfin's browser-style sign-in.
//!
//! Flow (researched against the server implementation, see docs/JELLYFIN.md):
//! the client POSTs `/QuickConnect/Initiate` and receives a long `Secret`
//! plus a short human-readable `Code`; the user types the code (or opens the
//! shown URL) on any already-authenticated device; the client polls
//! `GET /QuickConnect/Connect?secret=…` until `Authenticated` is `true`, then
//! POSTs `/Users/AuthenticateWithQuickConnect` with the secret to exchange it
//! for the long-lived access token.
//!
//! Status semantics implemented here:
//!
//! * `404` on poll or exchange: the secret is unknown or has aged out →
//!   [`QuickConnect::Expired`] (the caller starts a new session).
//! * `401` on initiate, or `enabled == false`: Quick Connect is switched off
//!   server-side → [`JfError::QuickConnectDisabled`], for which the caller's
//!   answer is the password fallback ([`crate::auth::sign_in_password`]).

use serde::Deserialize;

use crate::device::DeviceInfo;
use crate::error::JfError;
use crate::http::Http;
use crate::SecretString;

/// Path of the availability endpoint (`GET`, answers `true`/`false`).
pub const ENABLED_PATH: &str = "/QuickConnect/Enabled";

/// Path that starts a pairing (`POST`, no token).
pub const INITIATE_PATH: &str = "/QuickConnect/Initiate";

/// Path polled while waiting for activation (`GET ?secret=…`).
pub const CONNECT_PATH: &str = "/QuickConnect/Connect";

/// Path that trades an activated secret for a token (`POST`).
pub const AUTHENTICATE_PATH: &str = "/Users/AuthenticateWithQuickConnect";

/// A started pairing: [`Self::code`] is what the user reads to the other
/// device; [`Self::secret`] is the bearer-ish pairing secret (short-lived,
/// single-device, and — unlike the access token — sent as a query parameter
/// by the server's own API design).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ticket {
    /// Long opaque secret (64 hex characters in practice).
    pub secret: SecretString,
    /// Short human-readable code (6 digits in practice).
    pub code: String,
}

/// One poll of a pairing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuickConnect {
    /// Not activated yet; keep polling.
    Pending,
    /// Activated and exchanged: the server answered with a token.
    Authorized(Authentication),
    /// The secret is unknown or expired (`404`); start a new session.
    Expired,
}

/// The token answer both authenticate endpoints share
/// (`AuthenticateWithQuickConnect` and `AuthenticateByName`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Authentication {
    /// The long-lived bearer token.
    #[serde(rename = "AccessToken")]
    pub access_token: String,
    /// The id of the authenticated user, when the server includes it.
    #[serde(rename = "User", default)]
    pub user: Option<UserId>,
    /// The id of the server that issued the token.
    #[serde(rename = "ServerId", default)]
    pub server_id: Option<String>,
}

/// The `User` object of an authenticate answer (only its id is kept).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct UserId {
    /// The user's stable id.
    #[serde(rename = "Id")]
    pub id: String,
}

impl Authentication {
    /// The access token as a [`SecretString`] (inert in `Debug`/`Display`),
    /// consuming the answer.
    #[must_use]
    pub fn into_token(self) -> SecretString {
        SecretString::new(self.access_token)
    }
}

/// Whether this server accepts Quick Connect at all.
///
/// # Errors
///
/// Transport/status/shape errors.
pub fn enabled(http: &Http) -> Result<bool, JfError> {
    let response = http.get(ENABLED_PATH, &[], None)?;
    let value: bool = response.json()?;
    Ok(value)
}

/// Start a pairing. No token is sent; the identity fields of the
/// `MediaBrowser` header are what the server records against the pairing.
///
/// # Errors
///
/// [`JfError::QuickConnectDisabled`] for a `401`, status/shape errors for
/// anything else.
pub fn initiate(http: &Http, device: &DeviceInfo) -> Result<Ticket, JfError> {
    let authorization = crate::header::media_browser_header(device, None);
    let response = match http.post(INITIATE_PATH, &[], None, Some(&authorization)) {
        Err(JfError::AuthRejected { .. }) => return Err(JfError::QuickConnectDisabled),
        other => other?,
    };
    #[derive(Deserialize)]
    struct Initiated {
        #[serde(rename = "Secret")]
        secret: String,
        #[serde(rename = "Code")]
        code: String,
    }
    let initiated: Initiated = response.json()?;
    if initiated.secret.is_empty() || initiated.code.is_empty() {
        return Err(JfError::ResponseShape);
    }
    Ok(Ticket {
        secret: SecretString::new(initiated.secret),
        code: initiated.code,
    })
}

/// Poll a pairing once, and — only once the server reports it activated —
/// immediately exchange the secret for the access token (the activation
/// window is short, so there is no reason to defer the exchange).
///
/// # Errors
///
/// [`JfError::AuthRejected`] if the server rejects the exchange mid-flight
/// (treat as expired and restart); other transport/status/shape errors.
pub fn check(http: &Http, device: &DeviceInfo, ticket: &Ticket) -> Result<QuickConnect, JfError> {
    let authorization = crate::header::media_browser_header(device, None);
    let query = [("secret", ticket.secret.expose())];
    let response = match http.get(CONNECT_PATH, &query, Some(&authorization)) {
        Err(JfError::HttpStatus { status: 404 }) => return Ok(QuickConnect::Expired),
        other => other?,
    };
    #[derive(Deserialize)]
    struct Connect {
        #[serde(rename = "Authenticated")]
        authenticated: bool,
    }
    let connect: Connect = response.json()?;
    if !connect.authenticated {
        return Ok(QuickConnect::Pending);
    }
    match authenticate(http, device, ticket) {
        // The activation window closed between poll and exchange.
        Err(JfError::HttpStatus { status: 404 }) => Ok(QuickConnect::Expired),
        other => other.map(QuickConnect::Authorized),
    }
}

/// Exchange an activated secret for the access token. Callers normally get
/// here through [`check`], which only calls this after the poll said
/// `Authenticated`.
///
/// # Errors
///
/// [`JfError::HttpStatus`]`{404}` for an unknown/expired secret,
/// [`JfError::AuthRejected`] for a rejection, plus transport/shape errors.
pub fn authenticate(
    http: &Http,
    device: &DeviceInfo,
    ticket: &Ticket,
) -> Result<Authentication, JfError> {
    let authorization = crate::header::media_browser_header(device, None);
    // Built by hand so the secret never appears in a `Debug` of an
    // intermediate value.
    let body = format!(r#"{{"Secret":"{}"}}"#, ticket.secret.expose());
    let response = http.post(AUTHENTICATE_PATH, &[], Some(&body), Some(&authorization))?;
    let auth: Authentication = response.json()?;
    if auth.access_token.is_empty() {
        return Err(JfError::ResponseShape);
    }
    Ok(auth)
}
