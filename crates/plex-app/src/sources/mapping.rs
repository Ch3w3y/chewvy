//! One Jellyfin server's rows as the browse model's rows.
//!
//! The screens read [`plex_core::library::Item`] and [`Hub`] from either source,
//! so every Jellyfin row is turned into one here and nowhere else. A field the
//! server does not name for a given row is left unset rather than filled from a
//! guess: the captured media carry no overview, no genres, no backdrop and no
//! rating, so those are `None` for them and set for a server that names them.
//! [`crate::source`] explains the ids and artwork addresses written here.

use jellyfin_core::browse::ImageKind;
use jellyfin_core::dto::{Item as JfItem, Person, QueryResult};
use plex_core::library::{Hub, Item, ItemImage, Role};

use crate::source;

/// Jellyfin counts time in 100 ns ticks; the browse model holds milliseconds.
const TICKS_PER_MS: i64 = 10_000;

/// The image roles the browse model names artwork by, and the Jellyfin image
/// each one is read from. `Primary` is the poster every Jellyfin item keys its
/// artwork by; `Backdrop`, `Logo` and `Thumb` are what a hero, a clear logo and
/// an episode's 16:9 card want. A role the row names no tag for is left out.
const ROLES: [(&str, ImageKind); 4] = [
    ("coverPoster", ImageKind::Primary),
    ("background", ImageKind::Backdrop),
    ("clearLogo", ImageKind::Logo),
    ("thumb", ImageKind::Thumb),
];

/// One row of one Jellyfin server as a browse-model row.
///
/// `server` is the id later requests for this row go back to, and goes into its
/// rating key and artwork addresses.
pub(crate) fn item(server: &str, row: &JfItem) -> Item {
    let kind = kind(row);
    let named = |id: &str| source::jellyfin_id(server, id);
    // A season's parent is the series its own id belongs to; an episode names
    // its season and its series separately. Nothing else has a parent here. The
    // names come from the same rows, since Jellyfin labels a parent inline.
    let (parent_title, parent, grandparent_title, grandparent) = match kind.as_deref() {
        Some("season") => (
            row.series_name.as_deref(),
            row.series_id.as_deref(),
            None,
            None,
        ),
        Some("episode") => (
            row.season_name.as_deref(),
            row.season_id.as_deref(),
            row.series_name.as_deref(),
            row.series_id.as_deref(),
        ),
        _ => (None, None, None, None),
    };
    let leaves = row.recursive_item_count;
    let unplayed = row
        .user_data
        .as_ref()
        .and_then(|user| user.unplayed_item_count);
    Item {
        rating_key: Some(named(&row.id)),
        kind: kind.clone(),
        title: row.name.clone(),
        index: row.index_number,
        parent_index: row.parent_index_number,
        parent_title: parent_title.map(str::to_owned),
        parent_rating_key: parent.map(named),
        grandparent_title: grandparent_title.map(str::to_owned),
        grandparent_rating_key: grandparent.map(named),
        // An episode's 16:9 still is what its cards are drawn from, where Plex
        // names one; otherwise the poster is the `Primary` tag.
        thumb: address(server, row, ImageKind::Thumb)
            .or_else(|| address(server, row, ImageKind::Primary)),
        art: address(server, row, ImageKind::Backdrop),
        images: artwork(server, row),
        year: row
            .production_year
            .and_then(|year| u64::try_from(year).ok()),
        summary: row.overview.clone(),
        content_rating: row.official_rating.clone(),
        genres: row.genres.clone(),
        roles: cast(&row.people),
        directors: directors(&row.people),
        view_offset: row
            .user_data
            .as_ref()
            .and_then(|user| millis(user.playback_position_ticks)),
        view_count: row
            .user_data
            .as_ref()
            .and_then(|user| user.play_count)
            .filter(|count| *count > 0),
        leaf_count: leaves,
        // Jellyfin names how many leaves are *left* unwatched; the model holds
        // how many were watched, which is the difference of the two it names.
        viewed_leaf_count: match (leaves, unplayed) {
            (Some(leaves), Some(unplayed)) => Some(leaves.saturating_sub(unplayed)),
            _ => None,
        },
        duration: millis(row.run_time_ticks),
        ..Item::default()
    }
}

