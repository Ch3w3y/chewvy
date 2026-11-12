//! Format and age-rating marks: the real Dolby, DTS, HDR and BBFC logos where a
//! badge names one, a typographic tag where it does not (`4K`, `HEVC`, `5.1`).
//!
//! The logos ship inside the binary as their Commons SVGs (`assets/marks`,
//! sources in its `SOURCES.md`) because the server's own media-flag bundle is
//! not served. Each is rasterised by `resvg` at the exact pixel height it is
//! drawn at, so it stays sharp at any display scale, and cached per height.
//! Format logos are drawn bare in white over the page; tags share their height
//! so a row reads as one set of marks.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use egui::{pos2, vec2, Color32, Rect, TextureHandle, TextureOptions};

/// A row of marks is this tall, in points.
pub const HEIGHT: f32 = 26.0;

/// How a logo's SVG colours become the mark that is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ink {
    /// Dark ink on transparent: the darker the pixel, the more opaque white.
    Dark,
    /// Light ink on a dark box: the lighter the pixel, the more opaque white.
    Light,
    /// Every painted pixel white.
    Silhouette,
    /// Drawn in its own colours (an age-rating symbol).
    Colour,
}

/// One logo shipped with the app.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Logo {
    /// Its file name, also its texture name.
    pub name: &'static str,
    svg: &'static [u8],
    ink: Ink,
    /// Its drawn height in points (optical size: a two-line mark is taller).
    height: f32,
}

impl Logo {}

macro_rules! logo {
    ($name:literal, $ink:expr, $height:expr) => {
        Logo {
            name: $name,
            svg: include_bytes!(concat!("../../assets/marks/", $name, ".svg")),
            ink: $ink,
            height: $height,
        }
    };
}

const DOLBY_VISION: Logo = logo!("dolby_vision", Ink::Dark, 15.0);
const DOLBY_ATMOS: Logo = logo!("dolby_atmos", Ink::Dark, 15.0);
const DOLBY_DIGITAL: Logo = logo!("dolby_digital", Ink::Dark, 17.0);
const DOLBY_DIGITAL_PLUS: Logo = logo!("dolby_digital_plus", Ink::Dark, 20.0);
const DOLBY_TRUEHD: Logo = logo!("dolby_truehd", Ink::Dark, 18.0);
const DTS: Logo = logo!("dts", Ink::Light, 18.0);
const DTS_HD_MA: Logo = logo!("dts_hd_ma", Ink::Silhouette, 22.0);
const HDR10: Logo = logo!("hdr10", Ink::Dark, 16.0);
const HDR10_PLUS: Logo = logo!("hdr10_plus", Ink::Dark, 16.0);
const BBFC: [(&str, Logo); 6] = [
    ("u", logo!("bbfc_u", Ink::Colour, HEIGHT)),
    ("pg", logo!("bbfc_pg", Ink::Colour, HEIGHT)),
    ("12", logo!("bbfc_12", Ink::Colour, HEIGHT)),
    ("12a", logo!("bbfc_12a", Ink::Colour, HEIGHT)),
    ("15", logo!("bbfc_15", Ink::Colour, HEIGHT)),
    ("18", logo!("bbfc_18", Ink::Colour, HEIGHT)),
];

/// The Plex wordmark (white letters, its own chevron), naming the service a
/// library or server belongs to.
pub const PLEX: Logo = logo!("plex_wordmark", Ink::Colour, 12.0);

macro_rules! icon {
    ($name:literal) => {
        Logo {
            name: concat!("icon-", $name),
            svg: include_bytes!(concat!("../../assets/icons/", $name, ".svg")),
            ink: Ink::Silhouette,
            height: 14.0,
        }
    };
}

/// Interface icons: single-colour line drawings, tinted where they are drawn.
pub mod icon {
    use super::{Ink, Logo};

