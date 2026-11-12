//! Track / subtitle selection values for the libmpv `audio-id` / `sub-id` properties.
//!
//! `track-list` is a node array and the player layer does not decode nodes (see
//! `docs/CAPABILITIES.md`), so selection here is by the *value* the properties
//! accept, not by a decoded track list. Both property names are upstream mpv
//! documentation names, **not** header symbols
//! ([`crate::commands::SourceLabel::Unverified`]); the `--probe-json` report records
//! whether a real libmpv run accepted them.

use crate::commands::{PropertyValue, Request};
use crate::events::PlayerError;

/// A track selection for `audio-id` or `sub-id`.
#[derive(Clone, Debug, PartialEq)]
pub enum TrackSelection {
    /// Disable this track type (mpv `no`).
    None,
    /// Select by track index as mpv numbers them (`select #n` style index).
    Index(i64),
    /// Select by the string id mpv reports for the track (e.g. a foreign id).
    Id(String),
    /// Select by mpv's `auto` (first suitable track).
    Auto,
}

impl TrackSelection {
    /// The exact string value a set-property request would carry.
    ///
    /// `audio-id`/`sub-id` are set as strings so `no`/`auto` are expressible;
    /// mpv parses numeric strings as indexes.
    #[must_use]
    pub fn value(&self) -> String {
        match self {
            Self::None => "no".to_owned(),
            Self::Index(i) => i.to_string(),
            Self::Id(id) => id.clone(),
            Self::Auto => "auto".to_owned(),
        }
    }

    /// Reject values libmpv would not accept (empty, embedded NUL, or a value
    /// that looks like an option rather than an id).
    pub fn validated(&self) -> Result<Self, PlayerError> {
        match self {
            Self::Index(i) if *i < 0 => Err(PlayerError::UnsupportedItem {
                item: format!("track index {i}"),
                reason: "negative track index".to_owned(),
            }),
            Self::Id(id) if id.trim().is_empty() || id.contains('\0') => {
                Err(PlayerError::UnsupportedItem {
                    item: "track id".to_owned(),
                    reason: "empty or NUL-containing track id".to_owned(),
                })
            }
            other => Ok(other.clone()),
        }
    }

    /// `mpv_set_property_async` request for the given property of this crate's
    /// command vocabulary.
    pub fn request_for(&self, property: crate::commands::Named) -> Result<Request, PlayerError> {
        let selection = self.validated()?;
        Ok(Request::SetProperty {
            name: property,
            value: PropertyValue::Str(selection.value()),
        })
    }
}

/// Build an `audio-id` selection request.
///
/// # Errors
///
/// [`PlayerError::UnsupportedItem`] when the selection is not expressible.
pub fn select_audio(selection: &TrackSelection) -> Result<Request, PlayerError> {
    selection.request_for(crate::commands::props::AUDIO_ID)
}

/// Build a `sub-id` selection request (same value shape as [`select_audio`]).
///
/// # Errors
///
/// [`PlayerError::UnsupportedItem`] when the selection is not expressible.
pub fn select_subtitle(selection: &TrackSelection) -> Result<Request, PlayerError> {
    selection.request_for(crate::commands::props::SUB_ID)
}

/// The `select` command shape (`select <type> <id|auto|no>`), kept as an
/// alternative to the property setters. **Unverified**: neither `select` nor its
/// argument words are header symbols, and the player layer does not issue it; it is here so a
/// later milestone can compare the two shapes against a real run.
#[must_use]
pub fn select_command(track_type: &str, selection: &TrackSelection) -> Request {
    Request::Command {
        name: SELECT,
        args: vec![track_type.to_owned(), selection.value()],
    }
}

/// `select` is an upstream mpv command name, not a header symbol, and M4 does
/// not issue it (see the `--probe-json` report for the names a real run confirmed).
const SELECT: crate::commands::Named = crate::commands::Named {
    name: "select",
    label: crate::commands::SourceLabel::Unverified {
        source: "https://mpv.io/manual/stable/#commands",
        accessed: "2026-11-05",
    },
};
