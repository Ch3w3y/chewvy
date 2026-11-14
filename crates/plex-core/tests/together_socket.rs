//! The Watch Together sync socket against a mock server: what the handshake
//! carries, how frames go both ways, how a read waits, and how it ends.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::time::{Duration, Instant};

use plex_core::error::CoreError;
use plex_core::together::exchange::{Exchange, Reading};
use plex_core::together::message::{Message, Outgoing, UserName};
use plex_core::together::socket::SyncSocket;
use serde_json::Value;

#[path = "mock/socket.rs"]
mod mock_socket;

const STATE: &str = r#"{"State": {"ping": {"latencyCalculation": 1700000000.5, "serverRtt": 0}, "playstate": {"position": 12.5, "paused": false, "doSeek": false, "setBy": "{\"deviceIdentifier\":\"other-device-0001\",\"deviceName\":\"Other\",\"userID\":\"1000001\"}"}}}"#;

fn user() -> UserName {
    UserName {
        device_identifier: "web-device-0001".to_owned(),
        device_name: "Chewvy".to_owned(),
        user_id: "1000002".to_owned(),
    }
}

#[test]
fn a_session_says_hello_answers_a_state_and_closes() {
    let (url, server) = mock_socket::serve(|ws| {
        let (hello, _) = mock_socket::collect(ws, Duration::from_millis(500));
        mock_socket::send(
            ws,
            r#"{"Hello": {"username": "x", "room": {"name": "fake-room-0001"}, "version": "1.6.4"}}"#,
        );
        mock_socket::send(ws, "not json");
        mock_socket::send(ws, r#"{"Chat": {"message": "hi"}}"#);
        mock_socket::send(ws, STATE);
        let (rest, closed) = mock_socket::collect(ws, Duration::from_secs(3));
        (hello, rest, closed)
    });
    let mut socket = SyncSocket::connect(&url).expect("connects");
    let user = user();
    socket
        .send(&Outgoing::Hello {
            room: "fake-room-0001",
            user: &user,
        })
        .unwrap();

    let mut messages = Vec::new();
    while messages.len() < 4 {
        messages.push(
            socket
                .read(Duration::from_secs(2))
                .unwrap()
                .expect("a frame"),
        );
    }
    assert!(matches!(messages[0], Message::Hello(_)));
    assert!(matches!(messages[1], Message::Malformed { .. }));
    assert!(matches!(messages[2], Message::Other { .. }));
    let Message::State(state) = &messages[3] else {
        panic!("{:?}", messages[3]);
    };
    let reading = Reading {
        position: 12.0,
        paused: false,
        play_pause_pending: false,
        seek_pending: false,
    };
    let answer = Exchange::new(&user.device_identifier).on_state(state, reading, 1.0);
    socket.send(&Outgoing::State(&answer.reply)).unwrap();
    socket.send(&Outgoing::List).unwrap();
    socket.close().expect("closes");

    let (handshake, (hello, rest, closed)) = server.join().unwrap();
    assert_eq!(handshake.path_and_query, "/ws");
    for (name, value) in &handshake.headers {
        assert!(
            !name.to_ascii_lowercase().starts_with("x-plex") && name != "authorization",
            "no Plex header or credential on this socket: {name}"
        );
        assert!(!value.contains("Token"), "{name}");
    }
    assert_eq!(hello.len(), 1);
    let hello: Value = serde_json::from_str(&hello[0]).unwrap();
    assert_eq!(hello["Hello"]["room"]["name"], "fake-room-0001");
    assert_eq!(hello["Hello"]["username"], user.to_wire());
    assert_eq!(rest.len(), 2);
    let reply: Value = serde_json::from_str(&rest[0]).unwrap();
    assert_eq!(reply["State"]["playstate"]["paused"], false);
    assert_eq!(
        reply["State"]["ping"]["latencyCalculation"],
        1_700_000_000.5
    );
    assert_eq!(rest[1], r#"{"List":{}}"#);
    assert!(closed, "the client closed the socket");
}

#[test]
fn a_quiet_read_waits_its_timeout_and_a_lost_socket_is_a_transport_error() {
    let (url, server) = mock_socket::serve(|ws| {
        std::thread::sleep(Duration::from_millis(600));
        drop(ws.close(None));
        mock_socket::collect(ws, Duration::from_millis(500))
    });
    let mut socket = SyncSocket::connect(&url).unwrap();
    let started = Instant::now();
    assert_eq!(socket.read(Duration::from_millis(300)).unwrap(), None);
    assert!(started.elapsed() >= Duration::from_millis(300));
    let deadline = Instant::now() + Duration::from_secs(3);
    let outcome = loop {
        match socket.read(Duration::from_millis(200)) {
            Ok(None) if Instant::now() < deadline => {}
            other => break other,
        }
    };
    assert_eq!(outcome, Err(CoreError::Transport));
    server.join().unwrap();
}

#[test]
fn a_refused_upgrade_and_unusable_urls_are_typed_errors() {
    let (url, server) = mock_socket::refuse(403);
    assert_eq!(
        SyncSocket::connect(&url).unwrap_err(),
        CoreError::HttpStatus { status: 403 }
    );
    server.join().unwrap();

    for url in [
        "",
        "not a url",
        "https://sync.example.invalid/ws",
        "wss://user:secret@sync.example.invalid/ws",
    ] {
        assert_eq!(
            SyncSocket::connect(url).unwrap_err(),
            CoreError::InvalidInput,
            "{url:?}"
        );
    }
}
