//! The background as the shell keeps it: the theme library reading the
//! connected servers, the stage painting the picture, the Settings section
//! that picks a theme, and the pin a detail page sets.

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::shell::live::LiveClient;
use crate::sources::browse;
use crate::theme_art::{Hemisphere, LocalTime, ThemeId};
use crate::theme_library::{self, Dirs, Pinned, ThemeLibrary};
use crate::ui::backdrop::{self, choice, Kind, Showing, Stage, PHOTOS};
use crate::ui::settings::Background;

use super::Ui;

/// How long the servers must stay as they are before the library reads them
/// again: a sign-in names Plex, its sections and the Jellyfin pool in quick
/// succession, and each restart would read every catalog anew.
const SETTLE: Duration = Duration::from_millis(1500);
/// How often the local clock is read again; the rotation moves in minutes.
const CLOCK_EVERY: Duration = Duration::from_secs(30);

/// The background's state in the shell.
pub(super) struct Backgrounds {
    stage: Stage,
    library: ThemeLibrary,
    /// The connected Plex session the library reads, with no name until the
    /// sections answer names it.
    plex: Option<Arc<LiveClient>>,
    jellyfin: browse::Jellyfin,
    /// When the servers last changed while the library has not caught up.
    changed: Option<Instant>,
    hemisphere: Hemisphere,
    /// The UTC offset in force and when it was read.
    clock: Option<(Instant, i32)>,
    dirs: Dirs,
    /// What the stage showed last frame, for the credit line.
    shown: Option<Showing>,
}

impl Backgrounds {
    pub(super) fn new() -> Self {
        // Tests never read or write the install's own pin.
        let dirs = if cfg!(test) {
            Dirs::default()
        } else {
            Dirs::for_install()
        };
        Self {
            stage: Stage::default(),
            library: ThemeLibrary::start(Vec::new(), dirs.clone()),
            plex: None,
            jellyfin: browse::Jellyfin::default(),
            changed: None,
            hemisphere: theme_library::hemisphere(),
            clock: None,
            dirs,
            shown: None,
        }
    }

    /// The connected Plex session, or none after a sign-out.
    pub(super) fn set_plex(&mut self, client: Option<Arc<LiveClient>>) {
        self.plex = client;
        self.changed = Some(Instant::now());
    }

    /// The Jellyfin servers the browse side now has.
    pub(super) fn set_jellyfin(&mut self, pool: browse::Jellyfin) {
        self.jellyfin = pool;
        self.changed = Some(Instant::now());
    }

    /// The connected Plex server's name changed: read the servers again once
    /// they settle.
    pub(super) fn servers_changed(&mut self) {
        self.changed = Some(Instant::now());
    }

    /// Read the servers again, now: the art setting changed.
    pub(super) fn restart_now(&mut self) {
        self.changed = Some(Instant::now() - SETTLE);
    }

    fn now(&mut self) -> LocalTime {
        let offset = match self.clock {
            Some((read, offset)) if read.elapsed() < CLOCK_EVERY => offset,
            _ => {
                let offset = theme_library::utc_offset_secs();
                self.clock = Some((Instant::now(), offset));
                offset
            }
        };
        LocalTime::from_unix(crate::sources::unix_now(), offset)
    }

    /// The connected Plex server's machine identifier, which a Plex pin and a
    /// Plex rating key are only meaningful with.
    fn plex_machine(&self) -> Option<&str> {
        self.plex
            .as_deref()
            .and_then(LiveClient::machine_identifier)
    }

    /// The saved pin, when it stands for a server the session has now.
    fn pin(&self) -> Option<&Pinned> {
        self.library
            .pinned()
            .filter(|pin| pin.applies(self.plex_machine()))
    }

    /// What the background should show now.
    fn want(&mut self, choice: Background, library_art: bool) -> choice::Want {
        let now = self.now();
        let theme = choice::theme_for(choice, now, self.hemisphere);
        let pinned = self.pin().map(|pin| pin.artwork.clone());
        let library = library_art
            .then(|| self.library.showing(theme, now))
            .flatten()
            .map(|pick| pick.artwork.clone());
        choice::want(theme, now, pinned.as_deref(), library.as_deref())
    }

    /// The credit line of what is on screen.
    fn credit(&self) -> Option<String> {
        match self.shown.as_ref()? {
            Showing::Photo(index) => PHOTOS.get(*index).map(backdrop::Photo::credit),
            Showing::Library(address) => {
                if let Some(pin) = self.pin().filter(|pin| &pin.artwork == address) {
                    return Some(pin.credit());
                }
                ThemeId::ALL
                    .iter()
                    .flat_map(|theme| self.library.picks(*theme))
                    .find(|pick| &pick.artwork == address)
                    .map(theme_library::ArtPick::credit)
            }
        }
    }

    /// The best backdrop the library measured for `title_key` of the server
    /// `source` names, if it ranked one for any theme.
    fn best_art(&self, title_key: &str, source: &str) -> Option<String> {
        ThemeId::ALL
            .iter()
            .flat_map(|theme| self.library.picks(*theme))
            .find(|pick| pick.title_key == title_key && pick.source == source)
            .map(|pick| pick.artwork.clone())
    }

