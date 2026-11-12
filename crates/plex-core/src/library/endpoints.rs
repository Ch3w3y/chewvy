//! The remaining read-only library routes: hubs, children of a show/season,
//! search grouped into hubs, on-deck, a section's sort listings, and the
//! artwork (`/photo/:/transcode`) request shape.
//!
//! Like the rest of this module, every field name here is a name a captured
//! real answer in `tests/fixtures/real/` actually uses (`hubIdentifier`,
//! `hubKey`, `descKey`, …). A field no capture names is absent from the model:
//! the `/hubs/search` answers name no `hubKey`/`key`/`promoted`, so those are
//! `None` there rather than guessed.
//!
//! ## Declared count vs embedded rows
//!
//! A hub names its own `size` (how many items the server says the hub *has*)
//! and separately carries the `Metadata[]` rows this one answer embedded. The
//! captures are redacted with arrays cut to 3 items, so a captured hub reports
//! `size` 6 and embeds 3 rows; a live `/hubs?count=N` can legitimately embed
//! fewer rows than a hub's own `size` too. [`Hub::available`] and
//! [`Hub::items`] therefore stay two separate readings and no completion or
//! paging decision is inferred from comparing them — [`crate::library::ItemPage`]
//! is the only paging model here, and it reads `totalSize`.
//!
//! ## Request query keys
//!
//! [`QUERY_HUB_COUNT`], [`QUERY_SEARCH_LIMIT`], [`QUERY_SORT`] and
//! [`QUERY_UNWATCHED`] are the keys this client asks with. The captures record
//! answers, not request lines, so these names come from the live capture run
//! that produced them rather than from the JSON:
//! `fixtures/real/films_sorted_added.json` is in `addedAt` order and
//! `films_unwatched.json` reports a different `totalSize` (1611) than
//! `films_all.json` (1654). Whether a running server accepts them for this
//! account is a live-driver check, not something these tests can prove.
//!
//! ## The artwork request
//!
//! [`photo_transcode_path`] builds the `/photo/:/transcode` path and query; the
//! token never enters it (see [`crate::http`], which also refuses a query key
//! named `X-Plex-Token`). No code in this workspace requests that path yet: the
//! crate builds the request target, which a shell that renders artwork would
//! request once it asks for a resized image.

use serde::Deserialize;

use crate::client_info::TOKEN_HEADER;
use crate::error::CoreError;
use crate::server::Server;

use super::container::{clean, ContainerMeta};
use super::items::{into_items, Item, WireItem};

/// `GET /hubs` (the home screen's shelves).
pub const HUBS_PATH: &str = "/hubs";

/// `GET /hubs/search?query=…` (search results grouped into per-type hubs).
pub const SEARCH_HUBS_PATH: &str = "/hubs/search";

/// `GET /library/onDeck` (what this account should watch next).
pub const ON_DECK_PATH: &str = "/library/onDeck";

/// `GET /library/metadata/{ratingKey}/children` (seasons of a show, episodes
/// of a season). `{key}` is validated with [`crate::library::validate_segment`].
pub const CHILDREN_PATH: &str = "/library/metadata";

/// `GET /library/sections/{key}/sorts` (the sorts one section offers).
pub const SECTION_SORTS_PATH: &str = "/library/sections";

/// `GET /photo/:/transcode` — the artwork request path (see [`photo_transcode_path`]).
pub const PHOTO_TRANSCODE_PATH: &str = "/photo/:/transcode";

/// Query key [`hubs`] sends its row bound with.
pub const QUERY_HUB_COUNT: &str = "count";

/// Query key [`search_hubs`] sends its per-hub row bound with.
pub const QUERY_SEARCH_LIMIT: &str = "limit";

/// Query key a browse request sends its sort value with (see [`validate_sort`]).
pub const QUERY_SORT: &str = "sort";

/// Query key a browse request sends [`BrowseOptions::unwatched`] with. Its
/// captured answer reports a smaller `totalSize` than the unfiltered browse.
pub const QUERY_UNWATCHED: &str = "unwatched";

/// The value [`QUERY_UNWATCHED`] is sent with when [`BrowseOptions::unwatched`]
/// is set.
pub const QUERY_UNWATCHED_ON: &str = "1";

/// Query keys of the artwork request (see [`photo_transcode_path`]).
///
/// No capture records a `/photo/:/transcode` **request line**, so these four
/// keys and the flag values they carry are the shape this client builds, not
/// names a captured answer uses.
pub const QUERY_PHOTO_URL: &str = "url";

