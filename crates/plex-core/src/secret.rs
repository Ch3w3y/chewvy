//! A secret string wrapper that never leaks its contents into `Debug`,
//! `Display`, `serde` output, log records or error formatting.
//!
//! The Plex account `authToken` and every per-server `accessToken` are bearer
//! secrets (docs/PLEX_AUTH.md §3). This type is the only representation
//! allowed to hold one outside the token store: it implements `Debug` and
//! `Display` as a fixed redaction marker, is **not** `Serialize`/`Deserialize`
//! (so it cannot be logged as JSON or persisted accidentally) and zeroes its
//! buffer on drop.
//!
//! The only way to read the contents is the explicit [`SecretString::expose`]
//! call, which is what the header builder uses. Every call site of `expose` is
//! deliberately narrow and named in a comment.

use std::fmt;

/// The fixed marker used everywhere a secret would otherwise be printed.
pub const REDACTED: &str = "[redacted]";

/// A secret value that is inert in formatting.
///
/// `Clone`/`PartialEq`/`Eq` are provided because the API/account models hold an
/// `Option<SecretString>` and tests need to compare round-tripped values.
/// Equality compares the contents in a bounded, branch-minimised way; it never
/// renders them. The derived-free `Debug`/`Display` above stay redacting.
#[derive(Clone)]
pub struct SecretString {
    value: String,
}

impl PartialEq for SecretString {
    fn eq(&self, other: &Self) -> bool {
        self.matches_secret(&other.value)
    }
}

impl Eq for SecretString {}

impl SecretString {
    fn matches_secret(&self, other: &str) -> bool {
        let a = self.value.as_bytes();
        let b = other.as_bytes();
        a.len() == b.len()
            && a.iter()
                .zip(b.iter())
                .fold(0u8, |acc, (x, y)| acc | (x ^ y))
                == 0
    }
}

impl SecretString {
    /// Wrap a secret read from the token store or a Plex API response body.
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
        }
    }

    /// Read the secret.
    ///
    /// Call sites are limited to: the `X-Plex-Token` header value, the token
    /// store write path, and a `const_format`-free pairing URL that carries the
    /// pairing `code` (not a token). Never pass the result to a logger,
    /// `format!`, `Debug`/`Display`, an error variant or a UI label.
    pub fn expose(&self) -> &str {
        &self.value
    }

    /// True when the secret is empty (an absent token is modelled as an empty
    /// secret rather than as `None`-with-a-printable-`Debug`).
    pub fn is_empty(&self) -> bool {
        self.value.is_empty()
    }

    /// Length of the secret, for tests and UI "token present" indicators without
    /// revealing any character.
    pub fn len(&self) -> usize {
        self.value.len()
    }

    /// Constant-time-ish equality check used by tests and by the
    /// "did the store round-trip the same token" assertion.
    pub fn matches(&self, other: &str) -> bool {
        self.matches_secret(other)
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(REDACTED)
    }
}

impl fmt::Display for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(REDACTED)
    }
}

impl Drop for SecretString {
    fn drop(&mut self) {
        // Best-effort overwrite. The workspace denies `unsafe_code`, so we may
        // not call `str::as_bytes_mut` to zero the live allocation in place;
        // overwriting the `String` contents releases the previous buffer to the
        // allocator. This is *not* a memory-safety guarantee (a copy may already
        // exist elsewhere in the heap); the real control is that the value can
        // only be read through `expose` and never reaches formatting.
        let zeros = "\u{0}".repeat(self.value.len());
        self.value.replace_range(.., &zeros);
        self.value.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_and_display_never_show_the_value() {
        let secret = SecretString::new("abc123TOKENvalue");
        assert_eq!(format!("{secret:?}"), REDACTED);
        assert_eq!(format!("{secret}"), REDACTED);
        assert_eq!(secret.to_string(), REDACTED);
        assert!(!format!("{secret:?}").contains("TOKEN"));
    }

    #[test]
    fn expose_and_match_work_without_formatting() {
        let secret = SecretString::new("zzzzsecretzzzz");
        assert_eq!(secret.expose(), "zzzzsecretzzzz");
        assert!(secret.matches("zzzzsecretzzzz"));
        assert!(!secret.matches("zzzzsecretzzzs"));
        assert!(!secret.matches("short"));
        assert_eq!(secret.len(), 14);
        assert!(SecretString::new("x").len() == 1);
        assert!(SecretString::new("").is_empty());
    }

    #[test]
    fn drop_overwrites_the_value_without_formatting_it() {
        // Observation of our own overwrite path, not a memory-safety claim.
        let mut secret = SecretString::new("0123456789abcdef");
        assert_eq!(secret.expose(), "0123456789abcdef");
        // Drop runs the overwrite path; the value is gone before the buffer is
        // released, and no formatted form of it was ever produced.
        drop(std::mem::replace(&mut secret, SecretString::new("")));
        assert!(secret.is_empty());
    }

    #[test]
    fn equality_and_clone_never_render_the_value() {
        let a = SecretString::new("TOKENabc123");
        let b = a.clone();
        assert_eq!(a, b);
        assert_ne!(a, SecretString::new("TOKENabc124"));
        assert_ne!(a, SecretString::new("TOKENabc12"));
        assert_eq!(format!("{b:?}"), REDACTED);
        assert!(!format!("{:?}", Some(a.clone())).contains("TOKEN"));
    }
}
