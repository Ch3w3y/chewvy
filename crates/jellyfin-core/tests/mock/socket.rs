//! A one-connection WebSocket mock of the session socket: it records the
//! handshake, then runs a test's script against the accepted socket.
// The handshake callback's error type is tungstenite's own.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::result_large_err
)]

use std::net::{TcpListener, TcpStream};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tungstenite::{Message, WebSocket};

/// What the client sent on the handshake.
#[derive(Debug, Clone)]
pub struct Handshake {
    /// Path and query as requested.
    pub path_and_query: String,
    /// The `Authorization` header value, if present.
    pub authorization: Option<String>,
}

/// The accepted socket, for a test's script.
pub type Ws = WebSocket<TcpStream>;

/// Accept one connection on an ephemeral port, record its handshake and run
/// `script` on it. Returns the base URL and the handle to join.
pub fn serve<T: Send + 'static>(
    script: impl FnOnce(&mut Ws) -> T + Send + 'static,
) -> (String, JoinHandle<(Handshake, T)>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("mock socket must bind");
    let base = format!("http://{}", listener.local_addr().unwrap());
    let handle = std::thread::spawn(move || {
        let (stream, _) = listener.accept().expect("one connection");
        let mut seen = None;
        let mut ws = tungstenite::accept_hdr(stream, |request: &Request, response: Response| {
            seen = Some(record(request));
            Ok(response)
        })
        .expect("handshake");
        let outcome = script(&mut ws);
        (seen.expect("handshake recorded"), outcome)
    });
    (base, handle)
}

/// Accept one connection and refuse its handshake with `status`.
pub fn refuse(status: u16) -> (String, JoinHandle<Handshake>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("mock socket must bind");
    let base = format!("http://{}", listener.local_addr().unwrap());
    let handle = std::thread::spawn(move || {
        let (stream, _) = listener.accept().expect("one connection");
        let mut seen = None;
        let _ = tungstenite::accept_hdr(stream, |request: &Request, _response: Response| {
            seen = Some(record(request));
            let mut refusal = ErrorResponse::new(None);
            *refusal.status_mut() = tungstenite::http::StatusCode::from_u16(status).unwrap();
            Err(refusal)
        });
        seen.expect("handshake recorded")
    });
    (base, handle)
}

fn record(request: &Request) -> Handshake {
    Handshake {
        path_and_query: request
            .uri()
            .path_and_query()
            .map(ToString::to_string)
            .unwrap_or_default(),
        authorization: request
            .headers()
            .get("Authorization")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned),
    }
}

/// Send one text frame.
pub fn send(ws: &mut Ws, text: &str) {
    ws.send(Message::text(text)).expect("mock send");
}

/// Every text frame the client sends for `span`, and whether it closed the
/// socket in that time.
pub fn collect(ws: &mut Ws, span: Duration) -> (Vec<String>, bool) {
    let deadline = Instant::now() + span;
    let mut texts = Vec::new();
    while Instant::now() < deadline {
        let wait = deadline.saturating_duration_since(Instant::now());
        ws.get_ref()
            .set_read_timeout(Some(wait.max(Duration::from_millis(1))))
            .unwrap();
        match ws.read() {
            Ok(Message::Text(text)) => texts.push(text.as_str().to_owned()),
            Ok(Message::Close(_)) => {
                // Drive the reply to the close until the socket is done.
                while ws.read().is_ok() {}
                return (texts, true);
            }
            Ok(_) => {}
            Err(tungstenite::Error::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => return (texts, false),
        }
    }
    (texts, false)
}
