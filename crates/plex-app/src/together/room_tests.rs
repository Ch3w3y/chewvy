use plex_core::together::exchange::{Exchange, Target};
use plex_core::together::message::{IgnoringOnTheFly, PlayState, State};
use plex_core::together::people::{Friend, HomeUser};
use plex_core::together::room::{self, RoomUser};

use super::clock::GroupTime;
use super::follow::{Command, Request};
use super::plex::{Pulse, Steer};
use super::rooms::{invitable, rows};

const ME: &str = "this-device";

fn target(position: f64, paused: bool, do_seek: bool) -> Target {
    Target {
        position,
        paused,
        do_seek,
        set_by: None,
    }
}

fn pulse(position: f64, paused: bool, stalled: bool, at: f64) -> Pulse {
    Pulse {
        position,
        paused,
        stalled,
        at,
    }
}

#[test]
fn the_first_state_points_the_follower_at_the_group() {
    let mut steer = Steer::default();
    assert_eq!(
        steer.follow(&target(12.0, true, false), 100.0),
        Some(Command::PauseAt { position: 12.0 })
    );
    let mut steer = Steer::default();
    // A play starts from now: the state's position already allows for its
    // time on the way.
    assert_eq!(
        steer.follow(&target(12.0, false, false), 100.0),
        Some(Command::PlayAt {
            at: GroupTime(100.0),
            position: 12.0
        })
    );
}

#[test]
fn a_playing_group_where_it_was_expected_needs_no_command() {
    let mut steer = Steer::default();
    steer.follow(&target(10.0, false, false), 0.0);
    for second in 1..30 {
        let now = f64::from(second);
        // Jitter in the state's timing stays inside the band.
        let wobble = if second % 2 == 0 { 0.06 } else { -0.06 };
        assert_eq!(
            steer.follow(&target(10.0 + now + wobble, false, false), now),
            None
        );
    }
}

#[test]
fn a_playing_group_that_moved_off_is_followed_again() {
    let mut steer = Steer::default();
    steer.follow(&target(10.0, false, false), 0.0);
    // The slowest participant sets the group's position: it ran 0.3 s behind.
    assert_eq!(
        steer.follow(&target(14.7, false, false), 5.0),
        Some(Command::PlayAt {
            at: GroupTime(5.0),
            position: 14.7
        })
    );
    // From then on the new line is the one expected.
    assert_eq!(steer.follow(&target(15.7, false, false), 6.0), None);
}

#[test]
fn pause_and_play_from_the_group_are_followed() {
    let mut steer = Steer::default();
    steer.follow(&target(10.0, false, false), 0.0);
    assert_eq!(
        steer.follow(&target(13.0, true, false), 3.0),
        Some(Command::PauseAt { position: 13.0 })
    );
    // A paused group's position moving a little is not a move.
    assert_eq!(steer.follow(&target(13.2, true, false), 4.0), None);
    assert_eq!(
        steer.follow(&target(13.0, false, false), 9.0),
        Some(Command::PlayAt {
            at: GroupTime(9.0),
            position: 13.0
        })
    );
}

#[test]
fn a_seek_is_followed_even_when_close() {
    let mut steer = Steer::default();
    steer.follow(&target(40.0, true, false), 0.0);
    assert_eq!(
        steer.follow(&target(40.2, true, true), 1.0),
        Some(Command::SeekTo { position: 40.2 })
    );
    steer.follow(&target(40.2, false, false), 2.0);
    assert_eq!(
        steer.follow(&target(41.25, false, true), 3.0),
        Some(Command::PlayAt {
            at: GroupTime(3.0),
            position: 41.25
        })
    );
}

#[test]
fn the_persons_own_pause_moves_this_player_at_once_and_waits_for_the_next_answer() {
    let mut steer = Steer::default();
    steer.follow(&target(10.0, false, false), 0.0);
    let command = steer.request(Request::Pause, 15.0, 5.0);
    assert_eq!(command, Command::PauseAt { position: 15.0 });
    let reading = steer.reading(Some(pulse(15.0, false, false, 5.0)), 5.4);
    assert!(reading.paused && reading.play_pause_pending && !reading.seek_pending);
    assert!((reading.position - 15.0).abs() < 1e-9);
    steer.sent();
    // Once sent, the player's own reading speaks again.
    let reading = steer.reading(Some(pulse(15.0, true, false, 5.5)), 6.0);
    assert!(reading.paused && !reading.play_pause_pending);
}