/// See [`QUERY_PHOTO_URL`].
pub const QUERY_PHOTO_MIN_SIZE: &str = "minSize";

/// See [`QUERY_PHOTO_URL`].
pub const QUERY_PHOTO_WIDTH: &str = "width";

/// See [`QUERY_PHOTO_URL`].
pub const QUERY_PHOTO_HEIGHT: &str = "height";

/// See [`QUERY_PHOTO_URL`].
pub const QUERY_PHOTO_UPSCALE: &str = "upscale";

/// Largest row bound [`hubs`] and [`search_hubs`] will ask for.
pub const MAX_HUB_ROWS: usize = 100;

/// Largest width or height [`photo_transcode_path`] will put in a request.
pub const MAX_PHOTO_DIMENSION: u32 = 4096;

/// Longest sort value accepted (`titleSort`, `addedAt:desc`, …).
pub const MAX_SORT_LEN: usize = 128;

/// A shelf of items, as one hub answer names it.
///
/// Every name is a name the captured hub answers use. Both captures name
/// `hubIdentifier`, `title`, `type`, `size`, `context`, `style` and `more` for
/// every hub; only the home capture names `hubKey`, `key` and `promoted`, so a
/// search hub reports those three as `None`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Hub {
    /// The hub's stable name (`hubIdentifier`): `home.continue`, `home.ondeck`,
    /// `home.movies.recent`, `continueWatching`, or a search hub's type name
    /// (`movie`, `show`, `artist`). `None` only when an answer names none.
    pub hub_id: Option<String>,

    /// The hub's own path (`key`), e.g. `/hubs/home/onDeck`. Absent in the
    /// captured search hubs.
    pub path: Option<String>,

    /// The item keys behind this hub (`hubKey`), e.g.
    /// `/library/metadata/43492,14431`. A server-supplied list of rating keys
    /// separated by commas, **not** a path: never pass it to
    /// [`crate::library::validate_segment`] as if it were one key. Absent in
    /// the captured search hubs.
    pub item_keys: Option<String>,

    /// The label to show (`title`).
    pub title: Option<String>,

    /// What the hub holds (`type`): `mixed`, `episode`, `movie`, `show`, `artist`.
    pub kind: Option<String>,

    /// The hub's own `size`: how many items the server says this hub has.
    ///
    /// This is **not** `items.len()`: a capture is cut to 3 rows and a live
    /// `count=N` answer can embed fewer rows than a hub reports. Compare the
    /// two only to decide that more items exist, never that they do not.
    pub available: Option<usize>,

    /// The hub's context string (`context`): `hub.home.continue`,
    /// `hub.continueWatching`. A search hub answers `context: ""`, which is
    /// reported as `None`.
    pub context: Option<String>,

    /// How the server says to present it (`style`): `hero` or `shelf`. Both
    /// captured hub answers name a style for every hub.
    pub style: Option<String>,

    /// `more`: whether the server offers a full list for this hub. The captured
    /// home hubs name `true`, the captured search hubs `false`.
    pub more: Option<bool>,

    /// `promoted`: named only by the captured home answer (`true` on all three
    /// shelves); `None` everywhere else.
    pub promoted: Option<bool>,

    /// The `Metadata[]` rows this answer embedded, as [`Item`] rows.
    pub items: Vec<Item>,
}

/// The children of one item: a show's seasons or one season's episodes.
///
/// Both captured children answers report the row count in the container's own
/// `size` (8 for the show, 13 for the season) with **no** `totalSize` and no
/// `offset`, so these are not a paged listing and must not be treated as one:
/// [`Children::available`] is that declared `size`, and [`Children::total`]
/// stays `None` whenever the answer names no `totalSize`.
///
/// The show capture also carries a navigational `Directory` row (`title`
/// "All episodes", `key` `/library/metadata/35773/allLeaves`, `leafCount` 100)
/// beside its season rows. That row is a browse affordance the server offers,
/// not a child, so it is not in [`Children::items`].
#[derive(Debug, Clone, PartialEq)]
pub struct Children {
    /// The child rows (season `Metadata` or episode `Metadata`).
    pub items: Vec<Item>,

    /// The container's declared `size`, kept separate from `items.len()` (see
    /// the module docs on declared counts).
    pub available: Option<usize>,

