//! **A request that leaves on a connection the server has just let go.**
//!
//! A server closes a kept-alive connection it has left idle, and the transport
//! reuses a pooled connection whatever its age. A request sent at that moment
//! is never read: the connection closes with no answer. Plex's room service
//! does this after 20 s, which is how long choosing whom to invite can take.
//!
//! The host here is a bare loopback listener, because the fault is in how the
//! connection ends rather than in any answer: it answers one request, keeps
//! the connection, then closes it on the next request without a word.
//!
//! What is proven by execution: the unanswered request is sent once more and
//! succeeds on a new connection; it is sent only once more; and an answer of
//! any status is not sent again.

#![allow(clippy::unwrap_used, clippy::expect_used)] // test scaffolding; product code keeps these lints on

mod common;

use std::io::{Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::thread;

use plex_core::error::CoreError;

/// Read one request (head and declared body) and return its method.
fn read_request(stream: &mut TcpStream) -> Option<String> {
    let mut seen = Vec::new();
    let mut byte = [0_u8; 1];
    while !seen.ends_with(b"\r\n\r\n") {
        if stream.read(&mut byte).ok()? == 0 {
            return None;
        }
        seen.push(byte[0]);
    }
    let head = String::from_utf8_lossy(&seen).into_owned();
    let length = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())?
        })
        .unwrap_or(0);
    let mut body = vec![0_u8; length];
    stream.read_exact(&mut body).ok()?;
    head.split_whitespace().next().map(str::to_owned)
}

fn answer(stream: &mut TcpStream, status: &str) {
    let text = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{{}}"
    );
    stream
        .write_all(text.as_bytes())
        .expect("answer must be written");
}

/// A host that answers the first request, then closes that same connection
/// on the next `dropped` requests without answering, then serves normally with
/// `last` as the status. Returns its base URL and the methods it read.
fn host(dropped: usize, last: &'static str) -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("loopback port must be bindable");
    let base = format!("http://{}", listener.local_addr().expect("bound address"));
    let (seen, methods) = mpsc::channel();
    thread::spawn(move || {
        let mut left = dropped;
        let mut first = true;
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            while let Some(method) = read_request(&mut stream) {
                if seen.send(method).is_err() {
                    return;
                }
                if first {
                    first = false;
                    answer(&mut stream, "200 OK");
                } else if left > 0 {
                    left -= 1;
                    break; // closes with no answer
                } else {
                    answer(&mut stream, last);
                }
            }
        }
    });
    (base, methods)
}

#[test]
fn an_unanswered_request_on_a_closed_connection_is_sent_once_more() {
    let dir = common::TempDir::new("dropped-once");
    let client = common::client(&dir);
    let (base, methods) = host(1, "201 Created");
    let http = common::http_at(&base, &client);

    assert_eq!(
        http.get("/rooms", &[], None).expect("first call").status,
        200
    );
    let made = http
        .post("/rooms", &[], Some("{}"), None)
        .expect("the room is made on the second connection");
    assert_eq!(made.status, 201);

    let read: Vec<String> = methods.try_iter().collect();
    assert_eq!(read, ["GET", "POST", "POST"], "one re-send, no more");
}

#[test]
fn a_request_dropped_twice_is_a_transport_error_and_not_sent_a_third_time() {
    let dir = common::TempDir::new("dropped-twice");
    let client = common::client(&dir);
    let (base, methods) = host(2, "201 Created");
    let http = common::http_at(&base, &client);

    assert_eq!(
        http.get("/rooms", &[], None).expect("first call").status,
        200
    );
    let failed = http.post("/rooms", &[], Some("{}"), None);
    assert!(
        matches!(failed, Err(CoreError::Transport)),
        "got {failed:?}"
    );

    let read: Vec<String> = methods.try_iter().collect();
    assert_eq!(read, ["GET", "POST", "POST"]);
}

#[test]
fn an_answer_is_never_sent_again() {
    let dir = common::TempDir::new("dropped-none");
    let client = common::client(&dir);
    let (base, methods) = host(0, "503 Service Unavailable");
    let http = common::http_at(&base, &client);

    assert_eq!(
        http.get("/rooms", &[], None).expect("first call").status,
        200
    );
    let refused = http.post("/rooms", &[], Some("{}"), None);
    assert!(refused.is_err(), "a 503 is an error, got {refused:?}");

    let read: Vec<String> = methods.try_iter().collect();
    assert_eq!(read, ["GET", "POST"], "a status answer is surfaced once");
}
