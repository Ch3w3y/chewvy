//! How good one backdrop will look behind the glass, measured on a small
//! decoded thumbnail the caller passes in.
//!
//! The backdrop is blurred, dimmed and seen through tinted glass, so fine
//! detail does not matter and colour, exposure and tonal range do. Four
//! measures, each `0..=1`:
//!
//! * colourfulness, Hasler and Süsstrunk (2003): `σ_rgyb + 0.3·μ_rgyb` over the
//!   opponent channels `rg = R − G` and `yb = (R + G)/2 − B`, on 0 to 255
//!   values, where about 80 is "highly colourful";
//! * exposure, from mean luma: near-black frames vanish under the dim, and
//!   blown-out ones break text contrast;
//! * spread, the 5th to 95th percentile luma range: a flat frame blurs to a
//!   wash;
//! * text, a heuristic for a title or logo burned into the art: lettering puts
//!   many strong left-to-right luma steps into one horizontal band, while
//!   photographic texture spreads them over the frame, so it scores the
//!   densest band's excess over the median band.

/// Backdrops narrower than this are never shown: they go soft when scaled to
/// a desktop window.
pub const MIN_WIDTH: u32 = 1920;

/// An image's full size in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size {
    /// Width.
    pub width: u32,
    /// Height.
    pub height: u32,
}

/// A decoded RGBA8 thumbnail. Only a buffer whose length matches its size can
/// be built.
#[derive(Debug, Clone, Copy)]
pub struct Thumbnail<'a> {
    width: usize,
    height: usize,
    rgba: &'a [u8],
}

impl<'a> Thumbnail<'a> {
    /// `rgba` row-major, four bytes a pixel, at least 2 by 2 pixels.
    pub fn new(width: u32, height: u32, rgba: &'a [u8]) -> Option<Self> {
        let (w, h) = (usize::try_from(width).ok()?, usize::try_from(height).ok()?);
        let ok = w >= 2 && h >= 2 && w.checked_mul(h)?.checked_mul(4)? == rgba.len();
        ok.then_some(Self {
            width: w,
            height: h,
            rgba,
        })
    }
}

/// What a thumbnail looks like, each measure `0..=1`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Look {
    /// Hasler-Süsstrunk colourfulness, scaled so 80 reads as 1.
    pub colour: f32,
    /// How well the mean luma suits a dimmed backdrop; 0 for near-black or
    /// blown out.
    pub exposure: f32,
    /// Tonal range; 0 for a flat frame.
    pub spread: f32,
    /// Likelihood of burned-in title text.
    pub text: f32,
}

/// Below this an image is not worth showing even if it is the title's only one.
const MIN_LOOK: f32 = 0.12;
/// The score of an image nobody has measured yet: below a good measured one,
/// above a bad one.
const UNMEASURED: f32 = 0.5;
const TEXT_PENALTY: f32 = 0.25;

impl Look {
    /// One score, `0..=1`. Exposure gates the rest, because a black frame is
    /// useless however colourful its few lit pixels are.
    pub fn score(&self) -> f32 {
        let body = 0.55 * self.colour + 0.45 * self.spread;
        (self.exposure * body * (1.0 - TEXT_PENALTY * self.text)).clamp(0.0, 1.0)
    }
}

/// Measure a thumbnail.
pub fn look(thumb: &Thumbnail) -> Look {
    let pixels = thumb.rgba.as_chunks::<4>().0;
    let lumas: Vec<f32> = pixels.iter().map(luma).collect();
    let mean = lumas.iter().sum::<f32>() / lumas.len() as f32;
    Look {
        colour: (colourfulness(pixels) / 80.0).clamp(0.0, 1.0),
        exposure: exposure(mean),
        spread: (spread(&lumas) / 0.55).clamp(0.0, 1.0),
        text: text_band(&lumas, thumb.width, thumb.height),
    }
}

/// The score an image ranks on, or `None` when it must never be shown.
pub(super) fn image_score(size: Option<Size>, look: Option<&Look>) -> Option<f32> {
    if size.is_some_and(|s| s.width < MIN_WIDTH) {
        return None;
    }
    let score = look.map_or(UNMEASURED, Look::score);
    (score >= MIN_LOOK).then_some(score)
}

fn luma(&[r, g, b, _]: &[u8; 4]) -> f32 {
    (0.2126 * f32::from(r) + 0.7152 * f32::from(g) + 0.0722 * f32::from(b)) / 255.0
}

fn colourfulness(pixels: &[[u8; 4]]) -> f32 {
    let (mut n, mut s_rg, mut s_yb, mut q_rg, mut q_yb) = (0.0f64, 0.0, 0.0, 0.0, 0.0);
    for &[r, g, b, _] in pixels {
        let [r, g, b] = [r, g, b].map(f64::from);
        let rg = r - g;
        let yb = 0.5 * (r + g) - b;
        n += 1.0;
        s_rg += rg;
        s_yb += yb;
        q_rg += rg * rg;
        q_yb += yb * yb;
    }
    let (m_rg, m_yb) = (s_rg / n, s_yb / n);
    let var_rg = (q_rg / n - m_rg * m_rg).max(0.0);
    let var_yb = (q_yb / n - m_yb * m_yb).max(0.0);
    let sigma = (var_rg + var_yb).sqrt();
    let mu = (m_rg * m_rg + m_yb * m_yb).sqrt();
    (sigma + 0.3 * mu) as f32
}

