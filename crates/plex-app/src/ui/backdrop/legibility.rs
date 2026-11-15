//! How much window colour one photograph needs over it, measured from its own
//! pixels, so a bright photograph is dimmed more than a dark one and text
//! drawn straight on it keeps the same contrast on every background.

use egui::Color32;

use crate::ui::theme::{self, Backdrop, Palette};

/// The share of a photograph's pixels text is held to: titles and captions
/// must read on everything but its brightest 5%.
const BAND: usize = 95;
/// Primary text on the browsing copy's bright band, as the backdrop tests ask.
const BROWSE_TEXT: f32 = 7.0;
/// Secondary text there; a shadow is drawn under it ([`theme::SMOKE`]).
const BROWSE_SECONDARY: f32 = 3.0;
/// The least dim over the browsing copy, so posters always stand off it.
pub const BROWSE_FLOOR: f32 = 0.25;
/// Primary text on the sign-in copy, which shows the scene itself: its text
/// sits on glass cards, so the photograph is held only to the large-text
/// level and only the brightest are toned down.
const SIGN_IN_TEXT: f32 = 3.0;
/// The most either copy is dimmed: past this a photograph reads as black.
const CEILING: f32 = 0.85;
/// How finely a dim is searched for.
const STEP: f32 = 0.01;
/// Pixels sampled from a copy: enough for a stable percentile.
const SAMPLES: usize = 8192;
/// Headroom on each ratio, so a percentile read from other pixels of the
/// same copy still clears the target.
const MARGIN: f32 = 1.03;

/// The backdrop values for a photograph whose sign-in copy has `sharp`
/// pixels and whose browsing copy has `soft` pixels, as RGB triples.
#[must_use]
pub fn backdrop_for(palette: &Palette, sharp: &[[u8; 3]], soft: &[[u8; 3]]) -> Backdrop {
    let sharp = sample(sharp);
    let soft = sample(soft);
    let browse_dim = least_dim(BROWSE_FLOOR, |dim| {
        let band = bright_band(&soft, palette.window, dim);
        on(palette.text, band) >= BROWSE_TEXT * MARGIN
            && on(palette.secondary, band) >= BROWSE_SECONDARY * MARGIN
    });
    let sign_in_dim = least_dim(0.0, |dim| {
        on(palette.text, bright_band(&sharp, palette.window, dim)) >= SIGN_IN_TEXT * MARGIN
    });
    Backdrop {
        sign_in_dim,
        browse_dim,
        text_shadow: theme::SMOKE.backdrop.text_shadow,
    }
}

/// The luminance `share` of the `pixels` stays under once `dim` of `window`
/// lies over them: the brightest ground text there can meet.
#[must_use]
pub fn bright_band(pixels: &[Color32], window: Color32, dim: f32) -> f32 {
    let mut levels: Vec<f32> = pixels
        .iter()
        .map(|pixel| theme::luminance(theme::mix(*pixel, window, dim)))
        .collect();
    levels.sort_by(f32::total_cmp);
    levels
        .get(levels.len() * BAND / 100)
        .copied()
        .unwrap_or(1.0)
}

/// The contrast of `ink` over a ground of luminance `band`.
#[must_use]
pub fn on(ink: Color32, band: f32) -> f32 {
    (theme::luminance(ink) + 0.05) / (band + 0.05)
}

fn least_dim(floor: f32, holds: impl Fn(f32) -> bool) -> f32 {
    let mut dim = floor;
    while dim < CEILING && !holds(dim) {
        dim += STEP;
    }
    (dim.min(CEILING) * 100.0).ceil() / 100.0
}

fn sample(pixels: &[[u8; 3]]) -> Vec<Color32> {
    let stride = (pixels.len() / SAMPLES).max(1);
    pixels
        .iter()
        .step_by(stride)
        .map(|rgb| theme::pixel(*rgb))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(level: u8) -> Vec<[u8; 3]> {
        vec![[level; 3]; 64]
    }

    #[test]
    fn a_brighter_photograph_is_dimmed_more() {
        let palette = theme::SMOKE.palette;
        let dark = backdrop_for(&palette, &flat(40), &flat(40));
        let bright = backdrop_for(&palette, &flat(230), &flat(230));
        assert_eq!(dark.browse_dim, BROWSE_FLOOR);
        assert_eq!(dark.sign_in_dim, 0.0);
        assert!(bright.browse_dim > dark.browse_dim + 0.2, "{bright:?}");
        assert!(bright.sign_in_dim > 0.0, "{bright:?}");
    }
}
