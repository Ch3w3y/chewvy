//! The one seam between the browse screens and the two kinds of media source.
//!
//! Every screen already reads [`plex_core::library::Item`] rows and addresses
//! them by one kind of id, so the seam is that id: a Jellyfin row carries its
//! server inside its rating key (`jellyfin:{server}:{item}`) while a Plex key is
//! left exactly as the server named it. A row can therefore say which source to
//! go back to — for its artwork, its detail page, its episodes — without the UI
//! holding a second identifier and without either client learning about the
//! other. [`crate::sources::mapping`] is what writes these ids.

use plex_core::library::Item;

/// Which media source a row, an image or a request belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Origin {
    /// The Plex server the session is connected to.
    Plex,
    /// One Jellyfin server, by the id its `sources.json` row is keyed by.
    Jellyfin(String),
}

impl Origin {
    /// The Jellyfin server this names, when it names one.
    #[must_use]
    pub fn server_id(&self) -> Option<&str> {
        match self {
            Self::Plex => None,
            Self::Jellyfin(server) => Some(server),
        }
    }

    /// Whether these rows' artwork and requests go to a Jellyfin server.
    #[must_use]
    pub fn is_jellyfin(&self) -> bool {
        self.server_id().is_some()
    }

    /// The word a header or a badge uses for a source: the service, not one of
    /// the servers running it.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Plex => "Plex",
            Self::Jellyfin(_) => "Jellyfin",
        }
    }
}

/// The marker every Jellyfin id this app carries starts with.
pub const JELLYFIN_PREFIX: &str = "jellyfin:";

/// The id one row of one Jellyfin server carries: that server's id, then the
/// id the server gave the row.
#[must_use]
pub fn jellyfin_id(server_id: &str, item_id: &str) -> String {
    format!("{JELLYFIN_PREFIX}{server_id}:{item_id}")
}

/// The source one browse id names, and the id to ask that source with.
///
/// An id with the marker but nothing behind it is not treated as a Jellyfin id:
/// it goes back to whichever Plex server was asked, which refuses it rather
/// than reaching a server that never named the row.
#[must_use]
pub fn split(id: &str) -> (Origin, &str) {
    let Some(rest) = id.strip_prefix(JELLYFIN_PREFIX) else {
        return (Origin::Plex, id);
    };
    match rest.split_once(':') {
        Some((server, item)) if !item.is_empty() => (Origin::Jellyfin(server.to_owned()), item),
        _ => (Origin::Plex, id),
    }
}

/// The source one row came from, as its own rating key names it. Rows of both
/// sources are listed together on Home, so a card has to speak for itself.
#[must_use]
pub fn origin_of(item: &Item) -> Origin {
    item.rating_key
        .as_deref()
        .map(|key| split(key).0)
        .unwrap_or(Origin::Plex)
}

/// The artwork address of one image of one Jellyfin server: the marker, that
/// server's id, and the image route the server answers —
/// `jellyfin:{server}/Items/{id}/Images/Primary?tag={tag}`.
///
/// A row's artwork values carry their source for the same reason its id does:
/// cards, shelves and the artwork loader hold rows of both sources together and
/// have to know which server a picture comes from. The width a loader wants is
/// not part of the address ([`crate::ui::images`] adds it).
#[must_use]
pub fn artwork_address(server_id: &str, route: &str) -> String {
    format!("{JELLYFIN_PREFIX}{server_id}{route}")
}

/// The Jellyfin server an artwork value names, and the image route behind the
/// marker. `None` for a Plex artwork value, which needs no naming.
#[must_use]
pub fn split_artwork(value: &str) -> Option<(&str, &str)> {
    let rest = value.strip_prefix(JELLYFIN_PREFIX)?;
    let (server, _) = rest.split_once('/')?;
    let route = rest.strip_prefix(server)?;
    (!server.is_empty() && route.starts_with("/Items/")).then_some((server, route))
}

/// One library of the rail: what it is called, the id it is asked with, and the
/// source whose header it sits under.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Library {
    /// The server this library belongs to.
    pub origin: Origin,
    /// That source's own label: the connected Plex server's display name, or the
    /// Jellyfin server's name as its registry row holds it.
    pub server: String,
    /// The section id: a Plex section key, or a Jellyfin view id through
    /// [`jellyfin_id`].
    pub id: String,
    /// The library's title, as the source named it.
    pub title: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(key: Option<&str>) -> Item {
        Item {
            rating_key: key.map(str::to_owned),
            ..Item::default()
        }
    }

    #[test]
    fn a_plex_key_is_left_as_the_server_named_it() {
        assert_eq!(split("43492"), (Origin::Plex, "43492"));
        assert_eq!(origin_of(&item(Some("43492"))), Origin::Plex);
        assert_eq!(origin_of(&item(None)), Origin::Plex);
    }

    #[test]
    fn a_jellyfin_id_names_its_server_and_keeps_its_own_id() {
        let id = jellyfin_id("a5707433", "323fe870cae813");
        assert_eq!(
            split(&id),
            (Origin::Jellyfin("a5707433".to_owned()), "323fe870cae813")
        );
        assert_eq!(
            origin_of(&item(Some(&id))),
            Origin::Jellyfin("a5707433".to_owned())
        );
    }

    #[test]
    fn an_artwork_address_names_its_server_and_its_route() {
        let value = artwork_address("a5707433", "/Items/323fe870/Images/Primary?tag=f3bf");
        assert_eq!(
            split_artwork(&value),
            Some(("a5707433", "/Items/323fe870/Images/Primary?tag=f3bf"))
        );
        // A Plex thumb value, and any other source's route, names no server.
        assert_eq!(split_artwork("/library/sections/3/12345/thumb/1"), None);
        assert_eq!(split_artwork("jellyfin:"), None);
        assert_eq!(split_artwork("jellyfin:a5707433:323fe870"), None);
        // An address is not an id and an id is not an address.
        assert_eq!(split(&value).0, Origin::Plex);
    }

    #[test]
    fn a_marked_id_with_nothing_behind_it_is_answered_by_plex() {
        assert_eq!(
            split("jellyfin:a5707433:"),
            (Origin::Plex, "jellyfin:a5707433:")
        );
        assert_eq!(
            split("jellyfin:not-an-id"),
            (Origin::Plex, "jellyfin:not-an-id")
        );
    }
}