    /// Whether the background is pinned to `title_key` on the server it
    /// belongs to now.
    pub(super) fn is_pinned(&self, title_key: &str) -> bool {
        self.library
            .pinned()
            .is_some_and(|pin| pin.is_for(title_key, self.plex_machine()))
    }
}

impl Ui {
    /// Take what the library finished, restart it once the servers settle,
    /// and paint the background over `rect` in the `kind` copy.
    pub(super) fn paint_background(&mut self, ctx: &egui::Context, rect: egui::Rect, kind: Kind) {
        self.restart_library(ctx);
        self.backgrounds.library.poll();
        let want = self
            .backgrounds
            .want(self.settings.background, self.settings.library_art);
        // Sign-in comes before any server answers, so it shows the theme's
        // own photograph rather than waiting on a fetch that cannot run.
        let showing = match kind {
            Kind::Sharp => &want.fallback,
            Kind::Soft => &want.showing,
        };
        self.backgrounds.shown = self.backgrounds.stage.paint(
            ctx,
            rect,
            kind,
            [showing, &want.fallback],
            &mut self.artwork,
        );
        // The rotation moves on with no input to wake the window.
        ctx.request_repaint_after(CLOCK_EVERY);
    }

    fn restart_library(&mut self, ctx: &egui::Context) {
        let Some(changed) = self.backgrounds.changed else {
            return;
        };
        let waited = changed.elapsed();
        if waited < SETTLE {
            ctx.request_repaint_after(SETTLE - waited);
            return;
        }
        self.backgrounds.changed = None;
        let sources = if self.settings.library_art {
            let plex = self
                .backgrounds
                .plex
                .clone()
                .filter(|_| !self.server.is_empty())
                .map(|client| (client, self.server.as_str()));
            theme_library::sources(plex, &self.backgrounds.jellyfin)
        } else {
            Vec::new()
        };
        self.backgrounds.library = ThemeLibrary::start(sources, self.backgrounds.dirs.clone());
    }

    /// Pin `item`'s best backdrop as the background, or take the pin off when
    /// it is this title's already.
    pub(super) fn toggle_pin(&mut self, item: &plex_core::library::Item) {
        let Some(title_key) = item.rating_key.clone() else {
            return;
        };
        if self.backgrounds.is_pinned(&title_key) {
            self.set_pin(None);
            return;
        }
        let Some(source) = theme_library::source_of(&title_key, self.backgrounds.plex_machine())
        else {
            return;
        };
        let Some(artwork) = self
            .backgrounds
            .best_art(&title_key, &source)
            .or_else(|| item.art.clone())
        else {
            return;
        };
        let servers = crate::source::Servers {
            plex: &self.server,
            jellyfin: &self.sources.jellyfin,
        };
        let pin = Pinned {
            source,
            title_key,
            artwork,
            title: item.title.clone().unwrap_or_default(),
            year: item.year.and_then(|year| u16::try_from(year).ok()),
            server: servers.of(item).1.to_owned(),
        };
        self.set_pin(Some(pin));
    }

    fn set_pin(&mut self, pin: Option<Pinned>) {
        if let Err(error) = self.backgrounds.library.set_pinned(pin) {
            log::warn!("the background choice was not saved: {error}");
        }
    }

    /// Settings' Background section: Automatic or a theme as tiles of its
    /// photograph, the credit of what shows now, the library switch, and the
    /// pin with its way off.
    pub(super) fn background_section(&mut self, ui: &mut egui::Ui) {
        let now = self.backgrounds.now();
        let seasonal = choice::theme_for(Background::Automatic, now, self.backgrounds.hemisphere);
        let chosen = self.settings.background;
        let pinned = self.backgrounds.pin().cloned();
        let credit = self.backgrounds.credit();
        super::app_settings::section_panel(ui, "Background", |ui| {
            ui.label(crate::ui::theme::secondary(
                "The photograph behind the glass. Automatic follows the season and the \
                 holidays where you live.",
            ));
            ui.add_space(crate::ui::theme::SPACING_BASE);
            let picked = tiles::show(
                ui,
                &mut self.backgrounds.stage,
                chosen,
                seasonal,
                pinned.is_some(),
            );
            if let Some(background) = picked {
                self.set_settings(|settings| settings.background = background);
                // A pin saved on a server not connected now goes too: the
                // theme is the user's newer choice.
                if self.backgrounds.library.pinned().is_some() {
                    self.set_pin(None);
                }
            }
            if let Some(credit) = credit.as_ref().filter(|_| pinned.is_none()) {
                ui.add_space(crate::ui::theme::SPACING_BASE);
                ui.label(
                    crate::ui::theme::secondary(format!("Showing {credit}"))
                        .size(crate::ui::theme::CAPTION_SIZE),
                );
            }
            if let Some(pin) = &pinned {
                ui.add_space(crate::ui::theme::SPACING_BASE);
                if tiles::pinned_row(ui, &pin.credit()) {
                    self.set_pin(None);
                }
            }
            let on = self.settings.library_art;
            if super::app_settings::setting_row(
                ui,
                "Use art from my libraries",
                "Themes show backdrops of well-loved films and shows from your servers",
                on,
            ) {
                self.set_settings(|settings| settings.library_art = !on);
                self.backgrounds.restart_now();
            }
        });
    }
}

#[path = "background/tiles.rs"]
mod tiles;
