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
use crate::ui::{grid, home, images, search, settings, theme};
use crate::watch::{Ask, Cmd, Ev};

/// The screens before a connected server, then the connected shell itself.
enum Screen {
    /// Signed out, or signed in and reading the account.
    Starting,
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
    /// The search query the last ask was made for, kept so an answer for a
    /// query the field no longer holds fills nothing.
    search_asked: Option<String>,
    /// The trimmed query the search field holds that has not been asked for.
    search_pending: Option<String>,
    /// When [`Ui::search_pending`] last changed, the start of its debounce.
    search_at: Option<Instant>,
    /// The search answer, kept as the server grouped it.
    search_groups: Vec<search::Group>,
    /// The search card the arrow keys have selected.
    search_focus: Option<search::Selection>,
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
    /// The connected server's `/hubs` shelves, as last read for the Home entry.
    hubs: Vec<Hub>,
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
            search_asked: None,
            search_pending: None,
            search_at: None,
            search_groups: Vec::new(),
            search_focus: None,
            card_focus: None,
            focus_search: false,
            rail_query: String::new(),
            typing: false,
            server: String::new(),
            server_choices: Vec::new(),
            settings: settings::load(),
            chips: grid::Chips::default(),
            sorts: Vec::new(),
            hubs: Vec::new(),
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
                self.sections = sections;
                self.history = History::default();
                self.current = None;
                self.screen = Screen::Connected;
                // A connected server opens on the rail's Home entry; browsing a
                // library stays the user's click, as it was before the rail.
                self.go(Entry::Home);
            }
            Ev::Artwork(client) => self.artwork.attach(client),
            Ev::Jellyfin {
                pool,
                libraries,
                notes,
            } => {
                // The loader reads artwork from these servers too, so it holds the
                // same cheap handle the screens browse with.
                self.artwork.attach_jellyfin(pool);
                self.jellyfin = libraries;
                self.jellyfin_notes = notes;
            }
            Ev::Hubs(hubs) => {
                self.error = None;
                self.status = String::new();
                self.hubs = hubs;
            }
            Ev::SearchHubs { query, hubs } => self.apply_search(&query, hubs),
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
            Ev::Sources(sources) => self.sources = sources,
            Ev::Jf(answer) => self.apply_jellyfin(answer),
            Ev::SignedOut => {
                self.sections.clear();
                self.current = None;
                self.history = History::default();
                self.screen = Screen::Starting;
            }
        }
    }

    /// One poster page, filed under the browsing that asked for it: a section
    /// page fills [`Ui::cards`]. An answer whose [`Ask`] is not the last one
    /// this screen sent is stale, so a slow page cannot land in the list of a
    /// screen that never asked for it.
    fn apply_cards(
        &mut self,
        cards: Vec<grid::Card>,
        append: bool,
        more: bool,
        total: Option<u64>,
        asked: Ask,
    ) {
        if self.asked.as_ref() != Some(&asked) {
            return;
        }
        self.error = None;
        self.status = String::new();
        self.card_paging = false;
        self.card_more = more;
        self.card_total = total;
        match asked {
            Ask::Section { start, .. } => {
                // The next page continues after the rows this answer filled,
                // which is not `cards.len()` when the server served a shorter
                // window than [`grid::PAGE`].
                self.card_start = start + cards.len();
                if append {
                    self.cards.extend(cards);
                } else {
                    self.cards = cards;
                }
            }
        }
    }

    /// A section's own sorts: the sort in effect becomes the default this
    /// section names, then its first page is asked for once, in that order. An
    /// answer for a section the user has left fills nothing, and a section never
    /// sees another library's sort keys.
    fn apply_sorts(&mut self, section: &str, options: Vec<SortOption>) {
        if self.awaiting_sorts.as_deref() != Some(section) {
            return;
        }
        self.awaiting_sorts = None;
        self.sorts = options;
        self.chips.sort = grid::default_sort(&self.sorts);
        let sort = self.chips.sort.clone();
        self.browse(section, 0, sort);
    }

    /// Ask the worker for one page of a section's posters and remember exactly
    /// that ask, so only its answer fills the grid.
    fn browse(&mut self, section: &str, start: usize, sort: Option<String>) {
        let unwatched = self.chips.unwatched;
        self.asked = Some(Ask::Section {
            section: section.to_owned(),
            start,
            sort: sort.clone(),
            unwatched,
        });
        self.send(Cmd::Browse {
            section: section.to_owned(),
            start,
            sort,
            unwatched,
        });
    }

    /// What the grid answered with: which page to re-ask, or which card to open.
    fn apply_grid(&mut self, action: grid::Action, section: String) {
        match action {
            grid::Action::Open(index) => {
                let card = self.cards.get(index);
                self.open_card(card.cloned());
            }
            grid::Action::Sort(value) => {
                self.chips.sort = value;
                self.reload(&section);
            }
            grid::Action::Unwatched(on) => {
                self.chips.unwatched = on;
                self.reload(&section);
            }
            grid::Action::Next => {
                self.card_paging = true;
                let start = self.card_start;
                let sort = self.chips.sort.clone();
                self.browse(&section, start, sort);
            }
            grid::Action::Select(selected) => self.card_focus = selected,
        }
    }

    /// Ask for the same browsing again from the first poster.
    fn reload(&mut self, section: &str) {
        self.card_paging = true;
        self.cards.clear();
        self.card_more = false;
        self.card_start = 0;
        let sort = self.chips.sort.clone();
        self.browse(section, 0, sort);
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

    /// Move to a rail entry, remembering where the user was.
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

    fn go(&mut self, to: Entry) {
        if let Some(from) = self.current.clone() {
            self.history.leave(&from);
        }
        self.current = Some(to.clone());
        self.card_focus = None;
        self.search_focus = None;
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
                self.search_asked = None;
                self.search_pending = None;
                self.search_at = None;
                self.search_groups.clear();
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
        crate::ui::chrome::controls(root.ctx(), !player::is_active() || near_top);

        // These keys are read before any field of this frame is drawn, so they
        // are judged against the focus the frame before reported.
        let typed_before = std::mem::take(&mut self.typing);

        if let Some(request) = self.pending_play.take() {
            self.begin_playback(*request, root.ctx());
        }
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
            crate::ui::backdrop::paint(root.ctx(), screen, crate::ui::backdrop::Kind::Soft, 0.25);
        }

        // The browsing keyboard is read before its screens draw, so Back and
        // Search do not depend on what egui happened to lay out.
        self.browse_keys(root.ctx(), typed_before);

        // The strip is a message, not chrome: with nothing to say it is left off
        // entirely so the screen gets the height back.
        let message = self
            .error
            .as_ref()
            .map(|error| (true, error.clone()))
            .or_else(|| (!self.status.trim().is_empty()).then(|| (false, self.status.clone())));
        if let Some((failed, text)) = message.filter(|_| matches!(self.screen, Screen::Connected)) {
            let color = if failed {
                theme::DANGER
            } else {
                theme::SECONDARY
            };
            egui::Panel::bottom("status").show(root, |ui| {
                ui.colored_label(color, text);
            });
        }

        match &self.screen {
            Screen::Starting | Screen::Pairing { .. } | Screen::Servers(_) => {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(root, |ui| self.sign_in(ui));
            }
            Screen::Connected => {
                crate::ui::backdrop::release_sharp();
                let libraries = self.libraries();
                let entries = rail::entries(&libraries);
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
                    &mut field,
                );
                // The field reports its own focus, which is what gates this
                // frame's card selection and the next frame's Browse keys.
                self.typing |= field.typed;
                match selection {
                    Some(rail::Choice::Go(entry)) => self.go(entry),
                    Some(rail::Choice::Search { query, submitted }) => {
                        if submitted {
                            self.ask_search(&query);
                        } else if !matches!(self.current, Some(Entry::Search)) {
                            self.go(Entry::Search);
                        }
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
                // The shelves are taken out so a card can ask the artwork
                // loader while the drawing loop still borrows them.
                let hubs = std::mem::take(&mut self.hubs);
                let resume = home::show(ui, &hubs, &mut self.artwork, idle, &self.server);
                self.hubs = hubs;
                if let Some(resume) = resume.as_ref().filter(|r| r.overview) {
                    let to = Entry::Detail {
                        id: resume.rating_key.clone(),
                        title: resume.title.clone(),
                    };
                    self.go(to);
                } else if let Some(resume) = resume {
                    self.status = format!("Opening “{}”…", resume.title);
                    self.send(Cmd::Play {
                        rating_key: resume.rating_key,
                        title: resume.title,
                        start: resume.start,
                        quality: None,
                        wakeup: ui.ctx().clone(),
                    });
                }
            }
            Some(Entry::Settings) => self.draw_settings(ui),
            Some(Entry::Sources) => self.draw_sources(ui),
            Some(Entry::Section { id, title }) => self.grid_center(ui, title, id),
            Some(Entry::Detail { .. }) => self.draw_detail(ui),
            Some(Entry::Search) => self.draw_search(ui),
            None => {
                ui.heading("Chewvy");
                ui.label(&self.status);
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
            Some(crate::ui::marks::icon::BACK),
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

    /// The library browsing screen: the server's own posters in the responsive
    /// grid, under its title and `totalSize`, with a next page and its chips
    /// belonging to `section`.
    fn grid_center(&mut self, ui: &mut egui::Ui, title: String, section: String) {
        // The cards are taken out so a poster can ask the artwork loader while
        // the grid still borrows them.
        let cards = std::mem::take(&mut self.cards);
        let view = grid::View {
            cards: &cards,
            title,
            total: self.card_total,
            more: self.card_more,
            chips: self.chips.clone(),
            sorts: &self.sorts,
            paging: self.card_paging,
            size: self.settings.poster_size,
            focus: self.card_focus,
            navigate: true,
            typing: self.typing,
        };
        let action = grid::show(ui, view, &mut self.artwork, player::is_active());
        self.cards = cards;
        if let Some(action) = action {
            self.apply_grid(action, section);
        }
    }

    /// The sign-in screens: the space backdrop, and a glass card that says
    /// what the app is doing (an orbit while it works), shows the pairing code,
    /// or lists the account's servers.
    fn sign_in(&mut self, ui: &mut egui::Ui) {
        use crate::ui::splash;
        let area = ui.max_rect();
        crate::ui::backdrop::paint(
            ui.ctx(),
            ui.ctx().content_rect(),
            crate::ui::backdrop::Kind::Sharp,
            0.0,
        );
        let screen = match &self.screen {
            Screen::Pairing { code, url } => Screen::Pairing {
                code: code.clone(),
                url: url.clone(),
            },
            Screen::Servers(names) => Screen::Servers(names.clone()),
            _ => Screen::Starting,
        };
        let error = self.error.clone();
        let status = self.status.clone();
        splash::card(ui, area, |ui| match &screen {
            Screen::Pairing { code, url } => {
                splash::heading(ui, "Link this device", "Sign in with your Plex account");
                ui.label(
                    theme::secondary("Go to plex.tv/link and enter this code").font(theme::body()),
                );
                ui.add_space(16.0);
                splash::code_tiles(ui, code);
                ui.add_space(24.0);
                if splash::primary(ui, "Open plex.tv/link") {
                    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
                }
                ui.add_space(8.0);
                if splash::secondary(ui, "Get a new code") {
                    self.send(Cmd::Pair);
                }
                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(
                        theme::secondary("Waiting for the code to be entered").font(theme::meta()),
                    );
                });
            }
            Screen::Servers(names) => {
                let subtitle = if names.is_empty() {
                    "No servers found on this account"
                } else {
                    "Choose a server to browse"
                };
                splash::heading(ui, "Your servers", subtitle);
                for (index, name) in names.iter().enumerate() {
                    if splash::server_row(ui, name) {
                        self.send(Cmd::Connect(index));
                    }
                }
            }
            _ => {
                splash::heading(ui, "Chewvy", "Your libraries, beautifully");
                splash::orbit(ui);
                ui.add_space(16.0);
                let line = if status.trim().is_empty() {
                    "Starting…".to_owned()
                } else {
                    status.clone()
                };
                ui.label(theme::secondary(line).font(theme::body()));
            }
        });
        splash::not_affiliated(ui, area);
        if let Some(error) = error {
            let strip = egui::Rect::from_center_size(
                egui::pos2(area.center().x, area.max.y - 48.0),
                egui::vec2(area.width().min(640.0), 40.0),
            );
            crate::ui::glass::panel(
                ui.painter(),
                strip,
                20.0,
                crate::ui::glass_tints::STATUS_STRIP,
            );
            ui.painter().text(
                strip.center(),
                egui::Align2::CENTER_CENTER,
                error,
                theme::body(),
                theme::TEXT,
            );
        }
    }
}

#[path = "app/hdr.rs"]
mod hdr;
#[path = "app/playback.rs"]
mod playback;

#[path = "app/detail.rs"]
mod detail;

#[path = "app/search.rs"]
mod app_search;

#[path = "app/settings.rs"]
mod app_settings;

#[path = "app/sources.rs"]
mod app_sources;

#[cfg(test)]
#[path = "app/tests.rs"]
mod tests;
