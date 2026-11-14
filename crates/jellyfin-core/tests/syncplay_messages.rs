//! Socket frames as a 12.4 server sent them during a two-member group,
//! read into the closed message types.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::time::Duration;

use jellyfin_core::syncplay::clock::UtcTime;
use jellyfin_core::syncplay::message::{CommandKind, GroupUpdate, Message, Update};
use jellyfin_core::syncplay::GroupState;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/live-12.4/");
const GROUP_ID: &str = "a69bbb19e2fd498595de88161eba1fbb";
const PLAYLIST_ITEM_ID: &str = "a7c11b921e6f4ca9959c05e714f6485f";
const F01: &str = "4966f0f90dd1fa4c2539dd7f319b0f57";

fn parse(name: &str) -> Message {
    let text = std::fs::read_to_string(format!("{FIXTURES}{name}")).expect("fixture");
    Message::parse(&text)
}

fn update(name: &str) -> Update {
    match parse(name) {
        Message::GroupUpdate(GroupUpdate { group_id, update }) => {
            assert_eq!(group_id, GROUP_ID);
            update
        }
        other => panic!("{name}: expected a group update, got {other:?}"),
    }
}

fn time(text: &str) -> UtcTime {
    UtcTime::parse(text).unwrap()
}

#[test]
fn keep_alive_frames() {
    assert_eq!(
        parse("ws_force_keep_alive.json"),
        Message::ForceKeepAlive {
            timeout: Duration::from_secs(60)
        }
    );
    assert_eq!(parse("ws_keep_alive.json"), Message::KeepAlive);
}

#[test]
fn unpause_is_scheduled_a_second_ahead_on_the_servers_clock() {
    let Message::Command(command) = parse("ws_command_unpause.json") else {
        panic!("expected a command");
    };
    assert_eq!(command.command, CommandKind::Unpause);
    assert_eq!(command.group_id, GROUP_ID);
    assert_eq!(command.playlist_item_id, PLAYLIST_ITEM_ID);
    assert_eq!(command.when, time("2026-11-12T22:05:11.6075097Z"));
    assert_eq!(command.emitted_at, time("2026-11-12T22:05:10.6075151Z"));
    assert_eq!(command.position(), Some(Duration::ZERO));
    let lead = command.when.seconds_since(command.emitted_at);
    assert!((0.99..=1.01).contains(&lead), "{lead}");
}

#[test]
fn every_command_kind_reads() {
    for (name, kind) in [
        ("ws_command_unpause.json", CommandKind::Unpause),
        ("ws_command_pause.json", CommandKind::Pause),
        ("ws_command_seek.json", CommandKind::Seek),
        ("ws_command_stop.json", CommandKind::Stop),
    ] {
        match parse(name) {
            Message::Command(command) => assert_eq!(command.command, kind, "{name}"),
            other => panic!("{name}: {other:?}"),
        }
    }
    let Message::Command(seek) = parse("ws_command_seek.json") else {
        panic!("expected a command");
    };
    assert_eq!(seek.position(), Some(Duration::from_secs(30)));
}

#[test]
fn joining_names_the_group_and_its_members() {
    let Update::GroupJoined(group) = update("ws_group_joined.json") else {
        panic!("expected GroupJoined");
    };
    assert_eq!(group.group_id, GROUP_ID);
    assert_eq!(group.group_name, "Movie night");
    assert_eq!(group.state, GroupState::Idle);
    assert_eq!(group.participants, vec!["peer".to_owned()]);
}

#[test]
fn members_coming_and_going() {
    assert_eq!(
        update("ws_user_joined.json"),
        Update::UserJoined("guest".to_owned())
    );
    assert_eq!(
        update("ws_user_left.json"),
        Update::UserLeft("guest".to_owned())
    );
    // The one place the server writes a group id hyphenated.
    assert_eq!(
        update("ws_group_left.json"),
        Update::GroupLeft("a69bbb19-e2fd-4985-95de-88161eba1fbb".to_owned())
    );
}

#[test]
fn a_missing_group_is_reported_on_the_socket() {
    let Message::GroupUpdate(missing) = parse("ws_group_does_not_exist.json") else {
        panic!("expected a group update");
    };
    assert_eq!(missing.update, Update::GroupDoesNotExist(String::new()));
}

#[test]
fn a_state_update_names_the_state_and_its_reason() {
    let Update::StateUpdate(state) = update("ws_state_update.json") else {
        panic!("expected StateUpdate");
    };
    assert_eq!(state.state, GroupState::Playing);
    assert_eq!(state.reason, "Ready");
}

#[test]
fn the_queue_names_the_entry_to_load() {
    let Update::PlayQueue(queue) = update("ws_play_queue.json") else {
        panic!("expected PlayQueue");
    };
    assert_eq!(queue.reason, "NewPlaylist");
    assert_eq!(queue.start_position_ticks, 0);
    assert!(!queue.is_playing);
    assert_eq!(queue.shuffle_mode, "Sorted");
    assert_eq!(queue.repeat_mode, "RepeatNone");
    let entry = queue.playing_item().expect("an entry to play");
    assert_eq!(entry.item_id, F01);
    assert_eq!(entry.playlist_item_id, PLAYLIST_ITEM_ID);
}

#[test]
fn update_types_this_client_does_not_use_are_kept_by_name() {
    let text = r#"{"MessageType":"SyncPlayGroupUpdate","Data":{"GroupId":"g","Type":"ChatMessage","Data":"hi"}}"#;
    let Message::GroupUpdate(update) = Message::parse(text) else {
        panic!("expected a group update");
    };
    assert_eq!(
        update.update,
        Update::Other {
            kind: "ChatMessage".to_owned()
        }
    );
}

#[test]
fn hand_written_refusals_read_as_text() {
    for (kind, expected) in [
        ("NotInGroup", Update::NotInGroup(String::new())),
        (
            "LibraryAccessDenied",
            Update::LibraryAccessDenied(String::new()),
        ),
    ] {
        let text = format!(
            r#"{{"MessageType":"SyncPlayGroupUpdate","Data":{{"GroupId":"g","Type":"{kind}","Data":""}}}}"#
        );
        let Message::GroupUpdate(update) = Message::parse(&text) else {
            panic!("expected a group update");
        };
        assert_eq!(update.update, expected);
    }
}

#[test]
fn no_frame_fails_to_parse() {
    assert_eq!(
        Message::parse(r#"{"MessageType":"UserDataChanged","Data":{"UserId":"u"}}"#),
        Message::Other {
            message_type: "UserDataChanged".to_owned()
        }
    );
    for (text, message_type) in [
        ("", ""),
        ("[]", ""),
        (r#"{"Data":1}"#, ""),
        (
            r#"{"MessageType":"ForceKeepAlive","Data":"soon"}"#,
            "ForceKeepAlive",
        ),
        (
            r#"{"MessageType":"SyncPlayGroupUpdate","Data":{"GroupId":"g","Type":"StateUpdate","Data":{"State":"Dancing","Reason":"Ready"}}}"#,
            "SyncPlayGroupUpdate",
        ),
        (
            r#"{"MessageType":"SyncPlayCommand","Data":{"GroupId":"g","PlaylistItemId":"p","When":"yesterday","PositionTicks":0,"Command":"Pause","EmittedAt":"2026-11-12T22:05:10Z"}}"#,
            "SyncPlayCommand",
        ),
    ] {
        assert_eq!(
            Message::parse(text),
            Message::Malformed {
                message_type: message_type.to_owned()
            },
            "{text}"
        );
    }
}
