use std::collections::BTreeSet;

use egui::vec2;

use super::choice::{bundled, theme_for, want, Want};
use super::decode::{derive, next_job, Job, Variant, SOFT_WIDTH};
use super::legibility::{bright_band, on, BROWSE_FLOOR};
use super::*;
use crate::theme_art::{CivilDate, Festival, Genre, Hemisphere, LocalTime, Season};
use crate::ui::settings::Background;
use crate::ui::theme;

fn decoded(photo: &Photo) -> image::RgbImage {
    image::load_from_memory(photo.bytes)
        .unwrap_or_else(|e| panic!("{}: {e}", photo.id))
        .to_rgb8()
}

/// Noon on `date`, UTC.
fn noon(year: i32, month: u8, day: u8) -> LocalTime {
    let date = CivilDate::new(year, month, day).unwrap();
    let mut time = LocalTime::from_unix(0, 0);
    while time.date() < date {
        time = LocalTime::from_unix(time.day_number() * 86_400 + 86_400, 0);
    }
    LocalTime::from_unix(time.day_number() * 86_400 + 12 * 3600, 0)
}

#[test]
fn every_theme_has_two_photos_and_the_aurora_its_own() {
    for theme in ThemeId::ALL {
        let count = photos_of(theme).count();
        let wanted = if theme == ThemeId::Aurora { 1 } else { 2 };
        assert_eq!(count, wanted, "{theme:?}");
    }
    let ids: BTreeSet<&str> = PHOTOS.iter().map(|photo| photo.id).collect();
    assert_eq!(ids.len(), PHOTOS.len(), "ids are unique");
}

#[test]
fn every_photo_decodes_at_its_listed_size_within_the_byte_budget() {
    for photo in &PHOTOS {
        let image = decoded(photo);
        assert_eq!([image.width(), image.height()], photo.size, "{}", photo.id);
        assert!(image.width() >= 2048, "{}", photo.id);
        assert!(
            photo.bytes.len() <= 500_000,
            "{}: {} bytes",
            photo.id,
            photo.bytes.len()
        );
    }
}

/// Titles and captions are drawn straight on the photograph. On the
/// browsing copy of every bundled photo, dimmed as its own pixels ask,
/// primary text must hold 7:1 on the bright band and secondary text 3:1 with
/// the shadow drawn under it.
#[test]
fn text_holds_on_the_bright_band_of_every_photograph() {
    let palette = theme::SMOKE.palette;
    let shadowed = theme::SMOKE
        .backdrop
        .text_shadow
        .iter()
        .any(|s| s.alpha > 0);
    assert!(shadowed, "secondary text on a photograph needs its shadow");
    for photo in &PHOTOS {
        let derived = derive(&decoded(photo));
        assert_eq!(derived.soft.width(), SOFT_WIDTH, "{}", photo.id);
        let dim = derived.backdrop.browse_dim;
        assert!(
            (BROWSE_FLOOR..=0.85).contains(&dim),
            "{}: dim {dim}",
            photo.id
        );
        let pixels: Vec<egui::Color32> = derived
            .soft
            .pixels()
            .step_by(7)
            .map(|p| egui::Color32::from_rgb(p.0[0], p.0[1], p.0[2]))
            .collect();
        let band = bright_band(&pixels, palette.window, dim);
        let text = on(palette.text, band);
        let secondary = on(palette.secondary, band);
        assert!(text >= 7.0, "{}: text {text:.2}:1 at dim {dim}", photo.id);
        assert!(secondary >= 3.0, "{}: secondary {secondary:.2}:1", photo.id);
    }
}

#[test]
fn a_brighter_photograph_gets_more_dim_than_a_dark_one() {
    let dim = |id: &str| {
        let photo = PHOTOS.iter().find(|photo| photo.id == id).unwrap();
        derive(&decoded(photo)).backdrop.browse_dim
    };
    let (snow, night) = (dim("winter-1"), dim("christmas-1"));
    assert!(snow > night, "winter-1 {snow} against christmas-1 {night}");
}

