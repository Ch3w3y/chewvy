//! The sidebar: the app's navigation and its search field, a frosted strip
//! attached to the window's left edge.
//!
//! The sidebar, not a per-screen button, is what changes the visible screen,
//! so [`entries`] builds the entry list once from every library this install can
//! browse and [`show`] paints it: the source switcher and Home at the top, each
//! source's libraries grouped under one header naming that server, Settings at
//! the foot. Search is a field under the wordmark, not an entry: its text is
//! the query the Search screen answers.

use egui::{pos2, vec2, Align2, Color32, FontId, Rect, Sense, Ui};

use crate::source::{Library, Origin};
use crate::ui::{brand, chips, glass, glass_tints, marks, theme};

/// Sidebar width the shell reserves.
pub const WIDTH: f32 = 248.0;
/// Height of a main navigation row (Home, Settings).
pub const ROW_HEIGHT: f32 = 38.0;
/// Height of a library row under its server.
const SUB_HEIGHT: f32 = 32.0;
/// Vertical gap between rows.
pub const ROW_GAP: f32 = 2.0;
/// The search field's placeholder.
pub const SEARCH_HINT: &str = "Search everything";
/// The sidebar's own padding.
const PAD_X: f32 = 14.0;
/// The sidebar's top padding.
const PAD_TOP: f32 = 22.0;
/// A row's text inset from its left edge.
const INSET: f32 = 12.0;
/// A row's icon box.
const ICON: f32 = 14.0;
/// The fill of the selected row: white at 9%.
const SELECTED: Color32 = Color32::from_rgba_premultiplied(23, 23, 23, 23);
/// The fill of a hovered row: white at 5%.
const HOVER: Color32 = Color32::from_rgba_premultiplied(13, 13, 13, 13);
/// A library row's label colour.
const SUB_TEXT: Color32 = Color32::from_rgb(0xa9, 0xae, 0xd0);
/// The switcher's label when no single server is in front.
pub const ALL_SOURCES: &str = "All sources";

/// One navigation destination in the rail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    /// The landing screen (its hub rows arrive with the Home milestone).
    Home,
    /// One library section, keyed by its server-side library id.
    Section { id: String, title: String },
    /// An item's own detail page: its `/library/metadata` answer plus the
    /// `/children` read for it, keyed by `ratingKey`. Reached from a poster or
    /// card and left with Back.
    Detail { id: String, title: String },
    /// The Search screen the rail's [`Field`] opens. [`entries`] never lists
    /// it as a row, so that screen draws with no rail entry selected.
    Search,
    /// Settings (placeholder: nothing is editable yet).
    Settings,
    /// The Sources screen the Settings summary opens: every account and server
    /// this app can play from. [`entries`] never lists it, so while it is open
    /// the rail keeps Settings highlighted as the screen it belongs to.
    Sources,
}

impl Entry {
    /// The label the rail draws.
    #[must_use]
    pub fn label(&self) -> &str {
        match self {
            Self::Home => "Home",
            Self::Search => "Search",
            Self::Settings => "Settings",
            Self::Sources => "Sources",
            Self::Section { title, .. } => title,
            Self::Detail { title, .. } => title,
        }
    }
}

/// Rail entries for every library this install can browse: Home first, then each
/// source's libraries in the order that source listed them, then Settings.
/// Search is not an entry: [`show`] draws its field under the wordmark.
#[must_use]
pub fn entries(libraries: &[Library]) -> Vec<Entry> {
    let mut list = Vec::with_capacity(libraries.len() + 2);
    list.push(Entry::Home);
    list.extend(libraries.iter().map(|library| Entry::Section {
        id: library.id.clone(),
        title: library.title.clone(),
    }));
    list.push(Entry::Settings);
    list
}

/// Where the rail's headers go: one per server, naming its source and itself, at
/// the index of the first of that server's libraries. Libraries of one server
/// that arrive in two runs get two headers, which is what the caller listed.
#[must_use]
pub fn headers(libraries: &[Library]) -> Vec<(usize, Origin, String)> {
    let mut built: Vec<(usize, Origin, String)> = Vec::new();
    for (index, library) in libraries.iter().enumerate() {
        let same = built.last().is_some_and(|(_, origin, server)| {
            *origin == library.origin && *server == library.server
        });
        if !same {
            built.push((index, library.origin.clone(), library.server.clone()));
        }
    }
    built
}

