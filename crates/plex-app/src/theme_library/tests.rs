use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::build::{self, SHORTLIST, THUMB_WIDTH};
use super::source::{page_through, Stop, STOPPED};
use super::store::{Catalog, Store, VERSION};
use super::*;
use crate::theme_art::{Festival, Genre, ImageKey, ImageRef, Kind, Score10, TitleKey};

const SCIFI: ThemeId = ThemeId::Genre(Genre::SciFi);
const DAY_SECS: i64 = 24 * 60 * 60;
const NOW: i64 = 1_800_000_000;

/// A server in memory: its titles, the sizes it reports, the thumbnails it
/// answers, and every call made of it.
struct Fake {
    id: String,
    name: String,
    titles: Mutex<Vec<Candidate>>,
    sizes: HashMap<ImageKey, Size>,
    thumbs: HashMap<ImageKey, Vec<u8>>,
    offline: AtomicBool,
    catalog_reads: AtomicU64,
    fetched: Mutex<Vec<(String, u32)>>,
}

impl Fake {
    fn new(id: &str, titles: Vec<Candidate>) -> Self {
        let thumbs = titles
            .iter()
            .flat_map(|t| &t.images)
            .map(|i| (i.key.clone(), png(rich)))
            .collect();
        Self {
            id: id.to_owned(),
            name: "Living Room".to_owned(),
            titles: Mutex::new(titles),
            sizes: HashMap::new(),
            thumbs,
            offline: AtomicBool::new(false),
            catalog_reads: AtomicU64::new(0),
            fetched: Mutex::new(Vec::new()),
        }
    }

    fn fetched(&self) -> Vec<String> {
        self.fetched
            .lock()
            .unwrap()
            .iter()
            .map(|(k, _)| k.clone())
            .collect()
    }

    fn reachable(&self) -> Result<(), String> {
        match self.offline.load(Ordering::SeqCst) {
            true => Err("offline".to_owned()),
            false => Ok(()),
        }
    }
}

impl ArtSource for Fake {
    fn id(&self) -> &str {
        &self.id
    }
    fn name(&self) -> &str {
        &self.name
    }
    fn count(&self) -> Result<u64, String> {
        self.reachable()?;
        Ok(self.titles.lock().unwrap().len() as u64)
    }
    fn catalog(&self, _stop: Stop<'_>) -> Result<Vec<Candidate>, String> {
        self.reachable()?;
        self.catalog_reads.fetch_add(1, Ordering::SeqCst);
        Ok(self.titles.lock().unwrap().clone())
    }
    fn sizes(&self, title: &Candidate) -> Result<Vec<(ImageKey, Size)>, String> {
        Ok(title
            .images
            .iter()
            .filter_map(|i| Some((i.key.clone(), *self.sizes.get(&i.key)?)))
            .collect())
    }
    fn image(&self, image: &ImageKey, width: u32) -> Result<Vec<u8>, String> {
        self.reachable()?;
        self.fetched.lock().unwrap().push((image.0.clone(), width));
        self.thumbs.get(image).cloned().ok_or("404".to_owned())
    }
}

fn rich(x: u32, y: u32) -> [u8; 3] {
    if y < 50 {
        [(230 - y) as u8, (120 + x / 3) as u8, (60 + y) as u8]
    } else {
        [(30 + x / 4) as u8, (60 + y / 3) as u8, (110 + x / 3) as u8]
    }
}

fn dark(_: u32, _: u32) -> [u8; 3] {
    [8, 8, 10]
}

