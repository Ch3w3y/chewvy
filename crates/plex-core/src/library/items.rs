//! Library sections, items, pages and search results — the parsed shape of a
//! `Metadata[]` / `Directory[]` row.
//!
//! Every field name here is a name a captured real answer in
//! `tests/fixtures/real/` uses; a field no captured answer names is absent from
//! the model. Values are stored as the server recorded them: this module does
//! not convert units, resolve artwork, or decide what can be played.

use serde::Deserialize;

use crate::error::CoreError;

use super::container::{clean, ContainerMeta};
use super::media::{
    chapters_from, markers_from, media_from, part_from, Chapter, Marker, Media, MediaPart,
    WireChapter, WireMarker, WireMedia, WirePart,
};

// ------------------------------------------------------------ sections

/// One library section (`title` + `type` + the `key` that addresses it).
///
/// The captured `GET /library/sections` answer is the source for these names;
/// an absent field is `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// The section's own `key` as recorded (often `/library/sections/12345`).
    pub key: String,
    /// UI label.
    pub title: Option<String>,
    /// Library kind (`type`), e.g. `movie` / `show` / `artist` / `photo`.
    ///
    /// The captured answer records `movie`, `show` and `photo`; the full set a
    /// PMS can report is open, so this client stores it verbatim and never
    /// branches on an unverified value.
    pub kind: Option<String>,
}

impl Section {
    /// The last path segment of [`Section::key`], which is what
    /// [`super::browse_items`] takes.
    ///
    /// Validated with [`super::validate_segment`]; a key this client cannot
    /// address safely is reported instead of used.
    pub fn id(&self) -> Result<String, CoreError> {
        let last = self
            .key
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or_default();
        super::validate_segment(last)
    }
}

// ----------------------------------------------------------------- items

/// One artwork entry of an item's `Image[]`.
///
/// Captured answers record `coverPoster`, `background`, `clearLogo` and
/// `snapshot` here; `url` is a server path
/// (`/library/metadata/{key}/thumb/{ts}`) recorded verbatim.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ItemImage {
    /// Which artwork this is, as recorded (`coverPoster` / `background` /
    /// `clearLogo`, …).
    pub image_type: Option<String>,
    /// Server path of the artwork, as recorded.
    pub url: Option<String>,
    /// Accessible name the server recorded for this artwork.
    pub alt: Option<String>,
}

/// One credited person of an item's `Role[]`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Role {
    /// Person name as recorded.
    pub tag: Option<String>,
    /// Character or crew role as recorded (`Theo O'Dell`, `Director`, …).
    pub role: Option<String>,
    /// Person photo as recorded; can be an absolute `https://` metadata URL.
    pub thumb: Option<String>,
}

