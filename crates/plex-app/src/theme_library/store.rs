//! The library's disk cache, one file per server, and the user's pinned
//! background.
//!
//! A cache file holds the server's whole catalog as candidates plus what was
//! learned about each backdrop (its size, its [`Look`]), keyed by the image's
//! own address, which names the server's item and image tag (Jellyfin) or the
//! art path with its timestamp (Plex): a changed image has a new address and is
//! measured again, and an unchanged one never is. The format is versioned; a
//! file of another version is ignored and rebuilt.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::theme_art::{Candidate, ImageKey, ImageRef, Kind, Look, Score10, Size, TitleKey};

/// The cache format. Bump it when the stored shape or its meaning changes.
pub(super) const VERSION: u32 = 1;
/// A catalog is read again at most this often, unless its size changes.
pub(super) const MAX_AGE_SECS: i64 = 24 * 60 * 60;
/// The cache subdirectory, beside the artwork cache.
pub const SUBDIR: &str = "theme-art";
/// The pinned background, in the config dir: a user choice, not a cache.
const PIN_FILE: &str = "background.json";

/// One server's catalog as last read.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Catalog {
    /// [`super::ArtSource::id`].
    pub source: String,
    /// Unix seconds of the read.
    pub built_at: i64,
    /// The server's title count at that read.
    pub count: u64,
    /// Its titles, with whatever sizes and looks are known.
    pub titles: Vec<Candidate>,
}

impl Catalog {
    /// Whether this catalog still stands for a server that now counts `count`
    /// titles, at `now`: read within the day, and the same size. A clock that
    /// went backwards counts as stale.
    pub(super) fn fresh(&self, count: u64, now: i64) -> bool {
        let age = now.saturating_sub(self.built_at);
        self.count == count && (0..MAX_AGE_SECS).contains(&age)
    }

    /// What is known about each image, by its key.
    pub(super) fn known_images(&self) -> BTreeMap<ImageKey, (Option<Size>, Option<Look>)> {
        self.titles
            .iter()
            .flat_map(|title| &title.images)
            .filter(|image| image.size.is_some() || image.look.is_some())
            .map(|image| (image.key.clone(), (image.size, image.look)))
            .collect()
    }
}

/// Where the cache files live; `None` keeps everything in memory.
#[derive(Debug, Clone)]
pub(super) struct Store {
    dir: Option<PathBuf>,
}

impl Store {
    pub(super) fn new(dir: Option<PathBuf>) -> Self {
        Self { dir }
    }

    fn path(&self, source: &str) -> Option<PathBuf> {
        let name: String = source
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        Some(self.dir.as_ref()?.join(format!("{name}.v{VERSION}.json")))
    }

    /// The cached catalog of `source`, if one of this version is on disk.
    pub(super) fn load(&self, source: &str) -> Option<Catalog> {
        let text = std::fs::read(self.path(source)?).ok()?;
        let file: StoredCatalog = serde_json::from_slice(&text).ok()?;
        (file.version == VERSION && file.source == source).then(|| file.into_catalog())
    }

    /// Write `catalog` whole: to a temporary file, then renamed over the old
    /// one, so a crash mid-write leaves the previous catalog rather than half
    /// of a new one. A failed write is logged, not an error: the cache stays a
    /// cache.
    pub(super) fn save(&self, catalog: &Catalog) {
        let Some(path) = self.path(&catalog.source) else {
            return;
        };
        let written = serde_json::to_vec(&StoredCatalog::from(catalog))
            .map_err(std::io::Error::other)
            .and_then(|json| write_whole(&path, &json));
        if let Err(error) = written {
            log::debug!("theme art cache not written: {error}");
        }
    }
}

/// Write `bytes` to `path` whole, owner-only as `settings.json` is: the
/// catalog holds what the user watched and how often.
fn write_whole(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        plex_core::tokens::create_private_dir(dir)?;
    }
    // A handle restarted for a new server list can overlap the one it
    // replaces, so each write has a temporary file of its own.
    static WRITES: AtomicU64 = AtomicU64::new(0);
    let partial = path.with_extension(format!(
        "{}-{}.partial",
        std::process::id(),
        WRITES.fetch_add(1, Ordering::Relaxed)
    ));
    plex_core::tokens::write_private_file(&partial, bytes)?;
    std::fs::rename(&partial, path)
}