/// The rail's search field, shared with the Search screen's own field so the
/// query typed here is the query that screen answers.
pub struct Field<'a> {
    /// The query, the same [`String`] the Search screen's field edits.
    pub query: &'a mut String,
    /// What [`show`] last answered with. A change is judged against this, not
    /// against [`Self::query`]: egui writes this frame's keystrokes into that
    /// shared [`String`] while drawing the field, so comparing against it would
    /// only ever find leading or trailing whitespace.
    pub reported: &'a mut String,
    /// Ask for the keyboard focus this frame; [`show`] clears it once asked.
    pub focus: &'a mut bool,
    /// Set by [`show`]: the field held the keyboard focus while it was drawn.
    pub typed: bool,
}

/// One Jellyfin server as the switcher lists it.
pub struct SwitcherServer {
    /// The server's id, which its libraries carry.
    pub id: String,
    /// The name the server gave itself.
    pub name: String,
    /// Whether it listed its libraries this session.
    pub answering: bool,
}

/// What the source switcher at the top of the rail lists.
pub struct Switcher<'a> {
    /// The Plex account's servers by name, in the order
    /// [`Choice::Connect`] indexes; empty until the account listed them.
    pub plex: &'a [String],
    /// The connected Plex server's name; empty with none connected.
    pub connected: &'a str,
    /// The Jellyfin servers on file.
    pub jellyfin: &'a [SwitcherServer],
}

impl Switcher<'_> {
    /// The row's label: the one server in front, or [`ALL_SOURCES`] when the
    /// rail lists more than one.
    #[must_use]
    pub fn label(&self) -> &str {
        match (self.connected.is_empty(), self.jellyfin) {
            (false, []) => self.connected,
            (true, [only]) => &only.name,
            _ => ALL_SOURCES,
        }
    }

    /// The kind of the one server in front, whose mark the row carries; `None`
    /// when the row stands for several.
    #[must_use]
    pub fn front(&self) -> Option<Origin> {
        match (self.connected.is_empty(), self.jellyfin) {
            (false, []) => Some(Origin::Plex),
            (true, [only]) => Some(Origin::Jellyfin(only.id.clone())),
            _ => None,
        }
    }
}

/// What the user asked the rail for.
pub enum Choice {
    /// Show this entry.
    Go(Entry),
    /// Search for `query` (`submitted` says Enter committed it rather than a
    /// keystroke that the Search screen's own debounce will ask for).
    Search { query: String, submitted: bool },
    /// Connect to the Plex server at this index of [`Switcher::plex`].
    Connect(usize),
    /// Show this Jellyfin server's first library.
    Server(String),
    /// Open the Sources screen with its add panel up.
    AddSource,
    /// Open the Plex rooms panel beside this row.
    Rooms(Rect),
}

/// Draw the sidebar as a fixed-width left panel. Answers with what the user
/// chose this frame.
///
/// `current` is the screen being drawn (clicking its own row again is a no-op);
/// `section` is the row to light up, which for a drill-in is the library or
/// home screen it was reached from. `libraries` groups the library rows by
/// source, and `notes` names a source that has libraries this frame cannot list.
/// `rooms` counts the Plex Watch Together rooms the account is in; while there
/// are any, a row under the Plex server's libraries opens them.
#[allow(clippy::too_many_arguments)]
pub fn show(
    root: &mut Ui,
    entries: &[Entry],
    current: Option<&Entry>,
    section: Option<&Entry>,
    libraries: &[Library],
    notes: &[String],
    switcher: &Switcher<'_>,
    field: &mut Field<'_>,
    rooms: usize,
) -> Option<Choice> {
    let mut choice = None;
    egui::Panel::left("plex-nav")
        .exact_size(WIDTH)
        .resizable(false)
        .show_separator_line(false)
        .frame(egui::Frame::NONE)
        .show(root, |ui| {
            // The glass is added before anything the sidebar draws, so the blur
            // of the backdrop and hero art sits under its labels.
            let panel = ui.max_rect();
            glass::panel(ui.painter(), panel, 0.0, glass_tints::RAIL);
            ui.painter().vline(
                panel.right() - 0.5,
                panel.y_range(),
                egui::Stroke::new(1.0, theme::RULE),
            );
            let inner = Rect::from_min_max(
                pos2(panel.min.x + PAD_X, panel.min.y + PAD_TOP),
                pos2(panel.max.x - PAD_X, panel.max.y - 18.0),
            );
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(inner)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            choice = contents(
                &mut child, entries, current, section, libraries, notes, switcher, field, rooms,
            );
        });
    choice
}

