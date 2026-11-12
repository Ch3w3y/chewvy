//! The `MediaContainer` envelope shared by every read route.
//!
//! Captured real answers (`tests/fixtures/real/`) always wrap the rows in a
//! `MediaContainer` object and put item rows in `Metadata[]`, filter/sort rows
//! in `Directory[]`. This module keeps that envelope in one place and accepts
//! the two renderings this project's older mock shapes also use (a container
//! without the wrapper, and a bare array of rows) so those tests keep working;
//! a captured real answer never needs them.
//!
//! Paging keys come from the captured answers: they report `offset`, not
//! `start`, so `offset` is the primary name and `start` is an alias.

use serde::Deserialize;

use crate::error::CoreError;

/// Trim a recorded string and report an empty one as `None`.
///
/// Captured answers carry `""` for some optional labels (a search hub's
/// `context`); storing that as `Some("")` would let a UI render an empty
/// string as if the server had answered something.
pub(crate) fn clean(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// What a container said about the window it served.
///
/// `total` is the answer's own `totalSize` and `None` when it names none.
/// `declared_rows` is the container's own `size`: a count of the rows that
/// container holds, not of the items behind them, so it is kept in a separate
/// field and never read as a paging total.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct ContainerMeta {
    pub(crate) total: Option<usize>,
    pub(crate) start: Option<usize>,
    pub(crate) declared_rows: Option<usize>,
    pub(crate) identifier: Option<String>,
}

/// One container's rows, in the arrays PMS uses.
///
/// A hub answer puts its shelves in `Hub[]` (a browse answer puts its rows in
/// `Metadata[]`, a filter/sort answer in `Directory[]`), so `hub` is what makes
/// `/hubs` and `/hubs/search` readable at all: without it those bodies parse
/// into zero rows.
#[derive(Deserialize)]
#[serde(bound(deserialize = "T: Deserialize<'de>"))]
pub(crate) struct WireList<T> {
    #[serde(rename = "Directory", default)]
    pub(crate) directory: Vec<T>,
    #[serde(rename = "Video", default)]
    pub(crate) video: Vec<T>,
    #[serde(rename = "Metadata", default)]
    pub(crate) metadata: Vec<T>,
    #[serde(rename = "Hub", default)]
    pub(crate) hub: Vec<T>,
    #[serde(rename = "size", default)]
    pub(crate) size: Option<usize>,
    #[serde(rename = "totalSize", default)]
    pub(crate) total_size: Option<usize>,
    #[serde(rename = "offset", alias = "start", default)]
    pub(crate) offset: Option<usize>,
    #[serde(rename = "identifier", default)]
    pub(crate) identifier: Option<String>,
}

/// The container renderings accepted (module docs).
#[derive(Deserialize)]
#[serde(untagged)]
#[serde(bound(deserialize = "T: Deserialize<'de>"))]
pub(crate) enum WireContainer<T> {
    /// `{"MediaContainer": { … }}` — every captured real answer.
    Wrapped {
        #[serde(rename = "MediaContainer")]
        container: WireList<T>,
    },
    /// `{ "Metadata": [ … ] }` with no wrapper object.
    Flat(WireList<T>),
    /// A bare array of rows.
    Bare(Vec<T>),
}

impl<T> WireContainer<T> {
    /// The container plus what it reported about the window it served.
    fn into_list(self) -> (WireList<T>, ContainerMeta) {
        match self {
            Self::Wrapped { container } | Self::Flat(container) => {
                let meta = ContainerMeta {
                    total: container.total_size,
                    start: container.offset,
                    declared_rows: container.size,
                    identifier: clean(container.identifier.clone()),
                };
                (container, meta)
            }
            Self::Bare(rows) => (
                WireList {
                    directory: Vec::new(),
                    video: Vec::new(),
                    metadata: rows,
                    hub: Vec::new(),
                    size: None,
                    total_size: None,
                    offset: None,
                    identifier: None,
                },
                ContainerMeta::default(),
            ),
        }
    }

    /// The item-shaped rows: `Metadata[]`, then `Video[]`, then `Directory[]`.
    ///
    /// `Hub[]` is deliberately **not** mixed in here. A hub-shaped answer
    /// (`/hubs`, `/hubs/search`) names its shelves in `Hub[]`, and every field
    /// of an item row is optional, so reading a hub container as item rows
    /// would turn a shelf's `title`/`key`/`type` into an item that is nothing
    /// of the sort. A hub-shaped answer is read by [`crate::library::hubs`] and
    /// [`crate::library::search_hubs`] through [`WireContainer::into_hub_rows`],
    /// and an item-shaped read that is handed a hub container reports no rows
    /// rather than fabricated ones.
    pub(crate) fn into_rows(self) -> (Vec<T>, ContainerMeta) {
        let (list, meta) = self.into_list();
        let mut rows = list.metadata;
        rows.extend(list.video);
        rows.extend(list.directory);
        (rows, meta)
    }

    /// Only the `Hub[]` rows (see [`WireContainer::into_rows`]).
    pub(crate) fn into_hub_rows(self) -> (Vec<T>, ContainerMeta) {
        let (list, meta) = self.into_list();
        (list.hub, meta)
    }

    /// Only the `Metadata[]` rows.
    ///
    /// A show's children answer carries a navigational `Directory` row
    /// (`title` "All episodes", `key` `/library/metadata/35773/allLeaves`,
    /// `leafCount` 100) beside its season rows: that row is a browse affordance
    /// the server offers, not a child, so it stays out of the item rows.
    pub(crate) fn into_metadata_rows(self) -> (Vec<T>, ContainerMeta) {
        let (list, meta) = self.into_list();
        (list.metadata, meta)
    }
}

/// Reject a non-2xx answer before any parsing, so a 401/404/500 is reported as
/// that status and never as a shape problem.
pub(crate) fn require_success(status: u16, body: &[u8]) -> Result<&[u8], CoreError> {
    if !(200..300).contains(&status) {
        return Err(CoreError::from_status(status));
    }
    Ok(body)
}