#[test]
fn the_persons_own_seek_keeps_the_groups_pause_and_moves_on_while_playing() {
    let mut paused = Steer::default();
    paused.follow(&target(10.0, true, false), 0.0);
    assert_eq!(
        paused.request(Request::Seek { position: 90.0 }, 10.0, 1.0),
        Command::SeekTo { position: 90.0 }
    );
    let reading = paused.reading(None, 3.0);
    assert!(reading.paused && reading.seek_pending && !reading.play_pause_pending);
    assert!((reading.position - 90.0).abs() < 1e-9);

    let mut playing = Steer::default();
    playing.follow(&target(10.0, false, false), 0.0);
    assert_eq!(
        playing.request(Request::Seek { position: 90.0 }, 12.0, 2.0),
        Command::PlayAt {
            at: GroupTime(2.0),
            position: 90.0
        }
    );
    let reading = playing.reading(None, 2.5);
    assert!(!reading.paused && reading.seek_pending);
    assert!((reading.position - 90.5).abs() < 1e-9);
}

#[test]
fn a_play_and_a_seek_before_the_next_answer_go_out_together() {
    let mut steer = Steer::default();
    steer.follow(&target(10.0, true, false), 0.0);
    steer.request(Request::Unpause, 10.0, 1.0);
    steer.request(Request::Seek { position: 50.0 }, 10.2, 1.2);
    let reading = steer.reading(None, 1.5);
    assert!(!reading.paused && reading.play_pause_pending && reading.seek_pending);
    assert!((reading.position - 50.3).abs() < 1e-9);
}

#[test]
fn the_reading_answered_is_where_the_player_is_going() {
    let mut steer = Steer::default();
    // Before anything is known: paused at the start.
    let reading = steer.reading(None, 0.0);
    assert!(reading.paused && reading.position == 0.0);
    steer.follow(&target(30.0, false, false), 10.0);
    // No player yet: where the group was last seen, moved on.
    assert!((steer.reading(None, 11.0).position - 31.0).abs() < 1e-9);
    // A playing player read 0.25 s ago has moved on since.
    let reading = steer.reading(Some(pulse(31.0, false, false, 11.0)), 11.25);
    assert!((reading.position - 31.25).abs() < 1e-9 && !reading.paused);
    // Mid-seek the player still reads where it was.
    let reading = steer.reading(Some(pulse(2.0, false, true, 11.0)), 12.0);
    assert!((reading.position - 32.0).abs() < 1e-9);
}

#[test]
fn an_own_change_leaves_with_the_next_answer_and_its_echo_is_not_followed() {
    let mut steer = Steer::default();
    let mut exchange = Exchange::new(ME);
    let playing = |position: f64| State {
        playstate: Some(PlayState {
            position,
            paused: false,
            do_seek: false,
            set_by: None,
        }),
        ..State::default()
    };
    let answer = exchange.on_state(&playing(10.0), steer.reading(None, 0.0), 0.0);
    let target = answer.follow.expect("the group's state");
    steer.follow(&target, 0.0);
    steer.request(Request::Pause, 10.5, 0.5);
    let answer = exchange.on_state(&playing(11.0), steer.reading(None, 1.0), 1.0);
    assert!(answer.sent_change && answer.follow.is_none());
    assert!(answer.reply.playstate.paused);
    assert_eq!(answer.reply.playstate.position, 10.5);
    assert_eq!(
        answer.reply.ignoring_on_the_fly.and_then(|i| i.client),
        Some(1)
    );
    steer.sent();
    // The server imposes the pause on everyone, naming this device.
    let echo = State {
        playstate: Some(PlayState {
            position: 10.5,
            paused: true,
            do_seek: false,
            set_by: Some(
                plex_core::together::message::UserName {
                    device_identifier: ME.into(),
                    device_name: "Chewvy".into(),
                    user_id: "1".into(),
                }
                .to_wire(),
            ),
        }),
        ignoring_on_the_fly: Some(IgnoringOnTheFly {
            server: Some(1),
            client: Some(1),
        }),
        ..State::default()
    };
    let reading = steer.reading(Some(pulse(10.5, true, false, 1.5)), 2.0);
    let answer = exchange.on_state(&echo, reading, 2.0);
    assert!(answer.follow.is_none());
}