/// One Jellyfin home shelf as a hub Home draws: `hub_id` is the name Home
/// recognises (`homecontinue`, `home.onDeck`), `title` the label it shows —
/// including the server, because shelves of several servers are listed together.
pub(crate) fn shelf(server: &str, hub_id: &str, title: &str, answer: &QueryResult<JfItem>) -> Hub {
    let items = answer
        .items
        .iter()
        .map(|row| item(server, row))
        .collect::<Vec<_>>();
    let kind = shelf_kind(&items);
    Hub {
        hub_id: Some(hub_id.to_owned()),
        title: Some(title.to_owned()),
        kind: kind.map(str::to_owned),
        // The shelf's own row count is the only one Jellyfin names for it.
        available: answer
            .total_record_count
            .and_then(|n| usize::try_from(n).ok()),
        style: Some("shelf".to_owned()),
        items,
        ..Hub::default()
    }
}

/// The shelf's type as Plex names it: the kind every row has, `mixed` when the
/// rows differ (which is what the captured resume answer holds), and `None` for
/// a shelf that names no rows at all.
fn shelf_kind(items: &[Item]) -> Option<&'static str> {
    let mut kind: Option<&'static str> = None;
    for item in items {
        let leaf = match item.kind.as_deref() {
            Some("movie") => "movie",
            Some("episode") => "episode",
            _ => return Some("mixed"),
        };
        match kind {
            None => kind = Some(leaf),
            Some(seen) if seen != leaf => return Some("mixed"),
            _ => {}
        }
    }
    kind
}

/// The browse model's kind for one Jellyfin `Type`, spelled the way Plex spells
/// it. Any other type keeps its own name lowercased, so the grid still lists it
/// — as a card it can draw but neither open nor play.
fn kind(row: &JfItem) -> Option<String> {
    let named = match row.kind.as_deref() {
        Some("Series") => Some("show"),
        Some("Movie") => Some("movie"),
        Some("Episode") => Some("episode"),
        Some("Season") => Some("season"),
        Some(other) => return Some(other.to_lowercase()),
        None => None,
    };
    named.map(str::to_owned)
}

/// The artwork address of one image of one row: `None` when the row names no
/// tag for it, since a Jellyfin image cannot be asked for without its tag.
fn address(server: &str, row: &JfItem, kind: ImageKind) -> Option<String> {
    let route = jellyfin_core::browse::image_address(&row.id, kind, row.image_tag(kind)?)?;
    Some(source::artwork_address(server, &route))
}

/// The row's artwork as the model names it by role, so the hero and the cards
/// find a poster, a backdrop and a clear logo the same way they find Plex's.
fn artwork(server: &str, row: &JfItem) -> Vec<ItemImage> {
    ROLES
        .iter()
        .filter_map(|(role, kind)| {
            Some(ItemImage {
                image_type: Some((*role).to_owned()),
                url: Some(address(server, row, *kind)?),
                ..ItemImage::default()
            })
        })
        .collect()
}

/// The cast of one row, as the model holds them. Jellyfin's own `Type` says
/// which credit a person is and its `Role` the character they played; a credit
/// with no name is not someone, and is left out.
fn cast(people: &[Person]) -> Vec<Role> {
    people
        .iter()
        .filter(|person| person.kind.as_deref() == Some("Actor"))
        .filter_map(|person| {
            Some(Role {
                tag: Some(person.name.clone()?),
                role: person.role.clone(),
                // A person's picture is not part of the row: Jellyfin keys it by
                // the person, whom these rows name only by name.
                thumb: None,
            })
        })
        .collect()
}

/// The directors of one row, by name.
fn directors(people: &[Person]) -> Vec<String> {
    people
        .iter()
        .filter(|person| person.kind.as_deref() == Some("Director"))
        .filter_map(|person| person.name.clone())
        .collect()
}

/// Server ticks to the model's milliseconds; a runtime or position Jellyfin
/// counts as zero is no runtime and no resume point.
fn millis(ticks: Option<i64>) -> Option<u64> {
    ticks
        .filter(|ticks| *ticks > 0)
        .and_then(|ticks| u64::try_from(ticks / TICKS_PER_MS).ok())
}

#[cfg(test)]
mod tests;
