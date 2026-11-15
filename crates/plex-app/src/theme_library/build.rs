//! One pass of the library: bring each server's catalog up to date, measure
//! the backdrops that could be picked, and rank every theme.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use crate::theme_art::{
    look, rank, rank_n, Candidate, ImageKey, Look, Size, ThemeId, Thumbnail, TitleKey, MIN_WIDTH,
};

use super::source::Stop;
use super::store::{Catalog, Store};
use super::{ArtPick, ArtSource, Picks};

/// How many titles of each theme are measured before the final ranking: twice
/// the picks kept, so a theme still fills when some backdrops turn out dark,
/// flat or small.
pub(super) const SHORTLIST: usize = 2 * crate::theme_art::DEFAULT_PICKS;
/// The width a backdrop is fetched at to be measured.
pub const THUMB_WIDTH: u32 = 320;
/// The most backdrops of one title measured. A server lists a title's chosen
/// backdrop first, three are enough to find one that sits well behind the
/// glass, and a title with dozens of fan-art backdrops would otherwise cost
/// dozens of thumbnail requests in every pass.
pub(super) const MAX_BACKDROPS: usize = 3;
/// Largest thumbnail answer accepted for decoding.
const MAX_THUMB_BYTES: usize = 2 * 1024 * 1024;
/// Largest thumbnail side accepted: the server was asked for [`THUMB_WIDTH`].
const MAX_THUMB_DIM: u32 = 1024;

/// One server's catalog and the source it came from.
pub(super) struct Shelf {
    pub source: Arc<dyn ArtSource>,
    pub catalog: Catalog,
}

/// The cached catalogs alone, without asking any server: what the library
/// shows while the first pass of a run is still reading.
pub(super) fn cached(sources: &[Arc<dyn ArtSource>], store: &Store) -> Vec<Shelf> {
    sources
        .iter()
        .filter_map(|source| {
            let catalog = store.load(source.id())?;
            Some(Shelf {
                source: Arc::clone(source),
                catalog,
            })
        })
        .collect()
}

/// Bring every server's catalog up to date at `now`, measure what could be
/// picked, and save what changed. A server that will not answer keeps its
/// cached catalog, however old, rather than losing its art.
///
/// `stop` is asked before every request; once it answers true no request
/// follows, and what was measured until then is still saved.
pub(super) fn refresh(
    sources: &[Arc<dyn ArtSource>],
    store: &Store,
    now: i64,
    stop: Stop<'_>,
) -> Vec<Shelf> {
    let mut shelves: Vec<Shelf> = sources
        .iter()
        .take_while(|_| !stop())
        .filter_map(|source| {
            let catalog = current(source.as_ref(), store, now, stop)?;
            Some(Shelf {
                source: Arc::clone(source),
                catalog,
            })
        })
        .collect();
    measure(&mut shelves, stop);
    for shelf in &shelves {
        store.save(&shelf.catalog);
    }
    shelves
}

/// The cached catalog when it is fresh, else a new read that keeps every size
/// and look already learned for an image it still has.
fn current(source: &dyn ArtSource, store: &Store, now: i64, stop: Stop<'_>) -> Option<Catalog> {
    let cached = store.load(source.id());
    let count = match source.count() {
        Ok(count) => count,
        Err(why) => {
            log::debug!("theme art: {} did not count its titles: {why}", source.id());
            return cached;
        }
    };
    if let Some(catalog) = cached.as_ref().filter(|c| c.fresh(count, now)) {
        return Some(catalog.clone());
    }
    if stop() {
        return cached;
    }
    let mut titles = match source.catalog(stop) {
        Ok(titles) => titles,
        Err(why) => {
            log::debug!("theme art: {} did not list its titles: {why}", source.id());
            return cached;
        }
    };
    let known = cached.map(|c| c.known_images()).unwrap_or_default();
    for image in titles.iter_mut().flat_map(|t| t.images.iter_mut()) {
        if let Some((size, look)) = known.get(&image.key) {
            image.size = image.size.or(*size);
            image.look = image.look.or(*look);
        }
    }
    Some(Catalog {
        source: source.id().to_owned(),
        built_at: now,
        count,
        titles,
    })
}

