//! The library poster grid: one responsive card per item, its own paging, and
//! the chips that change what the next page asks for.
//!
//! [`show`] is the whole browsing screen: the header (section title plus the
//! server's own `totalSize`), the chips row (Sort from
//! `/library/sections/{key}/sorts` and the Unwatched toggle), the poster grid,
//! and the request for the next page when the scroll nears the last row drawn.
//! It answers with one [`Action`]: what to ask the worker for. Nothing here
//! talks to a server; paging, sort and watch-state travel as [`crate::watch::Cmd`]
//! messages, and only fields a captured answer names are read.

use egui::{Align, Color32, CornerRadius, FontId, Rect, Sense, Vec2};

use plex_core::library::{Item, SortOption};

use crate::ui::settings::PosterSize;
use crate::ui::{cards, chips, home, images, rail, theme};

/// Space between cards, in both directions.
pub const GAP: f32 = theme::GAP;
/// Poster height over width: Plex posters are 2:3.
pub const POSTER_RATIO: f32 = 1.5;
/// Items asked for per page.
pub const PAGE: usize = 60;
/// The last row drawn this close to the bottom of the visible area asks for the
/// next page.
pub const NEAR_END: f32 = 400.0;
/// The sort a section starts with when it offers one.
pub const DEFAULT_SORT_KEY: &str = "titleSort";
/// Kinds the grid opens into their children (a show's seasons, a season's
/// episodes) instead of playing.
pub const BROWSE_KINDS: [&str; 2] = ["show", "season"];
/// Kinds the grid plays.
pub const PLAY_KINDS: [&str; 4] = ["movie", "episode", "clip", "video"];

/// Clear space a page keeps on each side of its content: from the rail and
/// from the window's right edge, the same gutter `egui::Frame` gives the panel
/// these screens are drawn in.
const GUTTER: f32 = theme::PAGE_GUTTER;
/// Clear space between a poster and its title.
const POSTER_GAP: f32 = 10.0;
/// Clear space between a title and the year or episode line under it.
const TITLE_GAP: f32 = 2.0;
/// The height one title line is given: [`CAPTION_TITLE_ROWS`] of them are
/// reserved for every row, so a two-line title cannot push a card's meta line
/// below its neighbours'. [`title_lines`] clips a title to that many lines.
const TITLE_LINE: f32 = 18.0;
/// How many title lines a row reserves.
const CAPTION_TITLE_ROWS: usize = 2;
/// The height of one meta line (a year, or `S1 E7`).
const META_LINE: f32 = 16.0;
/// The caption block's own height: the reserved title lines, the gap between
/// title and meta, and the meta line.
const CAPTION_HEIGHT: f32 = CAPTION_TITLE_ROWS as f32 * TITLE_LINE + TITLE_GAP + META_LINE;
/// The pill that counts a show's unwatched episodes.
const BADGE: [f32; 2] = [26.0, 22.0];
/// Badge and marker inset from the poster's top-right corner.
const BADGE_INSET: f32 = 8.0;
/// The accent dot that marks a movie never played.
const MARK_RADIUS: f32 = 5.0;

/// The card width band, `[min, max]`, one [`PosterSize`] lays out within: a
/// wider band only ever means a wider card, never a forced one. `Medium` is the
/// band the grid had before the setting existed.
#[must_use]
pub fn band(size: PosterSize) -> [f32; 2] {
    match size {
        PosterSize::Small => [110.0, 150.0],
        PosterSize::Medium => [160.0, 200.0],
        PosterSize::Large => [210.0, 280.0],
    }
}

/// The width `window` leaves the content of a page: less the `rail` panel and
/// less [`Self::GUTTER`] on each side. [`Self::columns`] and [`Self::card_width`]
/// lay their columns in exactly this width, which is how a page keeps its
/// gutters whoever drew the panel around it.
#[must_use]
pub fn content_width(window: f32, rail: f32) -> f32 {
    (window - rail - 2.0 * GUTTER).max(0.0)
}

