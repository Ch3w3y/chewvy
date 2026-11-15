//! The user's own servers as a source of theme backgrounds.
//!
//! [`ThemeLibrary::start`] reads every connected server's films and series on
//! a background thread, ranks them per theme with [`crate::theme_art`],
//! fetches a [`THUMB_WIDTH`]-pixel thumbnail of each backdrop that could be
//! picked, measures how it will look behind the glass, and ranks again on
//! measured images only. The UI thread calls [`ThemeLibrary::poll`] each frame
//! and reads [`ThemeLibrary::picks`] and [`ThemeLibrary::showing`]; nothing
//! here blocks it.
//!
//! # Data shape
//!
//! * [`ArtSource`] is one server: a Plex server through the session's
//!   [`LiveClient`], or one Jellyfin server of the browse pool. Its images are
//!   keyed by the app's own artwork address, which is what the artwork loader
//!   fetches, so an [`ArtPick::artwork`] goes straight to it at any width.
//! * The disk cache (under the app's cache dir, [`SUBDIR`]) holds one
//!   versioned file per server: the catalog, and each backdrop's size and look
//!   keyed by its address, which names the item and its image tag. A catalog is
//!   read again at most once a day, or when the server's title count changes;
//!   a backdrop is measured once per address.
//! * [`Pinned`] is the user's "Use as background" choice, saved in the config
//!   dir with the server it belongs to ([`Pinned::applies`]). [`ThemeLibrary::showing`] does not consult it: a pin overrides every
//!   theme, so the caller shows [`ThemeLibrary::pinned`] first when there is
//!   one.
//!
//! Requests are reads only. Tokens travel in headers, as for every other read:
//! a Jellyfin address carries an image tag and a width and a Plex one is built
//! by [`plex_core::library::photo_transcode_path`], which refuses a token.

mod build;
mod jellyfin;
mod plex;
mod source;
mod store;
mod system;

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "tests may unwrap; product code keeps these lints on"
)]
mod tests;

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "a live proof may unwrap; product code keeps these lints on"
)]
mod live;

use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::Arc;
use std::time::Duration;

use crate::shell::live::LiveClient;
use crate::sources::browse;
use crate::theme_art::{self, Candidate, LocalTime, Look, Pick, Size, ThemeId};

pub use build::THUMB_WIDTH;
pub use source::ArtSource;
pub use store::{Pinned, SUBDIR};
pub use system::{hemisphere, utc_offset_secs};

/// The width a background is fetched at.
pub const FULL_WIDTH: u32 = 3840;
/// How long one pick stays on screen before the rotation moves on.
pub const ROTATE_MINUTES: u32 = 30;

/// One backdrop chosen for a theme, with its credit.
#[derive(Debug, Clone, PartialEq)]
pub struct ArtPick {
    /// The artwork address the artwork loader fetches, at [`FULL_WIDTH`] for
    /// a background.
    pub artwork: String,
    /// The title's own id in the browse model, to open its detail page.
    pub title_key: String,
    /// The title as shown.
    pub title: String,
    /// Its year.
    pub year: Option<u16>,
    /// The server it came from, by name.
    pub server: String,
    /// That server's [`ArtSource::id`]: a Plex rating key is only unique on
    /// its own server.
    pub source: String,
    /// Critics' score out of 10.
    pub critic: Option<f32>,
    /// Audience score out of 10.
    pub audience: Option<f32>,
    /// True for a film of the cinematography canon.
    pub canon: bool,
    /// The backdrop's full size, when the server reports one.
    pub size: Option<Size>,
    /// What its thumbnail looked like.
    pub look: Option<Look>,
    /// Title merit and image quality combined, `0..=1`.
    pub score: f32,
    /// The ranker's own record, which the rotation steps through.
    pick: Pick,
}

