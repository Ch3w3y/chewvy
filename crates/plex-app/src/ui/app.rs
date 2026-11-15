//! The app screens and what the rail selects inside them.
//!
//! [`Ui`] owns nothing that talks to Plex: it drains [`crate::watch::Ev`]
//! messages, keeps the visible state, and sends [`crate::watch::Cmd`] messages
//! back. The rail ([`crate::ui::rail`]) is the only navigation surface, so a
//! screen change is always a rail entry plus its place in [`History`].

use std::sync::mpsc::{Receiver, Sender};
use std::time::Instant;

use plex_core::library::{Hub, SortOption};

use crate::player::{self, PlayRequest};
use crate::source::{Library, Origin};
use crate::ui::player::Overlay;
use crate::ui::rail::{self, Entry};
use crate::ui::{grid, home, images, settings, theme};
use crate::watch::{Ask, Cmd, Ev};

/// The screens before a connected server, then the connected shell itself.
enum Screen {
    /// Reading what is on file, or signed in and reading the account.
    Starting,
    /// Nothing is on file: the choice between a Plex account and a Jellyfin
    /// server, and the Jellyfin sign-in when that is what was chosen.
    Welcome,
    Pairing {
        code: String,
        url: String,
    },
    /// Shown only when the account has several servers and none is remembered.
    Servers(Vec<String>),
    /// A connected server: [`Ui::current`] decides what the panel shows.
    Connected,
}

/// The rail entries the user navigated away from, newest last.
#[derive(Default)]
struct History {
    stack: Vec<Entry>,
}

impl History {
    fn leave(&mut self, from: &Entry) {
        if self.stack.last() != Some(from) {
            self.stack.push(from.clone());
        }
    }

    fn back(&mut self) -> Option<Entry> {
        self.stack.pop()
    }

    /// The rail entry a drill-in screen belongs to: the nearest entered screen
    /// the rail lists a row for, so a library keeps its highlight while one of
    /// its posters is open. Sources is skipped for the same reason it has no row.
    fn owning(&self) -> Option<&Entry> {
        self.stack
            .iter()
            .rev()
            .find(|entry| !matches!(entry, Entry::Detail { .. } | Entry::Sources))
    }
}

/// The row the Back button sits in, above a drilled-in page.
pub(crate) const BACK_ROW: f32 = 72.0;