    /// The container's `totalSize` when it names one (neither captured children
    /// answer does), so a caller can tell a paged listing from this one.
    pub total: Option<usize>,

    /// The container's `identifier` (`com.plexapp.plugins.library` in both
    /// captures) when it names one.
    pub identifier: Option<String>,
}

/// One sort a library section offers (`/library/sections/{key}/sorts`).
///
/// The captured sorts answer names `key`, `title`, `descKey`,
/// `defaultDirection` and, on one row only, `firstCharacterKey`. It also names
/// `default` (`"asc"` for `titleSort`) on that one row; no capture says what
/// `default` means as opposed to `defaultDirection`, so no field here claims to
/// interpret it.
#[derive(Debug, Clone, PartialEq)]
pub struct SortOption {
    /// The ascending form of this sort (`titleSort`, `originallyAvailableAt`,
    /// `rating`) — the value to send as [`QUERY_SORT`] for ascending order.
    pub key: String,

    /// The label to show (`Title`, `Release Date`, `Critic Rating`).
    pub title: Option<String>,

    /// The same sort reversed, as its own value (`titleSort:desc`).
    pub desc_key: Option<String>,

    /// The direction the server says this sort is normally shown in (`asc` or
    /// `desc`) when it names one.
    pub default_direction: Option<String>,

    /// The first-character browse path this sort supports
    /// (`/library/sections/2/firstCharacter`), when it names one. Not fetched
    /// by this crate.
    pub first_character_path: Option<String>,
}

/// Optional browse refinements for [`crate::library::browse_items`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BrowseOptions {
    /// A sort value taken from [`SortOption`] ([`SortOption::key`] or
    /// [`SortOption::desc_key`]), validated by [`validate_sort`]. One sort
    /// value only: no captured answer shows a combined or comma-separated one.
    pub sort: Option<String>,

    /// Ask for the unwatched items only (`unwatched=1`).
    pub unwatched: bool,
}

/// A sort value this client will put in a query, validated.
///
/// The shape accepted is exactly what a captured `/sorts` answer names: a sort
/// key (`A-Z a-z 0-9 . _ -`, same alphabet as [`crate::library::validate_segment`])
/// optionally followed by `:asc` or `:desc`. `titleSort:desc` passes;
/// `addedAt:DESC`, `titleSort:`, `:desc`, `a;b`, `titleSort:desc,addedAt` and
/// anything with a space, `/`, `?` or `%` are [`CoreError::InvalidInput`]. The
/// accepted value comes back **unchanged** apart from surrounding whitespace:
/// [`SortOption::desc_key`] (`titleSort:desc`) is passed through with its
/// direction, so validating it cannot silently request the other order.
pub fn validate_sort(value: &str) -> Result<String, CoreError> {
    let value = value.trim();
    if value.is_empty() || value.len() > MAX_SORT_LEN {
        return Err(CoreError::InvalidInput);
    }
    let (key, direction) = match value.split_once(':') {
        Some((key, direction)) => (key, Some(direction)),
        None => (value, None),
    };
    // Reuse the path-segment guard for the key: it is the same bounded
    // alphabet, and a sort key can never legally contain a path or query byte.
    let key = super::validate_segment(key)?;
    if let Some(direction) = direction {
        if direction != "asc" && direction != "desc" {
            return Err(CoreError::InvalidInput);
        }
        return Ok(format!("{key}:{direction}"));
    }
    Ok(key)
}

