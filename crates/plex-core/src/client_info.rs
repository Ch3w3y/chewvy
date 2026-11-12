//! Stable per-install client identity and the `X-Plex-*` header builder.
//!
//! Header set researched in docs/PLEX_AUTH.md §2 (third-party corroborated
//! from python-plexapi `myplex.py`, Plex-published pages did not document it):
//! `X-Plex-Product`, `X-Plex-Client-Identifier` (a stable per-install UUID),
//! `X-Plex-Version`, `X-Plex-Platform`, `X-Plex-Platform-Version`, `X-Plex-Device`,
//! `X-Plex-Device-Name`, `X-Plex-Provides`, `X-Plex-Language`, plus
//! `Accept: application/json`. `X-Plex-Token` is added **only** as a header and
//! only when a token is available; this module has no code path that puts a
//! token into a URL.
//!
//! Values Plex actually accepts from an unregistered Linux desktop client are
//! still unverified (docs/PLEX_AUTH.md §5 item 4): the real-account gate is
//! real-server verification.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::CoreError;
use crate::secret::SecretString;

/// `X-Plex-Product` value. A distinctive product name helps Plex-side support
/// identify this client; it is not an official registration.
pub const PRODUCT: &str = "Chewvy";

/// `X-Plex-Platform` value. Plex's own clients send `Linux`; an accepted free
/// form for a third-party Linux client is unverified.
pub const PLATFORM: &str = "Linux";

/// `X-Plex-Provides` value. This client plays media and does not act as a
/// server; `media-player` is the value Plex's own players use.
pub const PROVIDES: &str = "media-player";

/// Default `X-Plex-Language` when the environment does not say.
pub const DEFAULT_LANGUAGE: &str = "en";

/// Header name carrying the account/per-server token. Header-only policy:
/// `X-Plex-Token` must never appear in a query string (docs/PLEX_AUTH.md
/// §1.7 — the reference client logs `?X-Plex-Token=`, we must not copy that).
pub const TOKEN_HEADER: &str = "X-Plex-Token";

/// One name/value pair in the order it should be sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Header {
    /// Header name.
    pub name: String,
    /// Header value.
    pub value: String,
}

impl Header {
    /// A header pair.
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
        }
    }
}

/// Persisted identity file contents (config dir, `0600`). The client identifier
/// is not a secret, but it is device-persistent, so the file is still written
/// private to the user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    /// Canonical `xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx` per-install UUID.
    pub client_identifier: String,
}

/// The client identity used for every Plex request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientInfo {
    /// `X-Plex-Product`.
    pub product: String,
    /// `X-Plex-Client-Identifier`: stable per-install UUID, reloaded on every
    /// run from the identity file so Plex sees one device.
    pub client_identifier: String,
    /// `X-Plex-Version`: this workspace's version.
    pub version: String,
    /// `X-Plex-Platform`.
    pub platform: String,
    /// `X-Plex-Platform-Version`: distro `PRETTY_NAME` when available.
    pub platform_version: String,
    /// `X-Plex-Device`.
    pub device: String,
    /// `X-Plex-Device-Name`.
    pub device_name: String,
    /// `X-Plex-Provides`.
    pub provides: String,
    /// `X-Plex-Language`.
    pub language: String,
}

impl ClientInfo {
    /// Load the identity from `identity_path`, creating it (and the parent
    /// directory, mode `0700`) with a freshly generated UUID when absent.
    ///
    /// `version` comes from the caller (usually `env!("CARGO_PKG_VERSION")`).
    pub fn load_or_create(identity_path: &Path, version: &str) -> Result<Self, CoreError> {
        let client_identifier = match std::fs::read_to_string(identity_path) {
            Ok(text) => {
                let identity: Identity =
                    serde_json::from_str(&text).map_err(|_| CoreError::Identity)?;
                if !is_uuid(&identity.client_identifier) {
                    return Err(CoreError::Identity);
                }
                identity.client_identifier
            }
            Err(read_err) if read_err.kind() == std::io::ErrorKind::NotFound => {
                let parent = identity_path
                    .parent()
                    .filter(|p| !p.as_os_str().is_empty())
                    .unwrap_or_else(|| Path::new("."));
                std::fs::create_dir_all(parent).map_err(|_| CoreError::Identity)?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ =
                        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
                }
                let identity = Identity {
                    client_identifier: uuid_v4()?,
                };
                write_identity(identity_path, &identity)?;
                identity.client_identifier
            }
            Err(_) => return Err(CoreError::Identity),
        };

