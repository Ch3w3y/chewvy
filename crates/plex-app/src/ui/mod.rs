//! The UI foundation: the dark token set ([`theme`]), the app shell
//! ([`app`] + [`rail`]), the library poster grid ([`grid`]), the bounded async
//! artwork loader ([`images`]) and the artwork disk cache behind it
//! ([`art_cache`]).
//!
//! [`theme::apply`] rewrites both egui styles from the design tokens,
//! [`app::Ui`] draws the connected shell around [`rail::show`], and
//! [`images::Artwork`] fetches, decodes, caches and holds poster textures off
//! the UI thread.

/// Dark tokens, type scale, spacing and the white pill primary button.
pub mod theme;

/// The app screens, the rail-selected content area and the back history.
pub(crate) mod app;
/// The aurora photograph behind every screen.
pub(crate) mod backdrop;
pub(crate) mod bleed;
/// The Chewvy mark and wordmark.
pub(crate) mod brand;
/// The app's own window title bar, buttons and resize edges.
pub(crate) mod chrome;
/// Frosted glass panels (backdrop blur through an egui_glow paint callback).
pub(crate) mod glass;
/// The tints every glass surface uses, held to a WCAG contrast bar by tests.
pub(crate) mod glass_tints;
/// Dolby, DTS, HDR and BBFC logos for format and rating badges.
pub(crate) mod marks;
pub(crate) mod overlay;
/// The sign-in and start-up screens (glass card, orbit, pairing code).
pub(crate) mod splash;

/// The 220px navigation rail: Home, every library section, Search, Settings.
pub(crate) mod rail;

/// The Home hero carousel: pager dots, hover arrows and a timed auto-advance.
pub(crate) mod hero;
/// The Home screen: the hero carousel and one horizontally scrolling shelf per
/// hub.
pub(crate) mod home;

/// The library poster grid: 2:3 posters, paging, and the sort/unwatched chips.
pub(crate) mod grid;

/// Card chrome Home and the grid share: hover/focus ring, corner radii, pager
/// arrows and dots, and the cover crop (centred, and top-anchored for posters).
pub(crate) mod cards;

/// Search results as the server groups them: one section per search hub.
pub(crate) mod search;

/// The app's own settings, persisted as JSON in the config dir.
pub(crate) mod settings;

/// The item detail page: one movie, or one show / season with its episodes,
/// with resume, metadata pills, cast and version badges.
pub(crate) mod detail;

/// The pill controls a browsing screen carries: Sort, its order and Unwatched as
/// chips, and the menu a chip opens with a painted check on the row in effect.
pub(crate) mod chips;

/// Async `/photo/:/transcode` artwork loader with an LRU texture cap and a
/// SHA-256-keyed disk cache.
pub mod images;

/// The artwork cache slots under `$XDG_CACHE_HOME/chewvy/art`, bounded
/// in total size and age.
pub(crate) mod art_cache;

/// The playing screen's control overlay over the embedded video.
pub(crate) mod player;
