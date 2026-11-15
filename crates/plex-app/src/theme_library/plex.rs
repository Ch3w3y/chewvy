//! The connected Plex server as an art source.

use std::collections::BTreeMap;
use std::sync::Arc;

use plex_core::library::{photo_transcode_path, Item};

use crate::shell::live::LiveClient;
use crate::theme_art::{Candidate, ImageKey, ImageRef, Kind, Score10, Size, TitleKey};

use super::source::{page_through, plex_id, Stop};
use super::ArtSource;

/// Rows per catalog page, the most a Plex browse asks for.
const PAGE: usize = plex_core::library::MAX_PAGE_SIZE;

/// The connected Plex server.
pub(super) struct PlexArt {
    client: Arc<LiveClient>,
    id: String,
    name: String,
}

impl PlexArt {
    /// `None` until the client is connected to a server.
    pub(super) fn new(client: Arc<LiveClient>, name: &str) -> Option<Self> {
        let id = plex_id(client.machine_identifier()?);
        Some(Self {
            client,
            id,
            name: name.to_owned(),
        })
    }

    /// The film and show sections, by key.
    fn sections(&self) -> Result<Vec<String>, String> {
        Ok(self
            .client
            .sections()?
            .into_iter()
            .filter(|s| matches!(s.kind.as_deref(), Some("movie" | "show")))
            .map(|s| s.key)
            .collect())
    }
}

impl ArtSource for PlexArt {
    fn id(&self) -> &str {
        &self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn count(&self) -> Result<u64, String> {
        let mut count = 0u64;
        for section in self.sections()? {
            let page = self.client.catalog(&section, 0, 1)?;
            let total = page.total.unwrap_or(page.items.len());
            count = count.saturating_add(total as u64);
        }
        Ok(count)
    }

    fn catalog(&self, stop: Stop<'_>) -> Result<Vec<Candidate>, String> {
        let mut titles = Vec::new();
        for section in self.sections()? {
            let rows = page_through(PAGE, stop, |start| {
                let page = self.client.catalog(&section, start, PAGE)?;
                Ok((page.items, page.total.map(|t| t as u64)))
            })?;
            titles.extend(rows.iter().filter_map(candidate));
        }
        Ok(titles)
    }

    /// Plex names no image sizes in a listing.
    fn sizes(&self, _title: &Candidate) -> Result<Vec<(ImageKey, Size)>, String> {
        Ok(Vec::new())
    }

    fn image(&self, image: &ImageKey, width: u32) -> Result<Vec<u8>, String> {
        let path = photo_transcode_path(&image.0, width, width * 9 / 16)
            .map_err(|error| error.to_string())?;
        match self.client.raw_get(&path, &[])? {
            (200, body) if !body.is_empty() => Ok(body),
            (status, _) => Err(format!("artwork request returned {status}")),
        }
    }
}

/// One section row as a candidate; `None` for anything but a film or a show
/// with a title. Plex's `rating` and `audienceRating` are both out of 10 (see
/// [`plex_core::library::Item::rating`]).
pub(super) fn candidate(row: &Item) -> Option<Candidate> {
    let kind = match row.kind.as_deref()? {
        "movie" => Kind::Film,
        "show" => Kind::Show,
        _ => return None,
    };
    let images = row
        .art
        .iter()
        .map(|art| ImageRef {
            key: ImageKey(art.clone()),
            size: None,
            look: None,
        })
        .collect();
    Some(Candidate {
        key: TitleKey(row.rating_key.clone()?),
        kind,
        title: row.title.clone()?,
        original_title: row.original_title.clone(),
        year: row.year.and_then(|y| u16::try_from(y).ok()),
        genres: row.genres.clone(),
        collections: row.collections.clone(),
        keywords: Vec::new(),
        critic: row.rating.and_then(Score10::out_of_10),
        audience: row.audience_rating.and_then(Score10::out_of_10),
        view_count: row.view_count.unwrap_or(0),
        provider_ids: provider_ids(
            kind,
            row.guids.iter().filter_map(|guid| guid.split_once("://")),
        ),
        images,
    })
}

/// The external ids that say two servers hold one title: IMDb's, which are
/// unique across kinds, TVDB's for a series, and TMDB's for a film only,
/// because TMDB numbers films and series separately. Anything else (a TMDB
/// collection id, a Plex guid) is left out: two films of one collection are
/// not one film.
pub(super) fn provider_ids<'a>(
    kind: Kind,
    ids: impl Iterator<Item = (&'a str, &'a str)>,
) -> BTreeMap<String, String> {
    ids.filter_map(|(provider, id)| {
        let provider = provider.to_ascii_lowercase();
        let kept = match provider.as_str() {
            "imdb" => true,
            "tvdb" => kind == Kind::Show,
            "tmdb" => kind == Kind::Film,
            _ => false,
        };
        (kept && !id.is_empty()).then(|| (provider, id.to_owned()))
    })
    .collect()
}