/// Every row's height for `available` points of width at `size`: the 2:3 poster
/// at the columns' card width, [`Self::POSTER_GAP`], and the caption block's
/// reserved lines. [`Self::show`] lays its rows at exactly this and
/// [`Self::poster`] draws its card at exactly this, so a two-line title never
/// moves a row out of line with its neighbours.
#[must_use]
pub fn row_height(available: f32, size: PosterSize) -> f32 {
    poster_size(card_width(available, size)).y + POSTER_GAP + CAPTION_HEIGHT
}

/// Columns for `available` points of width at `size`: as many cards as fit at
/// that size's narrowest width with [`Self::GAP`] between them.
#[must_use]
pub fn columns(available: f32, size: PosterSize) -> usize {
    let [min, _] = band(size);
    if !available.is_finite() || available <= 0.0 {
        return 1;
    }
    let fit = ((available + GAP) / (min + GAP)).floor();
    if fit < 1.0 {
        return 1;
    }
    fit as usize
}

/// The width `columns(available, size)` cards take in `available` points: the
/// columns share the space evenly, clamped to that size's [`Self::band`].
#[must_use]
pub fn card_width(available: f32, size: PosterSize) -> f32 {
    let [min, max] = band(size);
    let columns = columns(available, size);
    let each = (available - (columns - 1) as f32 * GAP) / columns as f32;
    each.clamp(min, max)
}

/// The 2:3 poster box of a card `width` wide.
#[must_use]
pub fn poster_size(width: f32) -> Vec2 {
    Vec2::new(width, width * POSTER_RATIO)
}

/// How many rows an answer holds, in words: one row is one item. A section's
/// own total is used when the server gave one, so a paged answer is not misread
/// as its first page.
#[must_use]
pub fn item_count(count: u64) -> String {
    format!("{count} item{}", if count == 1 { "" } else { "s" })
}

/// One grid card, mapped from what the server named for that item.
#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    pub rating_key: Option<String>,
    pub title: String,
    /// The server's own `type` (`movie`, `show`, `season`, `episode`).
    pub kind: String,
    /// The caption's second line: a movie's or show's `year`, an episode's
    /// `S{parentIndex} E{index}`, a season's `leafCount`. Empty when the answer
    /// named none of those.
    pub subtitle: String,
    pub thumb: Option<String>,
    /// A show's or season's unwatched leaves (`leafCount` less
    /// `viewedLeafCount`), drawn as the top-right badge. `None` draws no badge.
    pub unwatched: Option<u64>,
    /// Where to open it, in seconds: the server names `viewOffset` in
    /// milliseconds.
    pub start: Option<f64>,
}

impl Card {
    #[must_use]
    pub fn from_item(item: &Item) -> Self {
        let kind = item.kind.clone().unwrap_or_default();
        Self {
            rating_key: item.rating_key.clone(),
            title: item
                .title
                .clone()
                .unwrap_or_else(|| "(untitled)".to_owned()),
            subtitle: subtitle(item, &kind),
            thumb: item.thumb.clone(),
            unwatched: unwatched(item, &kind),
            start: home::resume_start(item),
            kind,
        }
    }

    /// The grid opens this card into its children.
    #[must_use]
    pub fn browsable(&self) -> bool {
        BROWSE_KINDS.contains(&self.kind.as_str())
    }

    /// The grid plays this card.
    #[must_use]
    pub fn playable(&self) -> bool {
        PLAY_KINDS.contains(&self.kind.as_str())
    }
}

/// The caption's second line, from the fields this kind of row names.
fn subtitle(item: &Item, kind: &str) -> String {
    match kind {
        "episode" => match (item.parent_index, item.index) {
            (Some(parent), Some(index)) => format!("S{parent} E{index}"),
            (_, Some(index)) => format!("E{index}"),
            _ => String::new(),
        },
        "season" => item
            .leaf_count
            .map(|leaf| format!("{leaf} episode{}", if leaf == 1 { "" } else { "s" }))
            .unwrap_or_default(),
        _ => item.year.map(|year| year.to_string()).unwrap_or_default(),
    }
}