/// A 160 by 90 PNG painted by `f`.
fn png(f: fn(u32, u32) -> [u8; 3]) -> Vec<u8> {
    let img = image::RgbImage::from_fn(160, 90, |x, y| image::Rgb(f(x, y)));
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

fn film(key: &str, title: &str, year: u16, genres: &[&str], rating: f64) -> Candidate {
    Candidate {
        key: TitleKey(key.to_owned()),
        kind: Kind::Film,
        title: title.to_owned(),
        original_title: None,
        year: Some(year),
        genres: genres.iter().map(|g| (*g).to_owned()).collect(),
        collections: Vec::new(),
        keywords: Vec::new(),
        critic: None,
        audience: Score10::out_of_10(rating),
        view_count: 0,
        provider_ids: BTreeMap::new(),
        images: vec![ImageRef {
            key: ImageKey(format!("{key}/art")),
            size: None,
            look: None,
        }],
    }
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "chewvy-theme-library-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn sources(fake: &Arc<Fake>) -> Vec<Arc<dyn ArtSource>> {
    vec![Arc::clone(fake) as Arc<dyn ArtSource>]
}

fn titles(picks: &[ArtPick]) -> Vec<&str> {
    picks.iter().map(|p| p.title.as_str()).collect()
}

fn pass(fake: &Arc<Fake>, store: &Store, now: i64) -> Picks {
    build::choose(&build::refresh(&sources(fake), store, now, &|| false))
}

#[test]
fn a_theme_shows_measured_backdrops_and_never_a_small_dark_or_low_rated_one() {
    let mut fake = Fake::new(
        "jellyfin:a",
        vec![
            film("dune", "Dune", 2021, &["Science Fiction"], 8.0),
            film("small", "Small Art", 2020, &["Science Fiction"], 8.2),
            film("dark", "Dark Space", 2020, &["Science Fiction"], 7.9),
            film("cats", "Cats", 2019, &["Fantasy", "Science Fiction"], 4.1),
        ],
    );
    let size = |width, height| Size { width, height };
    fake.sizes
        .insert(ImageKey("dune/art".into()), size(3840, 2160));
    fake.sizes
        .insert(ImageKey("small/art".into()), size(1280, 720));
    fake.thumbs.insert(ImageKey("dark/art".into()), png(dark));
    let fake = Arc::new(fake);

    let picks = pass(&fake, &Store::new(None), NOW);
    let scifi = &picks[&SCIFI];
    assert_eq!(titles(scifi), ["Dune"]);
    let dune = &scifi[0];
    assert_eq!(dune.artwork, "dune/art");
    assert_eq!(dune.credit(), "Dune (2021) from Living Room");
    assert_eq!(dune.size, Some(size(3840, 2160)));
    assert!(
        dune.look.is_some_and(|l| l.score() > 0.3),
        "{:?}",
        dune.look
    );
    assert_eq!(dune.audience, Some(8.0));

    let fetched = fake.fetched();
    assert!(
        !fetched.contains(&"small/art".to_owned()),
        "a backdrop the server sizes under 1920 wide is never fetched: {fetched:?}"
    );
    assert!(
        !fetched.contains(&"cats/art".to_owned()),
        "a title rated too low is never measured"
    );
    assert!(fake
        .fetched
        .lock()
        .unwrap()
        .iter()
        .all(|(_, w)| *w == THUMB_WIDTH));
    assert!(
        picks.keys().all(|t| t.uses_library()),
        "a theme no library feeds has no entry"
    );
}

#[test]
fn only_each_themes_shortlist_is_measured() {
    let library: Vec<Candidate> = (0..SHORTLIST + 15)
        .map(|i| {
            let rating = 6.6 + (i % 30) as f64 * 0.1;
            film(
                &format!("k{i:03}"),
                &format!("Space {i}"),
                2000,
                &["Science Fiction"],
                rating,
            )
        })
        .collect();
    let fake = Arc::new(Fake::new("plex:m", library));
    let picks = pass(&fake, &Store::new(None), NOW);
    assert_eq!(fake.fetched().len(), SHORTLIST);
    assert_eq!(picks[&SCIFI].len(), crate::theme_art::DEFAULT_PICKS);
}

#[test]
fn a_fresh_cache_is_neither_read_again_nor_measured_again() {
    let dir = temp_dir("fresh");
    let store = Store::new(Some(dir.clone()));
    let fake = Arc::new(Fake::new(
        "jellyfin:a",
        vec![film("dune", "Dune", 2021, &["Science Fiction"], 8.0)],
    ));
    let first = pass(&fake, &store, NOW);
    let second = pass(&fake, &store, NOW + DAY_SECS - 60);
    assert_eq!(first, second);
    assert_eq!(fake.catalog_reads.load(Ordering::SeqCst), 1);
    assert_eq!(fake.fetched().len(), 1);

    // A fresh handle on the same dir shows the cached picks before any server
    // is asked.
    let cached = build::choose(&build::cached(&sources(&fake), &store));
    assert_eq!(cached, first);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_changed_count_or_a_day_reads_the_catalog_again_and_keeps_what_was_measured() {
    let dir = temp_dir("stale");
    let store = Store::new(Some(dir.clone()));
    let mut fake = Fake::new(
        "jellyfin:a",
        vec![film("dune", "Dune", 2021, &["Science Fiction"], 8.0)],
    );
    fake.thumbs
        .insert(ImageKey("arrival/art".into()), png(rich));
    let fake = Arc::new(fake);
    pass(&fake, &store, NOW);

    fake.titles
        .lock()
        .unwrap()
        .push(film("arrival", "Arrival", 2016, &["Science Fiction"], 7.9));
    let picks = pass(&fake, &store, NOW + 60);
    assert_eq!(
        fake.catalog_reads.load(Ordering::SeqCst),
        2,
        "the count changed"
    );
    assert_eq!(titles(&picks[&SCIFI]), ["Dune", "Arrival"]);
    assert_eq!(
        fake.fetched(),
        ["dune/art", "arrival/art"],
        "Dune's backdrop was not measured twice"
    );

    pass(&fake, &store, NOW + 60 + DAY_SECS);
    assert_eq!(fake.catalog_reads.load(Ordering::SeqCst), 3, "a day passed");
    assert_eq!(fake.fetched().len(), 2);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_server_that_will_not_answer_keeps_its_cached_art() {
    let dir = temp_dir("offline");
    let store = Store::new(Some(dir.clone()));
    let fake = Arc::new(Fake::new(
        "jellyfin:a",
        vec![film("dune", "Dune", 2021, &["Science Fiction"], 8.0)],
    ));
    let online = pass(&fake, &store, NOW);
    fake.offline.store(true, Ordering::SeqCst);
    assert_eq!(pass(&fake, &store, NOW + 3 * DAY_SECS), online);
    assert!(pass(&fake, &Store::new(None), NOW)[&SCIFI].is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_cache_of_another_version_or_server_is_not_read() {
    let dir = temp_dir("version");
    let store = Store::new(Some(dir.clone()));
    let catalog = Catalog {
        source: "jellyfin:a".to_owned(),
        built_at: NOW,
        count: 1,
        titles: vec![film("dune", "Dune", 2021, &[], 8.0)],
    };
    store.save(&catalog);
    assert_eq!(store.load("jellyfin:a"), Some(catalog));
    assert_eq!(store.load("jellyfin:b"), None);
    let file = dir.join(format!("jellyfin_a.v{VERSION}.json"));
    let text = std::fs::read_to_string(&file).unwrap();
    std::fs::write(&file, text.replace("\"version\":1", "\"version\":0")).unwrap();
    assert_eq!(store.load("jellyfin:a"), None);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_cached_catalog_is_stale_after_a_day_a_new_count_or_a_clock_set_back() {
    let catalog = Catalog {
        source: "s".to_owned(),
        built_at: NOW,
        count: 10,
        titles: Vec::new(),
    };
    assert!(catalog.fresh(10, NOW + DAY_SECS - 1));
    assert!(!catalog.fresh(10, NOW + DAY_SECS));
    assert!(!catalog.fresh(11, NOW));
    assert!(!catalog.fresh(10, NOW - 1));
}

#[test]
fn the_same_film_on_two_servers_is_picked_once() {
    let mut on_plex = film("1", "Dune", 2021, &["Science Fiction"], 8.0);
    on_plex.provider_ids.insert("tmdb".into(), "438631".into());
    let mut on_jellyfin = film(
        "jellyfin:a:9",
        "Dune: Part One",
        2021,
        &["Science Fiction"],
        8.0,
    );
    on_jellyfin
        .provider_ids
        .insert("tmdb".into(), "438631".into());
    let plex = Arc::new(Fake::new("plex:m", vec![on_plex]));
    let jelly = Arc::new(Fake::new("jellyfin:a", vec![on_jellyfin]));
    let both: Vec<Arc<dyn ArtSource>> = vec![plex, jelly];
    let picks = build::choose(&build::refresh(&both, &Store::new(None), NOW, &|| false));
    assert_eq!(picks[&SCIFI].len(), 1);
}

#[test]
fn the_handle_fills_off_the_ui_thread_and_rotates_its_picks() {
    let fake = Arc::new(Fake::new(
        "jellyfin:a",
        vec![
            film("dune", "Dune", 2021, &["Science Fiction"], 8.0),
            film("arrival", "Arrival", 2016, &["Science Fiction"], 7.9),
            film("grinch", "The Grinch", 2018, &["Animation"], 7.0),
        ],
    ));
    let mut library = ThemeLibrary::start(sources(&fake), Dirs::default());
    let deadline = Instant::now() + Duration::from_secs(10);
    while library.picks(SCIFI).is_empty() && Instant::now() < deadline {
        library.poll();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(titles(library.picks(SCIFI)), ["Dune", "Arrival"]);
    assert_eq!(
        titles(library.picks(ThemeId::Festival(Festival::Christmas))),
        ["The Grinch"]
    );
    assert!(library.picks(ThemeId::Aurora).is_empty());

    let at = |minute: i64| LocalTime::from_unix(NOW + minute * 60, 0);
    let shown: std::collections::BTreeSet<&str> = (0..48)
        .filter_map(|slot| library.showing(SCIFI, at(slot * i64::from(ROTATE_MINUTES))))
        .map(|p| p.title.as_str())
        .collect();
    assert_eq!(shown.len(), 2, "both picks take a turn: {shown:?}");
    assert_eq!(
        library.showing(SCIFI, at(0)),
        library.showing(SCIFI, at(i64::from(ROTATE_MINUTES) - 1))
    );
}

#[test]
fn a_pinned_background_is_kept_between_runs_until_cleared() {
    let dir = temp_dir("pin");
    std::fs::create_dir_all(&dir).unwrap();
    let dirs = || Dirs {
        cache: None,
        config: Some(dir.clone()),
    };
    let mut library = ThemeLibrary::start(Vec::new(), dirs());
    assert!(library.pinned().is_none());
    let pin = Pinned {
        source: "jellyfin:a".to_owned(),
        title_key: "jellyfin:a:9".to_owned(),
        artwork: "jellyfin:a/Items/9/Images/Backdrop/1?tag=t".to_owned(),
        title: "Lawrence of Arabia".to_owned(),
        year: Some(1962),
        server: "Living Room".to_owned(),
    };
    library.set_pinned(Some(pin.clone())).unwrap();
    assert_eq!(ThemeLibrary::start(Vec::new(), dirs()).pinned(), Some(&pin));
    assert_eq!(pin.credit(), "Lawrence of Arabia (1962) from Living Room");
    library.set_pinned(None).unwrap();
    library.set_pinned(None).unwrap();
    assert!(ThemeLibrary::start(Vec::new(), dirs()).pinned().is_none());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_catalog_is_paged_to_its_end_and_no_further() {
    let rows: Vec<u32> = (0..450).collect();
    let mut asked = Vec::new();
    let read = page_through(200, &|| false, |start| {
        asked.push(start);
        let page: Vec<u32> = rows.iter().copied().skip(start).take(200).collect();
        Ok((page, None))
    })
    .unwrap();
    assert_eq!(read, rows);
    assert_eq!(asked, [0, 200, 400]);

    let mut asked = 0;
    let exact = page_through(200, &|| false, |start| {
        asked += 1;
        let page: Vec<u32> = rows.iter().copied().skip(start).take(200).collect();
        Ok((page, Some(400)))
    })
    .unwrap();
    assert_eq!((exact.len(), asked), (400, 2), "the named total ends it");

    let failed = page_through(2, &|| false, |start| match start {
        0 => Ok((vec![1, 2], None)),
        _ => Err("gone".to_owned()),
    });
    assert!(failed.is_err(), "a half-read catalog is no catalog");
}

#[test]
fn bytes_that_are_not_a_small_image_have_no_look() {
    assert!(build::measure_bytes(b"not an image").is_none());
    assert!(build::measure_bytes(&png(rich)).is_some());
    let huge = image::RgbImage::new(2048, 8);
    let mut out = std::io::Cursor::new(Vec::new());
    huge.write_to(&mut out, image::ImageFormat::Png).unwrap();
    assert!(build::measure_bytes(&out.into_inner()).is_none());
}

const CATALOG: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../jellyfin-core/tests/fixtures/live-12.4-catalog/catalog_page.json"
);

#[test]
fn a_jellyfin_row_becomes_a_candidate_on_declared_scales() {
    let page: jellyfin_core::dto::QueryResult<jellyfin_core::dto::Item> =
        serde_json::from_str(&std::fs::read_to_string(CATALOG).unwrap()).unwrap();
    let odyssey = jellyfin::candidate("0f1e", &page.items[1]).unwrap();
    assert_eq!(odyssey.title, "2001: A Space Odyssey");
    assert_eq!(
        odyssey.key.0,
        "jellyfin:0f1e:21d49f90ff283515661cc97b3b13e1a9"
    );
    assert_eq!(odyssey.critic.map(Score10::get), Some(9.1), "91 of 100");
    assert_eq!(odyssey.audience.map(Score10::get), Some(8.042));
    assert_eq!(
        odyssey.provider_ids.keys().collect::<Vec<_>>(),
        ["imdb", "tmdb"],
        "a TMDB collection id is not an identity"
    );
    let backdrop = &odyssey.images[0].key.0;
    assert_eq!(
        backdrop,
        "jellyfin:0f1e/Items/21d49f90ff283515661cc97b3b13e1a9/Images/Backdrop/0?tag=1ef3197f14215b577c1e58e8dba2890c"
    );
    // The artwork loader takes it as it is, and asks for a width alone.
    let (_, path) = crate::ui::images::request_for(backdrop, FULL_WIDTH, 2160).unwrap();
    assert!(path.ends_with("&maxWidth=3840"), "{path}");
}

#[test]
fn a_plex_row_becomes_a_candidate_on_declared_scales() {
    let row = plex_core::library::Item {
        rating_key: Some("30874".into()),
        kind: Some("movie".into()),
        title: Some("#Adrift".into()),
        original_title: Some("#표류".into()),
        year: Some(2020),
        rating: Some(8.8),
        audience_rating: Some(6.3),
        art: Some("/library/metadata/30874/art/1789186896".into()),
        guids: vec![
            "imdb://tt2172380".into(),
            "tmdb://22969".into(),
            "tvdb://1648".into(),
        ],
        collections: vec!["Ocean".into()],
        ..Default::default()
    };
    let c = plex::candidate(&row).unwrap();
    assert_eq!(c.critic.map(Score10::get), Some(8.8));
    assert_eq!(c.audience.map(Score10::get), Some(6.3));
    assert_eq!(c.original_title.as_deref(), Some("#표류"));
    assert_eq!(c.collections, ["Ocean"]);
    assert_eq!(
        c.provider_ids.into_iter().collect::<Vec<_>>(),
        [
            ("imdb".into(), "tt2172380".into()),
            ("tmdb".into(), "22969".into())
        ],
        "a film's TVDB id is not kept"
    );
    assert_eq!(c.images[0].key.0, "/library/metadata/30874/art/1789186896");
    let episode = plex_core::library::Item {
        kind: Some("episode".into()),
        ..row
    };
    assert!(plex::candidate(&episode).is_none());
}

/// A [`Fake`] whose thumbnails take a while, counting the fetches made of it
/// and the most made at once.
struct Slow {
    inner: Fake,
    calls: AtomicU64,
    inflight: AtomicI64,
    max_inflight: AtomicI64,
}

impl ArtSource for Slow {
    fn id(&self) -> &str {
        self.inner.id()
    }
    fn name(&self) -> &str {
        self.inner.name()
    }
    fn count(&self) -> Result<u64, String> {
        self.inner.count()
    }
    fn catalog(&self, stop: Stop<'_>) -> Result<Vec<Candidate>, String> {
        self.inner.catalog(stop)
    }
    fn sizes(&self, title: &Candidate) -> Result<Vec<(ImageKey, Size)>, String> {
        self.inner.sizes(title)
    }
    fn image(&self, image: &ImageKey, width: u32) -> Result<Vec<u8>, String> {
        let now = self.inflight.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_inflight.fetch_max(now, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(40));
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inflight.fetch_sub(1, Ordering::SeqCst);
        self.inner.image(image, width)
    }
}

fn slow_source() -> Arc<Slow> {
    let titles: Vec<Candidate> = (0..40u16)
        .map(|i| {
            let name = format!("Space {i}");
            film(&format!("k{i}"), &name, 1980 + i, &["Science Fiction"], 8.0)
        })
        .collect();
    Arc::new(Slow {
        inner: Fake::new("jellyfin:a", titles),
        calls: AtomicU64::new(0),
        inflight: AtomicI64::new(0),
        max_inflight: AtomicI64::new(0),
    })
}

fn wait_calls(slow: &Slow, at_least: u64) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while slow.calls.load(Ordering::SeqCst) < at_least && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_dropped_handle_ends_its_pass_between_thumbnails() {
    let slow = slow_source();
    let library = ThemeLibrary::start(
        vec![Arc::clone(&slow) as Arc<dyn ArtSource>],
        Dirs::default(),
    );
    wait_calls(&slow, 1);
    drop(library);
    let at_drop = slow.calls.load(Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(500));
    let later = slow.calls.load(Ordering::SeqCst);
    assert!(
        later <= at_drop + 1,
        "only the fetch already under way finishes: {at_drop} at the drop, {later} after"
    );
}

#[test]
fn restarting_the_library_never_stacks_passes() {
    let slow = slow_source();
    for round in 1..=5u64 {
        let library = ThemeLibrary::start(
            vec![Arc::clone(&slow) as Arc<dyn ArtSource>],
            Dirs::default(),
        );
        wait_calls(&slow, round);
        drop(library);
    }
    std::thread::sleep(Duration::from_millis(200));
    let most = slow.max_inflight.load(Ordering::SeqCst);
    assert!(
        most <= 2,
        "at most the pass ending and the one replacing it fetch at once, saw {most}"
    );
}

#[test]
fn a_stopped_catalog_read_asks_for_no_further_page() {
    let asked = AtomicU64::new(0);
    let stop = || asked.load(Ordering::SeqCst) >= 1;
    let read = page_through(2, &stop, |_| {
        asked.fetch_add(1, Ordering::SeqCst);
        Ok((vec![1, 2], None))
    });
    assert_eq!(read, Err(STOPPED.to_owned()));
    assert_eq!(asked.load(Ordering::SeqCst), 1);
}

#[test]
fn no_more_than_three_backdrops_of_a_title_are_measured() {
    let titles: Vec<Candidate> = (0..40u16)
        .map(|i| {
            let name = format!("Space {i}");
            let mut c = film(&format!("k{i}"), &name, 1980 + i, &["Science Fiction"], 8.0);
            c.images = (0..25)
                .map(|n| ImageRef {
                    key: ImageKey(format!("k{i}/art/{n}")),
                    size: None,
                    look: None,
                })
                .collect();
            c
        })
        .collect();
    let fake = Arc::new(Fake::new("jellyfin:a", titles));
    let picks = pass(&fake, &Store::new(None), NOW);
    let mut per_title: HashMap<String, usize> = HashMap::new();
    for key in fake.fetched() {
        let title = key.split('/').next().unwrap().to_owned();
        *per_title.entry(title).or_default() += 1;
    }
    assert!(!per_title.is_empty());
    assert!(per_title.values().all(|n| *n <= 3), "{per_title:?}");
    assert!(
        !picks[&SCIFI].is_empty(),
        "the measured backdrops are still picked"
    );
}

#[cfg(unix)]
#[test]
fn the_cache_is_readable_by_its_owner_only() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = temp_dir("private").join(SUBDIR);
    let fake = Arc::new(Fake::new(
        "jellyfin:a",
        vec![film("dune", "Dune", 2021, &["Science Fiction"], 8.0)],
    ));
    pass(&fake, &Store::new(Some(dir.clone())), NOW);
    let mode =
        |path: &std::path::Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&dir), 0o700);
    let files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert!(!files.is_empty());
    for file in files {
        assert_eq!(mode(&file), 0o600, "{}", file.display());
    }
    let _ = std::fs::remove_dir_all(dir.parent().unwrap());
}

#[test]
fn a_plex_pin_stands_only_on_the_server_it_was_made_on() {
    let pin = Pinned {
        source: "plex:living-room".to_owned(),
        title_key: "123".to_owned(),
        artwork: "/library/metadata/123/art/1".to_owned(),
        title: "Dune".to_owned(),
        year: Some(2021),
        server: "Living Room".to_owned(),
    };
    assert_eq!(
        source_of("123", Some("living-room")).as_deref(),
        Some(pin.source.as_str())
    );
    assert!(pin.applies(Some("living-room")));
    assert!(pin.is_for("123", Some("living-room")));
    assert!(
        !pin.applies(Some("office")),
        "the same key on another server is another item"
    );
    assert!(!pin.is_for("123", Some("office")));
    assert!(!pin.applies(None));
    assert!(!pin.is_for("124", Some("living-room")));

    let jellyfin = Pinned {
        source: "jellyfin:a".to_owned(),
        title_key: "jellyfin:a:9".to_owned(),
        ..pin
    };
    assert!(
        jellyfin.applies(None),
        "a Jellyfin key names its own server"
    );
    assert!(jellyfin.is_for("jellyfin:a:9", Some("office")));
    assert!(!jellyfin.is_for("jellyfin:b:9", None));
}

#[test]
fn a_pin_saved_without_its_server_is_dropped() {
    let dir = temp_dir("old-pin");
    std::fs::create_dir_all(&dir).unwrap();
    let old = r#"{"title_key":"123","artwork":"/library/metadata/123/art/1","title":"Dune","year":2021,"server":"Living Room"}"#;
    std::fs::write(dir.join("background.json"), old).unwrap();
    let library = ThemeLibrary::start(
        Vec::new(),
        Dirs {
            cache: None,
            config: Some(dir.clone()),
        },
    );
    assert!(library.pinned().is_none());
    let _ = std::fs::remove_dir_all(dir);
}
