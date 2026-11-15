//! The library against a real server, run by hand:
//!
//! ```text
//! env -i PATH="$PATH" HOME=<profile>/home XDG_CONFIG_HOME=<profile>/config \
//!   XDG_CACHE_HOME=<profile>/cache DBUS_SESSION_BUS_ADDRESS=unix:path=/nonexistent \
//!   CHEWVY_THEME_LIVE_OUT=<dir> cargo test -p plex-app --lib theme_library::live -- --ignored --nocapture
//! ```
//!
//! It signs in as the profile's own `sources.json` and token file say, reads
//! every Jellyfin server there through the same pool the app browses with,
//! and writes `<dir>/picks.txt` (each library theme's top ten, then every
//! title with what kept it out) and `<dir>/contact-sheet.png` (each theme's
//! top five, one row per theme, fetched at 640 px). The cache goes to
//! `<dir>/cache`, never the profile's.

use std::fmt::Write as _;
use std::sync::Arc;

use plex_core::tokens::{open_store, Consent, StorePaths};

use super::build::{self, Shelf};
use super::store::Store;
use super::*;
use crate::theme_art::{seasonal, Genre, LocalTime};

const SHEET_COLUMNS: u32 = 5;
const TILE: (u32, u32) = (640, 360);

#[test]
#[ignore = "needs a signed-in profile and a live server; see the module doc"]
fn live_picks_and_contact_sheet() {
    let out = PathBuf::from(std::env::var("CHEWVY_THEME_LIVE_OUT").expect("CHEWVY_THEME_LIVE_OUT"));
    std::fs::create_dir_all(&out).unwrap();
    let registry = crate::sources::load();
    let device = crate::add_source::device_identity()
        .map_err(|e| e.message())
        .unwrap();
    let paths = StorePaths::resolve().unwrap();
    let store = open_store(&paths, Consent::granted(), crate::sources::unix_now()).ok();
    let (pool, notes) = browse::Jellyfin::open(&registry.jellyfin, store.as_deref(), &device);
    assert!(notes.is_empty(), "{notes:?}");
    let sources = super::sources(None, &pool);
    assert!(!sources.is_empty(), "the profile names a Jellyfin server");

    let cache = Store::new(Some(out.join("cache")));
    let started = std::time::Instant::now();
    let shelves = build::refresh(&sources, &cache, crate::sources::unix_now(), &|| false);
    let first = started.elapsed();
    let again = std::time::Instant::now();
    let reread = build::refresh(&sources, &cache, crate::sources::unix_now(), &|| false);
    let picks = build::choose(&shelves);
    assert_eq!(
        build::choose(&reread),
        picks,
        "a cached pass picks the same"
    );

    let mut report = String::new();
    let now = LocalTime::from_unix(crate::sources::unix_now(), 0);
    let _ = writeln!(
        report,
        "hemisphere {:?}; seasonal theme today {:?}; first pass {:.1?}, cached pass {:.1?}\n",
        hemisphere(),
        seasonal(now.date(), hemisphere()),
        first,
        again.elapsed()
    );
    for (theme, chosen) in &picks {
        let _ = writeln!(report, "== {} ({} picks)", theme.name(), chosen.len());
        for (rank, pick) in chosen.iter().take(10).enumerate() {
            let _ = writeln!(report, "{:>2}. {}", rank + 1, row(pick));
        }
        report.push('\n');
    }
    audit(&mut report, &shelves, &picks);
    small_art_is_refused(&mut report, &shelves);
    std::fs::write(out.join("picks.txt"), &report).unwrap();
    println!("{report}");

    let top = |theme: ThemeId, n: usize| -> Vec<String> {
        picks[&theme]
            .iter()
            .take(n)
            .map(|p| p.title.clone())
            .collect()
    };
    let has = |list: &[String], word: &str| list.iter().any(|t| t.contains(word));
    assert!(has(
        &top(ThemeId::Genre(Genre::Fantasy), 5),
        "The Lord of the Rings"
    ));
    assert!(has(&top(ThemeId::Genre(Genre::SciFi), 5), "Dune"));
    assert!(has(
        &top(ThemeId::Genre(Genre::Epic), 3),
        "Lawrence of Arabia"
    ));
    for (theme, chosen) in &picks {
        for never in ["Birdemic", "Cats", "The Room", "Plan 9"] {
            assert!(
                !chosen.iter().any(|p| p.title.starts_with(never)),
                "{never} in {theme:?}"
            );
        }
    }

    let sheet = contact_sheet(&sources, &picks);
    sheet.save(out.join("contact-sheet.png")).unwrap();
}

