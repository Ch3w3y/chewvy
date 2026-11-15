//! One Jellyfin server as an art source.

use std::sync::Arc;

use jellyfin_core::browse::backdrop_address;
use jellyfin_core::dto::Item as JfItem;

use crate::source;
use crate::sources::browse::Server;
use crate::theme_art::{Candidate, ImageKey, ImageRef, Kind, Score10, Size, TitleKey};

use super::source::{jellyfin_id, page_through, Stop};
use super::{plex::provider_ids, ArtSource};

/// Rows per catalog page: small answers, since the catalog asks for no
/// versions or credits.
const PAGE: u32 = 200;

/// One Jellyfin server of the browse pool.
pub(super) struct JellyfinArt {
    server: Arc<Server>,
    id: String,
}

impl JellyfinArt {
    pub(super) fn new(server: Arc<Server>) -> Self {
        let id = jellyfin_id(server.id());
        Self { server, id }
    }
}

impl ArtSource for JellyfinArt {
    fn id(&self) -> &str {
        &self.id
    }

    fn name(&self) -> &str {
        self.server.name()
    }

    fn count(&self) -> Result<u64, String> {
        self.server
            .catalog(0, 1)?
            .total_record_count
            .ok_or_else(|| "the server named no title count".to_owned())
    }

    fn catalog(&self, stop: Stop<'_>) -> Result<Vec<Candidate>, String> {
        let rows = page_through(PAGE as usize, stop, |start| {
            let start = u32::try_from(start).map_err(|e| e.to_string())?;
            let answer = self.server.catalog(start, PAGE)?;
            Ok((answer.items, answer.total_record_count))
        })?;
        Ok(rows
            .iter()
            .filter_map(|row| candidate(self.server.id(), row))
            .collect())
    }

    /// `GET /Items/{id}/Images`, which sizes each backdrop by its index.
    fn sizes(&self, title: &Candidate) -> Result<Vec<(ImageKey, Size)>, String> {
        let (_, item) = source::split(&title.key.0);
        let sized = self.server.images(item)?;
        Ok(sized
            .iter()
            .filter(|image| image.image_type.as_deref() == Some("Backdrop"))
            .filter_map(|image| {
                let index = usize::try_from(image.image_index?).ok()?;
                let key = backdrop_key(self.server.id(), item, index, image.image_tag.as_deref()?)?;
                let size = Size {
                    width: u32::try_from(image.width?).ok()?,
                    height: u32::try_from(image.height?).ok()?,
                };
                Some((key, size))
            })
            .collect())
    }

    fn image(&self, image: &ImageKey, width: u32) -> Result<Vec<u8>, String> {
        let (_, route) =
            source::split_artwork(&image.0).ok_or("that is not a Jellyfin artwork address")?;
        let join = if route.contains('?') { '&' } else { '?' };
        match self
            .server
            .image(&format!("{route}{join}maxWidth={width}"))?
        {
            (200, body) if !body.is_empty() => Ok(body),
            (status, _) => Err(format!("artwork request returned {status}")),
        }
    }
}

/// The app's artwork address of one backdrop, by its position.
fn backdrop_key(server: &str, item: &str, index: usize, tag: &str) -> Option<ImageKey> {
    let route = backdrop_address(item, index, tag)?;
    Some(ImageKey(source::artwork_address(server, &route)))
}

/// One catalog row as a candidate; `None` for a row that is neither a film
/// nor a series, or names no title. Jellyfin's critic score is a percentage
/// and its community score is out of 10, which is where both are told apart.
pub(super) fn candidate(server: &str, row: &JfItem) -> Option<Candidate> {
    let kind = match row.kind.as_deref()? {
        "Movie" => Kind::Film,
        "Series" => Kind::Show,
        _ => return None,
    };
    let images = row
        .backdrop_image_tags
        .iter()
        .enumerate()
        .filter_map(|(index, tag)| {
            Some(ImageRef {
                key: backdrop_key(server, &row.id, index, tag)?,
                size: None,
                look: None,
            })
        })
        .collect();
    Some(Candidate {
        key: TitleKey(source::jellyfin_id(server, &row.id)),
        kind,
        title: row.name.clone().filter(|name| !name.is_empty())?,
        original_title: row.original_title.clone().filter(|t| !t.is_empty()),
        year: row.production_year.and_then(|y| u16::try_from(y).ok()),
        genres: row.genres.clone(),
        collections: Vec::new(),
        keywords: row.tags.clone(),
        critic: row.critic_rating.and_then(Score10::out_of_100),
        audience: row.community_rating.and_then(Score10::out_of_10),
        view_count: row
            .user_data
            .as_ref()
            .and_then(|user| user.play_count)
            .unwrap_or(0),
        provider_ids: provider_ids(
            kind,
            row.provider_ids
                .iter()
                .map(|(provider, id)| (provider.as_str(), id.as_str())),
        ),
        images,
    })
}