/// A show's or season's unwatched leaf count as the answer names it. `None` for
/// a kind that names no `leafCount`, which is what a movie row does: no captured
/// section answer names `viewCount` for a movie row, so a movie's own watch
/// state is never inferred here — only the [`Chips::unwatched`] filter the
/// server itself applied marks a leaf poster.
fn unwatched(item: &Item, kind: &str) -> Option<u64> {
    if !BROWSE_KINDS.contains(&kind) {
        return None;
    }
    item.leaf_count
        .map(|leaf| leaf.saturating_sub(item.viewed_leaf_count.unwrap_or(0)))
}

/// The mark one poster carries, decided from what the answer named.
#[derive(Clone, Debug, PartialEq)]
pub enum Marker {
    /// A show's or season's own unwatched `leafCount` difference.
    Count(u64),
    /// A leaf row (movie, episode, clip, video) the server only returned
    /// because the unwatched filter was on.
    Dot,
    /// Nothing: the answer named no watch state for this row.
    None,
}

/// The mark `card` gets while `unwatched_filter` is the filter in effect.
///
/// A count badge needs the `leafCount`/`viewedLeafCount` pair a show or season
/// row names. The dot is claimed only under the server's own `unwatched=1`
/// filter: no captured section answer names `viewCount` for a movie row, so a
/// movie's watch state is never inferred from its absence.
#[must_use]
pub fn marker(card: &Card, unwatched_filter: bool) -> Marker {
    match card.unwatched.filter(|count| *count > 0) {
        Some(count) => Marker::Count(count),
        None if unwatched_filter && card.playable() => Marker::Dot,
        None => Marker::None,
    }
}

/// The chips the grid browses with. Changing either reloads from the start.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Chips {
    /// The value sent as `sort`, in the form [`SortOption`] names it
    /// (`titleSort`, `originallyAvailableAt:desc`).
    pub sort: Option<String>,
    pub unwatched: bool,
}

/// The sort to start a section with: [`DEFAULT_SORT_KEY`] ascending when the
/// section names that sort, else the first sort it names in the direction that
/// option says it is normally shown in.
#[must_use]
pub fn default_sort(sorts: &[SortOption]) -> Option<String> {
    if let Some(title) = sorts.iter().find(|sort| sort.key == DEFAULT_SORT_KEY) {
        return Some(title.key.clone());
    }
    sorts.first().and_then(|sort| sort_value(sort, None))
}

/// The value to ask `option` for given the sort in effect: the other direction
/// when that option is already active, otherwise the direction the option names
/// as its own default. `None` when it names no reversed form to switch to.
#[must_use]
pub fn sort_value(option: &SortOption, current: Option<&str>) -> Option<String> {
    let reversed = option.desc_key.as_deref();
    if current == Some(&option.key) {
        return reversed.map(str::to_owned);
    }
    if current == reversed {
        return Some(option.key.clone());
    }
    if option.default_direction.as_deref() == Some("desc") {
        return reversed.map(str::to_owned);
    }
    Some(option.key.clone())
}

/// The [`SortOption`] one sort value names, matched on its ascending key or its
/// `:desc` key so a field is found whatever direction it is in effect in.
#[must_use]
pub fn sort_field<'a>(sort: Option<&str>, sorts: &'a [SortOption]) -> Option<&'a SortOption> {
    let value = sort?;
    sorts
        .iter()
        .find(|option| option.key == value || option.desc_key.as_deref() == Some(value))
}