impl ArtPick {
    fn new(pick: Pick, title: &Candidate, source: &dyn ArtSource) -> Option<Self> {
        let image = title.images.iter().find(|i| i.key == pick.image)?;
        Some(Self {
            artwork: pick.image.0.clone(),
            title_key: title.key.0.clone(),
            title: title.title.clone(),
            year: title.year,
            server: source.name().to_owned(),
            source: source.id().to_owned(),
            critic: title.critic.map(theme_art::Score10::get),
            audience: title.audience.map(theme_art::Score10::get),
            canon: pick.canon,
            size: image.size,
            look: image.look,
            score: pick.score,
            pick,
        })
    }

    /// The credit line: `Dune (2021) from Living Room`.
    #[must_use]
    pub fn credit(&self) -> String {
        credit(&self.title, self.year, &self.server)
    }
}

impl Pinned {
    /// The credit line, as [`ArtPick::credit`].
    #[must_use]
    pub fn credit(&self) -> String {
        credit(&self.title, self.year, &self.server)
    }

    /// Whether this pin stands for a server the session has now, with the Plex
    /// server of `plex_machine` connected: a Plex pin only on the server it
    /// was made on, where its rating key and artwork path name the same item.
    #[must_use]
    pub fn applies(&self, plex_machine: Option<&str>) -> bool {
        source_of(&self.title_key, plex_machine).as_deref() == Some(self.source.as_str())
    }

    /// Whether this pin is `title_key`'s, with the Plex server of
    /// `plex_machine` connected.
    #[must_use]
    pub fn is_for(&self, title_key: &str, plex_machine: Option<&str>) -> bool {
        self.title_key == title_key && self.applies(plex_machine)
    }
}

/// The [`ArtSource::id`] of the server `title_key` belongs to: a Jellyfin key
/// names its server, and a Plex key belongs to the Plex server connected now,
/// `plex_machine`; `None` for a Plex key with no Plex server connected.
#[must_use]
pub fn source_of(title_key: &str, plex_machine: Option<&str>) -> Option<String> {
    match crate::source::split(title_key).0 {
        crate::source::Origin::Plex => plex_machine.map(source::plex_id),
        crate::source::Origin::Jellyfin(server) => Some(source::jellyfin_id(&server)),
    }
}

fn credit(title: &str, year: Option<u16>, server: &str) -> String {
    match year {
        Some(year) => format!("{title} ({year}) from {server}"),
        None => format!("{title} from {server}"),
    }
}

/// Each library theme's picks, best first.
type Picks = BTreeMap<ThemeId, Vec<ArtPick>>;

/// Where the library keeps its cache and the pin; `None` keeps that part in
/// memory only.
#[derive(Debug, Clone, Default)]
pub struct Dirs {
    /// The cache root's [`SUBDIR`].
    pub cache: Option<PathBuf>,
    /// The app's config dir.
    pub config: Option<PathBuf>,
}

impl Dirs {
    /// This install's own: the theme art directory beside the artwork cache,
    /// and the config dir `sources.json` lives in.
    #[must_use]
    pub fn for_install() -> Self {
        let cache =
            crate::ui::art_cache::env_dir().and_then(|art| Some(art.parent()?.join(SUBDIR)));
        Self {
            cache,
            config: crate::sources::config_dir(),
        }
    }
}

/// The servers of a session as art sources: the connected Plex server, with
/// the name its credit line shows, when there is one, then each Jellyfin
/// server of the pool.
#[must_use]
pub(crate) fn sources(
    plex: Option<(Arc<LiveClient>, &str)>,
    jellyfin: &browse::Jellyfin,
) -> Vec<Arc<dyn ArtSource>> {
    let plex = plex.and_then(|(client, name)| plex::PlexArt::new(client, name));
    plex.into_iter()
        .map(|p| Arc::new(p) as Arc<dyn ArtSource>)
        .chain(
            jellyfin
                .servers()
                .map(|s| Arc::new(jellyfin::JellyfinArt::new(Arc::clone(s))) as Arc<dyn ArtSource>),
        )
        .collect()
}

