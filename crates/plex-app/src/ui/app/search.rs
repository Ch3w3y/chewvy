//! The Search entry: the field, its debounce, and the answer in the server's
//! own groups.
//!
//! The field asks [`crate::watch::Cmd::SearchHubs`] once a query has been still
//! for [`DEBOUNCE`], and an answer is filed only for the query the field still
//! holds. Every result opens its own overview page, so a search never starts
//! playback: playback is what the overview page's own Play asks for.

use std::time::{Duration, Instant};

use egui::{Align, Response, ScrollArea, Sense, Vec2};

use plex_core::library::Hub;

use crate::player;
use crate::ui::marks::icon::{self, Icon, Size};
use crate::ui::search::{self, Selection};
use crate::ui::{cards, controls, grid, rail, theme};
use crate::watch::Cmd;

use super::Ui;

/// How long the field waits after the last keystroke before asking the server.
const DEBOUNCE: Duration = Duration::from_millis(300);
/// A people circle's diameter: a credited name has no poster, so it is initials
/// rather than artwork the search answer names only as an absolute URL.
const CIRCLE: f32 = 76.0;
/// The height the name line under a people circle is allotted, so every circle
/// in a row hangs from the same line.
const CAPTION: f32 = 34.0;
/// The name's own width: two circles' diameters, so a long credited name never
/// pushes its neighbours apart.
const CAPTION_WIDTH: f32 = CIRCLE * 2.0;
/// The round plate under an empty state's search mark.
const PLATE: f32 = 56.0;

/// What the screen holds under its heading: nothing yet, or the answer to one
/// query, which names that query so it is never shown as another's.
#[derive(Default)]
enum Answer {
    #[default]
    None,
    /// The server's groups for `query`; empty when nothing matched.
    Found {
        query: String,
        groups: Vec<search::Group>,
    },
    /// Why `query` could not be answered.
    Failed { query: String, error: String },
}

impl Answer {
    fn query(&self) -> Option<&str> {
        match self {
            Self::None => None,
            Self::Found { query, .. } | Self::Failed { query, .. } => Some(query),
        }
    }

    fn groups(&self) -> &[search::Group] {
        match self {
            Self::Found { groups, .. } => groups,
            Self::None | Self::Failed { .. } => &[],
        }
    }
}

/// The Search screen's state between frames.
#[derive(Default)]
pub(super) struct SearchState {
    /// The query the last ask was made for, kept so an answer for a query the
    /// field no longer holds fills nothing.
    asked: Option<String>,
    /// The trimmed query the field holds that is waiting out its debounce.
    pending: Option<String>,
    /// When [`SearchState::pending`] last changed, the start of its debounce.
    at: Option<Instant>,
    answer: Answer,
    /// The search card the arrow keys have selected.
    pub(super) focus: Option<Selection>,
}

impl SearchState {
    /// Forget the last ask and its answer, so the query the field holds is
    /// asked for again.
    pub(super) fn restart(&mut self) {
        self.asked = None;
        self.pending = None;
        self.at = None;
        self.answer = Answer::None;
    }

    /// Whether a query is waiting out its debounce or for its answer.
    #[cfg(test)]
    pub(super) fn waiting(&self) -> bool {
        self.pending.is_some()
    }
}

/// What the page shows for the field's `query`.
#[derive(Debug, PartialEq, Eq)]
enum Shown<'a> {
    /// The field is empty.
    Prompt,
    /// Nothing to show yet for `query`.
    Searching,
    /// `query` matched nothing.
    Nothing,
    /// `query` could not be answered, and why.
    Failed(&'a str),
    /// Groups for `answered`, which is `query` itself unless a newer query is
    /// still on its way.
    Results { answered: &'a str, newer: bool },
}

fn shown<'a>(query: &str, answer: &'a Answer) -> Shown<'a> {
    if query.is_empty() {
        return Shown::Prompt;
    }
    let current = answer.query() == Some(query);
    match answer {
        Answer::Found {
            query: answered,
            groups,
        } if !groups.is_empty() => Shown::Results {
            answered,
            newer: !current,
        },
        _ if !current => Shown::Searching,
        Answer::Failed { error, .. } => Shown::Failed(error),
        Answer::None | Answer::Found { .. } => Shown::Nothing,
    }
}