/// The sidebar's rows, top to bottom, inside its padding.
#[allow(clippy::too_many_arguments)]
fn contents(
    ui: &mut Ui,
    entries: &[Entry],
    current: Option<&Entry>,
    section: Option<&Entry>,
    libraries: &[Library],
    notes: &[String],
    switcher: &Switcher<'_>,
    field: &mut Field<'_>,
    rooms: usize,
) -> Option<Choice> {
    let mut choice = None;
    ui.spacing_mut().item_spacing.y = ROW_GAP;
    let (logo, _) = ui.allocate_exact_size(vec2(ui.available_width(), 46.0), Sense::hover());
    brand::wordmark(ui.painter(), pos2(logo.min.x + 8.0, logo.min.y), 28.0, 20.0);
    if let Some(asked) = switch_row(ui, switcher) {
        choice = Some(asked);
    }
    ui.add_space(6.0);
    if let Some((query, submitted)) = search_field(ui, field) {
        choice = Some(Choice::Search { query, submitted });
    }
    ui.add_space(12.0);
    let mut go = |entry: &Entry, clicked: bool| {
        if clicked && current != Some(entry) {
            choice = Some(Choice::Go(entry.clone()));
        }
    };
    for entry in entries.iter().filter(|e| matches!(e, Entry::Home)) {
        go(
            entry,
            nav_row(ui, entry.label(), marks::icon::HOME, section == Some(entry)),
        );
    }
    // One header per server, drawn once above that server's own rows.
    let rows: Vec<&Entry> = entries
        .iter()
        .filter(|e| matches!(e, Entry::Section { .. }))
        .collect();
    let headers = headers(libraries);
    let mut opened = None;
    for index in 0..libraries.len() {
        if let Some((_, origin, server)) = headers.iter().find(|(start, _, _)| *start == index) {
            caption(ui, origin, server);
        }
        if let Some(entry) = rows.get(index) {
            go(entry, sub_row(ui, entry.label(), section == Some(entry)));
        }
        let last_plex = libraries
            .get(index)
            .is_some_and(|l| l.origin == Origin::Plex)
            && libraries
                .get(index + 1)
                .is_none_or(|next| next.origin != Origin::Plex);
        if last_plex && rooms > 0 {
            let row = sub_response(ui, &format!("Watch together · {rooms}"), false);
            if row.clicked() {
                opened = Some(Choice::Rooms(row.rect));
            }
        }
    }
    let jellyfin_headered = headers
        .iter()
        .any(|(_, origin, _)| matches!(origin, Origin::Jellyfin(_)));
    if !notes.is_empty() && !jellyfin_headered {
        caption(ui, &Origin::Jellyfin(String::new()), "");
    }
    for note in notes {
        note_row(ui, note);
    }
    let footer = ROW_HEIGHT + 12.0 + ROW_GAP;
    let rest = (ui.available_height() - footer).max(0.0);
    ui.add_space(rest);
    let (rule, _) = ui.allocate_exact_size(vec2(ui.available_width(), 12.0), Sense::hover());
    ui.painter().hline(
        rule.x_range(),
        rule.top(),
        egui::Stroke::new(1.0, theme::RULE),
    );
    for entry in entries.iter().filter(|e| matches!(e, Entry::Settings)) {
        go(
            entry,
            nav_row(ui, entry.label(), marks::icon::GEAR, section == Some(entry)),
        );
    }
    choice.or(opened)
}