    /// Home.
    pub const HOME: Logo = icon!("home");
    /// Search.
    pub const SEARCH: Logo = icon!("search");
    /// Settings.
    pub const GEAR: Logo = icon!("gear");
    /// Play / resume.
    pub const PLAY: Logo = icon!("play");
    /// Details.
    pub const INFO: Logo = icon!("info");
    /// Back.
    pub const BACK: Logo = icon!("back");
    /// The chosen row.
    pub const CHECK: Logo = icon!("check");
}

/// One mark in a row.
#[derive(Clone, Debug, PartialEq)]
pub enum Mark {
    /// A shipped logo.
    Logo(Logo),
    /// A typographic tag.
    Text(String),
}

/// The logo for one badge word the detail page names, if the app ships one.
#[must_use]
pub fn logo(label: &str) -> Option<Logo> {
    Some(match label {
        "Dolby Vision" => DOLBY_VISION,
        "Atmos" => DOLBY_ATMOS,
        "Dolby Digital" => DOLBY_DIGITAL,
        "Dolby Digital Plus" => DOLBY_DIGITAL_PLUS,
        "Dolby TrueHD" => DOLBY_TRUEHD,
        "DTS" | "DTS-HD" => DTS,
        "DTS-HD MA" => DTS_HD_MA,
        "HDR" | "HDR10" => HDR10,
        "HDR10+" => HDR10_PLUS,
        _ => return None,
    })
}

/// The BBFC symbol for a `contentRating` such as `gb/15` or `12A`.
#[must_use]
pub fn rating(content_rating: &str) -> Option<Logo> {
    let value = content_rating.trim().to_ascii_lowercase();
    let value = value.strip_prefix("gb/").unwrap_or(&value);
    BBFC.iter()
        .find(|(key, _)| *key == value)
        .map(|(_, logo)| *logo)
}

/// A `contentRating` as text when it has no symbol: the `gb/` or `us/`
/// country prefix the server keeps is dropped (`us/PG-13` is `PG-13`).
#[must_use]
pub fn rating_text(content_rating: &str) -> String {
    let value = content_rating.trim();
    match value.split_once('/') {
        Some((country, rest)) if country.len() == 2 && !rest.is_empty() => rest.to_owned(),
        _ => value.to_owned(),
    }
}

/// A badge label split into marks: `Dolby TrueHD · Atmos` is two logos,
/// `Dolby Digital · 5.1` a logo and a tag.
#[must_use]
pub fn marks(label: &str) -> Vec<Mark> {
    label
        .split(" · ")
        .filter(|part| !part.is_empty())
        .map(|part| {
            logo(part)
                .or_else(|| rating(part))
                .map_or_else(|| Mark::Text(part.to_owned()), Mark::Logo)
        })
        .collect()
}

/// The part of an SVG's canvas its ink covers, as fractions of its width and
/// height, so a mark is drawn without the file's own margins.
type Crop = [f32; 4];

/// A parsed SVG with its ink crop.
type Parsed = Arc<(resvg::usvg::Tree, Crop)>;

/// The parsed SVG and its ink crop, kept for the life of the process.
fn tree(logo: Logo) -> Option<Parsed> {
    static TREES: OnceLock<Mutex<HashMap<&'static str, Parsed>>> = OnceLock::new();
    let trees = TREES.get_or_init(|| Mutex::new(HashMap::new()));
    let mut trees = trees.lock().ok()?;
    if let Some(found) = trees.get(logo.name) {
        return Some(Arc::clone(found));
    }
    let tree = resvg::usvg::Tree::from_data(logo.svg, &resvg::usvg::Options::default()).ok()?;
    let crop = ink_crop(&tree, logo.ink)?;
    let entry = Arc::new((tree, crop));
    trees.insert(logo.name, Arc::clone(&entry));
    Some(entry)
}