/// What the Sort chip says: the field in effect, or just `Sort` before a section
/// names one. The direction is its own control, so the field is named once.
#[must_use]
pub fn sort_label(sort: Option<&str>, sorts: &[SortOption]) -> String {
    match sort_field(sort, sorts) {
        Some(option) => format!(
            "Sort: {}",
            option.title.clone().unwrap_or_else(|| option.key.clone())
        ),
        None => sort.unwrap_or("Sort").to_owned(),
    }
}

/// The other direction for the sort in effect, as the section names it: the
/// `:desc` key of a field currently ascending, the plain key when it is
/// descending. `None` when no field the section lists is in effect.
#[must_use]
pub fn flip_sort(current: Option<&str>, sorts: &[SortOption]) -> Option<String> {
    let value = current?;
    sort_field(current, sorts).and_then(|option| sort_value(option, Some(value)))
}

/// The word the order chip shows for the sort in effect.
///
/// Spelled out rather than drawn: egui's bundled proportional fonts name no
/// arrow glyph.
#[must_use]
pub fn sort_direction(current: Option<&str>) -> &'static str {
    if current.is_some_and(|value| value.ends_with(":desc")) {
        "Descending"
    } else {
        "Ascending"
    }
}

/// What the user, or the scroll, asked the grid for.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Open the card at this index of the list [`View`] showed.
    Open(usize),
    /// Browse with this sort value, from the start.
    Sort(Option<String>),
    /// Browse with this watch-state filter, from the start.
    Unwatched(bool),
    /// Fetch the next page.
    Next,
    /// The keyboard selection the arrows moved, in this list's own indexes.
    /// `None` means the arrow left the grid.
    Select(Option<usize>),
}

/// The index of a card the grid opens, or `None` when it opens nothing: an
/// answer naming no `ratingKey`, or a kind in neither kind list.
#[must_use]
pub fn open(card: &Card, index: usize) -> Option<Action> {
    card.rating_key.as_ref()?;
    if card.browsable() || card.playable() {
        Some(Action::Open(index))
    } else {
        None
    }
}

/// Which way an arrow moves the keyboard selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arrow {
    Left,
    Right,
    Up,
    Down,
}

/// The card one arrow selects among `count` cards laid out in `columns`, from
/// `current` (index 0 is the first card): a pure rule the screens keep in their
/// own state so navigation does not depend on which rows egui happened to lay
/// out this frame. `None` means the arrow leaves the grid; with nothing
/// selected yet any arrow selects the first card.
#[must_use]
pub fn move_focus(
    columns: usize,
    count: usize,
    current: Option<usize>,
    arrow: Arrow,
) -> Option<usize> {
    let columns = i64::try_from(columns.max(1)).unwrap_or(i64::MAX);
    let last = i64::try_from(count.saturating_sub(1)).unwrap_or(i64::MAX);
    let current = match current {
        Some(index) => i64::try_from(index).unwrap_or(i64::MAX),
        None => return if count == 0 { None } else { Some(0) },
    };
    if current > last {
        return None;
    }
    let next = match arrow {
        Arrow::Left => current - 1,
        Arrow::Right => current + 1,
        Arrow::Up => current - columns,
        Arrow::Down => current + columns,
    };
    if next < 0 || next > last {
        return None;
    }
    usize::try_from(next).ok()
}

/// The browsing screen to draw.
pub struct View<'a> {
    pub cards: &'a [Card],
    /// The rail entry's title.
    pub title: String,
    /// The container's `totalSize`. `None` counts what the answer returned.
    pub total: Option<u64>,
    pub more: bool,
    pub chips: Chips,
    pub sorts: &'a [SortOption],
    /// A page is already in flight, so the grid does not ask again.
    pub paging: bool,
    /// The poster band to lay the cards out with.
    pub size: PosterSize,
    /// The card keyboard navigation has selected, if any.
    pub focus: Option<usize>,
    /// Read the arrow keys and Enter for the card selection. A screen that
    /// draws its own cards (the Search sections) turns this off so one screen
    /// reads the keys once.
    pub navigate: bool,
    /// A text field holds the keyboard focus, so the arrows and Backspace belong
    /// to it and not to the cards.
    pub typing: bool,
}