/// 0 at or below 0.04 and at or above 0.92, 1 between 0.18 and 0.55, linear
/// between. The plateau sits low because the app dims the backdrop further.
fn exposure(mean: f32) -> f32 {
    let rise = ((mean - 0.04) / (0.18 - 0.04)).clamp(0.0, 1.0);
    let fall = ((0.92 - mean) / (0.92 - 0.55)).clamp(0.0, 1.0);
    rise.min(fall)
}

fn spread(lumas: &[f32]) -> f32 {
    let mut sorted = lumas.to_vec();
    sorted.sort_by(f32::total_cmp);
    let at = |q: f32| {
        let i = ((sorted.len() - 1) as f32 * q).round() as usize;
        sorted.get(i).copied().unwrap_or(0.0)
    };
    at(0.95) - at(0.05)
}

const STRONG_STEP: f32 = 0.2;
const BANDS: usize = 8;

fn text_band(lumas: &[f32], width: usize, height: usize) -> f32 {
    let row_density: Vec<f32> = lumas
        .chunks_exact(width)
        .map(|row| {
            let steps = row
                .windows(2)
                .filter(|p| matches!(p, [a, b] if (a - b).abs() > STRONG_STEP));
            steps.count() as f32 / (width - 1) as f32
        })
        .collect();
    let band_rows = height.div_ceil(BANDS).max(1);
    let mut bands: Vec<f32> = row_density
        .chunks(band_rows)
        .map(|b| b.iter().sum::<f32>() / b.len() as f32)
        .collect();
    bands.sort_by(f32::total_cmp);
    let median = bands.get(bands.len() / 2).copied().unwrap_or(0.0);
    let max = bands.last().copied().unwrap_or(0.0);
    ((max - median - 0.05) / 0.15).clamp(0.0, 1.0)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
pub(super) mod tests {
    use super::*;

    pub const W: u32 = 160;
    pub const H: u32 = 90;

    /// A thumbnail painted by `f(x, y) -> [r, g, b]`.
    pub fn paint(f: impl Fn(u32, u32) -> [u8; 3]) -> Vec<u8> {
        let mut out = Vec::with_capacity((W * H * 4) as usize);
        for y in 0..H {
            for x in 0..W {
                let [r, g, b] = f(x, y);
                out.extend_from_slice(&[r, g, b, 255]);
            }
        }
        out
    }

    /// A sunset-like frame: warm sky over a cool, darker landscape.
    pub fn rich() -> Vec<u8> {
        paint(rich_pixel)
    }

    fn rich_pixel(x: u32, y: u32) -> [u8; 3] {
        let t = y as f32 / H as f32;
        let s = x as f32 / W as f32;
        if t < 0.55 {
            [
                (230.0 - 60.0 * t) as u8,
                (120.0 + 60.0 * s) as u8,
                (60.0 + 80.0 * t) as u8,
            ]
        } else {
            [
                (30.0 + 40.0 * s) as u8,
                (60.0 + 30.0 * t) as u8,
                (110.0 + 60.0 * s) as u8,
            ]
        }
    }

    fn look_of(px: &[u8]) -> Look {
        look(&Thumbnail::new(W, H, px).unwrap())
    }

    #[test]
    fn a_rich_frame_beats_a_dark_one_and_a_flat_one() {
        let rich = look_of(&rich()).score();
        let dark = look_of(&paint(|x, _| [8, 6, (10 + x % 4) as u8])).score();
        let flat = look_of(&paint(|_, _| [128, 128, 128])).score();
        assert!(rich > 0.4, "rich {rich}");
        assert!(dark < 0.05, "dark {dark}");
        assert!(flat < 0.05, "flat {flat}");
        assert!(image_score(None, Some(&look_of(&paint(|_, _| [3, 3, 3])))).is_none());
    }

    #[test]
    fn a_blown_out_frame_loses_to_a_rich_one() {
        let blown = look_of(&paint(|x, _| [250, 250, (240 + x % 10) as u8]));
        assert_eq!(blown.exposure, 0.0);
        assert!(blown.score() < look_of(&rich()).score());
    }

    #[test]
    fn a_burned_in_title_costs_a_little_but_does_not_disqualify() {
        let titled = paint(|x, y| {
            let in_band = (60..72).contains(&y) && (20..140).contains(&x);
            if in_band && (x / 2) % 2 == 0 {
                [250, 250, 250]
            } else {
                rich_pixel(x, y)
            }
        });
        let clean = look_of(&rich());
        let with_text = look_of(&titled);
        assert!(with_text.text > 0.5, "text {}", with_text.text);
        assert!(clean.text < 0.1, "clean text {}", clean.text);
        assert!(image_score(None, Some(&with_text)).is_some());
    }

    #[test]
    fn known_sizes_under_1920_wide_are_rejected() {
        let small = Size {
            width: 1280,
            height: 720,
        };
        let full = Size {
            width: 1920,
            height: 1080,
        };
        let good = look_of(&rich());
        assert!(image_score(Some(small), Some(&good)).is_none());
        assert!(image_score(Some(full), Some(&good)).is_some());
        assert!(
            image_score(None, None).is_some(),
            "unknown size, unmeasured"
        );
    }

    #[test]
    fn a_buffer_that_does_not_match_its_size_is_refused() {
        assert!(Thumbnail::new(4, 4, &[0; 63]).is_none());
        assert!(Thumbnail::new(1, 4, &[0; 16]).is_none());
        assert!(Thumbnail::new(4, 4, &[0; 64]).is_some());
    }
}