/// Render the whole canvas small once and find where the ink is.
fn ink_crop(tree: &resvg::usvg::Tree, ink: Ink) -> Option<Crop> {
    const PROBE: f32 = 256.0;
    let size = tree.size();
    let scale = PROBE / size.height().max(1.0);
    let (w, h) = ((size.width() * scale).ceil().max(1.0) as u32, PROBE as u32);
    let pixels = render(tree, ink, scale, (0.0, 0.0), (w, h))?;
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for (index, [_, _, _, alpha]) in pixels.as_chunks::<4>().0.iter().enumerate() {
        if *alpha > 8 {
            let (x, y) = (index as u32 % w, index as u32 / w);
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x + 1);
            y1 = y1.max(y + 1);
        }
    }
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let (wf, hf) = (w as f32, h as f32);
    Some([
        x0 as f32 / wf,
        y0 as f32 / hf,
        x1 as f32 / wf,
        y1 as f32 / hf,
    ])
}

/// Rasterise `tree` at `scale`, shifted by `offset` pixels, into a `size`
/// canvas, then turn its colours into `ink`: straight (unmultiplied) RGBA.
fn render(
    tree: &resvg::usvg::Tree,
    ink: Ink,
    scale: f32,
    offset: (f32, f32),
    size: (u32, u32),
) -> Option<Vec<u8>> {
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size.0, size.1)?;
    let transform =
        resvg::tiny_skia::Transform::from_row(scale, 0.0, 0.0, scale, offset.0, offset.1);
    resvg::render(tree, transform, &mut pixmap.as_mut());
    let mut out = Vec::with_capacity(pixmap.pixels().len() * 4);
    for px in pixmap.pixels() {
        let c = px.demultiply();
        let (r, g, b, a) = (c.red(), c.green(), c.blue(), c.alpha());
        let luma = (0.2126 * f32::from(r) + 0.7152 * f32::from(g) + 0.0722 * f32::from(b)) / 255.0;
        let alpha = f32::from(a);
        let white = |coverage: f32| [255, 255, 255, coverage.round().clamp(0.0, 255.0) as u8];
        out.extend_from_slice(&match ink {
            Ink::Dark => white(alpha * (1.0 - luma)),
            Ink::Light => white(alpha * luma),
            Ink::Silhouette => white(alpha),
            Ink::Colour => [r, g, b, a],
        });
    }
    Some(out)
}