#[test]
fn automatic_follows_the_calendar_and_the_hemisphere() {
    let automatic = |time, hemisphere| theme_for(Background::Automatic, time, hemisphere);
    assert_eq!(
        automatic(noon(2026, 12, 10), Hemisphere::North),
        ThemeId::Festival(Festival::Christmas)
    );
    assert_eq!(
        automatic(noon(2026, 10, 25), Hemisphere::South),
        ThemeId::Festival(Festival::Halloween)
    );
    assert_eq!(
        automatic(noon(2026, 7, 10), Hemisphere::North),
        ThemeId::Season(Season::Summer)
    );
    assert_eq!(
        automatic(noon(2026, 7, 10), Hemisphere::South),
        ThemeId::Season(Season::Winter)
    );
    let western = Background::Theme(ThemeId::Genre(Genre::Western));
    assert_eq!(
        theme_for(western, noon(2026, 12, 10), Hemisphere::North),
        ThemeId::Genre(Genre::Western)
    );
}

#[test]
fn a_theme_with_no_library_pick_shows_its_own_photographs_in_turn() {
    let theme = ThemeId::Genre(Genre::Fantasy);
    let morning = noon(2026, 11, 15);
    let Want {
        showing, fallback, ..
    } = want(theme, morning, None, None);
    assert_eq!(showing, fallback);
    let later = LocalTime::from_unix(morning.day_number() * 86_400 + 12 * 3600 + 30 * 60, 0);
    let shown: BTreeSet<String> = [morning, later]
        .into_iter()
        .map(|time| match bundled(theme, time) {
            Showing::Photo(index) => PHOTOS[index].id.to_owned(),
            Showing::Library(address) => address,
        })
        .collect();
    assert_eq!(
        shown,
        BTreeSet::from(["fantasy-1".to_owned(), "fantasy-2".to_owned()])
    );
}

#[test]
fn library_art_shows_over_the_photographs_and_a_pin_over_both() {
    let theme = ThemeId::Genre(Genre::SciFi);
    let time = noon(2026, 11, 15);
    let library = want(theme, time, None, Some("jellyfin:dune/art"));
    assert_eq!(
        library.showing,
        Showing::Library("jellyfin:dune/art".into())
    );
    let pinned = want(
        theme,
        time,
        Some("plex:/heat/art"),
        Some("jellyfin:dune/art"),
    );
    assert_eq!(pinned.showing, Showing::Library("plex:/heat/art".into()));
    assert_eq!(pinned.fallback, library.fallback);
    let unpinned = want(theme, time, None, Some("jellyfin:dune/art"));
    assert_eq!(unpinned, library, "unpinning returns to the theme");
}

#[test]
fn a_bundled_credit_names_the_title_author_and_licence() {
    let castle = PHOTOS.iter().find(|p| p.id == "fantasy-2").unwrap();
    assert_eq!(
        castle.credit(),
        "“Eilean Donan castle” by Eusebius, CC BY 3.0"
    );
    let fog = PHOTOS.iter().find(|p| p.id == "fantasy-1").unwrap();
    assert_eq!(
        fog.credit(),
        "“Morning fog in the Yellowstone River Valley” by Yellowstone National Park, public domain"
    );
    for photo in &PHOTOS {
        assert!(
            !photo.author.contains('&') || photo.author.contains(" & "),
            "{}",
            photo.id
        );
        assert!(!photo.author.contains('\n'), "{}", photo.id);
        assert!(
            photo.licence_url.is_empty() == (photo.licence == "Public domain"),
            "{}",
            photo.id
        );
    }
}

#[test]
fn cover_keeps_the_aspect_and_stays_inside_the_image() {
    let uv = cover_uv(vec2(2048.0, 1152.0), vec2(1280.0, 1000.0), vec2(0.5, 0.7));
    assert!(uv.min.x >= 0.0 && uv.max.x <= 1.0 + 1e-6);
    assert!(uv.min.y >= 0.0 && uv.max.y <= 1.0 + 1e-6);
    let drawn = (uv.width() * 2048.0) / (uv.height() * 1152.0);
    assert!((drawn - 1.28).abs() < 0.01);
}

#[test]
fn the_background_s_own_copy_is_decoded_before_picker_tiles() {
    let (send, inbox) = std::sync::mpsc::channel();
    for (index, variant) in [(1, Variant::Thumb), (2, Variant::Thumb), (3, Variant::Soft)] {
        send.send(Job { index, variant }).unwrap();
    }
    let mut pending = Vec::new();
    // Three takes for three jobs: a fourth would wait on the open channel.
    let order: Vec<usize> =
        std::iter::from_fn(|| next_job(&inbox, &mut pending).map(|job| job.index))
            .take(3)
            .collect();
    assert_eq!(order, [3, 1, 2]);
}