/// One item of a library section or of a search result.
///
/// Every field is optional: an item row in a list carries far fewer fields than
/// the same item read through [`super::metadata`], and an absent field is
/// `None` rather than a guess.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Item {
    /// Stable per-item key used for [`super::metadata`].
    pub rating_key: Option<String>,
    /// The item's own API path, as recorded.
    pub key: Option<String>,
    /// Item kind (`movie`, `show`, `season`, `episode`, `artist`, …).
    pub kind: Option<String>,
    /// Display title.
    pub title: Option<String>,
    /// Number of this item inside its parent (episode number, season number).
    pub index: Option<u64>,
    /// Number of the parent (season) inside the grandparent (show).
    pub parent_index: Option<u64>,
    /// Show / album title this item belongs to, when recorded.
    pub parent_title: Option<String>,
    /// Rating key of the parent (season / album), when recorded.
    pub parent_rating_key: Option<String>,
    /// Show title for an episode, when recorded.
    pub grandparent_title: Option<String>,
    /// Poster of the parent container (a season's show, an episode's season),
    /// when the answer names `parentThumb`.
    pub parent_thumb: Option<String>,
    /// Show poster for an episode, when recorded.
    pub grandparent_thumb: Option<String>,
    /// Show backdrop for an episode, when recorded.
    pub grandparent_art: Option<String>,
    /// Rating key of the grandparent (show), when recorded.
    pub grandparent_rating_key: Option<String>,
    /// Poster / 16:9 preview path, as recorded.
    pub thumb: Option<String>,
    /// Backdrop path, as recorded.
    pub art: Option<String>,
    /// The item's `Image[]` artwork entries (poster / backdrop / clear logo).
    pub images: Vec<ItemImage>,
    /// Year as recorded.
    pub year: Option<u64>,
    /// Synopsis text as recorded.
    pub summary: Option<String>,
    /// One-line marketing text as recorded.
    pub tagline: Option<String>,
    /// Content advisory as recorded (`gb/PG`, `TV-MA`, …).
    pub content_rating: Option<String>,
    /// Audience score as recorded; the captured answers carry 10-point values
    /// (`audienceRating` 7.1 for a film whose `audienceRatingImage` is
    /// `RottenTomatoes::v2_100pt_supercredpop`).
    pub audience_rating: Option<f64>,
    /// The server's `rating` value, **unit not asserted**: the captured answer
    /// carries 5.7 next to a 100-point Tomatometer `ratingImage`, and `rating`
    /// is `null` on an episode whose `audienceRating` is 10-point, so the scale
    /// is stored verbatim rather than guessed. Fractional network values are
    /// also why `Item` and [`ItemPage`] derive `PartialEq` but not `Eq`: a float
    /// has no reflexive equality, so anything that needs items as map keys must
    /// key on `rating_key`.
    pub rating: Option<f64>,
    /// `Genre[].tag` values, in the order recorded.
    pub genres: Vec<String>,
    /// `Role[]` credited people, in the order recorded.
    pub roles: Vec<Role>,
    /// `Director[].tag` values, in the order recorded.
    pub directors: Vec<String>,
    /// Resume offset as recorded (milliseconds in the captured answers).
    pub view_offset: Option<u64>,
    /// How many times this item was watched, when recorded.
    pub view_count: Option<u64>,
    /// Playable leaves under this item (episodes of a show), when recorded.
    pub leaf_count: Option<u64>,
    /// Watched playable leaves, when recorded.
    pub viewed_leaf_count: Option<u64>,
    /// Direct children (seasons of a show), when recorded.
    pub child_count: Option<u64>,
    /// Section id this item belongs to, when the answer recorded one.
    pub library_section_id: Option<u64>,
    /// Runtime as recorded (milliseconds in the captured answers).
    pub duration: Option<u64>,
    /// Unix seconds of when the server indexed the item, when recorded.
    pub added_at: Option<i64>,
    /// The `Media` versions of this item (empty when the answer carried none).
    pub media: Vec<Media>,
    /// All parts of all versions, flattened.
    ///
    /// This is the flattened list a caller plays from: a part key is addressed
    /// by position, and a captured item has one version with one part. Parts a
    /// row names directly and parts its versions name are merged, never
    /// repeated.
    pub parts: Vec<MediaPart>,
    /// Item-level `Marker[]` ranges (intro / credits).
    pub markers: Vec<Marker>,
    /// Item-level `Chapter[]` ranges.
    pub chapters: Vec<Chapter>,
}

impl Item {
    /// The rating key required to read full metadata, or `InvalidInput`.
    pub fn require_rating_key(&self) -> Result<String, CoreError> {
        self.rating_key
            .as_deref()
            .map(super::validate_segment)
            .transpose()?
            .ok_or(CoreError::ResponseShape)
    }

    /// True when this item has at least one media part the player could
    /// try to play. Deciding *how* to play it is a capability-matrix decision,
    /// not this module's.
    #[must_use]
    pub fn has_media(&self) -> bool {
        !self.parts.is_empty()
    }

    /// The recorded artwork path for one `Image[]` kind, e.g. `clearLogo`.
    ///
    /// Returns only what the server recorded, as a server path.
    #[must_use]
    pub fn image_url(&self, image_type: &str) -> Option<&str> {
        self.images
            .iter()
            .find(|image| image.image_type.as_deref() == Some(image_type))
            .and_then(|image| image.url.as_deref())
    }
}

/// A request for one page of a section's items.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Page {
    /// Zero-based item offset.
    pub start: usize,
    /// Requested page size, at most [`super::MAX_PAGE_SIZE`].
    pub size: usize,
}

impl Page {
    /// The first page, sized [`super::MAX_PAGE_SIZE`].
    #[must_use]
    pub fn first() -> Self {
        Self {
            start: 0,
            size: super::MAX_PAGE_SIZE,
        }
    }