/// Everything the app draws between the worker thread and the window.
pub struct Ui {
    cmd: Sender<Cmd>,
    ev: Receiver<Ev>,
    screen: Screen,
    status: String,
    error: Option<String>,
    sections: Vec<(String, String)>,
    /// The libraries every Jellyfin server listed, each carrying its server.
    jellyfin: Vec<Library>,
    /// What a Jellyfin server said when it listed no libraries.
    jellyfin_notes: Vec<String>,
    current: Option<Entry>,
    history: History,
    /// The posters the grid lists, in the order the server returned them.
    cards: Vec<grid::Card>,
    /// The container's `totalSize` for those cards, when its answer named one.
    card_total: Option<u64>,
    /// Another page may be asked for.
    card_more: bool,
    /// A page is already in flight, so the grid does not ask again.
    card_paging: bool,
    /// The offset the next page is asked at, so a page the server served
    /// shorter than asked for does not re-ask rows already in view.
    card_start: usize,
    /// The browsing the applied card answer was asked for. An answer whose
    /// [`Ask`] differs is stale and fills nothing.
    asked: Option<Ask>,
    /// A section whose first page waits on that section's own `/sorts` answer.
    awaiting_sorts: Option<String>,
    /// The Search screen: its asks, its debounce and its answer.
    search: app_search::SearchState,
    /// The grid card the arrow keys have selected.
    card_focus: Option<usize>,
    /// Ask the rail's search field for the keyboard focus on its next draw.
    focus_search: bool,
    /// The query the rail's search field last reported a change for, so the
    /// rail asks once per query rather than on every frame it holds text.
    rail_query: String,
    /// Whether one of the app's text fields holds the keyboard focus. The
    /// field sets it as it is drawn; the screen's card selection reads it in
    /// that frame and the Browse keys read it in the frame after.
    typing: bool,
    /// The connected server's own label, as its account resource named it.
    server: String,
    /// The account's servers, as Settings' server switch last listed them.
    server_choices: Vec<String>,
    /// What the Settings screen edits, read once at startup and saved on every
    /// change.
    settings: settings::Settings,
    /// The sort and watch-state filter the current cards were asked with.
    chips: grid::Chips,
    /// The sorts the section in view offered.
    sorts: Vec<SortOption>,
    /// The connected server's `/hubs` shelves, as last read for the Home entry;
    /// `None` until the server first answers.
    hubs: Option<Vec<Hub>>,
    /// The detail page in view: the item a card opened, as the worker read it.
    detail: Option<crate::ui::detail::Page>,
    query: String,
    /// The playing item's control overlay; `Some` while [`player`] plays.
    overlay: Option<Overlay>,
    /// When the playing screen last reported a position. `None` until the player
    /// has one to report, which is also how a newly opened item starts its
    /// reporting.
    progress_at: Option<Instant>,
    /// A resolved item waiting for the next frame to start it.
    pending_play: Option<Box<PlayRequest>>,
    /// The next episode is being resolved: the window stays on the black
    /// player screen instead of flashing the browser in between.
    handoff: Option<String>,
    /// Which window the next item plays in, and the HDR explainer's state.
    hdr: hdr::HdrState,
    /// Poster textures for the item rows, filled by the loader's own threads.
    artwork: images::Artwork,
    /// Every source this install can play from, as its own file keeps them.
    sources: crate::sources::Sources,
    /// The panel adding a Jellyfin server, while one is open.
    add: Option<crate::add_source::AddFlow>,
    /// The Sources screen is asking which kind of source to add.
    choosing: bool,
    /// Watching together: the group, its follower, panel and notices.
    together: together::Together,
    /// In-app updates: what the last check found and the install under way.
    updater: crate::update::Updater,
    /// The update notice was put away for this run.
    update_dismissed: bool,
    /// The update prompt is open.
    update_prompt: bool,
    /// The photograph behind the glass and the library art it may show.
    backgrounds: background::Backgrounds,
}

impl Ui {
    pub(crate) fn new(cmd: Sender<Cmd>, ev: Receiver<Ev>) -> Self {
        Self {
            cmd,
            ev,
            screen: Screen::Starting,
            status: "Starting…".into(),
            error: None,
            sections: Vec::new(),
            jellyfin: Vec::new(),
            jellyfin_notes: Vec::new(),
            current: None,
            history: History::default(),
            cards: Vec::new(),
            card_total: None,
            card_more: false,
            card_paging: false,
            card_start: 0,
            asked: None,
            awaiting_sorts: None,
            search: app_search::SearchState::default(),
            card_focus: None,
            focus_search: false,
            rail_query: String::new(),
            typing: false,
            server: String::new(),
            server_choices: Vec::new(),
            settings: settings::load(),
            chips: grid::Chips::default(),
            sorts: Vec::new(),
            hubs: None,
            detail: None,
            query: String::new(),
            overlay: None,
            pending_play: None,
            handoff: None,
            progress_at: None,
            hdr: hdr::HdrState::default(),
            artwork: images::Artwork::default(),
            sources: crate::sources::load(),
            add: None,
            choosing: false,
            together: together::Together::default(),
            updater: crate::update::Updater::default(),
            update_dismissed: false,
            update_prompt: false,
            backgrounds: background::Backgrounds::new(),
        }
    }

    fn send(&self, cmd: Cmd) {
        let _ = self.cmd.send(cmd);
    }