/// The source switcher: one row naming the server in front, which opens the
/// list of every source grouped by kind. A Plex server in the list connects to
/// it, a Jellyfin server shows its first library, and the last two rows lead to
/// the Sources screen.
fn switch_row(ui: &mut Ui, switcher: &Switcher<'_>) -> Option<Choice> {
    let response = ui
        .allocate_response(vec2(ui.available_width(), ROW_HEIGHT), Sense::click())
        .on_hover_text("Servers and accounts");
    let slot = chips::Slot {
        rect: response.rect,
        response: crate::ui::cards::pointer(response),
    };
    let mut choice = None;
    let rect = slot.rect;
    let open = chips::menu(ui, &slot, |ui| {
        // The list is as wide as the row it hangs from, not as the window.
        ui.set_width(rect.width() - 8.0);
        choice = switcher_menu(ui, switcher);
    });
    let fill = if open {
        SELECTED
    } else if slot.response.hovered() {
        HOVER
    } else {
        Color32::from_white_alpha(10)
    };
    ui.painter().rect_filled(rect, theme::CARD_RADIUS, fill);
    ui.painter().rect_stroke(
        rect,
        theme::CARD_RADIUS,
        egui::Stroke::new(theme::HAIRLINE, theme::RULE),
        egui::StrokeKind::Inside,
    );
    let left = rect.min.x + INSET;
    let mark = match switcher.front() {
        Some(origin) => marks::paint_source(ui.painter(), &origin, left, rect.center().y, 10.0),
        // Several sources: a plain dot rather than one service's mark.
        None => {
            ui.painter()
                .circle_filled(pos2(left + 4.0, rect.center().y), 4.0, theme::ACCENT);
            8.0
        }
    };
    let label = Rect::from_min_max(rect.min, pos2(rect.max.x - 26.0, rect.max.y));
    draw_label(
        ui,
        label,
        left + mark + 8.0,
        switcher.label(),
        theme::medium(14.0),
        theme::TEXT,
    );
    // The chevron that says the row opens a list.
    let tip = pos2(rect.max.x - 16.0, rect.center().y + 2.5);
    let stroke = egui::Stroke::new(1.5, theme::MUTED);
    ui.painter()
        .line_segment([tip + vec2(-4.0, -4.5), tip], stroke);
    ui.painter()
        .line_segment([tip, tip + vec2(4.0, -4.5)], stroke);
    choice
}

/// The switcher's list: Plex servers, then Jellyfin servers, then the two rows
/// that manage them.
fn switcher_menu(ui: &mut Ui, switcher: &Switcher<'_>) -> Option<Choice> {
    let mut choice = None;
    ui.spacing_mut().item_spacing.y = ROW_GAP;
    if !switcher.plex.is_empty() || !switcher.connected.is_empty() {
        menu_caption(ui, &Origin::Plex);
        if switcher.plex.is_empty() {
            // The account has not listed its servers yet: the connected one
            // is still shown as where the app is.
            chips::menu_row(ui, switcher.connected, true);
        }
        for (index, name) in switcher.plex.iter().enumerate() {
            let here = name == switcher.connected;
            if chips::menu_row(ui, name, here) && !here {
                choice = Some(Choice::Connect(index));
            }
        }
    }
    if !switcher.jellyfin.is_empty() {
        menu_caption(ui, &Origin::Jellyfin(String::new()));
        for server in switcher.jellyfin {
            let label = if server.answering {
                server.name.clone()
            } else {
                format!("{} (not answering)", server.name)
            };
            if chips::menu_row(ui, &label, false) && server.answering {
                choice = Some(Choice::Server(server.id.clone()));
            }
        }
    }
    ui.add_space(4.0);
    let (rule, _) = ui.allocate_exact_size(vec2(ui.available_width(), 5.0), Sense::hover());
    ui.painter().hline(
        rule.x_range(),
        rule.center().y,
        egui::Stroke::new(1.0, theme::RULE),
    );
    if chips::menu_row(ui, "Manage sources", false) {
        choice = Some(Choice::Go(Entry::Sources));
    }
    if chips::menu_row(ui, "+ Add a source", false) {
        choice = Some(Choice::AddSource);
    }
    choice
}

/// A service's mark over its servers in the switcher's list. The Plex wordmark
/// names itself; the Jellyfin icon is followed by its name.
fn menu_caption(ui: &mut Ui, origin: &Origin) {
    let width = ui
        .available_width()
        .max(chips::MENU_WIDTH + chips::CHECK_COLUMN);
    let (rect, _) = ui.allocate_exact_size(vec2(width, 26.0), Sense::hover());
    let left = rect.min.x + theme::SPACING_BASE;
    let y = rect.max.y - 10.0;
    let mark = marks::paint_source(ui.painter(), origin, left, y, 9.0);
    if origin.is_jellyfin() {
        ui.painter().text(
            pos2(left + mark + 6.0, y),
            Align2::LEFT_CENTER,
            origin.label().to_uppercase(),
            theme::semibold(11.0),
            theme::MUTED,
        );
    }
}