    /// Rejects an oversized page instead of silently asking for more.
    pub fn new(start: usize, size: usize) -> Result<Self, CoreError> {
        if size == 0 || size > super::MAX_PAGE_SIZE {
            return Err(CoreError::InvalidInput);
        }
        Ok(Self { start, size })
    }
}

/// One page of items plus whatever totals the server reported.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemPage {
    /// Items in this page, in the order the server returned them.
    pub items: Vec<Item>,
    /// Total item count if the server reported one.
    pub total: Option<usize>,
    /// The offset the server says it served, when reported.
    pub start: Option<usize>,
    /// The section identifier the server says it served, when reported.
    pub identifier: Option<String>,
}

impl ItemPage {
    /// True when more items are likely available.
    ///
    /// Uses only what the server reported: when no total came back this is
    /// `true` if the page was full (the honest "there may be more" answer),
    /// `false` when a known total has been reached.
    #[must_use]
    pub fn has_more(&self, requested: usize) -> bool {
        match self.total {
            Some(total) => self.items.len() + self.start.unwrap_or(0) < total,
            None => self.items.len() >= requested,
        }
    }
}

/// Results of one search.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchResults {
    /// Matching items, in the order the server returned them.
    pub items: Vec<Item>,
    /// Total matches if the server reported one.
    pub total: Option<usize>,
}

// ------------------------------------------------------------ wire shapes