/// The `/photo/:/transcode` path + query for one artwork URL.
///
/// Returns the full request target (`/photo/:/transcode?width=…&height=…&minSize=1&upscale=0&url=…`)
/// for a `thumb`/`art`/`Image[].url` value an item answer named, ready to pass
/// to [`Server::get`] (which adds the token as a **header**). The `url` value is
/// form-encoded, so a thumb path can never escape its own query parameter.
///
/// Rejects a target that is neither a server-named artwork path nor an `https`
/// image on Plex's own static host (see [`plex_static_url`]): an empty or
/// over-long value, anything with whitespace or a control character, a `?` or
/// `#` (which would change the request it becomes), or a value naming
/// `X-Plex-Token` (a token must never reach a URL). Bad width/height (zero, or
/// above [`MAX_PHOTO_DIMENSION`]) is [`CoreError::InvalidInput`] too.
pub fn photo_transcode_path(thumb: &str, width: u32, height: u32) -> Result<String, CoreError> {
    let thumb = thumb.trim();
    if thumb.is_empty()
        || thumb.len() > MAX_KEY_LEN_ARTWORK
        || thumb.contains('?')
        || thumb.contains('#')
        || thumb.chars().any(|c| c.is_whitespace() || c.is_control())
        || thumb.contains(TOKEN_HEADER)
        || width == 0
        || height == 0
        || width > MAX_PHOTO_DIMENSION
        || height > MAX_PHOTO_DIMENSION
    {
        return Err(CoreError::InvalidInput);
    }
    if !(thumb.starts_with('/') || plex_static_url(thumb)) {
        return Err(CoreError::InvalidInput);
    }
    let encoded: String = url::form_urlencoded::byte_serialize(thumb.as_bytes()).collect();
    Ok(format!(
        "{PHOTO_TRANSCODE_PATH}?{QUERY_PHOTO_WIDTH}={width}&{QUERY_PHOTO_HEIGHT}={height}&{QUERY_PHOTO_MIN_SIZE}={PHOTO_FLAG_ON}&{QUERY_PHOTO_UPSCALE}={PHOTO_FLAG_OFF}&{QUERY_PHOTO_URL}={encoded}"
    ))
}

/// Whether `thumb` is an `https` URL on Plex's own static media host (a cast
/// photo's `Role.thumb`), which the server is asked to fetch: the client never
/// contacts that host itself. `http`, other hosts, ports and userinfo are refused.
fn plex_static_url(thumb: &str) -> bool {
    url::Url::parse(thumb).is_ok_and(|url| {
        url.scheme() == "https"
            && url.port().is_none()
            && url.username().is_empty()
            && url.password().is_none()
            && url
                .host_str()
                .is_some_and(|host| host == "plex.tv" || host.ends_with(".plex.tv"))
    })
}

/// The value [`QUERY_PHOTO_MIN_SIZE`] is sent with: keep cover semantics, so
/// the answer still covers the requested box instead of fitting inside it.
const PHOTO_FLAG_ON: &str = "1";

/// The value [`QUERY_PHOTO_UPSCALE`] is sent with: a server that enlarges a
/// small source here hides its true size, because a stretched answer arrives at
/// the requested dimensions whatever the source was. Refusing the enlargement
/// leaves the answer at min(source, requested), which is what lets the client
/// see the source is small and resample it itself.
const PHOTO_FLAG_OFF: &str = "0";

/// Longest artwork URL accepted (server-named `/library/metadata/…` paths).
const MAX_KEY_LEN_ARTWORK: usize = 1024;

#[derive(Deserialize)]
struct WireHub {
    #[serde(rename = "hubIdentifier", default)]
    hub_id: Option<String>,
    #[serde(rename = "hubKey", default)]
    item_keys: Option<String>,
    #[serde(rename = "key", default)]
    path: Option<String>,
    #[serde(rename = "title", default)]
    title: Option<String>,
    #[serde(rename = "type", default)]
    kind: Option<String>,
    #[serde(rename = "size", default)]
    size: Option<usize>,
    #[serde(rename = "context", default)]
    context: Option<String>,
    #[serde(rename = "style", default)]
    style: Option<String>,
    #[serde(rename = "more", default)]
    more: Option<bool>,
    #[serde(rename = "promoted", default)]
    promoted: Option<bool>,
    #[serde(rename = "Metadata", default)]
    metadata: Vec<WireItem>,
}

#[derive(Deserialize)]
struct WireSort {
    #[serde(rename = "key", default)]
    key: Option<String>,
    #[serde(rename = "title", default)]
    title: Option<String>,
    #[serde(rename = "descKey", default)]
    desc_key: Option<String>,
    #[serde(rename = "defaultDirection", default)]
    default_direction: Option<String>,
    #[serde(rename = "firstCharacterKey", default)]
    first_character_path: Option<String>,
}

fn into_hubs(rows: Vec<WireHub>) -> Vec<Hub> {
    rows.into_iter()
        .map(|hub| Hub {
            hub_id: clean(hub.hub_id),
            path: clean(hub.path),
            item_keys: clean(hub.item_keys),
            title: clean(hub.title),
            kind: clean(hub.kind),
            available: hub.size,
            context: clean(hub.context),
            style: clean(hub.style),
            more: hub.more,
            promoted: hub.promoted,
            items: into_items(hub.metadata, ContainerMeta::default()).0,
        })
        .collect()
}