    fn apply(&mut self, ev: Ev) {
        match ev {
            Ev::Status(s) => {
                self.status = s;
                self.error = None;
            }
            Ev::Error(e) => {
                // The ask that failed has finished, so nothing is in flight:
                // without this the grid never asks for that page again.
                self.error = Some(e);
                self.card_paging = false;
                self.handoff = None;
            }
            Ev::Pairing { code, url } => {
                self.error = None;
                self.status = "Waiting for you to enter the code at plex.tv/link…".into();
                self.screen = Screen::Pairing { code, url };
            }
            // Asked from Settings while connected: the list fills its switcher
            // instead of leaving the library for the picker.
            Ev::Servers(names) if matches!(self.screen, Screen::Connected) => {
                self.server_choices = names;
            }
            Ev::Servers(names) => {
                self.status = if names.is_empty() {
                    "No servers found on this account.".into()
                } else {
                    "Choose a server.".into()
                };
                self.screen = Screen::Servers(names);
            }
            Ev::Sections { server, sections } => {
                self.error = None;
                self.server = server;
                self.backgrounds.servers_changed();
                self.sections = sections;
                self.history = History::default();
                self.current = None;
                self.screen = Screen::Connected;
                // The switcher lists the account's other servers, so they are
                // asked for once here rather than on its first click.
                self.send(Cmd::Servers);
                // A connected server opens on the rail's Home entry; browsing a
                // library stays the user's click, as it was before the rail.
                self.go(Entry::Home);
            }
            Ev::Artwork(client) => {
                self.backgrounds
                    .set_plex(Some(std::sync::Arc::clone(&client)));
                self.artwork.attach(client);
            }
            Ev::Jellyfin {
                pool,
                libraries,
                notes,
            } => {
                // The loader reads artwork from these servers too, so it holds the
                // same cheap handle the screens browse with.
                self.backgrounds.set_jellyfin(pool.clone());
                self.artwork.attach_jellyfin(pool);
                self.jellyfin = libraries;
                self.jellyfin_notes = notes;
            }
            Ev::Hubs(hubs) => {
                self.error = None;
                self.status = String::new();
                self.hubs = Some(hubs);
            }
            Ev::SearchHubs { query, hubs } => self.apply_search(&query, hubs),
            Ev::SearchFailed { query, error } => self.search_failed(&query, error),
            Ev::Cards {
                cards,
                append,
                more,
                total,
                asked,
            } => self.apply_cards(cards, append, more, total, asked),
            Ev::Sorts { section, options } => self.apply_sorts(&section, options),
            Ev::Detail(page) => {
                self.error = None;
                self.status = String::new();
                self.detail = Some(*page);
            }
            Ev::Episodes { season_key, items } => self.fill_episodes(&season_key, &items),
            Ev::PlayReady(request) => self.pending_play = Some(request),
            // The registry is the worker's, read from its file; the panel only
            // hears which step answered.
            Ev::Sources(sources) => {
                self.sources = sources;
                // The last source was removed from inside the shell: there is
                // nothing left to browse, so the choice comes back.
                let browsing = matches!(self.screen, Screen::Connected);
                if browsing && !self.sources.has_sign_in() {
                    self.open_without_plex();
                }
            }
            Ev::Jf(answer) => self.apply_jellyfin(answer),
            Ev::Together(news) => self.apply_together(news),
            Ev::SignedOut | Ev::NoPlex => {
                self.backgrounds.set_plex(None);
                self.open_without_plex();
            }
        }
    }

    /// Open one poster: its detail page, which is where Play / Resume starts
    /// playback and where a show or season lists what is one level down.
    fn open_card(&mut self, card: Option<grid::Card>) {
        let Some(card) = card else { return };
        let Some(id) = card.rating_key else {
            return;
        };
        let title = card.title;
        self.go(Entry::Detail { id, title });
    }

    /// The rail's libraries: the connected Plex server's sections first, then
    /// every Jellyfin library, each carrying the source and server it belongs to.
    #[must_use]
    fn libraries(&self) -> Vec<Library> {
        let mut rows: Vec<Library> = self
            .sections
            .iter()
            .map(|(id, title)| Library {
                origin: Origin::Plex,
                server: self.server.clone(),
                id: id.clone(),
                title: title.clone(),
            })
            .collect();
        rows.extend(self.jellyfin.iter().cloned());
        rows
    }