/// The search field: a rounded well with a magnifier, its own text being
/// [`Field::query`]. Answers with the trimmed query when it differs from
/// [`Field::reported`] or when Enter committed it, and clears [`Field::focus`]
/// once the focus has been asked for.
///
/// The comparison cannot read [`Field::query`] back as it was: egui writes this
/// frame's keystrokes into that shared `String` while drawing the field
/// (`text_edit::builder::events` takes `&mut dyn TextBuffer`), so a trimmed
/// query differs from the text it was trimmed from whenever the field holds
/// leading or trailing whitespace. Only a change from [`Field::reported`] asks
/// for anything; the Search screen's own debounce decides when the query is
/// asked for.
fn search_field(ui: &mut Ui, field: &mut Field<'_>) -> Option<(String, bool)> {
    let last = field.reported.clone();
    let (well, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::hover());
    ui.painter()
        .rect_filled(well, theme::CARD_RADIUS, Color32::from_white_alpha(18));
    marks::paint_tinted(
        ui.painter(),
        marks::icon::SEARCH,
        well.min.x + INSET,
        well.center().y,
        13.0,
        theme::MUTED,
    );
    let text = Rect::from_min_max(
        pos2(well.min.x + 38.0, well.min.y),
        pos2(well.max.x - 8.0, well.max.y),
    );
    let edit = ui
        .scope_builder(egui::UiBuilder::new().max_rect(text), |ui| {
            ui.add_sized(
                text.size(),
                egui::TextEdit::singleline(field.query)
                    .hint_text(egui::RichText::new(SEARCH_HINT).color(theme::MUTED))
                    .font(theme::body())
                    .frame(egui::Frame::NONE)
                    .vertical_align(egui::Align::Center)
                    .desired_width(text.width()),
            )
        })
        .inner;
    if *field.focus {
        edit.request_focus();
        *field.focus = false;
    }
    // The focus reported here is what gates the screen's card selection this
    // frame and its Backspace/`/` keys in the frame after.
    field.typed |= edit.has_focus();
    let submitted = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    let query = field.query.trim().to_owned();
    let changed = query != last;
    if submitted || changed {
        *field.reported = query.clone();
    }
    (submitted || changed).then_some((query, submitted))
}

/// A main navigation row: icon and label, lit when selected.
fn nav_row(ui: &mut Ui, label: &str, icon: marks::Logo, selected: bool) -> bool {
    let response = ui.allocate_response(vec2(ui.available_width(), ROW_HEIGHT), Sense::click());
    let response = crate::ui::cards::pointer(response);
    let rect = response.rect;
    let fill = if selected {
        SELECTED
    } else if response.hovered() {
        HOVER
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, theme::CARD_RADIUS, fill);
    let (ink, text) = if selected {
        (theme::ACCENT, theme::TEXT)
    } else {
        (
            Color32::from_rgb(0xc3, 0xc8, 0xe4),
            Color32::from_rgb(0xc3, 0xc8, 0xe4),
        )
    };
    marks::paint_tinted(
        ui.painter(),
        icon,
        rect.min.x + INSET,
        rect.center().y,
        ICON,
        ink,
    );
    let left = rect.min.x + INSET + 30.0;
    draw_label(ui, rect, left, label, theme::medium(14.5), text);
    response.clicked()
}

/// A library row under its server's caption.
fn sub_row(ui: &mut Ui, label: &str, selected: bool) -> bool {
    sub_response(ui, label, selected).clicked()
}

/// A row drawn as a library row is, answering its response.
fn sub_response(ui: &mut Ui, label: &str, selected: bool) -> egui::Response {
    let response = ui.allocate_response(vec2(ui.available_width(), SUB_HEIGHT), Sense::click());
    let response = crate::ui::cards::pointer(response);
    let rect = response.rect;
    let fill = if selected {
        SELECTED
    } else if response.hovered() {
        HOVER
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, theme::CARD_RADIUS, fill);
    if selected {
        ui.painter().circle_filled(
            pos2(rect.min.x + INSET - 4.0, rect.center().y),
            2.5,
            theme::ACCENT,
        );
    }
    let color = if selected { theme::TEXT } else { SUB_TEXT };
    draw_label(
        ui,
        rect,
        rect.min.x + INSET + 4.0,
        label,
        theme::body(),
        color,
    );
    response
}

