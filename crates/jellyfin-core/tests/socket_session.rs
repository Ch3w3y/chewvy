//! The session socket against a mock server: how it is authorized, how it
//! keeps itself open, how it reads with a timeout, and how it ends.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::time::{Duration, Instant};

use jellyfin_core::socket::{Socket, SOCKET_PATH};
use jellyfin_core::syncplay::message::{CommandKind, Message, KEEP_ALIVE};
use jellyfin_core::{DeviceInfo, JfError, SecretString};

#[path = "mock/socket.rs"]
mod mock_socket;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/live-12.4/");
const TOKEN: &str = "c8f0a1d2e3b4456789abcdef01234567";

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!("{FIXTURES}{name}")).expect("fixture")
}

fn device() -> DeviceInfo {
    DeviceInfo {
        app: "Chewvy".to_owned(),
        app_version: "0.1.0".to_owned(),
        device: "test-machine".to_owned(),
        device_id: "0a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d".to_owned(),
    }
}

fn connect(base: &str) -> Result<Socket, JfError> {
    Socket::connect(base, &device(), &SecretString::new(TOKEN))
}

#[test]
fn the_token_travels_in_the_header_and_never_in_the_url() {
    let (base, server) = mock_socket::serve(|ws| {
        mock_socket::send(ws, &fixture("ws_command_unpause.json"));
        mock_socket::collect(ws, Duration::from_secs(2))
    });
    let mut socket = connect(&base).expect("connects");
    let message = socket.read(Duration::from_secs(2)).expect("reads");
    assert!(matches!(message, Some(Message::Command(_))), "{message:?}");
    socket.close().expect("closes");

    let (handshake, _) = server.join().unwrap();
    assert_eq!(handshake.path_and_query, SOCKET_PATH);
    let authorization = handshake.authorization.expect("an Authorization header");
    assert!(authorization.starts_with("MediaBrowser "));
    assert!(authorization.contains(&format!("Token=\"{TOKEN}\"")));
    assert!(authorization.contains("DeviceId=\"0a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d\""));
}

#[test]
fn keep_alive_is_sent_at_half_the_servers_timeout_and_its_echo_is_swallowed() {
    let (base, server) = mock_socket::serve(|ws| {
        // A two-second timeout asks for a keep-alive every second.
        mock_socket::send(
            ws,
            r#"{"MessageType":"ForceKeepAlive","MessageId":"a","Data":2}"#,
        );
        let (first, _) = mock_socket::collect(ws, Duration::from_millis(500));
        mock_socket::send(ws, &fixture("ws_keep_alive.json"));
        let (rest, closed) = mock_socket::collect(ws, Duration::from_secs(4));
        (first, rest, closed)
    });
    let mut socket = connect(&base).expect("connects");
    let started = Instant::now();
    let message = socket.read(Duration::from_millis(2600)).expect("reads");
    assert_eq!(message, None, "keep-alive frames are never returned");
    assert!(started.elapsed() >= Duration::from_millis(2600));
    socket.close().expect("closes");

    let (_, (first, rest, closed)) = server.join().unwrap();
    assert_eq!(
        first,
        vec![KEEP_ALIVE.to_owned()],
        "one at once on the request"
    );
    assert!(
        (2..=3).contains(&rest.len()) && rest.iter().all(|text| text == KEEP_ALIVE),
        "then one a second: {rest:?}"
    );
    assert!(closed, "the close reached the server");
}

#[test]
fn a_read_with_nothing_to_read_returns_none_on_time() {
    let (base, server) = mock_socket::serve(|ws| mock_socket::collect(ws, Duration::from_secs(3)));
    let mut socket = connect(&base).expect("connects");
    for timeout in [Duration::ZERO, Duration::from_millis(300)] {
        let started = Instant::now();
        assert_eq!(socket.read(timeout), Ok(None));
        let took = started.elapsed();
        assert!(
            took >= timeout && took < timeout + Duration::from_millis(500),
            "{took:?}"
        );
    }
    socket.close().expect("closes");
    let (_, (sent, closed)) = server.join().unwrap();
    assert!(
        sent.is_empty(),
        "nothing is sent before the server asks: {sent:?}"
    );
    assert!(closed);
}

#[test]
fn unknown_and_unreadable_frames_do_not_end_the_session() {
    let (base, server) = mock_socket::serve(|ws| {
        mock_socket::send(
            ws,
            r#"{"MessageType":"LibraryChanged","Data":{"ItemsAdded":[]}}"#,
        );
        mock_socket::send(ws, "not json");
        mock_socket::send(
            ws,
            r#"{"MessageType":"SyncPlayCommand","Data":{"Command":"Fly"}}"#,
        );
        mock_socket::send(ws, &fixture("ws_command_pause.json"));
        mock_socket::collect(ws, Duration::from_secs(2))
    });
    let mut socket = connect(&base).expect("connects");
    let mut read = || socket.read(Duration::from_secs(2)).expect("reads");
    assert_eq!(
        read(),
        Some(Message::Other {
            message_type: "LibraryChanged".to_owned()
        })
    );
    assert_eq!(
        read(),
        Some(Message::Malformed {
            message_type: String::new()
        })
    );
    assert_eq!(
        read(),
        Some(Message::Malformed {
            message_type: "SyncPlayCommand".to_owned()
        })
    );
    match read() {
        Some(Message::Command(command)) => assert_eq!(command.command, CommandKind::Pause),
        other => panic!("expected the pause, got {other:?}"),
    }
    socket.close().expect("closes");
    server.join().unwrap();
}

#[test]
fn a_refused_handshake_is_the_servers_status() {
    for (status, expected) in [
        (403, JfError::AuthRejected { status: 403 }),
        (401, JfError::AuthRejected { status: 401 }),
        (404, JfError::HttpStatus { status: 404 }),
    ] {
        let (base, server) = mock_socket::refuse(status);
        assert_eq!(connect(&base).map(drop), Err(expected));
        assert!(server.join().unwrap().authorization.is_some());
    }
}

#[test]
fn a_dropped_connection_is_a_transport_error() {
    let (base, server) = mock_socket::serve(|ws| {
        mock_socket::send(ws, &fixture("ws_command_seek.json"));
    });
    let mut socket = connect(&base).expect("connects");
    server.join().unwrap();
    assert!(matches!(
        socket.read(Duration::from_secs(2)),
        Ok(Some(Message::Command(_)))
    ));
    assert_eq!(socket.read(Duration::from_secs(2)), Err(JfError::Transport));
}

#[test]
fn a_server_that_closes_ends_the_session() {
    let (base, server) = mock_socket::serve(|ws| {
        ws.close(None).unwrap();
        mock_socket::collect(ws, Duration::from_secs(2))
    });
    let mut socket = connect(&base).expect("connects");
    assert_eq!(socket.read(Duration::from_secs(2)), Err(JfError::Transport));
    server.join().unwrap();
}

#[test]
fn nothing_is_dialled_without_a_token_or_a_usable_address() {
    assert_eq!(
        Socket::connect("http://127.0.0.1:9", &device(), &SecretString::new("")).map(drop),
        Err(JfError::InvalidInput)
    );
    assert_eq!(
        connect("ftp://example.invalid").map(drop),
        Err(JfError::InvalidInput)
    );
}

#[test]
fn an_unreachable_server_is_a_transport_error() {
    // Bind and drop to find a port nothing listens on.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    assert_eq!(
        connect(&format!("http://127.0.0.1:{port}")).map(drop),
        Err(JfError::Transport)
    );
}
