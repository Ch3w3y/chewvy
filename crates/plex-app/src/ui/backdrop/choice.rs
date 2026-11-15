//! Which picture the background shows: the theme the setting and the
//! calendar name, then the user's pin, the theme's library art, or its
//! bundled photographs. Pure: the caller passes the time and what the
//! library has.

use crate::theme_art::{seasonal, Hemisphere, LocalTime, ThemeId};
use crate::theme_library::ROTATE_MINUTES;
use crate::ui::settings::Background;

use super::{photos_of, Showing};

/// The theme `choice` names at `now`: Automatic follows the season or
/// festival of the local date in `hemisphere`.
#[must_use]
pub fn theme_for(choice: Background, now: LocalTime, hemisphere: Hemisphere) -> ThemeId {
    match choice {
        Background::Automatic => seasonal(now.date(), hemisphere),
        Background::Theme(theme) => theme,
    }
}

/// What the background shows, and what stands in while it loads or when it
/// cannot be fetched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Want {
    /// The theme the picture belongs to.
    pub theme: ThemeId,
    /// The picture itself.
    pub showing: Showing,
    /// The theme's bundled photograph of the moment.
    pub fallback: Showing,
}

/// The picture for `theme` at `now`: the `pinned` artwork address when there
/// is one, else the `library` pick the theme shows now, else its bundled
/// photographs, which take turns every [`ROTATE_MINUTES`].
#[must_use]
pub fn want(theme: ThemeId, now: LocalTime, pinned: Option<&str>, library: Option<&str>) -> Want {
    let fallback = bundled(theme, now);
    let showing = pinned.or(library).map_or_else(
        || fallback.clone(),
        |address| Showing::Library(address.to_owned()),
    );
    Want {
        theme,
        showing,
        fallback,
    }
}

/// `theme`'s bundled photograph at `now`; the aurora for a theme with none.
#[must_use]
pub fn bundled(theme: ThemeId, now: LocalTime) -> Showing {
    let photos: Vec<usize> = photos_of(theme).collect();
    let per_day = i64::from(24 * 60 / ROTATE_MINUTES.max(1));
    let slot = now.day_number() * per_day + i64::from(now.minute_of_day() / ROTATE_MINUTES.max(1));
    let count = i64::try_from(photos.len()).unwrap_or(1).max(1);
    let index = usize::try_from(slot.rem_euclid(count)).unwrap_or(0);
    Showing::Photo(photos.get(index).copied().unwrap_or(0))
}