/// The small capitals over a source's libraries: the service's mark (or a letter
/// badge where this app ships no mark for it), then the server's own name.
fn caption(ui: &mut Ui, origin: &Origin, server: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::hover());
    let y = rect.max.y - 10.0;
    let logo = marks::paint_source(ui.painter(), origin, rect.min.x + INSET, y, 10.0);
    let name = if server.is_empty() {
        origin.label()
    } else {
        server
    }
    .to_uppercase();
    let left = rect.min.x + INSET + logo + 8.0;
    let width = rect.max.x - left;
    let font = theme::semibold(11.0);
    let shown = elide(ui.painter(), &name, width, &font);
    ui.painter().text(
        pos2(left, y),
        Align2::LEFT_CENTER,
        shown,
        font,
        theme::MUTED,
    );
}

/// A row that leads nowhere: what a source answered when its libraries did not
/// arrive, dimmed under that source's header.
fn note_row(ui: &mut Ui, note: &str) {
    let rect = ui
        .allocate_response(vec2(ui.available_width(), SUB_HEIGHT), Sense::hover())
        .rect;
    draw_label(
        ui,
        rect,
        rect.min.x + INSET + 4.0,
        note,
        theme::body(),
        theme::MUTED,
    );
}

/// Draw one left-aligned, vertically centred label from `left`, ellipsising it
/// when it would run past the row.
fn draw_label(ui: &Ui, rect: Rect, left: f32, text: &str, font: FontId, color: Color32) {
    let shown = elide(ui.painter(), text, rect.max.x - left - 8.0, &font);
    ui.painter().text(
        pos2(left, rect.center().y),
        Align2::LEFT_CENTER,
        shown,
        font,
        color,
    );
}