/// Draw [`View`] and answer what to ask the worker for, if anything.
///
/// `playing` disables card clicks while the player window has an item open.
pub fn show(
    ui: &mut egui::Ui,
    view: View<'_>,
    artwork: &mut images::Artwork,
    playing: bool,
) -> Option<Action> {
    header(ui, &view);
    if let Some(action) = chips_row(ui, &view.chips, view.sorts) {
        return Some(action);
    }
    ui.separator();

    // The grid holds its own gutters rather than trusting the panel it was put
    // in: columns, card width and row height all come from the width that keeps
    // a gutter clear on each side of the window.
    let gutters = content_width(ui.ctx().content_rect().width(), rail::WIDTH);
    let width = ui.available_width().min(gutters);
    let columns = columns(width, view.size);
    let card_width = card_width(width, view.size);
    let row_h = row_height(width, view.size);
    let mut opened = view
        .navigate
        .then(|| keyboard_action(ui, &view, columns, playing))
        .flatten();
    let mark = view.chips.unwatched;
    let per_row = columns.max(1);
    let rows = view.cards.len().div_ceil(per_row);
    // Every card has the same fixed size, so only the rows in view are laid out
    // and only their posters are asked for: a 1,600-item library costs what one
    // screenful costs, and the texture cache never holds more than it can show.
    let area = egui::ScrollArea::vertical()
        .auto_shrink([false; 2])
        .show_rows(ui, row_h, rows, |ui, visible| {
            for row in visible {
                let first = row * per_row;
                let Some(chunk) = view
                    .cards
                    .get(first..(first + per_row).min(view.cards.len()))
                else {
                    continue;
                };
                // Cards hang from the row's top edge, so a two-line title never
                // pushes its poster out of line with its neighbours.
                ui.with_layout(egui::Layout::left_to_right(Align::Min), |ui| {
                    ui.spacing_mut().item_spacing.x = GAP;
                    for (i, card) in chunk.iter().enumerate() {
                        let index = first + i;
                        let focused = keyboard_focus(ui, card, view.focus, index);
                        let clicked = poster(
                            ui,
                            artwork,
                            card,
                            [card_width, row_h],
                            playing,
                            mark,
                            focused,
                        );
                        if clicked && opened.is_none() {
                            opened = open(card, index);
                        }
                    }
                });
            }
        });
    // Of the content the area has laid out, how much is still below the
    // viewport: full content height, less what was scrolled past, less what the
    // viewport shows. All three come from the same `ScrollAreaOutput`.
    let hidden = area.content_size.y - area.state.offset.y - area.inner_rect.height();
    let next = view.more && !view.paging && hidden <= NEAR_END;

    opened.or({
        if next {
            Some(Action::Next)
        } else {
            None
        }
    })
}

/// The section title beside the count the server gave it.
fn header(ui: &mut egui::Ui, view: &View<'_>) {
    ui.horizontal(|ui| {
        ui.label(theme::strong(view.title.clone()).size(theme::H1_SIZE));
        let shown = item_count(match view.total {
            Some(total) => total,
            None => view.cards.len() as u64,
        });
        ui.label(theme::meta_text(shown));
    });
}

