//! Library sections, item browse, search and item metadata — **read-only**.
//!
//! The row models live in submodules: [`container`] holds the `MediaContainer`
//! envelope every read route shares, [`items`] holds sections, item rows, pages
//! and search results, [`media`] holds the `Media` element, its `Part[]`, the
//! `Stream[]` inside a part, and the item-level `Marker[]` / `Chapter[]`
//! ranges, [`endpoints`] holds the hub / children / on-deck / sort routes and
//! the artwork request shape. This module holds the routes themselves, the
//! input guards, and the re-exports a caller outside this crate uses.
//!
//! Every request here goes through [`Server::get`]: one already-connected Plex
//! Media Server, with **that server's own** access token, presented in the
//! `X-Plex-Token` **header** only — [`crate::http`] refuses a query named
//! `X-Plex-Token` and follows no redirect.
//!
//! ## Where the field names come from
//!
//! They are the names the captured real answers in `tests/fixtures/real/` use
//! (recorded from a live Plex Media Server through this client's own transport).
//! A field no captured answer names is **absent** from the model: nothing here
//! is invented to fill a gap. Parsing stays tolerant in the other direction too
//! — unknown fields are ignored and an absent field is `None`, so a server that
//! names something differently reports `None` rather than a guess.
//!
//! Those captures are parsed by the product code paths, not by a bespoke parser:
//! `tests/library_read_only.rs::the_shape_of_a_real_movie_answer` and
//! `::the_shape_of_a_real_episode_answer` call [`metadata`] against the mock
//! while it serves those exact bytes, and
//! `::a_real_section_answer_pages_with_the_container_window` does the same for
//! [`browse_items`]. What the mock answers with is the recorded file, so a model
//! that names a field the server does not send shows up as a failed assertion.
//!
//! A static capture is not the same as a live behaviour check: whether a running
//! server answers the same way for the same account is checked by running the
//! client against it.
//!
//! ## Read-only by construction
//!
//! This module builds `GET` paths only; it never `POST`s, `PUT`s or `DELETE`s,
//! writes no rating/view state, edits no metadata and touches no server setting.
//! `tests/library_read_only.rs` proves it by execution:
//! `library_traffic_is_read_only_and_only_uses_documented_routes` compares the
//! method + path of **every** request the mock recorded for a full section-list
//! + browse + search + metadata session against the fixed set of documented
//! `GET` routes. `tests/mutation_surface.rs` keeps the crate-wide write policy
//! pinned.
//!
//! ## Not implemented here
//!
//! Direct-play vs transcode decisions belong to the capability matrix:
//! [`MediaPart`] and [`Stream`] carry the recorded codec/container/bitrate fields
//! a later decision needs, and nothing here makes that decision. Managed-user /
//! Plex Home library scoping is **not implemented**: a managed or guest account
//! may see fewer sections than its token would technically allow, and this
//! client does not read `plex.tv/api/home/users*` or infer scopes. Search
//! pagination beyond one page at a time, first-character browse (`firstCharacterKey`
//! of a [`SortOption`]), the hub drill-down paths a home hub's `key` names, and
//! 429 handling are not implemented beyond surfacing the status.

mod container;
mod endpoints;
mod items;
mod media;

pub use endpoints::{
    children, hubs, on_deck, photo_transcode_path, search_hubs, sorts, validate_sort,
    BrowseOptions, Children, Hub, SortOption, CHILDREN_PATH, HUBS_PATH, MAX_HUB_ROWS,
    MAX_PHOTO_DIMENSION, MAX_SORT_LEN, ON_DECK_PATH, PHOTO_TRANSCODE_PATH, QUERY_HUB_COUNT,
    QUERY_PHOTO_HEIGHT, QUERY_PHOTO_MIN_SIZE, QUERY_PHOTO_UPSCALE, QUERY_PHOTO_URL,
    QUERY_PHOTO_WIDTH, QUERY_SEARCH_LIMIT, QUERY_SORT, QUERY_UNWATCHED, QUERY_UNWATCHED_ON,
    SEARCH_HUBS_PATH, SECTION_SORTS_PATH,
};
pub use items::{Item, ItemImage, ItemPage, Page, Role, SearchResults, Section};
pub use media::{Chapter, Marker, Media, MediaPart, Stream};

use crate::error::CoreError;
use crate::server::Server;

use container::{require_success, ContainerMeta, WireContainer};
use items::{into_items, into_sections, WireItem, WireSection};
use serde::de::DeserializeOwned;

/// Read the library section list (`GET /library/sections`).
pub const SECTIONS_PATH: &str = "/library/sections";

