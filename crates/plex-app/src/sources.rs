//! The sources this app can play from, as `<config_dir>/sources.json` keeps
//! them: the Plex account it is linked to plus any number of Jellyfin servers.
//!
//! This is the list the Settings › Sources screen manages and the source switcher
//! will read. It deliberately holds no token: every source's token stays in the
//! keyed [`plex_core::tokens::TokenStore`] slot named by its own stable id —
//! [`TokenKey::plex`] for the account, [`TokenKey::jellyfin`] for a server — so
//! a readable list and a secret never share a file.
//!
//! A missing, unreadable or half-written file answers with an empty list rather
//! than an error, exactly as [`crate::ui::settings`] answers with its defaults:
//! browsing must start with whatever is there.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use plex_core::tokens::{StorePaths, TokenKey};

#[path = "sources/worker.rs"]
pub mod worker;

/// The Jellyfin servers this install can browse, and the reads its screens ask
/// for them for.
pub(crate) mod browse;

/// A Jellyfin server's rows as the browse model's rows.
mod mapping;

/// The sources file's name under the app config dir.
pub const FILE: &str = "sources.json";

/// The layout this crate writes; a future reader can tell an old file apart.
pub const SCHEMA_VERSION: u8 = 1;

/// Seconds since the unix epoch, `0` when the clock cannot be read.
#[must_use]
pub fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| i64::try_from(since.as_secs()).unwrap_or(0))
}

/// The Plex account this install is linked to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlexAccount {
    /// The plex.tv account uuid, once it is known: it keys the token slot.
    /// `None` is a sign-in plex.tv has not named yet, whose token sits in the
    /// pre-keying slot.
    pub account_id: Option<String>,
    /// When this app was linked to the account, in unix seconds.
    pub added_at: i64,
}

impl Default for PlexAccount {
    fn default() -> Self {
        Self {
            account_id: None,
            added_at: unix_now(),
        }
    }
}

/// One Jellyfin server this app holds a sign-in for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct JellyfinServer {
    /// The server's system uuid from `/System/Info/Public`: the stable id, and
    /// the token slot's key.
    pub server_id: String,
    /// The name the server was asked for (`ServerName`).
    pub name: String,
    /// The API base the user's address normalised to, `scheme://host[:port]`.
    pub base_url: String,
    /// The signed-in user's id, when the server named one.
    pub user_id: Option<String>,
    /// The signed-in user's name, when the server named one.
    pub user_name: Option<String>,
    /// When this source was added, in unix seconds.
    pub added_at: i64,
}

impl Default for JellyfinServer {
    fn default() -> Self {
        Self {
            server_id: String::new(),
            name: String::new(),
            base_url: String::new(),
            user_id: None,
            user_name: None,
            added_at: unix_now(),
        }
    }
}

/// Every source this app can play from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sources {
    /// The file layout's version.
    pub schema_version: u8,
    /// The Plex account this install is linked to, if any. One account is all
    /// the sign-in flow can hold today; more accounts are a later milestone.
    pub plex: Option<PlexAccount>,
    /// The Jellyfin servers this install holds a sign-in for.
    pub jellyfin: Vec<JellyfinServer>,
}

impl Default for Sources {
    /// An empty list at the layout this crate writes.
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            plex: None,
            jellyfin: Vec::new(),
        }
    }
}

impl Sources {
    /// An empty list at the current layout version.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that this install is linked to a Plex account, keeping the
    /// original `added_at` while the same account is re-recorded.
    pub fn record_plex(&mut self, account_id: Option<String>) {
        let same_account = self
            .plex
            .as_ref()
            .is_some_and(|held| held.account_id == account_id);
        let added_at = if same_account {
            self.plex
                .as_ref()
                .map_or_else(unix_now, |held| held.added_at)
        } else {
            unix_now()
        };
        self.plex = Some(PlexAccount {
            account_id,
            added_at,
        });
    }

    /// Forget the Plex account, leaving the Jellyfin servers alone.
    pub fn clear_plex(&mut self) {
        self.plex = None;
    }

