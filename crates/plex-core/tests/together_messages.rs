//! Watch Together's Syncplay frames: every frame of a recorded web app
//! session reads, the frames this client sends match what the web app sent,
//! and Plex's names and source URIs read both ways.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use plex_core::together::message::{
    file_name, file_uri, Message, Outgoing, Set, UserEvent, UserName,
};
use plex_core::together::room::{parse_source_uri, source_uri, Source};
use serde_json::Value;

const SESSION: &str = include_str!("fixtures/together/web_session.jsonl");
const SOURCE: &str = "server://FAKEMACHINE0001/com.plexapp.plugins.library/library/metadata/1";

/// `(direction, frame text)` for every recorded frame.
fn frames() -> Vec<(String, String)> {
    SESSION
        .lines()
        .map(|line| {
            let record: Value = serde_json::from_str(line).unwrap();
            (
                record["dir"].as_str().unwrap().to_owned(),
                record["text"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

fn incoming() -> Vec<Message> {
    frames()
        .into_iter()
        .filter(|(dir, _)| dir == "in")
        .map(|(_, text)| Message::parse(&text))
        .collect()
}

fn web_user() -> UserName {
    UserName {
        device_identifier: "web-device-0001".to_owned(),
        device_name: "Chrome".to_owned(),
        user_id: "1000002".to_owned(),
    }
}

#[test]
fn every_recorded_frame_from_the_server_reads() {
    let messages = incoming();
    assert_eq!(messages.len(), 146);
    for message in &messages {
        assert!(
            !matches!(message, Message::Malformed { .. } | Message::Other { .. }),
            "{message:?}"
        );
    }
    let states = messages
        .iter()
        .filter(|m| matches!(m, Message::State(_)))
        .count();
    assert!(states > 100, "{states} states");
}

#[test]
fn hello_and_list_carry_the_room_and_its_people() {
    let messages = incoming();
    let hello = messages
        .iter()
        .find_map(|m| match m {
            Message::Hello(hello) => Some(hello.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(hello.room, "fake-room-0001");
    assert_eq!(hello.version, "1.6.4");
    assert_eq!(hello.real_version.as_deref(), Some("1.6.5"));
    assert_eq!(UserName::parse(&hello.user), Some(web_user()));

    let list = messages
        .iter()
        .find_map(|m| match m {
            Message::List(list) => Some(list.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(list.len(), 2);
    let other = list
        .iter()
        .find(|p| UserName::parse(&p.user).unwrap().device_identifier == "other-device-0001")
        .unwrap();
    assert_eq!(other.room, "fake-room-0001");
    assert_eq!(other.position, Some(0.0));
    assert_eq!(other.is_ready, Some(true));
    assert_eq!(
        file_uri(other.file.as_deref().unwrap()).as_deref(),
        Some(SOURCE)
    );
    let web = list
        .iter()
        .find(|p| UserName::parse(&p.user) == Some(web_user()))
        .unwrap();
    assert_eq!(web.file, None, "an empty file object is no file");
    assert_eq!(web.is_ready, None);
}

#[test]
fn set_frames_read_as_file_ready_leave_and_other() {
    let sets: Vec<Set> = incoming()
        .into_iter()
        .filter_map(|m| match m {
            Message::Set(set) => Some(set),
            _ => None,
        })
        .collect();
    assert!(sets.iter().any(|s| matches!(
        s,
        Set::User { event: None, file: Some(file), .. } if file_uri(file).as_deref() == Some(SOURCE)
    )));
    assert!(sets.iter().any(|s| matches!(
        s,
        Set::Ready {
            is_ready: Some(true),
            manually_initiated: false,
            ..
        }
    )));
    assert!(sets
        .iter()
        .any(|s| matches!(s, Set::Ready { is_ready: None, .. })));
    let left = sets
        .iter()
        .find_map(|s| match s {
            Set::User {
                user,
                event: Some(UserEvent::Left),
                room,
                ..
            } => Some((user.clone(), room.clone())),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        UserName::parse(&left.0).unwrap().device_identifier,
        "other-device-0001"
    );
    assert_eq!(left.1.as_deref(), Some("fake-room-0001"));
    assert!(sets
        .iter()
        .any(|s| matches!(s, Set::Other { kind } if kind == "playlistChange")));
}

#[test]
fn a_join_an_error_and_unknown_frames_never_end_the_session() {
    let joined = Message::parse(
        r#"{"Set":{"user":{"{\"deviceIdentifier\":\"d\",\"deviceName\":\"n\",\"userID\":\"1\"}":{"room":{"name":"r"},"event":{"joined":true}}}}}"#,
    );
    assert!(
        matches!(
            joined,
            Message::Set(Set::User {
                event: Some(UserEvent::Joined),
                ..
            })
        ),
        "{joined:?}"
    );
    assert_eq!(
        Message::parse(r#"{"Error":{"message":"Room is full"}}"#),
        Message::Error {
            message: "Room is full".to_owned()
        }
    );
    assert_eq!(
        Message::parse(r#"{"Chat":{"message":"hi"}}"#),
        Message::Other {
            kind: "Chat".to_owned()
        }
    );
    assert_eq!(
        Message::parse("not json"),
        Message::Malformed {
            kind: String::new()
        }
    );
    assert_eq!(
        Message::parse(r#"{"State":{"playstate":{"position":"x"}}}"#),
        Message::Malformed {
            kind: "State".to_owned()
        }
    );
    assert_eq!(
        Message::parse(r#"{"Hello":{},"List":{}}"#),
        Message::Malformed {
            kind: String::new()
        }
    );
}

#[test]
fn a_state_reads_its_ping_playstate_and_counters() {
    let Message::State(state) = Message::parse(
        r#"{"State": {"ping": {"latencyCalculation": 1700000000.5, "serverRtt": 0.02, "clientLatencyCalculation": 12.5}, "playstate": {"position": 208, "paused": true, "doSeek": false, "setBy": "{\"deviceIdentifier\":\"d\",\"deviceName\":\"n\",\"userID\":\"1\"}__"}, "ignoringOnTheFly": {"server": 1, "client": 1}}}"#,
    ) else {
        panic!("a state");
    };
    assert_eq!(state.ping.latency_calculation, Some(1_700_000_000.5));
    assert_eq!(state.ping.server_rtt, Some(0.02));
    assert_eq!(state.ping.client_latency_calculation, Some(12.5));
    let playstate = state.playstate.unwrap();
    assert!((playstate.position - 208.0).abs() < f64::EPSILON);
    assert!(playstate.paused);
    assert!(!playstate.do_seek);
    let by = UserName::parse(playstate.set_by.as_deref().unwrap()).unwrap();
    assert_eq!(
        by.device_identifier, "d",
        "trailing underscores are stripped"
    );
    let counters = state.ignoring_on_the_fly.unwrap();
    assert_eq!((counters.client, counters.server), (Some(1), Some(1)));
}

#[test]
fn frames_this_client_sends_match_the_web_apps() {
    let sent: Vec<Value> = frames()
        .into_iter()
        .filter(|(dir, _)| dir == "out")
        .map(|(_, text)| serde_json::from_str(&text).unwrap())
        .collect();
    let user = web_user();
    let ours = [
        Outgoing::Hello {
            room: "fake-room-0001",
            user: &user,
        },
        Outgoing::List,
        Outgoing::File { source_uri: SOURCE },
        Outgoing::Ready(false),
        Outgoing::Ready(true),
    ];
    for frame in ours {
        let value: Value = serde_json::from_str(&frame.to_text()).unwrap();
        assert!(sent.contains(&value), "{frame:?} was never sent as {value}");
    }
}

#[test]
fn user_names_and_file_names_read_both_ways() {
    let user = web_user();
    assert_eq!(
        user.to_wire(),
        r#"{"deviceIdentifier":"web-device-0001","deviceName":"Chrome","userID":"1000002"}"#
    );
    assert_eq!(
        UserName::parse(&format!("{}___", user.to_wire())),
        Some(user)
    );
    assert_eq!(UserName::parse("plain name"), None);

    assert_eq!(
        file_name(SOURCE),
        format!(r#"{{"ads":{{"playing":false}},"uri":"{SOURCE}"}}"#)
    );
    assert_eq!(file_uri(&file_name(SOURCE)).as_deref(), Some(SOURCE));
    assert_eq!(file_uri("movie.mkv"), None);
}

#[test]
fn source_uris_are_built_and_read() {
    assert_eq!(source_uri("FAKEMACHINE0001", "1").unwrap(), SOURCE);
    assert_eq!(
        parse_source_uri(SOURCE),
        Some(Source {
            machine_identifier: "FAKEMACHINE0001".to_owned(),
            rating_key: "1".to_owned(),
        })
    );
    for (machine, key) in [
        ("", "1"),
        ("m", ""),
        ("a/b", "1"),
        ("m", "1?x=y"),
        ("..", "1"),
    ] {
        assert!(source_uri(machine, key).is_err(), "{machine:?} {key:?}");
    }
    for uri in [
        "server://m/com.plexapp.plugins.library/library/metadata/",
        "server://m/other.provider/library/metadata/1",
        "http://m/com.plexapp.plugins.library/library/metadata/1",
        "server://m/com.plexapp.plugins.library/library/metadata/1/children",
    ] {
        assert_eq!(parse_source_uri(uri), None, "{uri}");
    }
}
