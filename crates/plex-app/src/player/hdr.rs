//! The HDR window: the one place a file can leave this app's own window.
//!
//! mpv's render API, which draws video into the app window, has no `gpu-next`
//! backend, and `gpu-next` is what applies Dolby Vision reshaping, HDR10+
//! curves and real HDR output. So for those files the same in-process mpv core
//! is started with `vo=gpu-next` and opens a window of its own, with mpv's own
//! on-screen controller and keys. The app window stays a remote control.
//!
//! This is player-led Dolby Vision: the picture is processed here and sent to
//! the display as HDR10. A TV's own Dolby Vision mode needs licensed hardware
//! in the source device and is out of reach of any desktop window.

use super::state::VideoInfo;

/// Where a playing item's picture is presented.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Output {
    /// Inside the app window, through the render API.
    #[default]
    Embedded,
    /// In mpv's own `gpu-next` window.
    HdrWindow {
        /// The window opens full screen rather than as a normal window.
        fullscreen: bool,
    },
}

impl Output {
    /// Whether this is the HDR window.
    #[must_use]
    pub fn is_hdr_window(self) -> bool {
        matches!(self, Self::HdrWindow { .. })
    }

    /// The video-output options this mode needs before `mpv_initialize`.
    ///
    /// The `libmpv` profile turns mpv's controller and keys off because an
    /// embedding app normally draws its own; the HDR window has nobody else to
    /// draw them, so they come back on.
    #[must_use]
    pub fn options(self) -> Vec<(&'static str, &'static str)> {
        match self {
            Self::Embedded => vec![("vo", "libmpv")],
            Self::HdrWindow { fullscreen } => vec![
                ("vo", "gpu-next"),
                // Tags the surface with the video's colour space, so a
                // colour-managed compositor (KWin, Windows HDR) switches the
                // display into HDR. Without known display parameters it stays
                // off and mpv tone-maps instead.
                ("target-colorspace-hint", "auto"),
                ("force-window", "yes"),
                ("fullscreen", if fullscreen { "yes" } else { "no" }),
                ("osc", "yes"),
                ("input-default-bindings", "yes"),
                ("input-vo-keyboard", "yes"),
            ],
        }
    }
}

/// The HDR window's keys for the next episode: `N`, and the keys mpv itself
/// uses for the next file.
pub const NEXT_KEYS: [&str; 5] = ["n", "N", ">", "ENTER", "NEXT"];

/// The mpv property those keys set. The HDR window's keys are mpv's own, so
/// a key press reaches the app as a property it reads.
pub const NEXT_ASKED: &str = "user-data/chewvy/next";

/// Whether [`NEXT_ASKED`] reads as set. mpv keeps such a value as data and
/// reads it back as JSON, so the text arrives in quotes.
#[must_use]
pub fn asked(value: Option<&str>) -> bool {
    value.is_some_and(|text| text.trim_matches('"') == "yes")
}

/// Whether the decoded video is HDR or Dolby Vision: the files the HDR window
/// is for. mpv reports this from the stream itself, so it holds for Plex and
/// Jellyfin alike, whatever the server's metadata says.
#[must_use]
pub fn is_hdr(video: &VideoInfo) -> bool {
    matches!(video.gamma.as_deref(), Some("pq" | "hlg")) || is_dolby_vision(video)
}

/// Whether mpv decoded Dolby Vision metadata for this video.
#[must_use]
pub fn is_dolby_vision(video: &VideoInfo) -> bool {
    video.matrix.as_deref() == Some("dolbyvision")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video(gamma: Option<&str>, matrix: Option<&str>) -> VideoInfo {
        VideoInfo {
            gamma: gamma.map(str::to_owned),
            matrix: matrix.map(str::to_owned),
            ..VideoInfo::default()
        }
    }

    /// The shapes mpv reported for the generated HDR fixtures.
    #[test]
    fn pq_hlg_and_dolby_vision_are_hdr_and_sdr_is_not() {
        assert!(is_hdr(&video(Some("pq"), Some("bt.2020-ncl"))));
        assert!(is_hdr(&video(Some("hlg"), Some("bt.2020-ncl"))));
        assert!(is_hdr(&video(Some("pq"), Some("dolbyvision"))));
        assert!(is_dolby_vision(&video(Some("pq"), Some("dolbyvision"))));
        assert!(!is_hdr(&video(Some("bt.1886"), Some("bt.709"))));
        assert!(!is_hdr(&video(None, None)));
    }

    /// The two forms mpv 0.41 gave for the property the next-episode keys set.
    #[test]
    fn a_next_episode_key_reads_as_asked_in_either_form() {
        assert!(asked(Some("\"yes\"")));
        assert!(asked(Some("yes")));
        assert!(!asked(Some("\"no\"")));
        assert!(!asked(None));
    }

    #[test]
    fn the_hdr_window_turns_mpvs_own_controls_back_on() {
        let options = Output::HdrWindow { fullscreen: true }.options();
        assert!(options.contains(&("vo", "gpu-next")));
        assert!(options.contains(&("osc", "yes")));
        assert!(options.contains(&("fullscreen", "yes")));
        let windowed = Output::HdrWindow { fullscreen: false }.options();
        assert!(windowed.contains(&("fullscreen", "no")));
        assert_eq!(Output::Embedded.options(), vec![("vo", "libmpv")]);
    }
}