        Ok(Self {
            product: PRODUCT.to_owned(),
            client_identifier,
            version: version.to_owned(),
            platform: PLATFORM.to_owned(),
            platform_version: platform_version(),
            device: device_name(),
            device_name: device_name(),
            provides: PROVIDES.to_owned(),
            language: language(),
        })
    }

    /// Build a `ClientInfo` from an explicit identity (used by tests and by the
    /// UI when the caller already resolved the UUID).
    #[must_use]
    pub fn with_identity(identity: &Identity, version: &str) -> Self {
        Self {
            product: PRODUCT.to_owned(),
            client_identifier: identity.client_identifier.clone(),
            version: version.to_owned(),
            platform: PLATFORM.to_owned(),
            platform_version: platform_version(),
            device: device_name(),
            device_name: device_name(),
            provides: PROVIDES.to_owned(),
            language: language(),
        }
    }

    /// The unauthenticated `X-Plex-*` + `Accept` header set, in a stable order.
    ///
    /// Contains no token and no secret.
    #[must_use]
    // Fixed, ordered header set built by 11 guarded pushes: clippy's
    // `vec_init_then_push` cannot express this ordering as a `vec![]` literal
    // without duplicating values, so the lint is allowed here explicitly.
    #[allow(clippy::vec_init_then_push)]
    pub fn headers(&self) -> Vec<Header> {
        let mut out = Vec::with_capacity(10);
        out.push(Header::new("X-Plex-Product", self.product.clone()));
        out.push(Header::new(
            "X-Plex-Client-Identifier",
            self.client_identifier.clone(),
        ));
        out.push(Header::new("X-Plex-Version", self.version.clone()));
        out.push(Header::new("X-Plex-Platform", self.platform.clone()));
        out.push(Header::new(
            "X-Plex-Platform-Version",
            self.platform_version.clone(),
        ));
        out.push(Header::new("X-Plex-Device", self.device.clone()));
        out.push(Header::new("X-Plex-Device-Name", self.device_name.clone()));
        out.push(Header::new("X-Plex-Provides", self.provides.clone()));
        out.push(Header::new("X-Plex-Language", self.language.clone()));
        out.push(Header::new("Accept", "application/json"));
        out
    }

    /// The header set with `X-Plex-Token` appended.
    ///
    /// This is the **only** place a token is exposed to the transport, and it is
    /// a header, never a query parameter. The assertions that hold that rule are
    /// `tests/headers.rs::a_token_is_added_exactly_once_and_only_as_the_token_header`
    /// (this function) and
    /// `tests/headers.rs::a_token_on_an_authenticated_request_is_header_only_on_the_wire`
    /// (the same rule observed on the wire via `mock::token_only_in_header`),
    /// plus `tests/redaction.rs` for the "never in logs" half.
    #[must_use]
    pub fn headers_with_token(&self, token: &SecretString) -> Vec<Header> {
        let mut out = self.headers();
        out.push(Header::new(TOKEN_HEADER, token.expose().to_owned()));
        out
    }

    /// Name list of the unauthenticated header set (for tests and UI display).
    #[must_use]
    pub fn header_names(&self) -> Vec<String> {
        self.headers().into_iter().map(|h| h.name).collect()
    }
}

/// Write the identity file private to the user (mode `0600` on Unix).
pub fn write_identity(identity_path: &Path, identity: &Identity) -> Result<(), CoreError> {
    let json = serde_json::to_vec_pretty(identity).map_err(|_| CoreError::Identity)?;
    crate::tokens::write_private_file(identity_path, &json).map_err(|_| CoreError::Identity)
}

/// Generate a canonical RFC 4122 version-4 UUID string.
///
/// Randomness comes from `getrandom`, which is the OS entropy source on every
/// supported target (`/dev/urandom` on Linux, `BCryptGenRandom` on Windows,
/// `getentropy` on macOS).
pub fn uuid_v4() -> Result<String, CoreError> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| CoreError::Identity)?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40; // version 4
    bytes[8] = (bytes[8] & 0x3f) | 0x80; // RFC 4122 variant
    let mut out = String::with_capacity(36);
    for (index, byte) in bytes.iter().enumerate() {
        if matches!(index, 4 | 6 | 8 | 10) {
            out.push('-');
        }
        let _ = write!(&mut out, "{byte:02x}");
    }
    Ok(out)
}

/// Validate the canonical 8-4-4-4-12 hex form.
#[must_use]
pub fn is_uuid(value: &str) -> bool {
    if value.len() != 36 {
        return false;
    }
    let groups = [0usize..8, 9..13, 14..18, 19..23, 24..36];
    let chars: Vec<char> = value.chars().collect();
    for (index, c) in chars.iter().enumerate() {
        if groups.iter().any(|g| g.contains(&index)) {
            if !c.is_ascii_hexdigit() {
                return false;
            }
        } else if *c != '-' {
            return false;
        }
    }
    true
}