/// The texture for `logo` at `height_px` device pixels, rasterised the first
/// time a window asks for that size.
fn texture(ctx: &egui::Context, logo: Logo, height_px: u32) -> Option<TextureHandle> {
    type Cache = HashMap<(&'static str, u32), TextureHandle>;
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    let key = (logo.name, height_px.max(1));
    if let Some(handle) = CACHE
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .ok()?
        .get(&key)
    {
        return Some(handle.clone());
    }
    let entry = tree(logo)?;
    let (tree, [x0, y0, x1, y1]) = (&entry.0, entry.1);
    let size = tree.size();
    let (ink_w, ink_h) = ((x1 - x0) * size.width(), (y1 - y0) * size.height());
    let scale = key.1 as f32 / ink_h.max(1e-3);
    let width = (ink_w * scale).ceil().max(1.0) as u32;
    let offset = (-x0 * size.width() * scale, -y0 * size.height() * scale);
    let pixels = render(tree, logo.ink, scale, offset, (width, key.1))?;
    let image = egui::ColorImage::from_rgba_unmultiplied([width as usize, key.1 as usize], &pixels);
    let handle = ctx.load_texture(
        format!("mark-{}-{}", logo.name, key.1),
        image,
        TextureOptions::LINEAR,
    );
    CACHE
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .ok()?
        .insert(key, handle.clone());
    Some(handle)
}

/// A logo's drawn size in points at `height`, from its ink's aspect ratio.
fn logo_size(logo: Logo, height: f32) -> Option<egui::Vec2> {
    let entry = tree(logo)?;
    let [x0, y0, x1, y1] = entry.1;
    let size = entry.0.size();
    let aspect = ((x1 - x0) * size.width()) / ((y1 - y0) * size.height()).max(1e-3);
    Some(vec2(height * aspect, height))
}

/// [`paint_logo`] in `tint`: a white silhouette (an icon) takes its colour.
pub fn paint_tinted(
    painter: &egui::Painter,
    logo: Logo,
    left: f32,
    center_y: f32,
    height: f32,
    tint: Color32,
) -> f32 {
    let ctx = painter.ctx();
    let Some(size) = logo_size(logo, height) else {
        return 0.0;
    };
    let px = (height * ctx.pixels_per_point()).round() as u32;
    let Some(handle) = texture(ctx, logo, px) else {
        return 0.0;
    };
    let rect = Rect::from_min_size(pos2(left, center_y - size.y / 2.0), size);
    painter.image(
        handle.id(),
        rect,
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        tint,
    );
    size.x
}

/// A logo's drawn width at `height` points.
#[must_use]
pub fn width_at(logo: Logo, height: f32) -> f32 {
    logo_size(logo, height).map_or(0.0, |size| size.x)
}

#[cfg(test)]
mod tests {
    use super::{logo, marks, rating, rating_text, tree, Mark, BBFC};

    #[test]
    fn audio_badges_split_into_logos_and_text() {
        let truehd = marks("Dolby TrueHD · Atmos");
        assert!(matches!(truehd.as_slice(), [Mark::Logo(a), Mark::Logo(b)]
            if a.name == "dolby_truehd" && b.name == "dolby_atmos"));
        let digital = marks("Dolby Digital · 5.1");
        assert!(matches!(digital.as_slice(), [Mark::Logo(a), Mark::Text(t)]
            if a.name == "dolby_digital" && t == "5.1"));
        assert_eq!(marks("AAC · 2.0").len(), 2);
        assert!(marks("AAC · 2.0")
            .iter()
            .all(|m| matches!(m, Mark::Text(_))));
    }

    #[test]
    fn picture_badges_name_their_logos() {
        assert_eq!(logo("Dolby Vision").map(|l| l.name), Some("dolby_vision"));
        assert_eq!(logo("HDR10+").map(|l| l.name), Some("hdr10_plus"));
        assert_eq!(logo("HDR").map(|l| l.name), Some("hdr10"));
        assert!(logo("4K").is_none());
    }

    #[test]
    fn uk_ratings_map_to_bbfc_symbols() {
        assert_eq!(rating("gb/15").map(|l| l.name), Some("bbfc_15"));
        assert_eq!(rating("12A").map(|l| l.name), Some("bbfc_12a"));
        assert_eq!(rating("gb/PG").map(|l| l.name), Some("bbfc_pg"));
        assert!(rating("PG-13").is_none());
        assert!(rating("TV-MA").is_none());
    }

    #[test]
    fn ratings_without_a_symbol_drop_the_country() {
        assert_eq!(rating_text("us/PG-13"), "PG-13");
        assert_eq!(rating_text("TV-MA"), "TV-MA");
        assert_eq!(rating_text("de/16"), "16");
    }

    #[test]
    fn every_shipped_logo_parses_and_has_ink() {
        for name in [
            "Dolby Vision",
            "Atmos",
            "Dolby Digital",
            "Dolby Digital Plus",
            "Dolby TrueHD",
            "DTS",
            "DTS-HD MA",
            "HDR",
            "HDR10+",
        ] {
            let logo = logo(name).unwrap_or_else(|| panic!("{name} has a logo"));
            let entry = tree(logo).unwrap_or_else(|| panic!("{name} renders with ink"));
            let [x0, y0, x1, y1] = entry.1;
            assert!(x1 > x0 && y1 > y0, "{name} crop");
        }
        for (key, logo) in BBFC {
            assert!(tree(logo).is_some(), "{key} renders with ink");
        }
    }
}