    /// Move to a rail entry, remembering where the user was.
    fn go(&mut self, to: Entry) {
        self.leave_search(&to);
        if let Some(from) = self.current.clone() {
            self.history.leave(&from);
        }
        self.current = Some(to.clone());
        self.card_focus = None;
        self.search.focus = None;
        self.status = String::new();
        self.card_paging = false;
        self.card_more = false;
        self.card_total = None;
        self.card_start = 0;
        self.asked = None;
        self.awaiting_sorts = None;
        match to {
            Entry::Home => {
                self.status = "Loading shelves…".into();
                self.send(Cmd::Home);
            }
            Entry::Section { id, .. } => {
                self.status = "Loading…".into();
                self.cards.clear();
                // Each library starts with its own default sort and no watch
                // filter: a sort key or an unwatched filter from the library the
                // user left is not this one's to answer with.
                self.chips = grid::Chips::default();
                self.sorts = Vec::new();
                // Its posters are asked once, after that section names its own
                // sorts, so the first page is already in an order it offers.
                self.awaiting_sorts = Some(id.clone());
                self.send(Cmd::Sorts { section: id });
            }
            Entry::Detail { id, .. } => {
                self.detail = None;
                self.status = "Loading…".into();
                self.send(Cmd::Detail { rating_key: id });
            }
            Entry::Search => {
                // The field asks on its own debounce, so re-entering the screen
                // re-asks for the query it still holds instead of showing an
                // answer filed for a screen the user left. The rail's field is
                // the only way here, so it keeps the keyboard focus it already
                // has rather than being asked for it again.
                self.search.restart();
            }
            Entry::Settings => {}
            // The registry is already the worker's, read before this screen was
            // reachable, so the screen opens on what is there.
            Entry::Sources => {}
        }
    }

