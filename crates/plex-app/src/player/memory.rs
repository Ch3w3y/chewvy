//! The audio and subtitle languages picked for a title, remembered across runs.
//!
//! Choices are stored by language, not track number, so a pick made on one
//! episode carries to the rest of the show. The file holds rating keys and
//! language codes only, in the app config dir with owner-only permissions.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::state::{language_name, Track, TrackKind};

/// File under the app config dir.
const FILE: &str = "track-choices.json";
/// Most titles kept; the oldest entries beyond this are dropped.
const MAX_TITLES: usize = 2_000;
/// What a subtitle choice of "none" is stored as.
pub const OFF: &str = "off";

/// One title's picks.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Choice {
    /// Audio language code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<String>,
    /// Subtitle language code, or [`OFF`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    /// Write order, for trimming.
    #[serde(default)]
    pub seq: u64,
}

fn path() -> Option<PathBuf> {
    plex_core::tokens::StorePaths::resolve()
        .ok()
        .map(|p| p.config_dir.join(FILE))
}

fn load_from(path: &Path) -> BTreeMap<String, Choice> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn save_to(path: &Path, map: &BTreeMap<String, Choice>) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let raw = serde_json::to_string(map).map_err(std::io::Error::other)?;
    plex_core::tokens::write_private_file(path, raw.as_bytes())
}

/// The remembered choice for `title`.
#[must_use]
pub fn recall(title: &str) -> Option<Choice> {
    load_from(&path()?).remove(title)
}

/// Merge `update` into `title`'s choice and store it; failures are logged only.
pub fn remember(title: &str, update: impl FnOnce(&mut Choice)) {
    let Some(path) = path() else { return };
    let mut map = load_from(&path);
    let seq = map.values().map(|c| c.seq).max().unwrap_or(0) + 1;
    let entry = map.entry(title.to_owned()).or_default();
    update(entry);
    entry.seq = seq;
    trim(&mut map);
    if let Err(e) = save_to(&path, &map) {
        log::debug!("could not store track choices: {e}");
    }
}

fn trim(map: &mut BTreeMap<String, Choice>) {
    while map.len() > MAX_TITLES {
        let oldest = map
            .iter()
            .min_by_key(|(_, c)| c.seq)
            .map(|(k, _)| k.clone());
        match oldest {
            Some(key) => map.remove(&key),
            None => break,
        };
    }
}

/// The track of `kind` whose language matches `lang`, preferring full
/// subtitles over forced ones. Codes match by language, so a choice stored as
/// `en-US` finds an `eng` track in the next file.
#[must_use]
pub fn pick<'a>(tracks: &'a [Track], kind: TrackKind, lang: &str) -> Option<&'a Track> {
    let wanted = language_name(lang);
    let mut matching = tracks.iter().filter(|t| {
        t.kind == kind
            && t.lang
                .as_deref()
                .is_some_and(|l| l == lang || language_name(l) == wanted)
    });
    let first = matching.clone().next();
    matching.find(|t| !t.forced).or(first)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{load_from, pick, save_to, trim, Choice, MAX_TITLES};
    use crate::player::state::{Track, TrackKind};

    fn track(id: i64, kind: TrackKind, lang: &str, forced: bool) -> Track {
        Track {
            id,
            kind,
            label: String::new(),
            selected: false,
            lang: Some(lang.to_owned()),
            forced,
        }
    }

    #[test]
    fn a_subtitle_pick_prefers_the_full_track_over_forced() {
        let tracks = vec![
            track(1, TrackKind::Subtitle, "eng", true),
            track(2, TrackKind::Subtitle, "eng", false),
            track(3, TrackKind::Audio, "eng", false),
        ];
        assert_eq!(
            pick(&tracks, TrackKind::Subtitle, "eng").map(|t| t.id),
            Some(2)
        );
        assert_eq!(
            pick(&tracks, TrackKind::Audio, "eng").map(|t| t.id),
            Some(3)
        );
        assert!(pick(&tracks, TrackKind::Audio, "jpn").is_none());
        assert_eq!(
            pick(&tracks, TrackKind::Audio, "en-US").map(|t| t.id),
            Some(3)
        );
    }

    #[test]
    fn choices_round_trip_through_the_file() {
        let dir = std::env::temp_dir().join(format!("plex-app-memory-{}", std::process::id()));
        let path = dir.join("choices.json");
        let mut map = BTreeMap::new();
        map.insert(
            "42".to_owned(),
            Choice {
                audio: Some("jpn".into()),
                subtitle: Some("eng".into()),
                seq: 1,
            },
        );
        assert!(save_to(&path, &map).is_ok());
        assert_eq!(load_from(&path), map);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn trimming_drops_the_oldest_titles() {
        let mut map: BTreeMap<String, Choice> = (0..MAX_TITLES as u64 + 3)
            .map(|i| {
                (
                    i.to_string(),
                    Choice {
                        seq: i,
                        ..Choice::default()
                    },
                )
            })
            .collect();
        trim(&mut map);
        assert_eq!(map.len(), MAX_TITLES);
        assert!(!map.contains_key("0"));
        assert!(map.contains_key(&(MAX_TITLES as u64 + 2).to_string()));
    }
}
