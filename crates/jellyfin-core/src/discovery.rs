//! Optional LAN discovery via the Jellyfin broadcast protocol (UDP port
//! `7359`, introduced by the predecessor Emby server and kept by Jellyfin).
//!
//! The client broadcasts `Who is JellyfinServer?`; every server on the
//! segment answers with a small JSON document (`{"Id":…,"ServerName":…,"Address":…}`).
//! The answer arrives **unauthenticated**, so nothing in it is trusted: it is
//! a hint the user can accept, after which [`crate::server::public_info`]
//! still decides whether the address is really the server.

use std::net::{SocketAddr, UdpSocket};
use std::time::Duration;

use crate::error::JfError;

/// UDP port every Jellyfin/Emby-compatible server listens on.
pub const DISCOVERY_PORT: u16 = 7359;

/// The exact query bytes the servers expect.
pub const DISCOVERY_QUERY: &str = "Who is JellyfinServer?";

/// Default per-listen read timeout: long enough for a busy Wi-Fi segment,
/// short enough not to stall server pickers.
pub const DISCOVERY_TIMEOUT: Duration = Duration::from_millis(1500);

/// One server answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredServer {
    /// The server's stable id (same value as `PublicInfo::id`).
    pub id: String,
    /// The name it advertises.
    pub name: String,
    /// The LAN address it advertises, if any.
    pub address: Option<String>,
}

/// Broadcast once and collect the answers until the read timeout expires,
/// deduplicated by server id (a multi-homed server answers once per
/// interface).
///
/// # Errors
///
/// [`JfError::Io`] if no UDP socket can be created or the broadcast cannot be
/// sent (answers are best-effort: zero answers is `Ok(vec![])`).
pub fn discover(timeout: Duration) -> Result<Vec<DiscoveredServer>, JfError> {
    let socket = UdpSocket::bind(("0.0.0.0", 0)).map_err(|_| JfError::Io)?;
    socket.set_broadcast(true).map_err(|_| JfError::Io)?;
    socket
        .set_read_timeout(Some(timeout))
        .map_err(|_| JfError::Io)?;
    let destination: SocketAddr = ([255, 255, 255, 255], DISCOVERY_PORT).into();
    socket
        .send_to(DISCOVERY_QUERY.as_bytes(), destination)
        .map_err(|_| JfError::Io)?;
    collect_answers(&socket, DISCOVERY_QUERY.len())
}

/// Drain answers from a socket whose read timeout has been set, stopping at
/// the timeout (`WouldBlock`/`TimedOut`). `min_len` is the size a datagram
/// must at least have to be worth parsing. `pub` (not `pub(crate)`) so the
/// integration tests can drive it with a pre-filled socket.
pub fn collect_answers(
    socket: &UdpSocket,
    min_len: usize,
) -> Result<Vec<DiscoveredServer>, JfError> {
    let mut out: Vec<DiscoveredServer> = Vec::new();
    let mut buf = vec![0u8; 2048];
    loop {
        match socket.recv(&mut buf) {
            Ok(len) if len >= min_len => {
                let Some(bytes) = buf.get(..len) else {
                    continue;
                };
                if let Some(server) = parse_answer(bytes) {
                    if !out.iter().any(|found| found.id == server.id) {
                        out.push(server);
                    }
                }
            }
            // A reply too short to hold the JSON envelope: ignore it and
            // keep waiting for the real servers.
            Ok(_) => continue,
            // The read timeout (or any datagram error — answers are
            // best-effort): the collection round is over.
            Err(_) => return Ok(out),
        }
    }
}

/// Parse one answer datagram. Unparseable answers are ignored (the broadcast
/// segment may also carry Emby or XBMC-format replies). `pub` for the same
/// testability reason as [`collect_answers`].
pub fn parse_answer(bytes: &[u8]) -> Option<DiscoveredServer> {
    #[derive(serde::Deserialize)]
    struct Answer {
        #[serde(rename = "Id")]
        id: String,
        #[serde(rename = "ServerName", default)]
        server_name: String,
        #[serde(rename = "Address", default)]
        address: Option<String>,
    }
    let answer: Answer = serde_json::from_slice(bytes).ok()?;
    if answer.id.is_empty() {
        return None;
    }
    Some(DiscoveredServer {
        id: answer.id,
        name: answer.server_name,
        address: answer.address.filter(|address| !address.is_empty()),
    })
}
