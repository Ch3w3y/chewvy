//! What the library needs from one media server, and the paging every catalog
//! read shares.

use crate::theme_art::{Candidate, ImageKey, Size};

/// One server the library reads titles and backdrops from. Every call blocks
/// and runs on the library's own thread. Tokens stay inside the implementation,
/// in request headers, as for every other read of the server.
pub trait ArtSource: Send + Sync {
    /// The id the disk cache is keyed by, stable across runs:
    /// `plex:{machine identifier}` or `jellyfin:{server id}`.
    fn id(&self) -> &str;

    /// The server's name, for the credit line under the background.
    fn name(&self) -> &str;

    /// How many films and series the server holds now. A different count
    /// from the cached one means the catalog changed and is read again.
    ///
    /// # Errors
    ///
    /// A bounded description when the server will not answer.
    fn count(&self) -> Result<u64, String>;

    /// Every film and series the server holds, as candidates whose images have
    /// keys but neither sizes nor looks yet. `stop` is asked before each page.
    ///
    /// # Errors
    ///
    /// As [`ArtSource::count`], and [`STOPPED`] once `stop` answers true.
    fn catalog(&self, stop: Stop<'_>) -> Result<Vec<Candidate>, String>;

    /// The full size of each image of `title` the server reports one for;
    /// empty when it reports none.
    ///
    /// # Errors
    ///
    /// As [`ArtSource::count`].
    fn sizes(&self, title: &Candidate) -> Result<Vec<(ImageKey, Size)>, String>;

    /// The encoded bytes of one image, scaled by the server to `width` pixels
    /// wide.
    ///
    /// # Errors
    ///
    /// As [`ArtSource::count`], and for a status other than 200.
    fn image(&self, image: &ImageKey, width: u32) -> Result<Vec<u8>, String>;
}

/// Asked between requests: true once the pass asking it should end, because
/// the handle that started it was dropped.
pub type Stop<'a> = &'a dyn Fn() -> bool;

/// The error of a read that `stop` cut short.
pub(super) const STOPPED: &str = "stopped";

/// The [`ArtSource::id`] of the Plex server with this machine identifier.
pub(crate) fn plex_id(machine: &str) -> String {
    format!("plex:{machine}")
}

/// The [`ArtSource::id`] of the Jellyfin server `sources.json` keys by `server`.
pub(crate) fn jellyfin_id(server: &str) -> String {
    format!("jellyfin:{server}")
}

/// The most titles one catalog is read to: far past any real library, and a
/// stop for a server that keeps answering full pages.
pub(super) const MAX_TITLES: usize = 100_000;

/// Read a catalog page by page: `page(start)` answers the rows from `start`
/// and the total the server names, if any. Stops at a short or empty page, at
/// the named total, or at [`MAX_TITLES`].
///
/// # Errors
///
/// The first page that fails, or [`STOPPED`] when `stop` answers true before
/// a page; a catalog is read whole or not at all, so a half-read one never
/// replaces the cached one.
pub(super) fn page_through<T>(
    limit: usize,
    stop: Stop<'_>,
    mut page: impl FnMut(usize) -> Result<(Vec<T>, Option<u64>), String>,
) -> Result<Vec<T>, String> {
    let mut rows = Vec::new();
    loop {
        if stop() {
            return Err(STOPPED.to_owned());
        }
        let (more, total) = page(rows.len())?;
        let got = more.len();
        rows.extend(more);
        let reached = total.is_some_and(|total| rows.len() as u64 >= total);
        if got < limit.max(1) || reached || rows.len() >= MAX_TITLES {
            return Ok(rows);
        }
    }
}
