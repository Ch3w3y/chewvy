//! Player state built **only** from observed properties.
//!
//! The UI must not ask libmpv for state mid-frame: [`ObservedState::apply`] is fed by
//! `MPV_EVENT_PROPERTY_CHANGE` deliveries, and a name that was never observed is
//! `None` rather than a guess. The `--probe-json` report is the run evidence for
//! which names were actually observed in a real libmpv session.

use crate::commands::{lookup_property, props, Named, PropertyValue, ValueKind};

/// The property formats we ask libmpv to deliver observations in.
///
/// Not a header fact: `client.h` documents `mpv_format`, not which format a
/// property has. The probe run (`src/probe.rs`) is what shows whether a name was
/// actually delivered in the requested format, and `MPV_FORMAT_NONE`/`String`
/// observations are how an unavailable property shows up.
#[must_use]
pub fn observed_format(property: Named) -> ValueKind {
    match property.name {
        "time-pos" | "playback-time" | "duration" | "volume" => ValueKind::Double,
        "pause" | "eof-reached" | "idle-active" | "paused-for-cache" => ValueKind::Flag,
        "eof-reason" | "audio-id" | "sub-id" | "media-title" => ValueKind::String,
        // `track-list` is a node array; the player layer does not decode nodes, so we ask for
        // change notification only (MPV_FORMAT_NONE).
        "track-list" => ValueKind::None,
        // Anything not in our table: ask for the stringified value, which mpv
        // can produce for scalars without us asserting a type.
        _ => ValueKind::String,
    }
}

/// State derived exclusively from property observations.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ObservedState {
    /// `time-pos` in seconds.
    pub time_pos: Option<f64>,
    /// `playback-time` in seconds.
    pub playback_time: Option<f64>,
    /// `duration` in seconds.
    pub duration: Option<f64>,
    /// `pause`.
    pub paused: Option<bool>,
    /// `eof-reached`.
    pub eof_reached: Option<bool>,
    /// `eof-reason` (`normal|error|stop`).
    pub eof_reason: Option<String>,
    /// `idle-active`.
    pub idle_active: Option<bool>,
    /// `paused-for-cache`.
    pub paused_for_cache: Option<bool>,
    /// `media-title`.
    pub media_title: Option<String>,
    /// `volume` as observed (not the value we asked for).
    pub volume: Option<f64>,
    /// `audio-id` as observed.
    pub audio_id: Option<String>,
    /// `sub-id` as observed.
    pub sub_id: Option<String>,
    /// Names delivered to us that are not in `props::ALL`: reported honestly as
    /// unbacked, never merged into the fields above.
    pub unrecognized: Vec<String>,
    /// Every name we have ever received a change for, with how many changes.
    pub observed_counts: Vec<(String, usize)>,
}

impl ObservedState {
    /// An empty state for a new item.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply one observation. Returns `true` when it landed in a known field.
    ///
    /// A `PropertyValue::None` means "this property is currently unavailable"
    /// (libmpv reports the format as NONE), which clears the field rather than
    /// leaving a stale value behind.
    pub fn apply(&mut self, observed: &crate::events::Observed) -> bool {
        let name = observed.name.as_str();
        bump(&mut self.observed_counts, name);
        let known = match lookup_property(name) {
            Some(_) => true,
            None => {
                if !self.unrecognized.contains(&observed.name) {
                    self.unrecognized.push(observed.name.clone());
                }
                false
            }
        };
        if !known {
            return false;
        }
        match (name, &observed.value) {
            ("time-pos", PropertyValue::Double(v)) => self.time_pos = Some(*v),
            ("playback-time", PropertyValue::Double(v)) => self.playback_time = Some(*v),
            ("duration", PropertyValue::Double(v)) => self.duration = Some(*v),
            ("volume", PropertyValue::Double(v)) => self.volume = Some(*v),
            ("pause", PropertyValue::Flag(v)) => self.paused = Some(*v),
            ("eof-reached", PropertyValue::Flag(v)) => self.eof_reached = Some(*v),
            ("idle-active", PropertyValue::Flag(v)) => self.idle_active = Some(*v),
            ("paused-for-cache", PropertyValue::Flag(v)) => self.paused_for_cache = Some(*v),
            ("eof-reason", PropertyValue::Str(v)) => self.eof_reason = Some(v.clone()),
            ("media-title", PropertyValue::Str(v)) => self.media_title = Some(v.clone()),
            ("audio-id", PropertyValue::Str(v)) => self.audio_id = Some(v.clone()),
            ("sub-id", PropertyValue::Str(v)) => self.sub_id = Some(v.clone()),
            // None = property currently unavailable for this item.
            (_, PropertyValue::None) => self.clear(name),
            // Format mismatch: keep the state untouched and let the probe report
            // it instead of coercing a value we do not understand.
            _ => return false,
        }
        true
    }

    fn clear(&mut self, name: &str) {
        match name {
            "time-pos" => self.time_pos = None,
            "playback-time" => self.playback_time = None,
            "duration" => self.duration = None,
            "volume" => self.volume = None,
            "pause" => self.paused = None,
            "eof-reached" => self.eof_reached = None,
            "idle-active" => self.idle_active = None,
            "paused-for-cache" => self.paused_for_cache = None,
            "eof-reason" => self.eof_reason = None,
            "media-title" => self.media_title = None,
            "audio-id" => self.audio_id = None,
            "sub-id" => self.sub_id = None,
            _ => {}
        }
    }

    /// `true` when the state says the current item is playable/paused (not ended).
    #[must_use]
    pub fn playing(&self) -> bool {
        self.eof_reached != Some(true) && self.idle_active != Some(true)
    }

    /// Reset for a new item; keeps the cumulative observation counts, which are
    /// run evidence rather than per-item state.
    pub fn reset_item(&mut self) {
        self.time_pos = None;
        self.playback_time = None;
        self.duration = None;
        self.paused = None;
        self.eof_reached = None;
        self.eof_reason = None;
        self.idle_active = None;
        self.paused_for_cache = None;
        self.media_title = None;
        self.volume = None;
        self.audio_id = None;
        self.sub_id = None;
    }

    /// `name=header|unverified` provenance for every name in `props::ALL`,
    /// annotated with whether *this run* delivered an observation for it.
    #[must_use]
    pub fn provenance(&self) -> Vec<String> {
        props::ALL
            .iter()
            .map(|named| {
                let observed = self
                    .observed_counts
                    .iter()
                    .find(|(n, _)| n == named.name)
                    .is_some();
                format!(
                    "{}={} observed={}",
                    named.name,
                    named.label,
                    if observed { "yes" } else { "no" }
                )
            })
            .collect()
    }
}

fn bump(counts: &mut Vec<(String, usize)>, name: &str) {
    if let Some((_, count)) = counts.iter_mut().find(|(n, _)| n == name) {
        *count += 1;
    } else {
        counts.push((name.to_owned(), 1));
    }
}
