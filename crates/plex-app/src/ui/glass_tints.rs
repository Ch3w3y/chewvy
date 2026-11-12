//! The glass tints the app-owned surfaces hand to [`crate::ui::glass::panel`]
//! and the legibility rule every one of them is held to.
//!
//! A glass panel shows what is behind it blurred, brightened and tinted, so
//! the worst case for text on it is a pure white backdrop: there the tint is
//! all the ink has to sit on. Keeping the tints in one module means one test
//! holds every surface named here to the same bar.

use egui::Color32;

/// The sidebar: night blue over the blurred backdrop and hero art, light
/// enough that the aurora reads through it.
pub const RAIL: Color32 = Color32::from_rgba_premultiplied(7, 8, 22, 184);

/// A Home shelf's round pager arrow: dark enough to
/// keep [`theme::TEXT`](crate::ui::theme) readable over bright artwork.
pub const SHELF_ARROW: Color32 = Color32::from_rgba_premultiplied(8, 9, 11, 190);

/// The detail page's Play-from panel over the page's art.
pub const PANEL: Color32 = Color32::from_rgba_premultiplied(8, 9, 24, 190);
/// The sign-in screens' card, which carries its own secondary status line.
pub const SPLASH_CARD: Color32 = Color32::from_rgba_premultiplied(9, 10, 16, 178);
/// The error strip the sign-in screens draw over their backdrop.
pub const STATUS_STRIP: Color32 = Color32::from_rgba_premultiplied(40, 8, 8, 190);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme;

    /// The WCAG AA contrast [`theme::TEXT`] keeps over a glass surface.
    const TEXT_CONTRAST: f32 = 4.5;
    /// The contrast [`theme::SECONDARY`] keeps over a glass surface.
    const SECONDARY_CONTRAST: f32 = 3.0;
    /// The backdrop every tint is tested over: bright artwork, where a light
    /// surface would lose its text.
    const WORST_BACKDROP: Color32 = Color32::WHITE;

    /// `front` over `back`, premultiplied: what the backdrop leaves visible
    /// under the tint.
    fn blend(front: Color32, back: Color32) -> Color32 {
        let [fr, fg, fb, fa] = front.to_array();
        let [br, bg, bb, _] = back.to_array();
        let a = f32::from(fa) / 255.0;
        let over = |f: u8, b: u8| (f32::from(f) * a + f32::from(b) * (1.0 - a)).round() as u8;
        Color32::from_rgb(over(fr, br), over(fg, bg), over(fb, bb))
    }

    /// The WCAG ratio [`theme::contrast`] gives, so the contrast a user
    /// actually gets is checked rather than eyeballed.
    fn contrast(a: Color32, b: Color32) -> f32 {
        theme::contrast(a, b)
    }

    /// Every tint the surfaces owned here paint glass with, and the ink each
    /// one carries: a surface with no secondary label only has to hold its own
    /// text. The window-control tab's tint is not here because that module is
    /// owned elsewhere (see the module doc).
    fn tints() -> Vec<(&'static str, Color32, bool)> {
        vec![
            ("rail", RAIL, true),
            ("shelf_arrow", SHELF_ARROW, true),
            ("panel", PANEL, true),
            ("splash_card", SPLASH_CARD, true),
            ("status_strip", STATUS_STRIP, false),
        ]
    }

    #[test]
    fn every_tint_keeps_text_and_secondary_readable_on_white() {
        for (name, tint, carries_secondary) in tints() {
            let surface = blend(tint, WORST_BACKDROP);
            let text = contrast(theme::TEXT, surface);
            assert!(text >= TEXT_CONTRAST, "{name}: TEXT {text:.2}:1 on white");
            if carries_secondary {
                let secondary = contrast(theme::SECONDARY, surface);
                assert!(
                    secondary >= SECONDARY_CONTRAST,
                    "{name}: SECONDARY {secondary:.2}:1 on white"
                );
            }
        }
    }

    #[test]
    fn blending_a_dark_tint_keeps_a_white_backdrop_dark() {
        let [r, g, b, _] = blend(SHELF_ARROW, WORST_BACKDROP).to_array();
        assert!(r < 90 && g < 90 && b < 90, "{r},{g},{b} over white");
        assert_eq!(
            blend(Color32::from_black_alpha(0), WORST_BACKDROP),
            WORST_BACKDROP
        );
        assert_eq!(blend(Color32::BLACK, WORST_BACKDROP), Color32::BLACK);
    }

    #[test]
    fn contrast_is_the_wcag_ratio_in_both_orders() {
        assert!(contrast(Color32::WHITE, Color32::BLACK) > 20.9);
        assert_eq!(
            contrast(Color32::WHITE, Color32::BLACK),
            contrast(Color32::BLACK, Color32::WHITE)
        );
    }
}
