//! A Plex room as the screens run it: who the player's panel offers to
//! invite, and what joining a room on another server asks of the worker.

use std::sync::mpsc::{self, Receiver};

use super::*;
use crate::together::{Ask, Elsewhere, News, Person, Room, State};
use crate::ui::player::together::{Guest, Guests};
use crate::ui::together::Choice;

fn ui_and_sent() -> (Ui, Receiver<Cmd>) {
    let (cmd, sent) = mpsc::channel();
    let (_ev_tx, ev) = mpsc::channel();
    (Ui::new(cmd, ev), sent)
}

/// The asks about watching together sent since the last call.
fn asks(sent: &Receiver<Cmd>) -> Vec<Ask> {
    sent.try_iter()
        .filter_map(|cmd| match cmd {
            Cmd::Together(ask) => Some(ask),
            _ => None,
        })
        .collect()
}

fn room(id: &str, ids: &[u64], elsewhere: Option<(&str, &str)>) -> Room {
    Room {
        id: id.to_owned(),
        title: "Heat".to_owned(),
        rating_key: elsewhere.is_none().then(|| "42".to_owned()),
        elsewhere: elsewhere.map(|(machine, name)| Elsewhere {
            machine: machine.to_owned(),
            name: name.to_owned(),
        }),
        people: Vec::new(),
        ids: ids.to_vec(),
    }
}

fn person(id: u64, name: &str, home: bool) -> Person {
    Person {
        id,
        name: name.to_owned(),
        home,
    }
}

fn joined(ui: &mut Ui, room: &str) {
    ui.apply(Ev::Together(News::Joined {
        server: String::new(),
        room: Some(room.to_owned()),
        name: "Heat".to_owned(),
        members: Vec::new(),
        state: State::Idle,
    }));
}

fn guests(ui: &Ui) -> Option<Guests> {
    ui.together.badge().and_then(|badge| badge.guests)
}

#[test]
fn the_players_panel_offers_whoever_is_not_in_the_room_yet_and_invites_the_ticked() {
    let (mut ui, sent) = ui_and_sent();
    ui.apply(Ev::Together(News::Rooms(Ok(vec![room(
        "r1",
        &[7, 1],
        None,
    )]))));
    joined(&mut ui, "r1");
    // Joining a room asks who it can invite, for the panel in the player.
    assert_eq!(asks(&sent), [Ask::People]);
    assert_eq!(guests(&ui), Some(Guests::Finding));

    ui.apply(Ev::Together(News::People(Ok(vec![
        person(1, "Ada", true),
        person(9, "Ben", true),
        person(12, "Cy", false),
    ]))));
    let offered = |ticked| {
        Some(Guests::Listed(vec![
            Guest {
                id: 9,
                name: "Ben".to_owned(),
                home: true,
                ticked,
            },
            Guest {
                id: 12,
                name: "Cy".to_owned(),
                home: false,
                ticked: false,
            },
        ]))
    };
    // Ada is in the room already.
    assert_eq!(guests(&ui), offered(false));

    ui.room_choice(Choice::Tick(9));
    assert_eq!(guests(&ui), offered(true));
    ui.room_choice(Choice::Invite);
    assert_eq!(
        asks(&sent),
        [Ask::Invite {
            room: "r1".to_owned(),
            people: vec![9],
        }]
    );
    assert_eq!(guests(&ui), offered(false), "the tick is spent");

    // The room's list, read again, has Ben in it: he is no longer offered.
    ui.apply(Ev::Together(News::Rooms(Ok(vec![room(
        "r1",
        &[7, 1, 9],
        None,
    )]))));
    let left: Vec<u64> = match guests(&ui) {
        Some(Guests::Listed(people)) => people.iter().map(|guest| guest.id).collect(),
        _ => Vec::new(),
    };
    assert_eq!(left, [12]);
}

#[test]
fn a_jellyfin_group_offers_nobody_to_invite() {
    let (mut ui, sent) = ui_and_sent();
    ui.apply(Ev::Together(News::Joined {
        server: "loft".to_owned(),
        room: None,
        name: "Loft group".to_owned(),
        members: Vec::new(),
        state: State::Idle,
    }));
    assert!(asks(&sent).is_empty());
    assert_eq!(guests(&ui), None);
}

#[test]
fn joining_a_room_on_another_server_names_that_server_to_the_worker() {
    let (mut ui, sent) = ui_and_sent();
    ui.apply(Ev::Together(News::Rooms(Ok(vec![
        room("here", &[7, 1], None),
        room("there", &[7, 1], Some(("attic-machine", "Attic"))),
    ]))));
    ui.room_choice(Choice::Enter("here".to_owned()));
    ui.room_choice(Choice::Enter("there".to_owned()));
    assert_eq!(
        asks(&sent),
        [
            Ask::Enter {
                room: "here".to_owned(),
                server: None,
            },
            Ask::Enter {
                room: "there".to_owned(),
                server: Some("attic-machine".to_owned()),
            },
        ]
    );
    assert_eq!(ui.status, "Connecting to Attic for the room…");
}