fn rating(critic: Option<f32>, audience: Option<f32>) -> String {
    let show = |v: Option<f32>| v.map_or("-".to_owned(), |v| format!("{v:.1}"));
    format!("critic {:>4} audience {:>4}", show(critic), show(audience))
}

fn row(pick: &ArtPick) -> String {
    format!(
        "{:<44} {} canon {:<5} size {:<9} look {:.3} score {:.3}",
        format!("{} ({})", pick.title, pick.year.unwrap_or(0)),
        rating(pick.critic, pick.audience),
        pick.canon,
        pick.size
            .map_or("?".to_owned(), |s| format!("{}x{}", s.width, s.height)),
        pick.look.map_or(0.0, |l| l.score()),
        pick.score,
    )
}

/// Every title, the themes it was picked for, and its images as measured.
fn audit(report: &mut String, shelves: &[Shelf], picks: &Picks) {
    let _ = writeln!(report, "== every title");
    for title in shelves.iter().flat_map(|s| &s.catalog.titles) {
        let themes: Vec<&str> = picks
            .iter()
            .filter(|(_, chosen)| chosen.iter().any(|p| p.title_key == title.key.0))
            .map(|(theme, _)| theme.name())
            .collect();
        let images: Vec<String> = title
            .images
            .iter()
            .map(|i| {
                let size = i
                    .size
                    .map_or("?".to_owned(), |s| format!("{}x{}", s.width, s.height));
                let look = i.look.map_or("unmeasured".to_owned(), |l| {
                    format!("look {:.3}", l.score())
                });
                format!("{size} {look}")
            })
            .collect();
        let _ = writeln!(
            report,
            "{:<44} {} [{}] picked for: {}",
            format!("{} ({})", title.title, title.year.unwrap_or(0)),
            rating(
                title.critic.map(|s| s.get()),
                title.audience.map(|s| s.get())
            ),
            images.join(", "),
            if themes.is_empty() {
                "none".to_owned()
            } else {
                themes.join(", ")
            },
        );
    }
}

/// Cats' only backdrop is 1280 wide. Its rating keeps it out before its size
/// is ever asked, so this asks anyway, rates it 9 and offers it to Fantasy:
/// the size alone must still refuse it.
fn small_art_is_refused(report: &mut String, shelves: &[Shelf]) {
    let Some((shelf, cats)) = shelves
        .iter()
        .flat_map(|s| s.catalog.titles.iter().map(move |t| (s, t)))
        .find(|(_, t)| t.title == "Cats")
    else {
        return;
    };
    let mut rated = cats.clone();
    rated.audience = crate::theme_art::Score10::out_of_10(9.0);
    let sizes = shelf.source.sizes(&rated).unwrap();
    for image in &mut rated.images {
        image.size = sizes.iter().find(|(k, _)| *k == image.key).map(|(_, s)| *s);
    }
    let picked = crate::theme_art::rank(ThemeId::Genre(Genre::Fantasy), &[rated.clone()]);
    let _ = writeln!(
        report,
        "\n== Cats rated 9 for the test: backdrop sizes {:?}, picked for Fantasy: {}",
        rated.images.iter().map(|i| i.size).collect::<Vec<_>>(),
        picked.len()
    );
    assert!(rated
        .images
        .iter()
        .all(|i| i.size.is_some_and(|s| s.width == 1280)));
    assert!(picked.is_empty(), "a 1280-wide backdrop is never shown");
}

fn contact_sheet(sources: &[Arc<dyn ArtSource>], picks: &Picks) -> image::RgbImage {
    let rows = u32::try_from(picks.len()).unwrap();
    let mut sheet = image::RgbImage::new(TILE.0 * SHEET_COLUMNS, TILE.1 * rows.max(1));
    for (y, chosen) in picks.values().enumerate() {
        for (x, pick) in chosen.iter().take(SHEET_COLUMNS as usize).enumerate() {
            let Some(bytes) = sources
                .iter()
                .find(|s| s.name() == pick.server)
                .and_then(|s| {
                    s.image(&crate::theme_art::ImageKey(pick.artwork.clone()), TILE.0)
                        .ok()
                })
            else {
                continue;
            };
            let Ok(tile) = image::load_from_memory(&bytes) else {
                continue;
            };
            let tile = tile
                .resize_to_fill(TILE.0, TILE.1, image::imageops::FilterType::Triangle)
                .into_rgb8();
            let at = (x as i64 * i64::from(TILE.0), y as i64 * i64::from(TILE.1));
            image::imageops::overlay(&mut sheet, &tile, at.0, at.1);
        }
    }
    sheet
}