impl Ui {
    /// Draw the Search entry: the field, then one section per search hub that
    /// answered with rows, each section laid out in its own fixed columns.
    ///
    /// The rail's field is the one search field: it is on screen everywhere,
    /// already holds what was typed, and keeps the keyboard focus while the
    /// answer arrives. A second field here would split what was typed and
    /// where the caret is, so the page names the query as its heading instead.
    pub(super) fn draw_search(&mut self, ui: &mut egui::Ui) {
        crate::ui::legible::label(ui, egui::RichText::new("Search").heading());
        self.search_debounce(ui);
        let query = self.query.trim().to_owned();
        let answer = std::mem::take(&mut self.search.answer);
        match shown(&query, &answer) {
            Shown::Prompt => search_state(
                ui,
                "Search your libraries",
                "Type in the search field in the sidebar: titles, shows, episodes and people.",
                false,
            ),
            Shown::Searching => search_state(
                ui,
                &format!("Searching for \u{201c}{query}\u{201d}"),
                "",
                true,
            ),
            Shown::Nothing => search_state(
                ui,
                &format!("Nothing matches \u{201c}{query}\u{201d}"),
                "Check the spelling, or try a title, a show or a person\u{2019}s name.",
                false,
            ),
            Shown::Failed(error) => search_state(
                ui,
                &format!("Could not search for \u{201c}{query}\u{201d}"),
                error,
                false,
            ),
            Shown::Results { answered, newer } => {
                ui.horizontal(|ui| {
                    crate::ui::legible::label(
                        ui,
                        theme::secondary(format!("Results for \u{201c}{answered}\u{201d}")),
                    );
                    if newer {
                        controls::spinner(ui);
                    }
                });
                self.search_rows(ui, answer.groups());
            }
        }
        self.search.answer = answer;
    }