/// Every theme that can take art from a library.
fn library_themes() -> impl Iterator<Item = ThemeId> {
    ThemeId::ALL.into_iter().filter(|t| t.uses_library())
}

/// Measure the first [`MAX_BACKDROPS`] images of every title in some theme's
/// [`SHORTLIST`] that have no look yet: their sizes first where the server
/// reports sizes, then a thumbnail, unless the size already rules it out. One
/// request at a time, each after asking `stop`.
fn measure(shelves: &mut [Shelf], stop: Stop<'_>) {
    let all: Vec<Candidate> = shelves
        .iter()
        .flat_map(|s| s.catalog.titles.iter().cloned())
        .collect();
    let wanted: BTreeSet<TitleKey> = library_themes()
        .flat_map(|theme| rank_n(theme, &all, SHORTLIST))
        .map(|pick| pick.title)
        .collect();
    for shelf in shelves.iter_mut() {
        let source = Arc::clone(&shelf.source);
        for title in shelf
            .catalog
            .titles
            .iter_mut()
            .filter(|t| wanted.contains(&t.key))
        {
            if stop() {
                return;
            }
            measure_title(source.as_ref(), title, stop);
        }
    }
}

fn measure_title(source: &dyn ArtSource, title: &mut Candidate, stop: Stop<'_>) {
    let measured = title.images.len().min(MAX_BACKDROPS);
    let Some(images) = title.images.get(..measured) else {
        return;
    };
    if images.iter().all(|i| i.look.is_some()) {
        return;
    }
    if images.iter().any(|i| i.size.is_none()) {
        match source.sizes(title) {
            Ok(sizes) => {
                let sizes: HashMap<ImageKey, Size> = sizes.into_iter().collect();
                for image in &mut title.images {
                    image.size = image.size.or_else(|| sizes.get(&image.key).copied());
                }
            }
            Err(why) => log::debug!("theme art: no sizes for {}: {why}", title.title),
        }
    }
    for image in title.images.iter_mut().take(MAX_BACKDROPS) {
        let too_small = image.size.is_some_and(|s| s.width < MIN_WIDTH);
        if image.look.is_some() || too_small {
            continue;
        }
        if stop() {
            return;
        }
        match source.image(&image.key, THUMB_WIDTH) {
            Ok(bytes) => image.look = measure_bytes(&bytes),
            Err(why) => log::debug!("theme art: no thumbnail for {}: {why}", title.title),
        }
    }
}

/// The look of an encoded thumbnail, `None` when it does not decode inside
/// the bounds a thumbnail should keep to.
pub(super) fn measure_bytes(bytes: &[u8]) -> Option<Look> {
    if bytes.len() > MAX_THUMB_BYTES {
        return None;
    }
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_THUMB_DIM);
    limits.max_image_height = Some(MAX_THUMB_DIM);
    reader.limits(limits);
    let rgba = reader.decode().ok()?.into_rgba8();
    let thumb = Thumbnail::new(rgba.width(), rgba.height(), rgba.as_raw())?;
    Some(look(&thumb))
}

/// Every library theme's picks over `shelves`, ranked on measured images only:
/// an image nobody has looked at, or that the server sized under
/// [`MIN_WIDTH`], is never shown.
pub(super) fn choose(shelves: &[Shelf]) -> Picks {
    let mut owner: HashMap<&TitleKey, (&Shelf, &Candidate)> = HashMap::new();
    let mut measured = Vec::new();
    for shelf in shelves {
        for title in &shelf.catalog.titles {
            owner.insert(&title.key, (shelf, title));
            let mut seen = title.clone();
            seen.images.retain(|i| i.look.is_some());
            if !seen.images.is_empty() {
                measured.push(seen);
            }
        }
    }
    let mut picks = BTreeMap::new();
    for theme in library_themes() {
        let chosen: Vec<ArtPick> = rank(theme, &measured)
            .into_iter()
            .filter_map(|pick| {
                let (shelf, title) = owner.get(&pick.title)?;
                ArtPick::new(pick, title, shelf.source.as_ref())
            })
            .collect();
        picks.insert(theme, chosen);
    }
    picks
}
