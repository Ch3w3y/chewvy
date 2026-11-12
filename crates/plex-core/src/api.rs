//! Plex account API (`plex.tv/api/v2`) used for sign-in.
//!
//! Two calls live here:
//!
//! * `GET /api/v2/user` — the signed-in account (`authToken`, `uuid`,
//!   `rememberExpiresAt`, `twoFactorEnabled`);
//! * `DELETE /api/v2/users/signout` — logout, paired with local token deletion.
//!
//! `api/v2/resources` discovery and connection probing/ordering belong to
//! `resources` and `server`.
//!
//! Token policy (docs/PLEX_AUTH.md §3): the account token is only ever
//! carried in the `X-Plex-Token` header via [`SecretString`]; the wire struct
//! that holds the JSON field is private to this module and has no `Debug`, so a
//! `{:?}` of any API type cannot print it.

use serde::Deserialize;

use crate::error::{is_auth_rejected, CoreError};
use crate::http::Http;
use crate::secret::SecretString;
use crate::tokens::{delete_plex_token, TokenKey, TokenStore};

/// Signed-in account lookup.
pub const USER_PATH: &str = "/api/v2/user";

/// Logout endpoint (docs/PLEX_AUTH.md §3: "Cancel/logout").
pub const SIGNOUT_PATH: &str = "/api/v2/users/signout";

/// `/api/v2/user` JSON field names (private wire representation).
#[derive(Deserialize)]
struct UserWire {
    #[serde(rename = "authToken")]
    auth_token: Option<String>,
    uuid: Option<String>,
    #[serde(rename = "rememberExpiresAt")]
    remember_expires_at: Option<i64>,
    #[serde(rename = "twoFactorEnabled")]
    two_factor_enabled: Option<bool>,
    username: Option<String>,
    email: Option<String>,
}

/// A Plex account, token-safe by construction: `Debug` is hand-written and
/// prints only [`crate::secret::REDACTED`] for the token slot.
#[derive(Clone, PartialEq, Eq)]
pub struct Account {
    /// The Plex account uuid (device-identifying, not a secret).
    pub uuid: String,
    /// The account-wide bearer token, if the response carried one.
    token: Option<SecretString>,
    /// `rememberExpiresAt`: unix seconds after which Plex stops remembering
    /// this account token (docs/PLEX_AUTH.md §3 "Expiry"). `None` when the
    /// response omits it; the TTL itself is unverified for this client.
    pub remember_expires_at: Option<i64>,
    /// `twoFactorEnabled`: 2FA is handled entirely in the browser sign-in page
    /// (this client has no password or 2FA code path).
    pub two_factor_enabled: bool,
    /// Display name (`X-Plex-Device-Name` companion, UI-only).
    pub username: Option<String>,
    /// Account e-mail, kept out of `Debug`.
    email: Option<String>,
}

impl std::fmt::Debug for Account {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Account")
            .field("uuid", &self.uuid)
            .field("token", &crate::secret::REDACTED)
            .field("has_token", &self.token.is_some())
            .field("remember_expires_at", &self.remember_expires_at)
            .field("two_factor_enabled", &self.two_factor_enabled)
            .field("username", &self.username)
            .field("email", &"<hidden>")
            .finish()
    }
}

impl Account {
    /// True when this account response carried a usable token.
    #[must_use]
    pub fn has_token(&self) -> bool {
        self.token.as_ref().is_some_and(|t| !t.is_empty())
    }

    /// Number of characters in the token without revealing any of them
    /// (UI "token present" indicator).
    #[must_use]
    pub fn token_len(&self) -> usize {
        self.token.as_ref().map_or(0, |t| t.len())
    }

    /// The token, for the caller to hand to the header builder or the token
    /// store. This is the only accessor that yields the value.
    pub fn take_token(self) -> Option<SecretString> {
        self.token
    }

    /// Clone the token for a request; the returned value prints as
    /// [`crate::secret::REDACTED`].
    #[must_use]
    pub fn token(&self) -> Option<SecretString> {
        self.token
            .as_ref()
            .map(|token| SecretString::new(token.expose()))
    }
}

/// Fetch `GET /api/v2/user` with the stored token.
///
/// A `401`/`422` maps to [`CoreError::AuthRejected`]; the **caller** must clear
/// the local token and start a new pin (no blind retry, no password fallback),
/// which is what [`clear_token_on_auth_rejection`] does.
pub fn fetch_account(http: &Http, token: &SecretString) -> Result<Account, CoreError> {
    let response = http.get(USER_PATH, &[], Some(token))?;
    if !response.is_success() {
        return Err(CoreError::from_status(response.status));
    }
    let wire: UserWire = response.json().map_err(|_| CoreError::ResponseShape)?;
    let uuid = wire
        .uuid
        .filter(|u| !u.is_empty())
        .ok_or(CoreError::ResponseShape)?;
    Ok(Account {
        uuid,
        token: wire
            .auth_token
            .filter(|t| !t.is_empty())
            .map(SecretString::new),
        remember_expires_at: wire.remember_expires_at,
        two_factor_enabled: wire.two_factor_enabled.unwrap_or(false),
        username: wire.username,
        email: wire.email,
    })
}

/// Outcome of a logout attempt; in both success cases the **local** token is
/// already deleted by the time the value is returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signout {
    /// The server accepted `DELETE /api/v2/users/signout`; local token deleted.
    LoggedOut,
    /// The server answered 401/422 — the token was already invalid, so this is
    /// logged-out locally and remotely. Local token deleted.
    AlreadySignedOut {
        /// 401 or 422.
        status: u16,
    },
}

/// `DELETE /api/v2/users/signout`, then delete the local token.
///
/// Local-deletion rule (tested in `tests/logout.rs`):
///
/// * 2xx -> local token deleted, [`Signout::LoggedOut`];
/// * 401/422 -> local token deleted (the token is worthless), returned as
///   [`Signout::AlreadySignedOut`] so the UI can say "already signed out";
/// * transport failure or any other status -> the **local token is kept** and
///   the error is returned, so the user can retry the logout instead of being
///   silently left "signed in at Plex, signed out here".
pub fn signout(
    http: &Http,
    token: &SecretString,
    store: &dyn TokenStore,
    key: Option<&TokenKey>,
) -> Result<Signout, CoreError> {
    match http.delete(SIGNOUT_PATH, &[], Some(token)) {
        Ok(response) if response.is_success() => {
            delete_plex_token(store, key)?;
            Ok(Signout::LoggedOut)
        }
        Ok(response) if is_auth_rejected(response.status) => {
            delete_plex_token(store, key)?;
            Ok(Signout::AlreadySignedOut {
                status: response.status,
            })
        }
        Ok(response) => Err(CoreError::from_status(response.status)),
        Err(CoreError::AuthRejected { status }) => {
            delete_plex_token(store, key)?;
            Ok(Signout::AlreadySignedOut { status })
        }
        Err(error) => Err(error),
    }
}

/// Apply the "401/422 clears the token" rule for an already-made request.
///
/// Returns the error the caller should surface (`AuthRejected`, with the local
/// token deleted and a new pin required). Any other error is passed through
/// untouched, so a plain network failure never masquerades as an auth failure.
pub fn clear_token_on_auth_rejection(
    store: &dyn TokenStore,
    key: Option<&TokenKey>,
    error: CoreError,
) -> Result<CoreError, CoreError> {
    match error {
        CoreError::AuthRejected { status } => {
            delete_plex_token(store, key)?;
            Ok(CoreError::AuthRejected { status })
        }
        other => Ok(other),
    }
}