fn room_user(id: u64, title: &str, username: &str) -> RoomUser {
    RoomUser {
        id,
        uuid: String::new(),
        title: title.into(),
        username: username.into(),
        thumb: None,
    }
}

fn service_room(id: &str, machine: &str, key: &str, created_by: u64) -> room::Room {
    room::Room {
        id: id.into(),
        title: "A film".into(),
        source_uri: room::source_uri(machine, key).expect("a source"),
        created_by: Some(created_by),
        starts_at: None,
        ends_at: None,
        updated_at: None,
        syncplay_host: "sync.example".into(),
        syncplay_port: Some(7776),
        users: vec![
            room_user(7, "Guest", ""),
            room_user(1, "Owner", "owner"),
            room_user(9, "", "friend9"),
        ],
    }
}

#[test]
fn a_room_names_its_title_only_on_the_connected_server_and_its_people_creator_first() {
    let rooms = [
        service_room("r1", "here", "42", 1),
        service_room("r2", "elsewhere", "42", 7),
    ];
    let listed = rows(&rooms, Some("here"), &[], Some(7));
    assert_eq!(listed[0].rating_key.as_deref(), Some("42"));
    assert_eq!(listed[0].people, ["Owner", "friend9"]);
    assert_eq!(listed[0].ids, [7, 1, 9]);
    assert_eq!(listed[1].rating_key, None);
    // This account's own room lists the others.
    assert_eq!(listed[1].people, ["Owner", "friend9"]);
    assert!(rows(&rooms, None, &[], None)
        .iter()
        .all(|r| r.rating_key.is_none()));
}

#[test]
fn a_room_on_another_of_the_accounts_servers_names_that_server() {
    let rooms = [
        service_room("r1", "here", "42", 1),
        service_room("r2", "attic", "42", 1),
        service_room("r3", "a-friends", "42", 1),
    ];
    let servers = [
        ("here".to_owned(), "Den".to_owned()),
        ("attic".to_owned(), "Attic".to_owned()),
    ];
    let listed = rows(&rooms, Some("here"), &servers, Some(7));
    // The connected server's room is joined as it is.
    assert_eq!(listed[0].elsewhere, None);
    // Another of the account's servers is named, and joining connects to it;
    // the same rating key there is a different title, so none is given.
    assert_eq!(listed[1].rating_key, None);
    let attic = listed[1]
        .elsewhere
        .as_ref()
        .map(|s| (s.machine.as_str(), s.name.as_str()));
    assert_eq!(attic, Some(("attic", "Attic")));
    // A server the account does not have cannot be joined from here.
    assert_eq!(
        (&listed[2].rating_key, &listed[2].elsewhere),
        (&None, &None)
    );
}

fn home(id: u64, title: &str, admin: bool) -> HomeUser {
    HomeUser {
        id,
        uuid: String::new(),
        title: title.into(),
        restricted: !admin,
        admin,
        thumb: None,
    }
}

fn friend(id: u64, name: &str) -> Friend {
    Friend {
        id,
        uuid: String::new(),
        username: name.to_lowercase(),
        display_name: name.into(),
        avatar: None,
    }
}

#[test]
fn people_to_invite_are_the_home_first_then_friends_never_this_account() {
    let homes = [home(1, "Owner", true), home(7, "Kid", false)];
    let friends = [friend(9, "Pal"), friend(7, "Kid again"), friend(1, "Owner")];
    let people = invitable(&homes, &friends, Some(1));
    let names: Vec<(&str, bool)> = people.iter().map(|p| (p.name.as_str(), p.home)).collect();
    assert_eq!(names, [("Kid", true), ("Pal", false)]);
    // Without this account's id the Home's administrator stands for it.
    let people = invitable(&homes, &[], None);
    assert_eq!(people.iter().map(|p| p.id).collect::<Vec<_>>(), [7]);
}