/// Read one section's items (`GET /library/sections/{key}/all`).
///
/// `{key}` is the last path segment of a section's own `key`; it is validated
/// with [`validate_segment`] before it is put in a path.
pub const SECTION_ITEMS_PATH: &str = "/library/sections";

/// Search this server (`GET /search?query=…`).
pub const SEARCH_PATH: &str = "/search";

/// Read one item's metadata (`GET /library/metadata/{ratingKey}`).
pub const METADATA_PATH: &str = "/library/metadata";

/// Query key that asks an item answer for its `Marker[]` ranges.
///
/// A captured item answer carries `Marker`/`Chapter` only when they were asked
/// for; [`metadata`] asks for both.
pub const QUERY_INCLUDE_MARKERS: &str = "includeMarkers";

/// See [`QUERY_INCLUDE_MARKERS`].
pub const QUERY_INCLUDE_CHAPTERS: &str = "includeChapters";

/// The value both [`QUERY_INCLUDE_MARKERS`] and [`QUERY_INCLUDE_CHAPTERS`] are
/// sent with.
pub const QUERY_ENABLED: &str = "1";

/// Paging query keys (this project's own choice of a container window).
///
/// The **response** paging keys a server uses are `offset`/`size`/`totalSize`
/// (captured; see [`container`]). These **request** keys are the window this
/// project asks with and are not confirmed by a captured answer: a server that
/// ignores them answers its own window, which is why [`ItemPage::has_more`] reads
/// what the container reported instead of assuming the window asked for.
pub const CONTAINER_START_QUERY: &str = "X-Plex-Container-Start";

/// See [`CONTAINER_START_QUERY`].
pub const CONTAINER_SIZE_QUERY: &str = "X-Plex-Container-Size";

/// Largest page size [`browse_items`] will ask for.
pub const MAX_PAGE_SIZE: usize = 200;

/// Longest search query accepted.
pub const MAX_QUERY_LEN: usize = 200;

/// Longest section key / rating key accepted (Plex keys are short).
pub const MAX_KEY_LEN: usize = 128;

/// Validate a section key or rating key before it becomes part of a path.
///
/// These values come from a server response, so they are treated as untrusted
/// input: anything that could change the request target — `/`, `\`, `?`, `#`,
/// `:`, `%`, whitespace, control characters, a `.`-only string, an empty or
/// over-long string — is refused with [`CoreError::InvalidInput`] before a
/// request is built. This is what keeps `GET /library/metadata/{ratingKey}` from
/// becoming a path-traversal request against the server.
///
/// The accepted alphabet (`A-Z a-z 0-9 . _ -`) covers the numeric `ratingKey`
/// and numeric/slug section keys the captured answers record; a server that uses
/// something else surfaces as `InvalidInput`, which is a fixable error rather
/// than a silent widening of the allowed surface.
pub fn validate_segment(segment: &str) -> Result<String, CoreError> {
    let segment = segment.trim();
    if segment.is_empty() || segment.len() > MAX_KEY_LEN {
        return Err(CoreError::InvalidInput);
    }
    if segment.chars().all(|c| c == '.') {
        return Err(CoreError::InvalidInput);
    }
    if !segment
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        || segment.chars().any(char::is_control)
    {
        return Err(CoreError::InvalidInput);
    }
    Ok(segment.to_owned())
}

/// Validate a search query: bounded length, no control characters.
///
/// The query is sent as a query **parameter** ([`crate::http`] encodes it and
/// refuses a query key named `X-Plex-Token`), so this guard is about absurd or
/// control-character input, not about escaping.
pub fn validate_query(query: &str) -> Result<String, CoreError> {
    let query = query.trim();
    if query.is_empty() || query.len() > MAX_QUERY_LEN || query.chars().any(char::is_control) {
        return Err(CoreError::InvalidInput);
    }
    Ok(query.to_owned())
}

/// Parse one answered container into its rows plus what it reported about the
/// window it served.
///
/// A non-2xx answer is reported as that status before any parsing, so a
/// 401/404/500 never looks like a shape problem. Bytes that are not JSON, or
/// JSON that is not a container this client understands, are
/// [`CoreError::ResponseShape`].
fn read_container<T>(status: u16, body: &[u8]) -> Result<(Vec<T>, ContainerMeta), CoreError>
where
    T: DeserializeOwned,
{
    let wire = read_wire(status, body)?;
    Ok(wire.into_rows())
}

/// As [`read_container`], but keeping only the container's `Metadata[]` rows
/// (see [`container::WireContainer::into_metadata_rows`] for why).
fn read_children_container<T>(
    status: u16,
    body: &[u8],
) -> Result<(Vec<T>, ContainerMeta), CoreError>
where
    T: DeserializeOwned,
{
    let wire = read_wire(status, body)?;
    Ok(wire.into_metadata_rows())
}

