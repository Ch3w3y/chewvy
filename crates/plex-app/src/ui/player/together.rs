//! The player's side of watching together: the group button's head count and
//! the panel it opens.

use egui::{pos2, vec2, Align2, Color32, CursorIcon, Rect, Sense};

use crate::ui::{chips, theme};

/// What the player shows of the group it follows.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Badge {
    /// The group's name.
    pub name: String,
    /// Who is in it.
    pub members: Vec<String>,
    /// Where it is, in words.
    pub state: &'static str,
    /// For a Plex room: who it can still invite.
    pub guests: Option<Guests>,
}

/// Who a Plex room can still invite.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Guests {
    /// Not listed yet.
    Finding,
    /// Why they could not be listed.
    Unknown(String),
    /// Everyone not invited yet, the account's Home first.
    Listed(Vec<Guest>),
}

/// Someone a Plex room can still invite.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Guest {
    /// The plex.tv user id.
    pub id: u64,
    /// The name to show.
    pub name: String,
    /// A member of the account's Plex Home, rather than a friend.
    pub home: bool,
    /// Ticked to invite.
    pub ticked: bool,
}

/// How tall the list of people to invite may grow before it scrolls, so the
/// panel's buttons stay in view under a long list of friends.
const GUESTS_HEIGHT: f32 = 176.0;

/// A choice made in the group panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Pick {
    /// Leave the group and keep playing here alone.
    Leave,
    /// Tick or untick someone to invite.
    Tick(u64),
    /// Invite whoever is ticked.
    Invite,
}

/// The number of people, in a dot on the group button's corner.
pub(super) fn count(root: &egui::Ui, button: Rect, people: usize) {
    let center = button.right_top() + vec2(-6.0, 6.0);
    let painter = root.painter();
    painter.circle_filled(center, 9.0, theme::ACCENT);
    painter.text(
        center,
        Align2::CENTER_CENTER,
        people.to_string(),
        theme::caption(),
        theme::PRIMARY_TEXT,
    );
}

/// The group panel: its name, where it is, who is in it, who a room can
/// still invite, and Leave.
pub(super) fn panel(ui: &mut egui::Ui, group: &Badge) -> Option<Pick> {
    ui.add_space(2.0);
    ui.label(
        egui::RichText::new("WATCHING TOGETHER")
            .font(theme::caption())
            .color(theme::SECONDARY),
    );
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new(&group.name)
            .font(theme::body())
            .color(theme::TEXT),
    );
    ui.label(theme::secondary(group.state).font(theme::meta()));
    ui.add_space(6.0);
    for name in &group.members {
        ui.label(
            egui::RichText::new(name)
                .font(theme::body())
                .color(theme::TEXT),
        );
    }
    ui.add_space(8.0);
    ui.label(theme::secondary("Play, pause and seek here move everyone.").font(theme::caption()));
    ui.add_space(8.0);
    let mut pick = group.guests.as_ref().and_then(|guests| invite(ui, guests));
    let leave = if group.guests.is_some() {
        "Leave room"
    } else {
        "Leave group"
    };
    if wide_button(ui, leave) {
        pick = Some(Pick::Leave);
    }
    pick
}

/// The people a room can still invite, to tick, and Invite once one is.
fn invite(ui: &mut egui::Ui, guests: &Guests) -> Option<Pick> {
    let guests = match guests {
        Guests::Finding => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(theme::secondary("Finding people to invite…").font(theme::caption()));
            });
            ui.add_space(8.0);
            return None;
        }
        Guests::Unknown(why) => {
            ui.label(theme::secondary(why).font(theme::caption()));
            ui.add_space(8.0);
            return None;
        }
        // Everyone is in the room already: nothing to offer.
        Guests::Listed(guests) if guests.is_empty() => return None,
        Guests::Listed(guests) => guests,
    };
    let mut pick = None;
    egui::ScrollArea::vertical()
        .id_salt("room-guests")
        .max_height(GUESTS_HEIGHT)
        // A long list takes its full height whatever room the panel had
        // when it opened: it opens before the people are listed.
        .min_scrolled_height(GUESTS_HEIGHT)
        .show(ui, |ui| {
            for (home, heading) in [(true, "INVITE FROM YOUR HOME"), (false, "INVITE A FRIEND")] {
                let listed: Vec<&Guest> =
                    guests.iter().filter(|guest| guest.home == home).collect();
                if listed.is_empty() {
                    continue;
                }
                ui.label(
                    egui::RichText::new(heading)
                        .font(theme::caption())
                        .color(theme::SECONDARY),
                );
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    for guest in listed {
                        if chips::toggle(ui, &guest.name, guest.ticked, "Invite to this room") {
                            pick = Some(Pick::Tick(guest.id));
                        }
                    }
                });
                ui.add_space(8.0);
            }
        });
    ui.add_space(4.0);
    if guests.iter().any(|guest| guest.ticked) {
        if wide_button(ui, "Invite") {
            pick = Some(Pick::Invite);
        }
        ui.add_space(6.0);
    }
    pick
}

/// One full-width button of the panel; `true` when clicked.
fn wide_button(ui: &mut egui::Ui, label: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
    let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
    let fill = if resp.hovered() {
        theme::PRIMARY_BG
    } else {
        Color32::from_white_alpha(26)
    };
    let ink = if resp.hovered() {
        theme::PRIMARY_TEXT
    } else {
        theme::TEXT
    };
    ui.painter().rect_filled(rect, 8.0, fill);
    ui.painter().text(
        pos2(rect.center().x, rect.center().y),
        Align2::CENTER_CENTER,
        label,
        theme::body(),
        ink,
    );
    resp.clicked()
}
