//! Watching together on screen: the panel a detail page opens, and the short
//! notices shown over any screen.

use egui::{pos2, vec2, Align2, Color32, Id, Rect, RichText, Sense, Stroke};

use crate::together::{Listed, Person, Room};
use crate::ui::chips;
use crate::ui::overlay::{self, BUTTON};
use crate::ui::player::GLASS;
use crate::ui::{glass, glass_tints, theme};

/// What the panel shows.
pub(crate) enum PanelView<'a> {
    /// Not in a group: start one, or join one the server listed.
    Choose {
        /// The server's groups once listed, or why it would not list them.
        groups: Option<&'a Result<Vec<Listed>, String>>,
        /// Why the last start or join did not work.
        refusal: Option<&'a str>,
    },
    /// A Plex title: start a room with the people ticked, or join a room
    /// already made for it.
    Title {
        /// Who can be invited once listed, or why they could not be.
        people: Option<&'a Result<Vec<Person>, String>>,
        ticked: &'a [u64],
        /// The rooms for this title.
        rooms: Vec<&'a Room>,
        refusal: Option<&'a str>,
    },
    /// Every Plex room this account is in or invited to.
    Rooms {
        rooms: Option<&'a Result<Vec<Room>, String>>,
        refusal: Option<&'a str>,
    },
    /// Waiting for the server to say this device is in.
    Joining,
    /// In a group.
    In {
        name: &'a str,
        members: &'a [String],
        state: &'static str,
        /// In a Plex room: who else can be invited.
        invite: Option<Invite<'a>>,
    },
}

/// The people a Plex room this device is in can still invite.
pub(crate) struct Invite<'a> {
    pub people: Option<&'a Result<Vec<Person>, String>>,
    pub ticked: &'a [u64],
    /// Already invited.
    pub invited: &'a [u64],
}

/// What the person chose in the panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Choice {
    Start,
    Join(String),
    Leave,
    Close,
    /// Tick or untick someone to invite.
    Tick(u64),
    /// Make a Plex room, inviting whoever is ticked.
    Create,
    /// Join a Plex room.
    Enter(String),
    /// Take a Plex room off the list.
    Remove(String),
    /// Invite whoever is ticked to the room this device is in.
    Invite,
}

/// The panel's width, and its distance from the button that opened it.
const WIDTH: f32 = 420.0;
const GAP: f32 = 10.0;
/// The panel's inner padding, as the detail page's Play-from panel has.
const PAD: i8 = 18;
/// The closest the panel comes to the window's edges.
const MARGIN: f32 = 16.0;

/// The watch-together panel, on the detail page's glass under the button at
/// `anchor` that opened it. A click outside it or Escape closes it.
pub(crate) fn panel(ctx: &egui::Context, anchor: Rect, view: &PanelView<'_>) -> Option<Choice> {
    let mut choice = None;
    let id = Id::new("watch-together");
    let window = ctx.content_rect();
    let width = WIDTH.min(window.width() - 2.0 * MARGIN);
    let left = anchor.left().min(window.right() - MARGIN - width);
    let top = anchor.bottom() + GAP;
    // Taller content scrolls rather than pushing the panel up over the button.
    let room = (window.bottom() - MARGIN - top - 2.0 * f32::from(PAD)).max(120.0);
    // The glass goes behind the content at the size it took last frame.
    let last = ctx.memory(|m| m.area_rect(id));
    let shown = egui::Area::new(id)
        .fixed_pos(pos2(left.max(window.left() + MARGIN), top))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            if let Some(rect) = last {
                glass::panel(ui.painter(), rect, 12.0, glass_tints::PANEL);
            }
            egui::Frame::new()
                .inner_margin(egui::Margin::same(PAD))
                .show(ui, |ui| {
                    ui.set_width(width - 2.0 * f32::from(PAD));
                    // An area offers only last frame's size; the list may
                    // have grown since.
                    ui.set_max_height(room);
                    egui::ScrollArea::vertical()
                        .max_height(room)
                        .show(ui, |ui| choice = contents(ui, view));
                });
        });
    if last != Some(shown.response.rect) {
        ctx.request_repaint();
    }
    // Escape is taken here so it closes the panel without leaving the page.
    let escape = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    if shown.response.clicked_elsewhere() || escape {
        choice = choice.or(Some(Choice::Close));
    }
    choice
}

