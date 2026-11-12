//! Stable per-install Jellyfin device identity.
//!
//! The `DeviceId` is a uuid v4 generated with `getrandom` (via
//! `plex_core::client_info::uuid_v4`) and persisted in a private `0600` file,
//! mirroring the plex-core client identity rule: the server shows it in its
//! Device list, and the Quick Connect authorization is bound to it, so it must
//! survive restarts but is safe to regenerate once (it is not a credential).

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::JfError;

/// The app name Jellyfin shows for this client (`Client` in the header).
pub const APP_NAME: &str = "Chewvy";

/// On-disk shape of the identity file. No `Debug` derive needed: it holds no
/// secret, only this device's own uuid.
#[derive(Serialize, Deserialize)]
struct StoredIdentity {
    /// File format version.
    schema_version: u8,
    /// The stable `DeviceId`.
    device_id: String,
}

/// What the `MediaBrowser` header needs beyond the token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    /// The app name sent as `Client`.
    pub app: String,
    /// The app version sent as `Version` (not a secret).
    pub app_version: String,
    /// A human-readable device label sent as `Device` (e.g. the hostname).
    pub device: String,
    /// The stable per-install uuid sent as `DeviceId`.
    pub device_id: String,
}

impl DeviceInfo {
    /// Load the persisted `DeviceId` from `identity_path`, creating and
    /// persisting a fresh uuid when the file does not exist yet.
    ///
    /// # Errors
    ///
    /// [`JfError::Identity`] when the file exists but is unreadable or
    /// corrupt, or when the uuid cannot be generated; [`JfError::Storage`]
    /// when the private file cannot be written.
    pub fn load_or_create(
        identity_path: &Path,
        app_version: &str,
        device_label: &str,
    ) -> Result<Self, JfError> {
        let device_id = match read_identity(identity_path)? {
            Some(id) => id,
            None => {
                // Absent, not corrupt: generate once and persist.
                let id = plex_core::client_info::uuid_v4().map_err(|_| JfError::Identity)?;
                write_identity(identity_path, &id)?;
                id
            }
        };
        Ok(Self {
            app: APP_NAME.to_owned(),
            app_version: app_version.to_owned(),
            device: device_label.to_owned(),
            device_id,
        })
    }
}

/// Read a previously written identity file: `Ok(None)` when the file does
/// not exist yet, `Err(Identity)` when it exists but is unreadable or
/// corrupt (the caller decides whether to overwrite).
fn read_identity(identity_path: &Path) -> Result<Option<String>, JfError> {
    let text = match std::fs::read_to_string(identity_path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(JfError::Identity),
    };
    let stored: StoredIdentity = serde_json::from_str(&text).map_err(|_| JfError::Identity)?;
    if stored.schema_version != 1 || !plex_core::client_info::is_uuid(&stored.device_id) {
        return Err(JfError::Identity);
    }
    Ok(Some(stored.device_id))
}

/// Write the identity file privately (`0600`, private parent), as plex-core
/// does for its own identity and token files.
///
/// # Errors
///
/// [`JfError::Storage`] on any io failure.
pub fn write_identity(identity_path: &Path, device_id: &str) -> Result<(), JfError> {
    if !plex_core::client_info::is_uuid(device_id) {
        return Err(JfError::Identity);
    }
    let stored = StoredIdentity {
        schema_version: 1,
        device_id: device_id.to_owned(),
    };
    let json = serde_json::to_vec_pretty(&stored)
        .map_err(|_| JfError::Storage(plex_core::error::StorageError::Payload))?;
    plex_core::tokens::write_private_file(identity_path, &json)
        .map_err(|_| JfError::Storage(plex_core::error::StorageError::Io))
}