fn into_sorts(rows: Vec<WireSort>) -> Vec<SortOption> {
    rows.into_iter()
        .filter_map(|row| {
            // A sort with no key is not selectable, so it is not a sort option.
            let key = clean(row.key)?;
            Some(SortOption {
                key,
                title: clean(row.title),
                desc_key: clean(row.desc_key),
                default_direction: clean(row.default_direction),
                first_character_path: clean(row.first_character_path),
            })
        })
        .collect()
}

/// Validate a row bound asked of the hub endpoints.
fn validate_rows(count: usize) -> Result<usize, CoreError> {
    if count == 0 || count > MAX_HUB_ROWS {
        return Err(CoreError::InvalidInput);
    }
    Ok(count)
}

/// `GET /hubs?count=N` — the home screen's shelves, each with its items.
pub fn hubs(server: &Server, count: usize) -> Result<Vec<Hub>, CoreError> {
    let count = validate_rows(count)?;
    let response = server.get(HUBS_PATH, &[(QUERY_HUB_COUNT, &count.to_string())])?;
    let (rows, _) = super::read_hub_container::<WireHub>(response.status, &response.body)?;
    Ok(into_hubs(rows))
}

/// `GET /hubs/search?query=…&limit=N` — search results grouped per type.
///
/// The groups are [`Hub`] rows (title, `type`, `size`) whose [`Hub::items`] are
/// the matches; a hub the server found nothing for (`artist`, `size` 0) answers
/// with no `Metadata` array at all and still appears here, with no items.
pub fn search_hubs(server: &Server, query: &str, limit: usize) -> Result<Vec<Hub>, CoreError> {
    let query = super::validate_query(query)?;
    let limit = validate_rows(limit)?;
    let limit_text = limit.to_string();
    let response = server.get(
        SEARCH_HUBS_PATH,
        &[("query", query.as_str()), (QUERY_SEARCH_LIMIT, &limit_text)],
    )?;
    let (rows, _) = super::read_hub_container::<WireHub>(response.status, &response.body)?;
    Ok(into_hubs(rows))
}

/// `GET /library/onDeck` — what this account should watch next.
///
/// A mixed listing: the captured answer holds both `episode` and `movie` rows
/// and, unlike a hub, reports `totalSize` (`21`) and `offset` (`0`), so it reads
/// as a normal [`crate::library::ItemPage`]. `viewOffset` is named on some
/// rows and not others (`null` for one episode), so it stays `Option`.
pub fn on_deck(server: &Server) -> Result<crate::library::ItemPage, CoreError> {
    let response = server.get(ON_DECK_PATH, &[])?;
    let (rows, meta) = super::read_container::<WireItem>(response.status, &response.body)?;
    let (items, total, start, identifier) = into_items(rows, meta);
    Ok(crate::library::ItemPage {
        items,
        total,
        start,
        identifier,
    })
}

/// `GET /library/metadata/{ratingKey}/children` — a show's seasons or a
/// season's episodes, in the order the server returns them.
///
/// The rows are ordinary item rows (see [`crate::library::Item`]): a season row
/// names `index`, `leafCount`, `viewedLeafCount` and `parentRatingKey`; an
/// episode row names `index`, `parentIndex`, `parentTitle`, `grandparentTitle`,
/// `thumb` and (when watched partially) `viewOffset`.
pub fn children(server: &Server, rating_key: &str) -> Result<Children, CoreError> {
    let key = super::validate_segment(rating_key)?;
    let response = server.get(&format!("{CHILDREN_PATH}/{key}/children"), &[])?;
    let (rows, meta) = super::read_children_container::<WireItem>(response.status, &response.body)?;
    let declared = meta.declared_rows;
    let (items, total, _, identifier) = into_items(rows, meta);
    Ok(Children {
        items,
        available: declared,
        total,
        identifier,
    })
}

/// `GET /library/sections/{key}/sorts` — the sorts one section offers.
pub fn sorts(server: &Server, section_key: &str) -> Result<Vec<SortOption>, CoreError> {
    let key = super::validate_segment(section_key)?;
    let response = server.get(&format!("{SECTION_SORTS_PATH}/{key}/sorts"), &[])?;
    let (rows, _) = super::read_container::<WireSort>(response.status, &response.body)?;
    Ok(into_sorts(rows))
}