#[derive(Deserialize)]
pub(crate) struct WireSection {
    #[serde(rename = "key", default)]
    key: Option<String>,
    #[serde(rename = "title", default)]
    title: Option<String>,
    #[serde(rename = "type", default)]
    kind: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct WireGenre {
    #[serde(rename = "tag", default)]
    tag: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct WireDirector {
    #[serde(rename = "tag", default)]
    tag: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct WireRole {
    #[serde(rename = "tag", default)]
    tag: Option<String>,
    #[serde(rename = "role", default)]
    role: Option<String>,
    #[serde(rename = "thumb", default)]
    thumb: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct WireImage {
    #[serde(rename = "type", default)]
    image_type: Option<String>,
    #[serde(rename = "url", default)]
    url: Option<String>,
    #[serde(rename = "alt", default)]
    alt: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct WireItem {
    #[serde(rename = "ratingKey", default)]
    rating_key: Option<String>,
    #[serde(rename = "key", default)]
    key: Option<String>,
    #[serde(rename = "type", default)]
    kind: Option<String>,
    #[serde(rename = "title", default)]
    title: Option<String>,
    #[serde(rename = "index", default)]
    index: Option<u64>,
    #[serde(rename = "parentIndex", default)]
    parent_index: Option<u64>,
    #[serde(rename = "parentTitle", default)]
    parent_title: Option<String>,
    #[serde(rename = "parentRatingKey", default)]
    parent_rating_key: Option<String>,
    #[serde(rename = "grandparentTitle", default)]
    grandparent_title: Option<String>,
    #[serde(rename = "parentThumb", default)]
    parent_thumb: Option<String>,
    #[serde(rename = "grandparentThumb", default)]
    grandparent_thumb: Option<String>,
    #[serde(rename = "grandparentArt", default)]
    grandparent_art: Option<String>,
    #[serde(rename = "grandparentRatingKey", default)]
    grandparent_rating_key: Option<String>,
    #[serde(rename = "thumb", default)]
    thumb: Option<String>,
    #[serde(rename = "art", default)]
    art: Option<String>,
    #[serde(rename = "Image", default)]
    images: Vec<WireImage>,
    #[serde(rename = "year", default)]
    year: Option<u64>,
    #[serde(rename = "summary", default)]
    summary: Option<String>,
    #[serde(rename = "tagline", default)]
    tagline: Option<String>,
    #[serde(rename = "contentRating", default)]
    content_rating: Option<String>,
    #[serde(rename = "audienceRating", default)]
    audience_rating: Option<f64>,
    #[serde(rename = "rating", default)]
    rating: Option<f64>,
    #[serde(rename = "Genre", default)]
    genres: Vec<WireGenre>,
    #[serde(rename = "Role", default)]
    roles: Vec<WireRole>,
    #[serde(rename = "Director", default)]
    directors: Vec<WireDirector>,
    #[serde(rename = "viewOffset", default)]
    view_offset: Option<u64>,
    #[serde(rename = "viewCount", default)]
    view_count: Option<u64>,
    #[serde(rename = "leafCount", default)]
    leaf_count: Option<u64>,
    #[serde(rename = "viewedLeafCount", default)]
    viewed_leaf_count: Option<u64>,
    #[serde(rename = "childCount", default)]
    child_count: Option<u64>,
    #[serde(rename = "librarySectionID", default)]
    library_section_id: Option<u64>,
    #[serde(rename = "duration", default)]
    duration: Option<u64>,
    #[serde(rename = "addedAt", default)]
    added_at: Option<i64>,
    #[serde(rename = "Media", default)]
    media: Vec<WireMedia>,
    /// A part can be attached directly to the row; every captured real answer
    /// nests parts under `Media[].Part[]` instead, so both read as parts.
    #[serde(rename = "Part", default)]
    parts: Vec<WirePart>,
    #[serde(rename = "Marker", default)]
    markers: Vec<WireMarker>,
    #[serde(rename = "Chapter", default)]
    chapters: Vec<WireChapter>,
}

pub(crate) fn into_items(
    rows: Vec<WireItem>,
    meta: ContainerMeta,
) -> (Vec<Item>, Option<usize>, Option<usize>, Option<String>) {
    let items = rows.into_iter().map(into_item).collect();
    (items, meta.total, meta.start, meta.identifier)
}

fn into_item(row: WireItem) -> Item {
    let media = media_from(row.media);
    // Every captured real answer names its parts under `Media[].Part[]`; this
    // workspace's older mock shapes name them on the row instead. A part named
    // in both places is kept once, from `Media`, which is where a real server
    // names its streams: callers address a part by its position in this list.
    let mut parts: Vec<MediaPart> = media
        .iter()
        .flat_map(|medium| medium.parts.clone())
        .collect();
    for part in row.parts.into_iter().map(part_from) {
        let already_named = part
            .key
            .as_deref()
            .is_some_and(|key| parts.iter().any(|seen| seen.key.as_deref() == Some(key)));
        if !already_named {
            parts.push(part);
        }
    }
    Item {
        rating_key: clean(row.rating_key),
        key: clean(row.key),
        kind: clean(row.kind),
        title: clean(row.title),
        index: row.index,
        parent_index: row.parent_index,
        parent_title: clean(row.parent_title),
        parent_rating_key: clean(row.parent_rating_key),
        parent_thumb: clean(row.parent_thumb),
        grandparent_title: clean(row.grandparent_title),
        grandparent_thumb: clean(row.grandparent_thumb),
        grandparent_art: clean(row.grandparent_art),
        grandparent_rating_key: clean(row.grandparent_rating_key),
        thumb: clean(row.thumb),
        art: clean(row.art),
        images: row
            .images
            .into_iter()
            .map(|image| ItemImage {
                image_type: clean(image.image_type),
                url: clean(image.url),
                alt: clean(image.alt),
            })
            .collect(),
        year: row.year,
        summary: clean(row.summary),
        tagline: clean(row.tagline),
        content_rating: clean(row.content_rating),
        audience_rating: row.audience_rating,
        rating: row.rating,
        genres: row
            .genres
            .into_iter()
            .filter_map(|genre| clean(genre.tag))
            .collect(),
        roles: row
            .roles
            .into_iter()
            .map(|role| Role {
                tag: clean(role.tag),
                role: clean(role.role),
                thumb: clean(role.thumb),
            })
            .collect(),
        directors: row
            .directors
            .into_iter()
            .filter_map(|director| clean(director.tag))
            .collect(),
        view_offset: row.view_offset,
        view_count: row.view_count,
        leaf_count: row.leaf_count,
        viewed_leaf_count: row.viewed_leaf_count,
        child_count: row.child_count,
        library_section_id: row.library_section_id,
        duration: row.duration,
        added_at: row.added_at,
        markers: markers_from(row.markers),
        chapters: chapters_from(row.chapters),
        parts,
        media,
    }
}

pub(crate) fn into_sections(rows: Vec<WireSection>) -> Vec<Section> {
    rows.into_iter()
        .filter_map(|row| {
            let key = clean(row.key)?;
            Some(Section {
                key,
                title: clean(row.title),
                kind: clean(row.kind),
            })
        })
        .collect()
}
