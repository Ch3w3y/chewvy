//! The Search entry: the field, its debounce, and the answer in the server's
//! own groups.
//!
//! The field asks [`crate::watch::Cmd::SearchHubs`] once a query has been still
//! for [`DEBOUNCE`], and an answer is filed only for the query the field still
//! holds. Every result opens its own overview page, so a search never starts
//! playback: playback is what the overview page's own Play asks for.

use std::time::Duration;

use egui::{Align, Response, ScrollArea, Sense, Vec2};

use plex_core::library::Hub;

use crate::player;
use crate::ui::search::{self, Selection};
use crate::ui::{cards, grid, rail, theme};
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

impl Ui {
    /// Draw the Search entry: the field, then one section per search hub that
    /// answered with rows, each section laid out in its own fixed columns.
    pub(super) fn draw_search(&mut self, ui: &mut egui::Ui) {
        ui.heading("Search");
        self.search_field(ui);
        self.search_rows(ui);
    }

    /// One section per hub that has rows, each section's cards laid out in
    /// [`grid`] columns at the Settings poster size.
    ///
    /// A section is chunked row by row rather than wrapped: every card of a row
    /// is allocated from that row's top edge, so a long name never lifts its
    /// poster out of line with its neighbours.
    fn search_rows(&mut self, ui: &mut egui::Ui) {
        let groups = std::mem::take(&mut self.search_groups);
        if groups.is_empty() {
            if self.search_asked.as_deref().is_some_and(|q| !q.is_empty()) {
                ui.label(theme::secondary("No results."));
            }
            self.search_groups = groups;
            return;
        }
        let width = ui.available_width();
        let size = self.settings.poster_size;
        let counts: Vec<usize> = groups.iter().map(|g| g.cards.len()).collect();
        let columns = search::columns(&groups, width, size);
        self.search_keys(ui, &counts, &columns);
        // The arrows' selection rings a card and makes Enter act on it, but only
        // while no text field holds the keyboard focus: a screen could never be
        // typed in if the arrows it drew also selected its cards.
        let selection = self.search_focus.filter(|_| !self.typing);
        let card_width = grid::card_width(width, size);
        let playing = player::is_active();
        let mut opened: Option<grid::Card> = None;
        ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                for (group, item) in groups.iter().enumerate() {
                    ui.add_space(theme::GAP * 0.5);
                    ui.label(theme::strong(item.title.clone()).size(theme::H2_SIZE));
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
        self.search_groups = groups;
        if let Some(card) = opened {
            self.open_search(&card);
        }
    }

    /// File a search answer only for the query the field still holds: an answer
    /// to an older keystroke fills nothing.
    pub(super) fn apply_search(&mut self, query: &str, hubs: Vec<Hub>) {
        if self.search_asked.as_deref() != Some(query) {
            return;
        }
        self.error = None;
        self.status = String::new();
        self.search_groups = search::group(&hubs);
        self.search_focus = None;
    }

    /// Ask for grouped results for `query` and remember exactly that ask, so
    /// only its answer is shown.
    pub(super) fn ask_search(&mut self, query: &str) {
        self.search_asked = Some(query.to_owned());
        self.search_pending = Some(query.to_owned());
        self.search_at = Some(std::time::Instant::now());
        self.search_focus = None;
        self.send(Cmd::SearchHubs(query.to_owned()));
    }

    /// The search field and its debounce: a query that changed since the last
    /// ask waits out [`DEBOUNCE`] before it is asked for, and Enter asks at
    /// once. An empty query asks for nothing and clears what is shown.
    fn search_field(&mut self, ui: &mut egui::Ui) {
        let field = ui.add(
            egui::TextEdit::singleline(&mut self.query)
                .hint_text("Titles, shows, episodes, actors")
                .font(theme::body())
                .desired_width(ui.available_width().min(420.0)),
        );
        if self.focus_search {
            field.request_focus();
        }
        self.typing |= field.has_focus();
        let query = self.query.trim().to_owned();
        if self.search_asked.as_deref() == Some(query.as_str()) {
            return;
        }
        let submitted = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if query.is_empty() {
            self.search_asked = Some(query);
            self.search_pending = None;
            self.search_at = None;
            self.search_groups.clear();
            self.search_focus = None;
            return;
        }
        if submitted {
            self.ask_search(&query);
            return;
        }
        if self.search_pending.as_deref() != Some(query.as_str()) {
            self.search_pending = Some(query.clone());
            self.search_at = Some(std::time::Instant::now());
        }
        match self.search_at {
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
    fn search_keys(&mut self, ui: &egui::Ui, counts: &[usize], columns: &[usize]) {
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
                self.search_focus =
                    search::move_selection(counts, columns, self.search_focus, arrow);
                return;
            }
        }
        if !player::is_active() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            if let Some(Selection { group, index }) = self.search_focus {
                if let Some(card) = self
                    .search_groups
                    .get(group)
                    .and_then(|group| group.cards.get(index))
                {
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
                .circle_filled(rect.center(), CIRCLE / 2.0, theme::ACCENT_TINT);
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                search::initials(&card.title),
                theme::h2(),
                theme::ACCENT,
            );
            ui.with_layout(egui::Layout::top_down(Align::Center), |ui| {
                ui.add_space(4.0);
                let name = rail::elide(ui.painter(), &card.title, CAPTION_WIDTH, &theme::meta());
                ui.label(theme::meta_text(name));
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
            egui::Stroke::new(cards::RING_STROKE, theme::ACCENT),
        );
    }
    cards::pointer(response).on_hover_text(card.title.clone())
}