fn contents(ui: &mut egui::Ui, view: &PanelView<'_>) -> Option<Choice> {
    ui.label(
        RichText::new("Watch together")
            .font(theme::bold(15.0))
            .color(theme::TEXT),
    );
    ui.add_space(8.0);
    match view {
        PanelView::Choose { groups, refusal } => {
            ui.label(
                theme::secondary(
                    "Watch this server's titles in step with other people: \
                     a play, pause or seek by anyone moves everyone.",
                )
                .font(theme::body()),
            );
            ui.add_space(12.0);
            if let Some(refusal) = refusal {
                ui.label(RichText::new(*refusal).color(theme::DANGER));
                ui.add_space(8.0);
            }
            match groups {
                None => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(theme::secondary("Asking the server for its groups…"));
                    });
                    ui.add_space(12.0);
                    buttons(ui, &[("Close", Choice::Close)])
                }
                Some(Err(why)) => {
                    ui.label(RichText::new(why).color(theme::DANGER));
                    ui.add_space(12.0);
                    buttons(ui, &[("Close", Choice::Close)])
                }
                Some(Ok(groups)) => {
                    let mut choice = buttons(
                        ui,
                        &[("Start a group", Choice::Start), ("Close", Choice::Close)],
                    );
                    ui.add_space(12.0);
                    if groups.is_empty() {
                        ui.label(theme::secondary("No groups on this server yet."));
                    }
                    for group in groups.iter() {
                        if join_row(ui, group) {
                            choice = Some(Choice::Join(group.id.clone()));
                        }
                    }
                    choice
                }
            }
        }
        PanelView::Joining => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(theme::secondary("Joining…"));
            });
            ui.add_space(12.0);
            buttons(ui, &[("Close", Choice::Close)])
        }
        PanelView::Title {
            people,
            ticked,
            rooms,
            refusal,
        } => title(ui, *people, ticked, rooms, *refusal),
        PanelView::Rooms { rooms, refusal } => all_rooms(ui, *rooms, *refusal),
        PanelView::In {
            name,
            members,
            state,
            invite,
        } => {
            ui.label(RichText::new(*name).font(theme::body()).color(theme::TEXT));
            ui.label(theme::secondary(*state).font(theme::meta()));
            ui.add_space(6.0);
            ui.label(theme::secondary(members.join(", ")).font(theme::meta()));
            ui.add_space(10.0);
            let Some(invite) = invite else {
                ui.label(
                    theme::secondary("Press Play to start a title for everyone in the group.")
                        .font(theme::body()),
                );
                ui.add_space(12.0);
                return buttons(
                    ui,
                    &[("Leave group", Choice::Leave), ("Close", Choice::Close)],
                );
            };
            let mut choice = pick(ui, invite.people, invite.ticked, invite.invited);
            ui.add_space(12.0);
            let row = if invite.ticked.is_empty() {
                vec![("Leave room", Choice::Leave), ("Close", Choice::Close)]
            } else {
                vec![
                    ("Invite", Choice::Invite),
                    ("Leave room", Choice::Leave),
                    ("Close", Choice::Close),
                ]
            };
            choice = buttons(ui, &row).or(choice);
            choice
        }
    }
}