/// As [`read_container`], but taking only the container's `Hub[]` rows: what
/// `GET /hubs` and `GET /hubs/search` put their shelves in.
fn read_hub_container<T>(status: u16, body: &[u8]) -> Result<(Vec<T>, ContainerMeta), CoreError>
where
    T: DeserializeOwned,
{
    let wire = read_wire(status, body)?;
    Ok(wire.into_hub_rows())
}

fn read_wire<T>(status: u16, body: &[u8]) -> Result<WireContainer<T>, CoreError>
where
    T: DeserializeOwned,
{
    require_success(status, body)?;
    serde_json::from_slice(body).map_err(|_| CoreError::ResponseShape)
}

/// `GET /library/sections` on one connected server, with its own token.
pub fn list_sections(server: &Server) -> Result<Vec<Section>, CoreError> {
    let response = server.get(SECTIONS_PATH, &[])?;
    let (rows, _) = read_container::<WireSection>(response.status, &response.body)?;
    Ok(into_sections(rows))
}

/// `GET /library/sections/{key}/all` with a bounded container window.
///
/// `section_key` is validated with [`validate_segment`]; an unvalidated key
/// never reaches the path.
/// `GET /library/sections/{key}/all` for one page of one section, with the
/// [`BrowseOptions`] refinements this client supports.
///
/// [`BrowseOptions::sort`] is a validated sort value (see [`validate_sort`])
/// taken from [`sorts`] for that section; [`BrowseOptions::unwatched`] asks for
/// the unwatched items only. The **request** keys these are sent with
/// ([`QUERY_SORT`], [`QUERY_UNWATCHED`]) are this client's choice: a capture
/// records an answer, not a request line, so whether a running server accepts
/// them is a live-driver check. A `Default` [`BrowseOptions`] sends exactly what
/// [`Page`] alone used to send.
pub fn browse_items(
    server: &Server,
    section_key: &str,
    page: Page,
    options: &BrowseOptions,
) -> Result<ItemPage, CoreError> {
    let key = validate_segment(section_key)?;
    let start = page.start.to_string();
    let size = page.size.to_string();
    let mut query = vec![
        (CONTAINER_START_QUERY, start.as_str()),
        (CONTAINER_SIZE_QUERY, size.as_str()),
    ];
    let sort;
    if let Some(value) = options.sort.as_deref() {
        sort = validate_sort(value)?;
        query.push((QUERY_SORT, sort.as_str()));
    }
    if options.unwatched {
        query.push((QUERY_UNWATCHED, QUERY_UNWATCHED_ON));
    }
    let response = server.get(&format!("{SECTION_ITEMS_PATH}/{key}/all"), &query)?;
    let (rows, meta) = read_container::<WireItem>(response.status, &response.body)?;
    let (items, total, start, identifier) = into_items(rows, meta);
    Ok(ItemPage {
        items,
        total,
        start,
        identifier,
    })
}

/// `GET /search?query=…` on one connected server, with its own token: every
/// match in one flat list, unlike [`search_hubs`], which keeps the server's own
/// groups. This crate's read-only library tests are what cover its query rules.
pub fn search(server: &Server, query: &str) -> Result<SearchResults, CoreError> {
    let query = validate_query(query)?;
    let response = server.get(SEARCH_PATH, &[("query", query.as_str())])?;
    let (rows, meta) = read_container::<WireItem>(response.status, &response.body)?;
    let (items, total, _, _) = into_items(rows, meta);
    Ok(SearchResults { items, total })
}

/// `GET /library/metadata/{ratingKey}` for one item, asking for its ranges.
///
/// The request carries [`QUERY_INCLUDE_MARKERS`] and [`QUERY_INCLUDE_CHAPTERS`]:
/// a captured item answer reports `Marker[]` / `Chapter[]` only when they were
/// asked for, and both are read from the **item** element. An answer with no item
/// is [`CoreError::ResponseShape`]; it is not turned into a fabricated item.
pub fn metadata(server: &Server, rating_key: &str) -> Result<Item, CoreError> {
    let key = validate_segment(rating_key)?;
    let response = server.get(
        &format!("{METADATA_PATH}/{key}"),
        &[
            (QUERY_INCLUDE_MARKERS, QUERY_ENABLED),
            (QUERY_INCLUDE_CHAPTERS, QUERY_ENABLED),
        ],
    )?;
    let (rows, meta) = read_container::<WireItem>(response.status, &response.body)?;
    let (mut items, ..) = into_items(rows, meta);
    items.pop().ok_or(CoreError::ResponseShape)
}
