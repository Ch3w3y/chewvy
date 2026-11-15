//! Which background art a theme shows: the theme vocabulary, the calendar that
//! picks a seasonal theme, and the scoring that chooses titles and images from
//! the user's own library. Everything here is pure: no I/O, no clock reads, no
//! network. The caller passes metadata, decoded thumbnails and the local time in.
//!
//! # Data shape
//!
//! * [`ThemeId`] is a closed sum, `Aurora | Genre(Genre) | Season(Season) |
//!   Festival(Festival)`, so a function that only makes sense for one family
//!   (the calendar returns a season or a festival; a genre feed names a genre)
//!   takes or returns that family and cannot be handed the wrong kind. Each
//!   theme's [`Feed`]s are data on the id, not branches in the ranker.
//! * [`Candidate`] is one library title in a server-neutral form, built by the
//!   Plex or Jellyfin adapter. Ratings are [`Score10`], a 0 to 10 value that can
//!   only be built from a declared scale (`out_of_10`, `out_of_100`), because
//!   Jellyfin's critic rating is a percentage and Plex's `rating` unit is not
//!   asserted by the client; the adapter decides once, here nothing guesses.
//! * [`ImageRef`] is one backdrop of a title: an opaque key, the size if the
//!   server reported one, and a [`Look`] once the caller has decoded a
//!   thumbnail and run [`look`] over it. An image is measured lazily; an
//!   unmeasured image ranks on a neutral prior.
//! * The canon ([`canon`]) is a const table of films renowned for their
//!   cinematography and scale, keyed by normalised title and year only. It
//!   carries no IMDb or TMDB ids, so a match is by [`normalise`]d title or
//!   original title, with the year allowed to differ by one.
//! * [`rank`] returns a theme's best [`Pick`]s, at most [`DEFAULT_PICKS`]:
//!   ordered by the theme feed that admitted them, then canon films first,
//!   then a score of rating and image quality, then title key. It does not
//!   read the time, so the same library always ranks the same way.
//! * [`showing`] says which pick is on screen at a moment. It orders the picks
//!   by a hash of each title with the local day and steps through that order
//!   every given minutes, so a day keeps one sequence and the next day has a
//!   different one, and picks kept across midnight never show a stale order.
//!
//! [`hemisphere`] reads a time zone name or a locale as the half of the globe
//! the seasons follow, North when neither names a place.
//!
//! [`LocalTime`] is the one time input: Unix seconds plus the UTC offset in
//! force, so the day boundary and the festive windows follow the user's clock.

mod calendar;
pub mod canon;
mod canon_table;
mod genre;
mod hemisphere;
mod image;
mod rank;
mod theme;
mod title;

pub use calendar::{seasonal, CivilDate, Hemisphere, LocalTime};
pub use canon::{normalise, CanonEntry};
pub use genre::genre_suites;
pub use hemisphere::hemisphere;
pub use image::{look, Look, Size, Thumbnail, MIN_WIDTH};
pub use rank::{rank, rank_n, showing, Pick, DEFAULT_PICKS};
pub use theme::{Feed, Festival, Genre, Season, ThemeId};
pub use title::{Candidate, ImageKey, ImageRef, Kind, Score10, TitleKey, MIN_RATING};