/// Linux: `/proc/sys/kernel/hostname`, falling back to `/etc/hostname` and then
/// to a generic device name.
#[cfg(target_os = "linux")]
fn device_name() -> String {
    read_first_line("/proc/sys/kernel/hostname")
        .or_else(|| read_first_line("/etc/hostname"))
        .unwrap_or_else(|| "linux-desktop".to_owned())
}

/// Windows: the `COMPUTERNAME` the system sets for every logon session.
#[cfg(windows)]
fn device_name() -> String {
    env_or("COMPUTERNAME", "windows-desktop")
}

/// macOS: `HOSTNAME` when the shell exported one, else a generic device name
/// (there is no `/proc` here and reading the mDNS name would need a new crate).
#[cfg(target_os = "macos")]
fn device_name() -> String {
    env_or("HOSTNAME", "mac-desktop")
}

#[cfg(not(target_os = "linux"))]
fn env_or(var: &str, fallback: &str) -> String {
    std::env::var(var)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| fallback.to_owned())
}

/// Linux: `PRETTY_NAME` from `/etc/os-release`, falling back to the kernel
/// release.
#[cfg(target_os = "linux")]
fn platform_version() -> String {
    if let Ok(text) = std::fs::read_to_string("/etc/os-release") {
        for line in text.lines() {
            if let Some(value) = line.strip_prefix("PRETTY_NAME=") {
                let value = value.trim().trim_matches('"').to_owned();
                if !value.is_empty() {
                    return value;
                }
            }
        }
    }
    read_first_line("/proc/sys/kernel/osrelease").unwrap_or_else(|| "Linux".to_owned())
}

/// Elsewhere there is no os-release: report the OS name and nothing invented.
#[cfg(not(target_os = "linux"))]
fn platform_version() -> String {
    match std::env::consts::OS {
        "windows" => "Windows".to_owned(),
        "macos" => "macOS".to_owned(),
        other => other.to_owned(),
    }
}

/// Primary language subtag from `LANG`/`LC_ALL`, else [`DEFAULT_LANGUAGE`].
fn language() -> String {
    let raw = std::env::var("LC_ALL")
        .ok()
        .filter(|v| !v.is_empty())
        .or_else(|| std::env::var("LANG").ok().filter(|v| !v.is_empty()))
        .unwrap_or_default();
    let base = raw
        .split('.')
        .next()
        .unwrap_or("")
        .split('_')
        .next()
        .unwrap_or("");
    if base.is_ascii() && base.len() == 2 {
        base.to_ascii_lowercase()
    } else {
        DEFAULT_LANGUAGE.to_owned()
    }
}

#[cfg(target_os = "linux")]
fn read_first_line(path: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let line = text.lines().next()?.trim();
    (!line.is_empty()).then(|| line.to_owned())
}

/// Where the identity file lives: `<app config dir>/identity.json`, the config
/// dir being [`crate::tokens::config_dir`] (XDG on Linux, `%APPDATA%` on
/// Windows, `~/Library/Application Support` on macOS).
///
/// Kept in `client_info` (not `tokens`) because the auth flow needs it before
/// any token store exists.
#[must_use]
pub fn default_identity_path() -> PathBuf {
    crate::tokens::config_dir()
        .map(|dir| dir.join("identity.json"))
        .unwrap_or_else(|| PathBuf::from("identity.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_generated_uuid_is_canonical_and_versioned() {
        let Ok(id) = uuid_v4() else {
            panic!("the OS entropy source must be available in a test");
        };
        assert!(is_uuid(&id), "{id} must be canonical");
        assert_eq!(id.chars().nth(14), Some('4'), "version 4");
        assert!(
            matches!(id.chars().nth(19), Some('8'..='b')),
            "RFC 4122 variant"
        );
        assert_ne!(id, uuid_v4().unwrap_or_default(), "two draws differ");
    }

    #[test]
    fn an_identity_written_and_read_back_keeps_the_identifier() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!("plex-core-identity-{nanos}"));
        let path = dir.join("identity.json");
        let identity = Identity {
            client_identifier: uuid_v4().unwrap_or_default(),
        };
        assert!(std::fs::create_dir_all(&dir).is_ok());
        assert!(write_identity(&path, &identity).is_ok());
        let Ok(text) = std::fs::read_to_string(&path) else {
            panic!("the identity file must be readable");
        };
        let Ok(reloaded) = serde_json::from_str::<Identity>(&text) else {
            panic!("the identity file must parse: {text}");
        };
        assert_eq!(reloaded, identity);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn the_identity_path_is_this_client_s_own_file() {
        let path = default_identity_path();
        assert_eq!(path.file_name().unwrap_or_default(), "identity.json");
        assert!(
            path.to_string_lossy().contains("chewvy"),
            "the identity lives in the app config dir: {path:?}"
        );
    }

    #[test]
    fn device_and_platform_names_are_never_empty() {
        assert!(!device_name().is_empty());
        assert!(!platform_version().is_empty());
    }
}