/// The chips row: the Sort chip over the section's own `/sorts` options, the
/// order chip that reverses whichever field is in effect, and the Unwatched
/// toggle. The menu names each field once and checks the one in effect; the
/// direction never rides along on the field's name.
fn chips_row(ui: &mut egui::Ui, state: &Chips, sorts: &[SortOption]) -> Option<Action> {
    let label = sort_label(state.sort.as_deref(), sorts);
    let order = sort_direction(state.sort.as_deref());
    let descending = order == "Descending";
    // Only a field this section lists can be reversed: an order chip would
    // otherwise offer a sort the server never named.
    let order_action =
        flip_sort(state.sort.as_deref(), sorts).map(|value| Action::Sort(Some(value)));
    let mut action = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = chips::GAP;
        let sort_chip = chips::slot(ui, &label);
        let order_chip = order_action.is_some().then(|| chips::slot(ui, order));
        let unwatched_hit = chips::toggle(
            ui,
            "Unwatched",
            state.unwatched,
            "Ask the server for only what it says is unwatched",
        );

        let menu_open = chips::menu(ui, &sort_chip, |ui| {
            if sorts.is_empty() {
                ui.label(theme::meta_text("This library named no sorts."));
                return;
            }
            let active = sort_field(state.sort.as_deref(), sorts).map(|option| &option.key);
            for option in sorts {
                // The value this row would ask for. `None` when the field is
                // already in effect and the section names no reversed form of
                // it: the row still names the field and still reads as checked,
                // and clicking it asks for nothing new.
                let value = sort_value(option, state.sort.as_deref());
                let title = option.title.as_deref().unwrap_or(&option.key);
                if chips::menu_row(ui, title, active == Some(&option.key)) {
                    if let Some(sort) = value {
                        action = Some(Action::Sort(Some(sort)));
                    }
                    ui.close();
                }
            }
        });
        if let Some(chip) = &order_chip {
            // The grid can only ask for one page per frame, so a sort taken
            // from the menu wins over the order chip in the same frame.
            if action.is_none()
                && chip
                    .response
                    .clone()
                    .on_hover_text("Reverse the order of the sort in effect")
                    .clicked()
            {
                action = order_action.clone();
            }
            chips::paint(ui, chip.rect, order, descending, false);
        }
        if unwatched_hit && action.is_none() {
            action = Some(Action::Unwatched(!state.unwatched));
        }
        chips::paint(ui, sort_chip.rect, &label, state.sort.is_some(), menu_open);
    });
    action
}

/// The arrow and Enter the browsing screen answers with: an arrow moves the
/// selection with [`move_focus`] (only the grid knows its own columns and card
/// count), Enter acts on the selected card through [`open`], which is the same
/// answer a click on that card gives.
fn keyboard_action(
    ui: &egui::Ui,
    view: &View<'_>,
    columns: usize,
    playing: bool,
) -> Option<Action> {
    // While a text field holds the keyboard focus the arrows move its caret and
    // Backspace edits it; Alt+Left belongs to the screen's Back.
    let alt = ui.input(|i| i.modifiers.alt);
    if !view.typing && !alt {
        for key in [
            egui::Key::ArrowLeft,
            egui::Key::ArrowRight,
            egui::Key::ArrowUp,
            egui::Key::ArrowDown,
        ] {
            if ui.input(|i| i.key_pressed(key)) {
                let arrow = match key {
                    egui::Key::ArrowLeft => Arrow::Left,
                    egui::Key::ArrowRight => Arrow::Right,
                    egui::Key::ArrowUp => Arrow::Up,
                    _ => Arrow::Down,
                };
                let current = view.focus.filter(|&index| index < view.cards.len());
                return Some(Action::Select(move_focus(
                    columns,
                    view.cards.len(),
                    current,
                    arrow,
                )));
            }
        }
    }
    if !playing && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        if let Some((index, card)) = view
            .focus
            .and_then(|index| view.cards.get(index).map(|card| (index, card)))
        {
            return open(card, index);
        }
    }
    None
}

/// The id a card's click box is registered under, so the keyboard selection
/// and the card's own focus are the same card.
#[must_use]
fn card_id(grid: egui::Id, card: &Card) -> egui::Id {
    grid.with(("grid-card", card.rating_key.as_deref().unwrap_or_default()))
}

