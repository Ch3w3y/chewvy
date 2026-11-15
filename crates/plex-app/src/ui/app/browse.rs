//! The library grid inside the shell: the poster pages a section answers
//! with, the sorts it offers, and what the grid asks for next.

use plex_core::library::SortOption;

use crate::player;
use crate::ui::grid;
use crate::watch::{Ask, Cmd};

use super::Ui;

impl Ui {
    /// One poster page, filed under the browsing that asked for it: a section
    /// page fills [`Ui::cards`]. An answer whose [`Ask`] is not the last one
    /// this screen sent is stale, so a slow page cannot land in the list of a
    /// screen that never asked for it.
    pub(super) fn apply_cards(
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
    pub(super) fn apply_sorts(&mut self, section: &str, options: Vec<SortOption>) {
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

    /// The library browsing screen: the server's own posters in the responsive
    /// grid, under its title and `totalSize`, with a next page and its chips
    /// belonging to `section`.
    pub(super) fn grid_center(&mut self, ui: &mut egui::Ui, title: String, section: String) {
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
}