    /// The row for `server_id`, if this list holds one.
    #[must_use]
    pub fn jellyfin(&self, server_id: &str) -> Option<&JellyfinServer> {
        self.jellyfin
            .iter()
            .find(|held| held.server_id == server_id)
    }

    /// Add `server`, replacing the row for its id when the same server is
    /// signed in to again.
    pub fn set_jellyfin(&mut self, server: JellyfinServer) {
        match self
            .jellyfin
            .iter_mut()
            .find(|held| held.server_id == server.server_id)
        {
            Some(held) => *held = server,
            None => self.jellyfin.push(server),
        }
    }

    /// Drop the row for `server_id`, answering whether there was one.
    pub fn remove_jellyfin(&mut self, server_id: &str) -> Option<JellyfinServer> {
        let at = self
            .jellyfin
            .iter()
            .position(|held| held.server_id == server_id)?;
        Some(self.jellyfin.remove(at))
    }

    /// Whether this app holds a sign-in for anything at all.
    #[must_use]
    pub fn has_sign_in(&self) -> bool {
        self.plex.is_some() || !self.jellyfin.is_empty()
    }

    /// The Settings line naming what this install can play from.
    #[must_use]
    pub fn summary(&self) -> String {
        let servers = match self.jellyfin.len() {
            0 => None,
            1 => Some("1 Jellyfin server".to_owned()),
            n => Some(format!("{n} Jellyfin servers")),
        };
        match (self.plex.is_some(), servers) {
            (true, None) => "Plex account".to_owned(),
            (true, Some(jellyfin)) => format!("Plex account and {jellyfin}"),
            (false, Some(jellyfin)) => jellyfin,
            (false, None) => "Nothing signed in yet".to_owned(),
        }
    }
}

/// The token slot a Jellyfin server's token belongs in, or `None` for a row
/// with no usable id.
#[must_use]
pub fn jellyfin_token_key(server: &JellyfinServer) -> Option<TokenKey> {
    TokenKey::jellyfin(&server.server_id).ok()
}

/// This install's sources file: an empty list when no config dir resolves or
/// its sources file is missing or unreadable.
#[must_use]
pub fn load() -> Sources {
    config_dir().map_or_else(Sources::new, |dir| load_from(&dir))
}

/// This install's config dir, or `None` when the OS names none.
#[must_use]
pub fn config_dir() -> Option<PathBuf> {
    StorePaths::resolve().ok().map(|paths| paths.config_dir)
}

/// Write `sources` to this install's config dir.
///
/// # Errors
///
/// The IO error when the file cannot be written, including when the OS names no
/// config dir to write it to.
pub fn save(sources: &Sources) -> Result<(), std::io::Error> {
    config_dir().map_or_else(
        || Err(std::io::Error::other("no config dir to write to")),
        |dir| save_to(&dir, sources),
    )
}

/// The sources file in `config_dir`, or an empty list when it is missing,
/// unreadable or not the shape this crate writes.
#[must_use]
pub fn load_from(config_dir: &Path) -> Sources {
    let path = config_dir.join(FILE);
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str::<Sources>(&text).ok())
        .unwrap_or_default()
}

/// Write `sources` to `<config_dir>/sources.json` private to its owner, and
/// create the directory when it is missing.
///
/// Privacy is `plex_core::tokens`' shared rule: mode `0600`/`0700` where POSIX
/// modes exist, and elsewhere the file inherits the private per-user folder.
///
/// # Errors
///
/// The IO error when the file cannot be written.
pub fn save_to(config_dir: &Path, sources: &Sources) -> Result<(), std::io::Error> {
    let path = config_dir.join(FILE);
    if config_dir.metadata().is_err() {
        plex_core::tokens::create_private_dir(config_dir)?;
    }
    let json = serde_json::to_vec_pretty(sources)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    plex_core::tokens::write_private_file(&path, &json)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "tests may unwrap; product code keeps these lints on"
)]
#[path = "sources/tests.rs"]
mod tests;