/// A Plex title's panel: who to invite, Start, and the rooms already made
/// for it.
fn title(
    ui: &mut egui::Ui,
    people: Option<&Result<Vec<Person>, String>>,
    ticked: &[u64],
    rooms: &[&Room],
    refusal: Option<&str>,
) -> Option<Choice> {
    ui.label(
        theme::secondary(
            "Start a room for this title and invite people: a play, pause or seek \
             by anyone moves everyone, here and in Plex's web app.",
        )
        .font(theme::body()),
    );
    ui.add_space(6.0);
    ui.label(
        theme::secondary(
            "Everyone you invite needs access to this server to play the title; \
             Plex does not check.",
        )
        .font(theme::meta()),
    );
    ui.add_space(12.0);
    if let Some(refusal) = refusal {
        ui.label(RichText::new(refusal).color(theme::DANGER));
        ui.add_space(8.0);
    }
    // The buttons come before a list of friends that can run long.
    let mut choice = buttons(
        ui,
        &[("Start a room", Choice::Create), ("Close", Choice::Close)],
    );
    ui.add_space(14.0);
    if !rooms.is_empty() {
        ui.label(caption("ROOMS FOR THIS TITLE"));
        ui.add_space(4.0);
        for room in rooms {
            choice = room_row(ui, room).or(choice);
        }
        ui.add_space(10.0);
    }
    ui.label(
        theme::secondary("Tick who to invite; you can invite more from the room.")
            .font(theme::meta()),
    );
    ui.add_space(8.0);
    pick(ui, people, ticked, &[]).or(choice)
}

/// Every room on the account, each with Join where this server has its
/// title, and Remove.
fn all_rooms(
    ui: &mut egui::Ui,
    rooms: Option<&Result<Vec<Room>, String>>,
    refusal: Option<&str>,
) -> Option<Choice> {
    if let Some(refusal) = refusal {
        ui.label(RichText::new(refusal).color(theme::DANGER));
        ui.add_space(8.0);
    }
    let mut choice = None;
    match rooms {
        None => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(theme::secondary("Looking for rooms…"));
            });
        }
        Some(Err(why)) => {
            ui.label(RichText::new(why).color(theme::DANGER));
        }
        Some(Ok(rooms)) if rooms.is_empty() => {
            ui.label(theme::secondary("You are in no rooms."));
        }
        Some(Ok(rooms)) => {
            for room in rooms {
                choice = room_row(ui, room).or(choice);
            }
        }
    }
    ui.add_space(12.0);
    buttons(ui, &[("Close", Choice::Close)]).or(choice)
}

/// The people to tick, Home first, leaving out `invited`.
fn pick(
    ui: &mut egui::Ui,
    people: Option<&Result<Vec<Person>, String>>,
    ticked: &[u64],
    invited: &[u64],
) -> Option<Choice> {
    let people = match people {
        None => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(theme::secondary("Finding people to invite…"));
            });
            return None;
        }
        Some(Err(why)) => {
            ui.label(RichText::new(why).color(theme::DANGER));
            return None;
        }
        Some(Ok(people)) => people,
    };
    let mut choice = None;
    for (home, heading) in [(true, "YOUR HOME"), (false, "FRIENDS")] {
        let group: Vec<&Person> = people
            .iter()
            .filter(|person| person.home == home && !invited.contains(&person.id))
            .collect();
        if group.is_empty() {
            continue;
        }
        ui.label(caption(heading));
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            for person in group {
                let on = ticked.contains(&person.id);
                if chips::toggle(ui, &person.name, on, "Invite") {
                    choice = Some(Choice::Tick(person.id));
                }
            }
        });
        ui.add_space(8.0);
    }
    if people.iter().all(|person| invited.contains(&person.id)) {
        ui.label(theme::secondary("Nobody else to invite."));
    }
    choice
}

