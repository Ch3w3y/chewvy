//! The app's own settings, persisted as JSON in the config dir beside the
//! token store.
//!
//! The Settings screen edits these and they are written back on every change.
//! Only the interface one is read by the browsing UI (the poster band in
//! [`crate::ui::grid`]). Hardware decoding is read when playback starts
//! (mpv `hwdec`); progress sync is stored only, as nothing reports progress yet.

use std::path::Path;

use serde::{Deserialize, Serialize};

use plex_core::tokens::StorePaths;

/// The settings file's name under the app config dir.
pub const FILE: &str = "settings.json";

/// How wide the library grid lays its posters out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PosterSize {
    Small,
    #[default]
    Medium,
    Large,
}

impl PosterSize {
    /// The label the Settings screen offers.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Small => "Small",
            Self::Medium => "Medium",
            Self::Large => "Large",
        }
    }

    /// The sizes the screen offers, in the order it draws them.
    pub const ALL: [Self; 3] = [Self::Small, Self::Medium, Self::Large];
}

/// What the Settings screen edits, with the values the app starts on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    /// Ask the server to keep this client's watched state in step. Off until it
    /// is asked for.
    pub progress_sync: bool,
    /// Prefer hardware decoding where the player can use it.
    pub hardware_decoding: bool,
    /// Poster width the grid lays out with.
    pub poster_size: PosterSize,
    /// Move HDR and Dolby Vision videos to the HDR window as soon as they are
    /// recognised, instead of offering the button.
    pub hdr_auto: bool,
    /// The HDR window opens full screen rather than as a normal window.
    pub hdr_fullscreen: bool,
    /// The HDR window's explanation has been read, so it is not shown again.
    pub hdr_explained: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            progress_sync: false,
            hardware_decoding: true,
            poster_size: PosterSize::default(),
            hdr_auto: false,
            hdr_fullscreen: true,
            hdr_explained: false,
        }
    }
}

/// A stored [`Settings`]: a field an older or partial file does not name keeps
/// its default.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(default)]
struct Stored {
    progress_sync: bool,
    hardware_decoding: bool,
    poster_size: PosterSize,
    hdr_auto: bool,
    hdr_fullscreen: bool,
    hdr_explained: bool,
}

// A field the file leaves out takes the app's default (hardware decoding on),
// not the type's zero value.
impl Default for Stored {
    fn default() -> Self {
        Self::from(Settings::default())
    }
}

impl From<Settings> for Stored {
    fn from(value: Settings) -> Self {
        Self {
            progress_sync: value.progress_sync,
            hardware_decoding: value.hardware_decoding,
            poster_size: value.poster_size,
            hdr_auto: value.hdr_auto,
            hdr_fullscreen: value.hdr_fullscreen,
            hdr_explained: value.hdr_explained,
        }
    }
}

impl From<Stored> for Settings {
    fn from(value: Stored) -> Self {
        Self {
            progress_sync: value.progress_sync,
            hardware_decoding: value.hardware_decoding,
            poster_size: value.poster_size,
            hdr_auto: value.hdr_auto,
            hdr_fullscreen: value.hdr_fullscreen,
            hdr_explained: value.hdr_explained,
        }
    }
}

/// The settings of this install's config dir: the defaults when no config dir
/// resolves or its settings file cannot be read.
#[must_use]
pub fn load() -> Settings {
    config_dir().map_or_else(Settings::default, |dir| load_from(&dir))
}

/// This install's config dir, or `None` when XDG names none (no `HOME` and no
/// `$XDG_CONFIG_HOME`): settings are then kept for this run only and nothing is
/// written.
#[must_use]
pub fn config_dir() -> Option<std::path::PathBuf> {
    StorePaths::resolve().ok().map(|paths| paths.config_dir)
}

/// Write `settings` to this install's config dir, doing nothing when XDG names
/// no config dir.
pub fn save(settings: Settings) {
    if let Some(dir) = config_dir() {
        let _ = save_to(&dir, settings);
    }
}

/// The settings file in `config_dir`, or the defaults when it is missing or
/// unreadable.
#[must_use]
pub fn load_from(config_dir: &Path) -> Settings {
    let path = config_dir.join(FILE);
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str::<Stored>(&text).ok())
        .map_or_else(Settings::default, Settings::from)
}

/// Write `settings` to `<config_dir>/settings.json` private to its owner, and
/// create the directory when it is missing.
///
/// Privacy is `plex_core::tokens`' shared rule: mode `0600`/`0700` where POSIX
/// modes exist, and on Windows the file simply inherits the private per-user
/// profile folder it lives in.
///
/// # Errors
///
/// The IO error when the file cannot be written.
pub fn save_to(config_dir: &Path, settings: Settings) -> Result<(), std::io::Error> {
    let path = config_dir.join(FILE);
    if config_dir.metadata().is_err() {
        plex_core::tokens::create_private_dir(config_dir)?;
    }
    let json = serde_json::to_vec_pretty(&Stored::from(settings))
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    plex_core::tokens::write_private_file(&path, &json)
}

#[cfg(test)]
#[path = "settings/tests.rs"]
mod tests;
