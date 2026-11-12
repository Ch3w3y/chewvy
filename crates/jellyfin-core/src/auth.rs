//! Sign-in fallback and sign-out.
//!
//! Password sign-in (`POST /Users/AuthenticateByName`) is Jellyfin's answer
//! to the case Quick Connect cannot serve: a server with Quick Connect
//! disabled ([`crate::quickconnect::enabled`] answers `false`, or initiate
//! answers `401`). The password travels **only** in the JSON request body —
//! never in a header or the URL — and is dropped the moment the serialised
//! body has been handed to the transport. Sign-out is
//! `POST /Sessions/Logout`, which ends the *session* server-side.

use crate::device::DeviceInfo;
use crate::error::JfError;
use crate::http::Http;
use crate::quickconnect::Authentication;
use crate::SecretString;

/// Path of the password sign-in endpoint (`POST`).
pub const AUTHENTICATE_BY_NAME_PATH: &str = "/Users/AuthenticateByName";

/// Path of the sign-out endpoint (`POST`, authenticated).
pub const LOGOUT_PATH: &str = "/Sessions/Logout";

/// Sign in with a username and a (possibly empty) password.
///
/// The empty-password case is real: a Jellyfin user can exist with no
/// password, and the endpoint accepts `Pw: ""` for it.
///
/// # Errors
///
/// [`JfError::AuthRejected`] for a `401`/`403` (wrong credentials), plus
/// transport/status/shape errors.
pub fn sign_in_password(
    http: &Http,
    device: &DeviceInfo,
    username: &str,
    password: &str,
) -> Result<Authentication, JfError> {
    if username.trim().is_empty() {
        return Err(JfError::InvalidInput);
    }
    let authorization = crate::header::media_browser_header(device, None);
    // Built by hand from the two escaped fields so the password never lands
    // in a Debuggable intermediate: `user` is server-independent text, and
    // the password is dropped once `body` is consumed below.
    let body = format!(
        r#"{{"Username":"{}","Pw":"{}"}}"#,
        escape_json(username),
        escape_json(password)
    );
    let response = http.post(
        AUTHENTICATE_BY_NAME_PATH,
        &[],
        Some(&body),
        Some(&authorization),
    )?;
    let auth: Authentication = response.json()?;
    if auth.access_token.is_empty() {
        return Err(JfError::ResponseShape);
    }
    Ok(auth)
}

/// End this client's server-side session and (by the caller, in the token
/// store) forget the token. The token is sent only as part of the
/// `Authorization` header.
///
/// # Errors
///
/// Transport/status errors; an unauthenticated call (`401`) is
/// [`JfError::AuthRejected`].
pub fn sign_out(http: &Http, device: &DeviceInfo, token: &SecretString) -> Result<(), JfError> {
    let authorization = crate::header::media_browser_header(device, Some(token));
    http.post(LOGOUT_PATH, &[], None, Some(&authorization))?;
    Ok(())
}

/// Minimal JSON string escaping for the two body fields: quotes, backslash
/// and control characters (the rest is UTF-8 and travels as-is).
fn escape_json(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out
}