/// One room: its title and who is in it, with Join and Remove at the end.
fn room_row(ui: &mut egui::Ui, room: &Room) -> Option<Choice> {
    let [height, pad, ..] = BUTTON;
    let width = |text: &str| {
        let label =
            ui.painter()
                .layout_no_wrap(text.to_owned(), theme::semibold(15.0), theme::TEXT);
        pad + label.rect.width() + pad
    };
    let remove_width = width("Remove");
    // A room can be joined where its title can be played: here, or on
    // another of the account's servers, which joining connects to.
    let joinable = room.rating_key.is_some() || room.elsewhere.is_some();
    let join_width = if joinable { width("Join") + 12.0 } else { 0.0 };
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let mut text = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(Rect::from_min_max(
                row.min,
                pos2(row.max.x - join_width - remove_width - 12.0, row.max.y),
            ))
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    text.add(egui::Label::new(RichText::new(&room.title).color(theme::TEXT)).truncate());
    let mut line = if room.people.is_empty() {
        "Only you".to_owned()
    } else {
        room.people.join(", ")
    };
    match &room.elsewhere {
        Some(server) => line.push_str(&format!(" · on {}", server.name)),
        None if room.rating_key.is_none() => {
            line.push_str(" · on a server this account cannot reach");
        }
        None => {}
    }
    text.add(egui::Label::new(theme::secondary(line).font(theme::meta())).truncate());
    let mut choice = None;
    let mut x = row.max.x - join_width - remove_width;
    if joinable {
        let id = ui.id().with(("join", &room.id));
        if overlay::secondary(ui, pos2(x, row.min.y), id, None, "Join", true).clicked() {
            choice = Some(Choice::Enter(room.id.clone()));
        }
        x += join_width;
    }
    let id = ui.id().with(("remove", &room.id));
    if overlay::secondary(ui, pos2(x, row.min.y), id, None, "Remove", true).clicked() {
        choice = Some(Choice::Remove(room.id.clone()));
    }
    ui.add_space(6.0);
    choice
}

fn caption(text: &str) -> RichText {
    RichText::new(text)
        .font(theme::caption())
        .color(theme::SECONDARY)
}

/// A row of the detail page's frosted buttons, spaced as its own are.
fn buttons(ui: &mut egui::Ui, row: &[(&str, Choice)]) -> Option<Choice> {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), BUTTON[0]), Sense::hover());
    let mut x = rect.min.x;
    let mut chosen = None;
    for (label, choice) in row {
        let id = ui.id().with(label);
        let button = overlay::secondary(ui, pos2(x, rect.min.y), id, None, label, true);
        if button.clicked() {
            chosen = Some(choice.clone());
        }
        x = button.rect.max.x + 12.0;
    }
    chosen
}

/// One group the server listed, with its Join button at the row's end.
fn join_row(ui: &mut egui::Ui, group: &Listed) -> bool {
    let [height, pad, ..] = BUTTON;
    let label = ui
        .painter()
        .layout_no_wrap("Join".to_owned(), theme::semibold(15.0), theme::TEXT);
    let join_width = pad + label.rect.width() + pad;
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let mut text = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(Rect::from_min_max(
                row.min,
                pos2(row.max.x - join_width - 12.0, row.max.y),
            ))
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    text.add(egui::Label::new(RichText::new(&group.name).color(theme::TEXT)).truncate());
    let line = format!("{} · {}", group.members.join(", "), group.state.words());
    text.add(egui::Label::new(theme::secondary(line).font(theme::meta())).truncate());
    let origin = pos2(row.max.x - join_width, row.min.y);
    let join = overlay::secondary(ui, origin, ui.id().with(&group.id), None, "Join", true);
    ui.add_space(6.0);
    join.clicked()
}

/// One short notice in a pill at the top of the window.
pub(crate) fn notice(ctx: &egui::Context, text: &str) {
    egui::Area::new(Id::new("together-notice"))
        .anchor(Align2::CENTER_TOP, vec2(0.0, 24.0))
        .order(egui::Order::Foreground)
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(GLASS)
                .stroke(Stroke::new(1.0, Color32::from_white_alpha(60)))
                .corner_radius(theme::CARD_RADIUS)
                .inner_margin(egui::Margin::symmetric(16, 8))
                .show(ui, |ui| {
                    // One line: the area would otherwise wrap at its first width.
                    ui.add(
                        egui::Label::new(
                            RichText::new(text).font(theme::body()).color(theme::TEXT),
                        )
                        .wrap_mode(egui::TextWrapMode::Extend),
                    );
                });
        });
}
