//! Player choices that carry over from one item to the next while the app runs.

use std::cell::Cell;

/// The most audio channels mpv may output.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Channels {
    /// What the output device reports.
    #[default]
    Auto,
    /// Two channels.
    Stereo,
    /// Six channels.
    Surround51,
    /// Eight channels.
    Surround71,
}

impl Channels {
    /// Every choice, in menu order.
    pub const ALL: [Self; 4] = [Self::Auto, Self::Stereo, Self::Surround51, Self::Surround71];

    /// What the menu shows.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::Stereo => "Stereo",
            Self::Surround51 => "5.1",
            Self::Surround71 => "7.1",
        }
    }

    fn mpv(self) -> &'static str {
        match self {
            Self::Auto => "auto-safe",
            Self::Stereo => "stereo",
            Self::Surround51 => "5.1",
            Self::Surround71 => "7.1",
        }
    }
}

/// The carried-over choices.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sticky {
    /// Deinterlace (for interlaced TV recordings).
    pub deinterlace: bool,
    /// Even out loud effects and quiet dialogue.
    pub normalize: bool,
    /// Send Dolby and DTS bitstreams untouched to an AV receiver.
    pub passthrough: bool,
    /// Downmix ceiling.
    pub channels: Channels,
    /// Resample to the display and interpolate frames for smoother motion.
    pub smooth: bool,
}

/// The loudness filter [`Sticky::normalize`] switches on.
const NORMALIZE: &str = "lavfi=[dynaudnorm=g=5:f=250:r=0.9:p=0.5]";
/// The bitstreams [`Sticky::passthrough`] passes to the receiver.
const PASSTHROUGH: &str = "ac3,eac3,dts,dts-hd,truehd";

impl Sticky {
    /// The mpv property values these choices stand for.
    #[must_use]
    pub fn properties(&self) -> [(&'static str, &'static str); 6] {
        let yes_no = |on: bool| if on { "yes" } else { "no" };
        [
            ("deinterlace", yes_no(self.deinterlace)),
            ("af", if self.normalize { NORMALIZE } else { "" }),
            (
                "audio-spdif",
                if self.passthrough { PASSTHROUGH } else { "" },
            ),
            ("audio-channels", self.channels.mpv()),
            (
                "video-sync",
                if self.smooth {
                    "display-resample"
                } else {
                    "audio"
                },
            ),
            ("interpolation", yes_no(self.smooth)),
        ]
    }
}

thread_local! {
    static STICKY: Cell<Sticky> = const { Cell::new(Sticky {
        deinterlace: false,
        normalize: false,
        passthrough: false,
        channels: Channels::Auto,
        smooth: false,
    }) };
}

/// The current choices.
#[must_use]
pub fn get() -> Sticky {
    STICKY.with(Cell::get)
}

/// Replace the choices (the playing item applies them separately).
pub fn set(sticky: Sticky) {
    STICKY.with(|cell| cell.set(sticky));
}

#[cfg(test)]
mod tests {
    use super::{Channels, Sticky};

    #[test]
    fn defaults_leave_mpv_on_its_own_behaviour() {
        let props = Sticky::default().properties();
        assert!(props.contains(&("audio-spdif", "")));
        assert!(props.contains(&("audio-channels", "auto-safe")));
        assert!(props.contains(&("video-sync", "audio")));
    }

    #[test]
    fn passthrough_and_smooth_motion_map_to_their_properties() {
        let sticky = Sticky {
            passthrough: true,
            smooth: true,
            channels: Channels::Surround51,
            ..Sticky::default()
        };
        let props = sticky.properties();
        assert!(props.contains(&("audio-spdif", "ac3,eac3,dts,dts-hd,truehd")));
        assert!(props.contains(&("video-sync", "display-resample")));
        assert!(props.contains(&("interpolation", "yes")));
        assert!(props.contains(&("audio-channels", "5.1")));
    }
}