/// The library's handle on the UI thread.
pub struct ThemeLibrary {
    updates: Receiver<Picks>,
    /// Dropped with the handle, which is what ends the background thread.
    _stop: Sender<()>,
    picks: Picks,
    pinned: Option<Pinned>,
    config: Option<PathBuf>,
}

impl ThemeLibrary {
    /// Start reading `sources` on a background thread: the cached picks first,
    /// then a refreshed set, then a refresh a day for as long as the handle
    /// lives. Call again with the new list when the servers change.
    #[must_use]
    pub fn start(sources: Vec<Arc<dyn ArtSource>>, dirs: Dirs) -> Self {
        let (updates_tx, updates) = channel();
        let (stop, stop_rx) = channel();
        let store = store::Store::new(dirs.cache);
        let spawned = std::thread::Builder::new()
            .name("theme-art".to_owned())
            .spawn(move || run(&sources, &store, &updates_tx, &stop_rx));
        if let Err(error) = spawned {
            log::warn!("theme art library did not start: {error}");
        }
        let pinned = dirs.config.as_deref().and_then(store::load_pin);
        Self {
            updates,
            _stop: stop,
            picks: Picks::new(),
            pinned,
            config: dirs.config,
        }
    }

    /// Take what the background thread has finished. True when the picks
    /// changed, so the caller knows to look again.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(picks) = self.updates.try_recv() {
            changed |= picks != self.picks;
            self.picks = picks;
        }
        changed
    }

    /// `theme`'s picks, best first; empty for a theme no library feeds and
    /// before the first pass is done.
    #[must_use]
    pub fn picks(&self, theme: ThemeId) -> &[ArtPick] {
        self.picks.get(&theme).map_or(&[], Vec::as_slice)
    }

    /// The pick `theme` shows at `now`, moving on every [`ROTATE_MINUTES`]
    /// in an order of its own each local day ([`theme_art::showing`]).
    #[must_use]
    pub fn showing(&self, theme: ThemeId, now: LocalTime) -> Option<&ArtPick> {
        let picks = self.picks(theme);
        let ranked: Vec<Pick> = picks.iter().map(|p| p.pick.clone()).collect();
        let every = NonZeroU32::new(ROTATE_MINUTES)?;
        let shown = theme_art::showing(&ranked, now, every)?;
        picks.iter().find(|p| p.pick.title == shown.title)
    }

    /// The user's pinned background, if there is one.
    #[must_use]
    pub fn pinned(&self) -> Option<&Pinned> {
        self.pinned.as_ref()
    }

    /// Pin `pin` as the background, or clear the pin with `None`, and save
    /// the choice.
    ///
    /// # Errors
    ///
    /// The IO error when the choice cannot be saved; the pin still holds for
    /// this run.
    pub fn set_pinned(&mut self, pin: Option<Pinned>) -> std::io::Result<()> {
        self.pinned = pin;
        match &self.config {
            Some(dir) => store::save_pin(dir, self.pinned.as_ref()),
            None => Ok(()),
        }
    }
}

const DAY: Duration = Duration::from_secs(24 * 60 * 60);

/// Hand `picks` to the UI and wake its window to take them; false once the
/// handle is gone.
fn publish(updates: &Sender<Picks>, picks: Picks) -> bool {
    let sent = updates.send(picks).is_ok();
    crate::watch::wake();
    sent
}

fn run(
    sources: &[Arc<dyn ArtSource>],
    store: &store::Store,
    updates: &Sender<Picks>,
    stop: &Receiver<()>,
) {
    let stopped = || !matches!(stop.try_recv(), Err(TryRecvError::Empty));
    let cached = build::choose(&build::cached(sources, store));
    if !cached.values().all(Vec::is_empty) && !publish(updates, cached) {
        return;
    }
    loop {
        let shelves = build::refresh(sources, store, crate::sources::unix_now(), &stopped);
        if stopped() || !publish(updates, build::choose(&shelves)) {
            return;
        }
        match stop.recv_timeout(DAY) {
            Err(RecvTimeoutError::Timeout) => {}
            Ok(()) | Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}