/// The user's own choice of background: one title's image, shown whatever the
/// theme. Stored as the credit it shows, the address it is fetched by, and the
/// server both belong to. A file from before `source` was kept does not read,
/// so such a pin is dropped rather than shown on whichever server is connected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pinned {
    /// The [`super::ArtSource::id`] of the server the title is on; see
    /// [`super::source_of`].
    pub source: String,
    /// The title's own id in the browse model (a Plex rating key or a
    /// `jellyfin:` id).
    pub title_key: String,
    /// The image's artwork address, as [`super::ArtPick::artwork`].
    pub artwork: String,
    /// The title as shown.
    pub title: String,
    /// Its year.
    pub year: Option<u16>,
    /// The server it came from, by name.
    pub server: String,
}

/// The pinned background in `config_dir`, if one was saved and still reads.
pub(super) fn load_pin(config_dir: &Path) -> Option<Pinned> {
    let text = std::fs::read(config_dir.join(PIN_FILE)).ok()?;
    serde_json::from_slice(&text).ok()
}

/// Save `pin` in `config_dir`, or clear it with `None`.
pub(super) fn save_pin(config_dir: &Path, pin: Option<&Pinned>) -> std::io::Result<()> {
    let path = config_dir.join(PIN_FILE);
    match pin {
        Some(pin) => write_whole(&path, &serde_json::to_vec_pretty(pin)?),
        None => match std::fs::remove_file(&path) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        },
    }
}

#[derive(Serialize, Deserialize)]
struct StoredCatalog {
    version: u32,
    source: String,
    built_at: i64,
    count: u64,
    titles: Vec<StoredTitle>,
}

#[derive(Serialize, Deserialize)]
struct StoredTitle {
    key: String,
    show: bool,
    title: String,
    original_title: Option<String>,
    year: Option<u16>,
    genres: Vec<String>,
    collections: Vec<String>,
    #[serde(default)]
    keywords: Vec<String>,
    /// Out of 10, as [`Score10`] holds it.
    critic: Option<f32>,
    /// Out of 10.
    audience: Option<f32>,
    view_count: u64,
    provider_ids: BTreeMap<String, String>,
    images: Vec<StoredImage>,
}

#[derive(Serialize, Deserialize)]
struct StoredImage {
    key: String,
    size: Option<(u32, u32)>,
    /// colour, exposure, spread, text.
    look: Option<[f32; 4]>,
}

impl From<&Catalog> for StoredCatalog {
    fn from(catalog: &Catalog) -> Self {
        Self {
            version: VERSION,
            source: catalog.source.clone(),
            built_at: catalog.built_at,
            count: catalog.count,
            titles: catalog.titles.iter().map(StoredTitle::from).collect(),
        }
    }
}

impl From<&Candidate> for StoredTitle {
    fn from(c: &Candidate) -> Self {
        Self {
            key: c.key.0.clone(),
            show: c.kind == Kind::Show,
            title: c.title.clone(),
            original_title: c.original_title.clone(),
            year: c.year,
            genres: c.genres.clone(),
            collections: c.collections.clone(),
            keywords: c.keywords.clone(),
            critic: c.critic.map(Score10::get),
            audience: c.audience.map(Score10::get),
            view_count: c.view_count,
            provider_ids: c.provider_ids.clone(),
            images: c
                .images
                .iter()
                .map(|image| StoredImage {
                    key: image.key.0.clone(),
                    size: image.size.map(|s| (s.width, s.height)),
                    look: image.look.map(|l| [l.colour, l.exposure, l.spread, l.text]),
                })
                .collect(),
        }
    }
}

impl StoredCatalog {
    fn into_catalog(self) -> Catalog {
        Catalog {
            source: self.source,
            built_at: self.built_at,
            count: self.count,
            titles: self
                .titles
                .into_iter()
                .map(StoredTitle::into_candidate)
                .collect(),
        }
    }
}

impl StoredTitle {
    fn into_candidate(self) -> Candidate {
        Candidate {
            key: TitleKey(self.key),
            kind: if self.show { Kind::Show } else { Kind::Film },
            title: self.title,
            original_title: self.original_title,
            year: self.year,
            genres: self.genres,
            collections: self.collections,
            keywords: self.keywords,
            critic: self.critic.and_then(|v| Score10::out_of_10(f64::from(v))),
            audience: self.audience.and_then(|v| Score10::out_of_10(f64::from(v))),
            view_count: self.view_count,
            provider_ids: self.provider_ids,
            images: self
                .images
                .into_iter()
                .map(|image| ImageRef {
                    key: ImageKey(image.key),
                    size: image.size.map(|(width, height)| Size { width, height }),
                    look: image.look.map(|[colour, exposure, spread, text]| Look {
                        colour,
                        exposure,
                        spread,
                        text,
                    }),
                })
                .collect(),
        }
    }
}
