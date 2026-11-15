//! Interface icons: one family of line drawings on a 20-unit grid (stroke 2,
//! round caps and joins), black on transparent in `assets/icons` and tinted
//! where they are drawn.
//!
//! An icon is drawn whole, never cropped to its ink, in a square box whose
//! corner sits on a device pixel. The set puts every straight edge on an even
//! grid unit, so in a box a whole multiple of 10 px wide each of those edges
//! lands on a whole pixel. [`SIZE`] is 20, 30 and 40 px at 1x, 1.5x and 2x.

use egui::{pos2, vec2, Color32, Painter, Pos2, Rect};

use super::{texture, Crop, Ink, Logo};

/// The size every icon is drawn at, in points: the grid's 20 units.
pub const SIZE: f32 = 20.0;

/// The whole canvas: an icon keeps its grid's own margins.
const WHOLE: Crop = [0.0, 0.0, 1.0, 1.0];

/// How large an icon is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Size {
    /// [`SIZE`]: every icon but the main play action.
    Regular,
    /// The play icon on the Play button and a card's play disc, which has to
    /// read before its label does.
    Primary,
    /// The player's play and pause, the screen's main control.
    Transport,
}

impl Size {
    /// The box's side in device pixels at `ppp` pixels per point. The two
    /// large sizes take the smallest box at least their size that is a whole
    /// multiple of 10 px, so they stay as crisp as [`SIZE`] at every scale.
    fn pixels(self, ppp: f32) -> u32 {
        let least = match self {
            Self::Regular => return (SIZE * ppp).round().max(1.0) as u32,
            Self::Primary => 26.0,
            Self::Transport => 30.0,
        };
        ((least * ppp / 10.0).ceil() * 10.0) as u32
    }
}

macro_rules! icons {
    ($($(#[doc = $doc:literal])* $variant:ident = $file:literal,)*) => {
        /// One interface icon: a file in `assets/icons`.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Icon {
            $($(#[doc = $doc])* $variant,)*
        }

        impl Icon {
            /// Every icon the app ships.
            #[cfg(test)]
            const ALL: &[Icon] = &[$(Icon::$variant,)*];

            fn logo(self) -> Logo {
                match self {
                    $(Icon::$variant => Logo {
                        name: concat!("icon-", $file),
                        svg: include_bytes!(concat!("../../../assets/icons/", $file, ".svg")),
                        ink: Ink::Silhouette,
                        height: SIZE,
                    },)*
                }
            }
        }
    };
}

icons! {
    /// The audio track menu.
    Audio = "audio",
    /// Back to the screen before.
    Back = "back",
    /// The chapter menu.
    Chapters = "chapters",
    /// The row or version in effect.
    Check = "check",
    /// A row that opens a list below it.
    ChevronDown = "chevron-down",
    /// Page back along a row.
    ChevronLeft = "chevron-left",
    /// Page on along a row, or a link that leads on.
    ChevronRight = "chevron-right",
    /// Close a notice, or the window.
    Close = "close",
    /// Download an update.
    Download = "download",
    /// Leave full screen.
    FullscreenExit = "fullscreen-exit",
    /// Enter full screen.
    Fullscreen = "fullscreen",
    /// The landing screen.
    Home = "home",
    /// Details about an item.
    Info = "info",
    /// Sound off.
    Mute = "mute",
    /// The next episode.
    Next = "next",
    /// Pause.
    Pause = "pause",
    /// The people watching together.
    People = "people",
    /// Play or resume.
    Play = "play",
    /// The quality menu.
    Quality = "quality",
    /// Search.
    Search = "search",
    /// Back 10 seconds.
    SeekBack = "seek-back",
    /// Forward 10 seconds.
    SeekForward = "seek-forward",
    /// Settings.
    Settings = "settings",
    /// The speed menu.
    Speed = "speed",
    /// The subtitle menu.
    Subtitles = "subtitles",
    /// Sound on, loud.
    VolumeHigh = "volume-high",
    /// Sound on, quiet.
    VolumeLow = "volume-low",
    /// Maximise the window.
    WinMaximise = "win-maximise",
    /// Minimise the window.
    WinMinimise = "win-minimise",
    /// Restore a maximised window.
    WinRestore = "win-restore",
}

/// Where an icon at `size` centred on `center` is drawn at `ppp`: its box in
/// device pixels, and that box in points with its corner on a whole pixel.
/// The texture is sampled linearly, so a corner half a pixel off would blur
/// every edge the grid put on a pixel.
fn placed(size: Size, center: Pos2, ppp: f32) -> (u32, Rect) {
    let px = size.pixels(ppp);
    let side = px as f32 / ppp;
    let snap = |v: f32| ((v - side / 2.0) * ppp).round() / ppp;
    let rect = Rect::from_min_size(pos2(snap(center.x), snap(center.y)), vec2(side, side));
    (px, rect)
}

/// Paint `icon` at `size`, centred on `center`, in `tint`.
pub fn paint(painter: &Painter, icon: Icon, size: Size, center: Pos2, tint: Color32) {
    let ctx = painter.ctx();
    let (px, rect) = placed(size, center, ctx.pixels_per_point());
    if let Some(handle) = texture(ctx, icon.logo(), WHOLE, px) {
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        painter.image(handle.id(), rect, uv, tint);
    }
}

#[cfg(test)]
mod tests {
    use super::super::tree;
    use super::{placed, Icon, Size, SIZE};

    const SCALES: [f32; 3] = [1.0, 1.5, 2.0];

    /// Each file is drawn on the set's grid, so one draw size fits them all.
    #[test]
    fn every_icon_is_drawn_on_the_twenty_unit_grid_and_has_ink() {
        for icon in Icon::ALL {
            let logo = icon.logo();
            let svg = std::str::from_utf8(logo.svg).unwrap_or_default();
            assert!(svg.contains(r#"viewBox="0 0 20 20""#), "{icon:?} grid");
            assert!(!svg.contains("<text"), "{icon:?} draws its letters");
            assert!(tree(logo).is_some(), "{icon:?} renders with ink");
        }
    }

    /// A box a whole multiple of 10 px puts each even grid unit on a pixel,
    /// and its corner lands on one wherever the caller centres it.
    #[test]
    fn every_size_is_pixel_aligned_at_one_one_and_a_half_and_two_x() {
        for ppp in SCALES {
            for size in [Size::Regular, Size::Primary, Size::Transport] {
                let (px, rect) = placed(size, egui::pos2(100.3, 41.7), ppp);
                assert_eq!(px % 10, 0, "{size:?} at {ppp}x is {px} px");
                for v in [rect.min.x, rect.min.y] {
                    let device = v * ppp;
                    assert!((device - device.round()).abs() < 1e-3, "{size:?} at {ppp}x");
                }
                assert!((rect.width() * ppp - px as f32).abs() < 1e-3);
            }
        }
    }

    /// The regular size is the grid itself; the play actions stay at least as
    /// large as the glyphs they replaced (pause bars 17.5 pt tall in the
    /// player, an 18 pt play triangle on the Play button).
    #[test]
    fn the_play_actions_stay_larger_than_the_regular_icons() {
        for ppp in SCALES {
            let points = |size: Size| placed(size, egui::pos2(0.0, 0.0), ppp).1.width();
            assert!((points(Size::Regular) - SIZE).abs() < 1e-3);
            assert!(
                points(Size::Transport) * 12.0 / 20.0 >= 17.5,
                "{ppp}x pause"
            );
            assert!(points(Size::Primary) * 14.0 / 20.0 >= 18.0, "{ppp}x play");
        }
    }
}
