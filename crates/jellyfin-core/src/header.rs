//! The `MediaBrowser` flavor of the `Authorization` header.
//!
//! Wire format (researched against the server's own parser, see
//! docs/JELLYFIN.md):
//!
//! ```text
//! Authorization: MediaBrowser Client="…", Device="…", DeviceId="…", Version="…", Token="…"
//! ```
//!
//! The server splits the value on top-level commas, splits each pair on the
//! first `=`, trims surrounding whitespace and double quotes, and then
//! *URL-decodes* the value. Quoting is therefore mandatory around values, and
//! `"`/`,`/`%` inside a value must be percent-encoded first (a literal `+`
//! means space after decoding, so it too is encoded) — [`escape`] implements
//! exactly that: percent-encode every byte outside the RFC 3986 unreserved
//! set.
//!
//! The token is always the last pair and always quoted; it never appears in a
//! URL for any request this crate makes.

use crate::device::DeviceInfo;
use crate::SecretString;

/// The header this crate always uses for authorization.
pub const AUTHORIZATION_HEADER: &str = "Authorization";

/// The scheme name inside the header value.
pub const AUTH_SCHEME: &str = "MediaBrowser";

/// Percent-encode one header value for the `MediaBrowser` format: every UTF-8
/// byte outside the unreserved set `A-Za-z0-9-._~` becomes `%XX` (upper-case
/// hex), so the value survives both the server's comma split and its
/// URL-decode step unchanged.
/// set (upper-case hex), so the value survives both the server's comma split
/// and its URL-decode step unchanged.
#[must_use]
pub fn escape(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(char::from(*byte));
            }
            _ => {
                out.push('%');
                let hi = HEX.get(usize::from(byte >> 4)).copied().unwrap_or(b'0');
                let lo = HEX.get(usize::from(byte & 0x0f)).copied().unwrap_or(b'0');
                out.push(char::from(hi));
                out.push(char::from(lo));
            }
        }
    }
    out
}

/// Build the `Authorization` header value for `device`, with or without a
/// bearer `token`.
///
/// Without a token (server info, ping, Quick Connect initiate/poll) the
/// identity fields are still sent — the server records them for the pairing.
#[must_use]
pub fn media_browser_header(device: &DeviceInfo, token: Option<&SecretString>) -> String {
    let mut value = format!("{AUTH_SCHEME} Client=\"{}\"", escape(&device.app));
    for (key, field) in [
        ("Device", device.device.as_str()),
        ("DeviceId", device.device_id.as_str()),
        ("Version", device.app_version.as_str()),
    ] {
        value.push_str(&format!(", {key}=\"{}\"", escape(field)));
    }
    if let Some(token) = token.filter(|token| !token.is_empty()) {
        value.push_str(&format!(", Token=\"{}\"", escape(token.expose())));
    }
    value
}