    pub(crate) fn draw(&mut self, root: &mut egui::Ui) {
        while let Ok(ev) = self.ev.try_recv() {
            self.apply(ev);
        }
        // Background threads wake the window when they have something; no
        // timer redraws an idle screen.
        crate::watch::register_window(root.ctx());
        self.artwork.drain(root.ctx());
        // The window's own chrome: drag strip and resize edges under every
        // panel, the buttons over them (during playback only near the top).
        crate::ui::chrome::behind(root);
        let near_top = root
            .ctx()
            .input(|i| i.pointer.hover_pos().is_some_and(|p| p.y < 96.0));
        // A fullscreen player has its own way out (its button, F, Esc), so the
        // window buttons stay off the picture there.
        let fullscreen = crate::ui::player::is_fullscreen(root.ctx());
        crate::ui::chrome::controls(
            root.ctx(),
            !player::is_active() || (near_top && !fullscreen),
        );
        // F11 is fullscreen on every screen; the player also answers F.
        if root.ctx().input(|i| i.key_pressed(egui::Key::F11)) {
            crate::ui::player::toggle_fullscreen(root.ctx());
        }

        // These keys are read before any field of this frame is drawn, so they
        // are judged against the focus the frame before reported.
        let typed_before = std::mem::take(&mut self.typing);

        if let Some(request) = self.pending_play.take() {
            self.begin_playback(*request, root.ctx());
        }
        self.update_at_start();
        self.together_frame(root.ctx());
        // A playing item owns the window: no rail, no status strip, only the
        // video behind and its controls on top.
        if player::is_active() {
            self.draw_playback(root);
            return;
        }
        if let Some(label) = &self.handoff {
            playback::handoff_screen(root, label);
            return;
        }
        // Browsing sits on the blurred, dimmed copy of the backdrop; the hero
        // art and every page run full-bleed under the window buttons.
        if matches!(self.screen, Screen::Connected) {
            let screen = root.ctx().content_rect();
            self.paint_background(root.ctx(), screen, crate::ui::backdrop::Kind::Soft);
        }

        // The browsing keyboard is read before its screens draw, so Back and
        // Search do not depend on what egui happened to lay out.
        // The update prompt takes Escape for itself.
        if !self.update_prompt_open() {
            self.browse_keys(root.ctx(), typed_before);
        }

        if matches!(self.screen, Screen::Connected) {
            self.status_toast(root.ctx());
        }

        match &self.screen {
            Screen::Starting | Screen::Welcome | Screen::Pairing { .. } | Screen::Servers(_) => {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(root, |ui| self.sign_in(ui));
            }
            Screen::Connected => {
                let libraries = self.libraries();
                let entries = rail::entries(&libraries);
                let rooms = self.room_count();
                let held: Vec<rail::SwitcherServer> = self
                    .sources
                    .jellyfin
                    .iter()
                    .map(|server| rail::SwitcherServer {
                        id: server.server_id.clone(),
                        name: server.name.clone(),
                        answering: self.jellyfin.iter().any(|library| {
                            library.origin.server_id() == Some(server.server_id.as_str())
                        }),
                    })
                    .collect();
                let switcher = rail::Switcher {
                    plex: &self.server_choices,
                    connected: &self.server,
                    jellyfin: &held,
                };
                let mut field = rail::Field {
                    query: &mut self.query,
                    reported: &mut self.rail_query,
                    focus: &mut self.focus_search,
                    typed: false,
                };
                let selection = rail::show(
                    root,
                    &entries,
                    self.current.as_ref(),
                    match &self.current {
                        Some(Entry::Detail { .. } | Entry::Sources) => self.history.owning(),
                        _ => self.current.as_ref(),
                    },
                    &libraries,
                    &self.jellyfin_notes,
                    &switcher,
                    &mut field,
                    rooms,
                );
                // The field reports its own focus, which is what gates this
                // frame's card selection and the next frame's Browse keys.
                self.typing |= field.typed;
                match selection {
                    Some(rail::Choice::Go(entry)) => self.go(entry),
                    Some(rail::Choice::Rooms(row)) => {
                        self.open_rooms(together::Subject::Rooms, row);
                    }
                    Some(rail::Choice::Connect(index)) => {
                        if let Some(name) = self.server_choices.get(index) {
                            self.status = format!("Connecting to {name}…");
                        }
                        self.send(Cmd::Connect(index));
                    }
                    Some(rail::Choice::Server(id)) => {
                        let first = libraries
                            .iter()
                            .find(|library| library.origin.server_id() == Some(id.as_str()));
                        if let Some(library) = first {
                            self.go(Entry::Section {
                                id: library.id.clone(),
                                title: library.title.clone(),
                            });
                        }
                    }
                    Some(rail::Choice::AddSource) => self.add_source(),
                    Some(rail::Choice::Search { query, submitted }) => {
                        self.rail_search(&query, submitted);
                    }
                    None => {}
                }
                // The page gutters: content starts a gutter in from the rail
                // and stops a gutter short of the window's right edge, so the
                // last grid column never touches it. Home's hero runs up to the
                // window's top edge, and a detail page's art does too under its Back
                // row; every other page clears the window buttons.
                let top = if matches!(
                    self.current,
                    Some(Entry::Home | Entry::Detail { .. } | Entry::Sources)
                ) {
                    0
                } else {
                    theme::PAGE_TOP as i8
                };
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE.inner_margin(egui::Margin {
                        left: theme::PAGE_GUTTER as i8,
                        right: theme::PAGE_GUTTER as i8,
                        top,
                        bottom: 0,
                    }))
                    .show(root, |ui| self.center(ui));
            }
        }
        // Every screen but the player can offer an update, sign-in included.
        self.update_overlays(root.ctx());
        crate::ui::controls::ring_the_rest(root.ctx());
    }

    /// The central panel for the selected rail entry.
    fn center(&mut self, ui: &mut egui::Ui) {
        // A top-level screen is where the rail already is, so Back there would
        // promise nothing: it is drawn only on a screen reached by a drill-in.
        if matches!(self.current, Some(Entry::Detail { .. } | Entry::Sources)) {
            self.back_button(ui);
        }
        match self.current.clone() {
            Some(Entry::Home) => {
                // A card opens an item only while nothing else is playing, the
                // same gate the item list's ▶ Play button uses.
                let idle = !player::is_active();
                // Until the server first answers, the status strip says Home
                // is loading, and an empty state would only flash.
                if self.hubs.is_none() && self.error.is_none() {
                    return;
                }
                // The shelves are taken out so a card can ask the artwork
                // loader while the drawing loop still borrows them.
                let hubs = self.hubs.take();
                let servers = crate::source::Servers {
                    plex: &self.server,
                    jellyfin: &self.sources.jellyfin,
                };
                let shown = hubs.as_deref().unwrap_or_default();
                let pick = home::show(ui, shown, &mut self.artwork, idle, servers);
                self.hubs = hubs;
                match pick {
                    Some(home::Pick::Open(page)) => self.go(Entry::Detail {
                        id: page.rating_key,
                        title: page.title,
                    }),
                    Some(home::Pick::Play(resume)) => {
                        self.start_title(resume.rating_key, resume.title, resume.start, ui.ctx());
                    }
                    Some(home::Pick::Sources) => self.go(Entry::Sources),
                    None => {}
                }
            }
            Some(Entry::Settings) => self.draw_settings(ui),
            Some(Entry::Sources) => self.draw_sources(ui),
            Some(Entry::Section { id, title }) => self.grid_center(ui, title, id),
            Some(Entry::Detail { .. }) => self.draw_detail(ui),
            Some(Entry::Search) => self.draw_search(ui),
            None => {
                ui.vertical_centered(|ui| {
                    ui.add_space(theme::PAGE_TOP);
                    crate::ui::controls::working(ui, &self.status);
                });
            }
        }
    }

    /// The keys the browsing screens answer with: Backspace, Esc and Alt+Left
    /// step back through the history the Back button uses; Ctrl+K and `/` give
    /// the keyboard to the rail's field. While one of the app's fields held the
    /// focus, Backspace, `/` and Ctrl+K belong to that field and only Alt+Left
    /// steps back; Escape always steps back, and egui drops the field's focus
    /// with it.
    fn browse_keys(&mut self, root: &egui::Context, typed_before: bool) {
        let (alt_left, escape, backspace, to_search) = root.input(|i| {
            (
                i.modifiers.alt && i.key_pressed(egui::Key::ArrowLeft),
                i.key_pressed(egui::Key::Escape),
                i.key_pressed(egui::Key::Backspace),
                i.key_pressed(egui::Key::Slash)
                    || (i.modifiers.command && i.key_pressed(egui::Key::K)),
            )
        });
        if alt_left || escape || (!typed_before && backspace) {
            if let Some(entry) = self.history.back() {
                self.go(entry);
            }
            return;
        }
        if !typed_before && to_search {
            self.focus_search = true;
            if !matches!(self.current, Some(Entry::Search)) {
                self.go(Entry::Search);
            }
        }
    }

    /// The Back affordance, in its own [`BACK_ROW`] so the page under it
    /// starts at the same height whether or not there is anywhere to go back.
    fn back_button(&mut self, ui: &mut egui::Ui) {
        let (row, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), BACK_ROW),
            egui::Sense::hover(),
        );
        if self.history.stack.is_empty() {
            return;
        }
        let origin = egui::pos2(
            row.min.x,
            row.center().y - crate::ui::overlay::BUTTON[0] / 2.0,
        );
        let response = crate::ui::overlay::secondary(
            ui,
            origin,
            ui.id().with("back"),
            Some(crate::ui::marks::icon::Icon::Back),
            "Back",
            true,
        )
        .on_hover_text("Back to the previous screen");
        if response.clicked() {
            if let Some(entry) = self.history.back() {
                self.go(entry);
            }
        }
    }
}

#[path = "app/background.rs"]
mod background;
#[path = "app/browse.rs"]
mod browse;
#[path = "app/hdr.rs"]
mod hdr;
#[path = "app/playback.rs"]
mod playback;
#[path = "app/sign_in.rs"]
mod sign_in;

#[path = "app/detail.rs"]
mod detail;

#[path = "app/search.rs"]
mod app_search;

#[path = "app/settings.rs"]
mod app_settings;

#[path = "app/sources.rs"]
mod app_sources;

#[path = "app/together.rs"]
mod together;

#[path = "app/rooms.rs"]
mod rooms;

#[path = "app/update.rs"]
mod app_update;

#[cfg(test)]
#[path = "app/room_tests.rs"]
mod room_tests;
#[cfg(test)]
#[path = "app/tests.rs"]
mod tests;