    /// One section per hub that has rows, each section's cards laid out in
    /// [`grid`] columns at the Settings poster size.
    ///
    /// A section is chunked row by row rather than wrapped: every card of a row
    /// is allocated from that row's top edge, so a long name never lifts its
    /// poster out of line with its neighbours.
    fn search_rows(&mut self, ui: &mut egui::Ui, groups: &[search::Group]) {
        let width = ui.available_width();
        let size = self.settings.poster_size;
        let counts: Vec<usize> = groups.iter().map(|g| g.cards.len()).collect();
        let columns = search::columns(groups, width, size);
        self.search_keys(ui, groups, &counts, &columns);
        // The arrows' selection rings a card and makes Enter act on it, but only
        // while no text field holds the keyboard focus: a screen could never be
        // typed in if the arrows it drew also selected its cards.
        let selection = self.search.focus.filter(|_| !self.typing);
        let card_width = grid::card_width(width, size);
        let playing = player::is_active();
        let mut opened: Option<grid::Card> = None;
        ScrollArea::vertical()
            .id_salt("search-page")
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                for (group, item) in groups.iter().enumerate() {
                    ui.add_space(theme::GAP * 0.5);
                    crate::ui::legible::label(
                        ui,
                        theme::strong(item.title.clone()).size(theme::H2_SIZE),
                    );
                    let per_row = columns.get(group).copied().unwrap_or(1).max(1);
                    // A circle is wider than its caption's allowance needs
                    // relative to a poster, and both already allocate a fixed
                    // box, so a row is only a spacing choice.
                    let gap = if item.people {
                        grid::GAP * 2.0
                    } else {
                        grid::GAP
                    };
                    for row in 0..item.cards.len().div_ceil(per_row) {
                        let first = row * per_row;
                        let Some(chunk) = item
                            .cards
                            .get(first..(first + per_row).min(item.cards.len()))
                        else {
                            continue;
                        };
                        ui.with_layout(egui::Layout::left_to_right(Align::Min), |ui| {
                            ui.spacing_mut().item_spacing.x = gap;
                            for (i, card) in chunk.iter().enumerate() {
                                let index = first + i;
                                let focused = selection == Some(Selection { group, index });
                                let clicked = if item.people {
                                    person(ui, card, focused).clicked()
                                } else {
                                    grid::poster(
                                        ui,
                                        &mut self.artwork,
                                        card,
                                        [card_width, grid::row_height(width, size)],
                                        playing,
                                        false,
                                        focused,
                                    )
                                };
                                if clicked {
                                    opened.get_or_insert(card.clone());
                                }
                            }
                        });
                    }
                }
            });
        if let Some(card) = opened {
            self.open_search(&card);
        }
    }

    /// File a search answer only for the query the field still holds: an answer
    /// to an older keystroke fills nothing.
    pub(super) fn apply_search(&mut self, query: &str, hubs: Vec<Hub>) {
        if self.search.asked.as_deref() != Some(query) {
            return;
        }
        self.error = None;
        self.status = String::new();
        self.search.answer = Answer::Found {
            query: query.to_owned(),
            groups: search::group(&hubs),
        };
        self.search.focus = None;
        // Answered: nothing is in flight until the query changes.
        self.search.pending = None;
    }

    /// File a failed search like an answer, so the page says why instead of
    /// waiting on an answer that is not coming.
    pub(super) fn search_failed(&mut self, query: &str, error: String) {
        if self.search.asked.as_deref() != Some(query) {
            return;
        }
        self.search.answer = Answer::Failed {
            query: query.to_owned(),
            error,
        };
        self.search.focus = None;
        self.search.pending = None;
    }

    /// What the rail's field answered with: a changed query or Enter. Either
    /// opens Search, since Enter on a query the field already reported is
    /// still the user asking for it; Enter also asks at once instead of
    /// waiting out the screen's debounce. Opening comes first because it
    /// forgets the last ask.
    pub(super) fn rail_search(&mut self, query: &str, submitted: bool) {
        if !matches!(self.current, Some(rail::Entry::Search)) {
            self.go(rail::Entry::Search);
        }
        if submitted && !query.is_empty() {
            self.ask_search(query);
        }
    }

    /// Leaving Search for anything but one of its results empties the rail's
    /// field, so a query never sits in the rail over a screen it is not for.
    /// Back from a result finds Search, and its query, as it was.
    pub(super) fn leave_search(&mut self, to: &rail::Entry) {
        let leaving = matches!(self.current, Some(rail::Entry::Search));
        if leaving && !matches!(to, rail::Entry::Detail { .. }) {
            self.query.clear();
            self.rail_query.clear();
        }
    }

    /// Ask for grouped results for `query` and remember exactly that ask, so
    /// only its answer is shown.
    pub(super) fn ask_search(&mut self, query: &str) {
        self.search.asked = Some(query.to_owned());
        self.search.pending = Some(query.to_owned());
        self.search.at = Some(Instant::now());
        self.search.focus = None;
        self.send(Cmd::SearchHubs(query.to_owned()));
    }

    /// The debounce on what the rail's field holds: a query that changed
    /// since the last ask waits out [`DEBOUNCE`] before it is asked for (the
    /// rail asks at once on Enter). An empty query asks for nothing and clears
    /// what is shown.
    fn search_debounce(&mut self, ui: &mut egui::Ui) {
        let query = self.query.trim().to_owned();
        if self.search.asked.as_deref() == Some(query.as_str()) {
            return;
        }
        if query.is_empty() {
            self.search.restart();
            self.search.asked = Some(query);
            self.search.focus = None;
            return;
        }
        if self.search.pending.as_deref() != Some(query.as_str()) {
            self.search.pending = Some(query.clone());
            self.search.at = Some(Instant::now());
        }
        match self.search.at {
            Some(at) if at.elapsed() >= DEBOUNCE => self.ask_search(&query),
            Some(at) => {
                ui.ctx()
                    .request_repaint_after(DEBOUNCE.saturating_sub(at.elapsed()));
            }
            None => self.ask_search(&query),
        }
    }

    /// The arrows and Enter of the Search screen: an arrow moves the selection
    /// across the sections with [`search::move_selection`], Enter opens the card
    /// it selected — the same thing a click on that card does. While a text
    /// field holds the keyboard focus the arrows belong to its caret, and
    /// Alt+Left belongs to Back.
    fn search_keys(
        &mut self,
        ui: &egui::Ui,
        groups: &[search::Group],
        counts: &[usize],
        columns: &[usize],
    ) {
        if self.typing || ui.input(|i| i.modifiers.alt) {
            return;
        }
        for key in [
            egui::Key::ArrowLeft,
            egui::Key::ArrowRight,
            egui::Key::ArrowUp,
            egui::Key::ArrowDown,
        ] {
            if ui.input(|i| i.key_pressed(key)) {
                let arrow = match key {
                    egui::Key::ArrowLeft => grid::Arrow::Left,
                    egui::Key::ArrowRight => grid::Arrow::Right,
                    egui::Key::ArrowUp => grid::Arrow::Up,
                    _ => grid::Arrow::Down,
                };
                self.search.focus =
                    search::move_selection(counts, columns, self.search.focus, arrow);
                return;
            }
        }
        if !player::is_active() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            if let Some(Selection { group, index }) = self.search.focus {
                if let Some(card) = groups.get(group).and_then(|group| group.cards.get(index)) {
                    let card = card.clone();
                    self.open_search(&card);
                }
            }
        }
    }

    /// A search result opens its own overview page, whether it is a film, a
    /// show, a season, an episode or a credited name: [`Self::open_card`] is the
    /// one path a card opens through, and it never plays.
    fn open_search(&mut self, card: &grid::Card) {
        self.open_card(Some(card.clone()));
    }
}