/// `text`, cut to fit `width` with a trailing `…`.
///
/// The whole label is measured once; the cut point is binary searched only
/// when it does not fit, so a rail row costs one text layout per frame in the
/// usual case.
pub(crate) fn elide(painter: &egui::Painter, text: &str, width: f32, font: &FontId) -> String {
    let measure = |s: &str| {
        painter
            .layout_no_wrap(s.to_owned(), font.clone(), Color32::PLACEHOLDER)
            .size()
            .x
    };
    if width <= 0.0 {
        return String::new();
    }
    if measure(text) <= width {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    let fits = |len: usize| {
        let prefix: String = chars.iter().take(len).collect();
        measure(&format!("{prefix}…")) <= width
    };
    let mut lo = 0usize;
    let mut hi = chars.len();
    while hi - lo > 1 {
        let mid = lo + (hi - lo) / 2;
        if fits(mid) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let kept: String = chars.iter().take(lo).collect();
    format!("{kept}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn library(origin: Origin, server: &str, id: &str, title: &str) -> Library {
        Library {
            origin,
            server: server.to_owned(),
            id: id.to_owned(),
            title: title.to_owned(),
        }
    }

    /// Two Plex sections on the connected server, then one library on each of two
    /// Jellyfin servers — the shape both sources arrive in.
    fn libraries() -> Vec<Library> {
        vec![
            library(Origin::Plex, "NAS", "11", "Movies"),
            library(Origin::Plex, "NAS", "12", "TV Shows"),
            library(
                Origin::Jellyfin("a5".to_owned()),
                "Aurora",
                &crate::source::jellyfin_id("a5", "lib1"),
                "Films",
            ),
            library(
                Origin::Jellyfin("b7".to_owned()),
                "Basalt",
                &crate::source::jellyfin_id("b7", "lib2"),
                "Shows",
            ),
        ]
    }

    #[test]
    fn rail_entries_follow_home_then_libraries_then_settings() {
        let built = entries(&libraries());
        let labels: Vec<&str> = built.iter().map(Entry::label).collect();
        assert_eq!(
            labels,
            ["Home", "Movies", "TV Shows", "Films", "Shows", "Settings"]
        );
        assert_eq!(
            built.get(1),
            Some(&Entry::Section {
                id: "11".to_owned(),
                title: "Movies".to_owned()
            })
        );
    }

    #[test]
    fn rail_keeps_home_and_settings_without_libraries() {
        let built = entries(&[]);
        assert_eq!(
            built.iter().map(Entry::label).collect::<Vec<_>>(),
            ["Home", "Settings"]
        );
    }

    /// One header per server: the two Plex sections share one, and the two
    /// Jellyfin servers each get their own — a library is never listed under a
    /// server that does not hold it.
    #[test]
    fn each_server_gets_one_header_over_its_own_libraries() {
        let grouped = headers(&libraries());
        let named: Vec<(&str, &str)> = grouped
            .iter()
            .map(|(_, origin, server)| (origin.label(), server.as_str()))
            .collect();
        assert_eq!(
            named,
            [
                ("Plex", "NAS"),
                ("Jellyfin", "Aurora"),
                ("Jellyfin", "Basalt")
            ]
        );
        assert_eq!(
            grouped
                .iter()
                .map(|(index, _, _)| *index)
                .collect::<Vec<_>>(),
            [0, 2, 3]
        );
        // With nothing listed there is nothing to head.
        assert!(headers(&[]).is_empty());
    }

    /// Two libraries of the same title, from two sources, are still two rows: the
    /// rail lights one row by id, and a Jellyfin id cannot be a Plex section key.
    #[test]
    fn a_library_row_is_told_apart_by_its_source_not_its_title() {
        let rows = vec![
            library(Origin::Plex, "NAS", "11", "Movies"),
            library(
                Origin::Jellyfin("a5".to_owned()),
                "Aurora",
                &crate::source::jellyfin_id("a5", "lib1"),
                "Movies",
            ),
        ];
        let built = entries(&rows);
        let ids: Vec<&str> = built
            .iter()
            .filter_map(|e| match e {
                Entry::Section { id, .. } => Some(id.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(ids.len(), 2);
        assert_ne!(
            ids.first().copied().unwrap_or_default(),
            ids.get(1).copied().unwrap_or_default()
        );
        assert_eq!(headers(&rows).len(), 2);
    }

    /// Search is the rail's field, not one of its rows: a rail entry the user
    /// could click would be a second way to the same screen.
    fn held(name: &str) -> SwitcherServer {
        SwitcherServer {
            id: name.to_lowercase(),
            name: name.to_owned(),
            answering: true,
        }
    }

    #[test]
    fn the_switcher_names_the_one_server_in_front_or_says_there_are_several() {
        let label = |connected: &str, jellyfin: &[SwitcherServer]| {
            Switcher {
                plex: &[],
                connected,
                jellyfin,
            }
            .label()
            .to_owned()
        };
        assert_eq!(label("Attic", &[]), "Attic");
        assert_eq!(label("", &[held("Loft")]), "Loft");
        assert_eq!(label("Attic", &[held("Loft")]), ALL_SOURCES);
        assert_eq!(label("", &[held("Loft"), held("Barn")]), ALL_SOURCES);
        assert_eq!(label("", &[]), ALL_SOURCES);
    }

    #[test]
    fn the_switcher_carries_a_mark_only_for_one_server_in_front() {
        let front = |connected: &str, jellyfin: &[SwitcherServer]| {
            Switcher {
                plex: &[],
                connected,
                jellyfin,
            }
            .front()
        };
        assert_eq!(front("Attic", &[]), Some(Origin::Plex));
        assert_eq!(
            front("", &[held("Loft")]),
            Some(Origin::Jellyfin("loft".to_owned()))
        );
        assert_eq!(front("Attic", &[held("Loft")]), None);
    }

    #[test]
    fn search_is_no_rail_entry() {
        assert!(!entries(&libraries()).contains(&Entry::Search));
    }

    #[test]
    fn a_section_is_selected_by_its_id_not_its_title() {
        let rows = libraries();
        let built = entries(rows.get(..2).unwrap_or_default());
        let current = Entry::Section {
            id: "12".to_owned(),
            title: "TV Shows".to_owned(),
        };
        assert_eq!(built.iter().filter(|e| *e == &current).count(), 1);
        assert_ne!(
            Entry::Section {
                id: "12".to_owned(),
                title: "Other".to_owned()
            },
            Entry::Section {
                id: "13".to_owned(),
                title: "TV Shows".to_owned()
            }
        );
    }
}