/// Whether this card is the one to ring: the screen's own keyboard selection, or
/// the card egui already holds the keyboard focus on. Only the rows in view are
/// laid out, so egui focus is read by id rather than by asking which widget is
/// focused.
fn keyboard_focus(ui: &egui::Ui, card: &Card, selection: Option<usize>, index: usize) -> bool {
    if selection == Some(index) {
        return true;
    }
    if !ui.input(|i| i.focused) {
        return false;
    }
    let id = card_id(ui.id(), card);
    ui.ctx().memory(|mem| mem.has_focus(id))
}

/// One poster card: 2:3 artwork with its badge or marker, then title and year,
/// clickable while nothing is playing. Answers whether the user clicked it.
///
/// `mark_unwatched` is the [`Chips::unwatched`] filter in effect: only rows the
/// server already said are unwatched get the accent dot. `focused` asks egui to
/// hold the keyboard focus on this card, which is what draws its ring when the
/// screen's keyboard selection is here.
pub(crate) fn poster(
    ui: &mut egui::Ui,
    artwork: &mut images::Artwork,
    card: &Card,
    size: [f32; 2],
    playing: bool,
    mark_unwatched: bool,
    focused: bool,
) -> bool {
    let [width, height] = size;
    let poster = poster_size(width);
    let clickable = !playing && (card.playable() || card.browsable());
    // Every card is given the same box - `width` across, `height` down, the
    // height [`Self::row_height`] reserves - so a long caption costs its own
    // card nothing and a row's posters, titles and meta lines stay in line.
    // The caption steps by [`Self::TITLE_GAP`]; the poster's own artwork,
    // badge and marker are painted into one allocation, so the one stepped
    // space before them is the [`Self::POSTER_GAP`] gap to the title.
    let inner = ui.allocate_ui_with_layout(
        egui::vec2(width, height),
        egui::Layout::top_down(Align::Min),
        |ui| {
            ui.spacing_mut().item_spacing.y = TITLE_GAP;
            let (rect, _) = ui.allocate_exact_size(poster, Sense::hover());
            poster_artwork(ui, artwork, card, rect);
            match marker(card, mark_unwatched) {
                Marker::Count(count) => badge(ui, rect, &count.to_string()),
                Marker::Dot => mark(ui, rect),
                Marker::None => {}
            }
            ui.add_space(POSTER_GAP - TITLE_GAP);
            caption(ui, card, width);
        },
    );
    let response = ui.interact(
        inner.response.rect,
        card_id(ui.id(), card),
        if clickable {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    if focused {
        response.request_focus();
    }
    cards::focus_ring(
        ui,
        inner.response.rect,
        &response,
        cards::CARD_RADIUS,
        focused,
    );
    let response = if clickable {
        cards::pointer(response)
    } else {
        response
    };
    response.on_hover_text(tooltip(card)).clicked()
}

fn poster_artwork(ui: &mut egui::Ui, artwork: &mut images::Artwork, card: &Card, rect: Rect) {
    ui.painter()
        .rect_filled(rect, theme::CARD_RADIUS, theme::CARD);
    if let Some((id, tex)) = artwork.texture(
        card.thumb.as_deref(),
        rect.width() as u32,
        rect.height() as u32,
    ) {
        cards::rounded_art(
            ui,
            id,
            rect,
            cards::cover_uv_top(tex, rect.size()),
            theme::CARD_RADIUS,
            Color32::WHITE,
        );
    }
}

/// The unwatched-episodes badge: an accent pill with the count in dark ink, in
/// the poster's top-right corner (a poster's own title art sits at its foot,
/// where a badge would read as part of the title). It widens for 3-digit counts.
fn badge(ui: &mut egui::Ui, rect: Rect, text: &str) {
    let font = theme::semibold(theme::CAPTION_SIZE);
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font, theme::PRIMARY_TEXT);
    let width = (galley.size().x + 12.0).max(BADGE[1]);
    let box_ = Rect::from_min_size(
        egui::pos2(rect.max.x - width - BADGE_INSET, rect.min.y + BADGE_INSET),
        egui::vec2(width, BADGE[1]),
    );
    ui.painter().rect_filled(
        box_.expand(1.5),
        CornerRadius::same(255),
        Color32::from_black_alpha(90),
    );
    ui.painter()
        .rect_filled(box_, CornerRadius::same(255), theme::ACCENT);
    ui.painter().galley(
        box_.center() - galley.size() * 0.5,
        galley,
        theme::PRIMARY_TEXT,
    );
}

/// A leaf marker: an accent dot ringed in dark, top-right like [`badge`].
fn mark(ui: &mut egui::Ui, rect: Rect) {
    let center = egui::pos2(
        rect.max.x - BADGE_INSET - MARK_RADIUS,
        rect.min.y + BADGE_INSET + MARK_RADIUS,
    );
    ui.painter()
        .circle_filled(center, MARK_RADIUS + 2.0, Color32::from_black_alpha(180));
    ui.painter()
        .circle_filled(center, MARK_RADIUS, theme::ACCENT);
}

/// Title and its year or episode word, each on its own line and stepped by
/// [`Self::TITLE_GAP`]: the title clipped to [`CAPTION_TITLE_ROWS`] lines that
/// each fit `width`, the meta line elided to `width` too. Nothing a caption
/// draws is ever wider than the poster above it.
fn caption(ui: &mut egui::Ui, card: &Card, width: f32) {
    let font = theme::card_title();
    let lines = title_lines(ui.painter(), &card.title, width, &font);
    ui.with_layout(egui::Layout::top_down(Align::Min), |ui| {
        ui.spacing_mut().item_spacing.y = TITLE_GAP;
        for line in lines {
            ui.label(theme::title(line));
        }
        if !card.subtitle.is_empty() {
            let meta = rail::elide(ui.painter(), &card.subtitle, width, &theme::meta());
            ui.label(theme::meta_text(meta));
        }
    });
}

/// A `text` laid out as at most [`CAPTION_TITLE_ROWS`] lines of `width` each:
/// whole words while they fit, and when something has to go, the last line kept
/// ends with `…` instead. Measured, so what a card shows and what
/// [`Self::row_height`] reserved for it are the same lines.
fn title_lines(painter: &egui::Painter, text: &str, width: f32, font: &FontId) -> Vec<String> {
    if width <= 0.0 {
        return Vec::new();
    }
    let fits = |line: &str| {
        painter
            .layout_no_wrap(line.to_owned(), font.clone(), Color32::PLACEHOLDER)
            .size()
            .x
            <= width
    };
    let mut lines: Vec<String> = Vec::new();
    let mut rest = text.trim();
    while !rest.is_empty() {
        if fits(rest) {
            lines.push(rest.to_owned());
            return lines;
        }
        // The longest run of whole words that still fits this line.
        let mut cut = 0usize;
        for word in rest.split_whitespace() {
            let next = if cut == 0 {
                word.len()
            } else {
                cut + 1 + word.len()
            };
            if next <= rest.len() && fits(rest[..next].trim_end()) {
                cut = next;
            } else {
                break;
            }
        }
        if cut == 0 {
            // A single word wider than the card: keep what of it fits.
            lines.push(rail::elide(painter, rest, width, font));
            return lines;
        }
        lines.push(rest[..cut].trim_end().to_owned());
        rest = rest[cut..].trim();
        if lines.len() == CAPTION_TITLE_ROWS {
            lines.push(rail::elide(painter, rest, width, font));
            return lines;
        }
    }
    lines
}

fn tooltip(card: &Card) -> String {
    let mut lines = vec![card.title.clone()];
    if !card.subtitle.is_empty() {
        lines.push(card.subtitle.clone());
    }
    if let Some(count) = card.unwatched.filter(|count| *count > 0) {
        lines.push(format!("{count} unwatched"));
    }
    lines.join("\n")
}

#[cfg(test)]
#[path = "grid/tests.rs"]
mod tests;