/// The page's state while it has no results to show: a search mark on a
/// round plate, a title, and a line of help, centred in what is left of the
/// page; with `working`, the spinner instead of the help.
fn search_state(ui: &mut egui::Ui, title: &str, help: &str, working: bool) {
    let height = ui.available_height().clamp(0.0, 420.0);
    ui.allocate_ui_with_layout(
        Vec2::new(ui.available_width(), height),
        egui::Layout::top_down(Align::Center),
        |ui| {
            ui.add_space((height * 0.5 - 90.0).max(theme::GAP));
            let (plate, _) = ui.allocate_exact_size(Vec2::splat(PLATE), Sense::hover());
            ui.painter()
                .circle_filled(plate.center(), PLATE / 2.0, theme::palette().wash(16));
            icon::paint(
                ui.painter(),
                Icon::Search,
                Size::Regular,
                plate.center(),
                theme::palette().secondary,
            );
            ui.add_space(theme::SPACING_BASE);
            crate::ui::legible::label(ui, theme::strong(title.to_owned()).size(theme::H2_SIZE));
            if working {
                controls::spinner(ui);
            } else if !help.is_empty() {
                crate::ui::legible::label(ui, theme::secondary(help));
            }
        },
    );
}

/// One credited name: a circle of initials over the name itself, ringed when it
/// is the result the keyboard has selected. Clicking it opens that person's own
/// overview page.
fn person(ui: &mut egui::Ui, card: &grid::Card, focused: bool) -> Response {
    let id = ui.id().with((
        "search-person",
        card.rating_key.as_deref().unwrap_or_default(),
    ));
    let inner = ui.allocate_ui_with_layout(
        Vec2::new(CIRCLE, CIRCLE + CAPTION),
        egui::Layout::top_down(Align::Center),
        |ui| {
            let (rect, _) = ui.allocate_exact_size(Vec2::new(CIRCLE, CIRCLE), Sense::hover());
            ui.painter()
                .circle_filled(rect.center(), CIRCLE / 2.0, theme::palette().accent_tint());
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                search::initials(&card.title),
                theme::h2(),
                theme::palette().accent,
            );
            ui.with_layout(egui::Layout::top_down(Align::Center), |ui| {
                ui.add_space(4.0);
                let name = rail::elide(ui.painter(), &card.title, CAPTION_WIDTH, &theme::meta());
                crate::ui::legible::label(ui, theme::meta_text(name));
            });
        },
    );
    let response = ui.interact(inner.response.rect, id, Sense::click());
    if focused {
        response.request_focus();
    }
    if focused || response.hovered() {
        ui.painter().circle_stroke(
            inner.response.rect.center() - egui::vec2(0.0, CAPTION / 2.0),
            CIRCLE / 2.0 + cards::RING_STROKE,
            egui::Stroke::new(cards::RING_STROKE, theme::palette().accent),
        );
    }
    cards::pointer(response).on_hover_text(card.title.clone())
}
