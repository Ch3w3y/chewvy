//! The Watch Together sync socket: a WebSocket to a room's Syncplay server.
//!
//! The URL comes from [`super::room::Room::syncplay_url`]. No token and no
//! `X-Plex-*` header travel on this socket: the room id inside `Hello` is
//! what admits a client.
//!
//! This is a blocking session with no thread of its own. The caller reads
//! with a timeout and answers each [`Message::State`] (see
//! [`super::exchange`]); the server's states once a second are what keep the
//! socket busy, so there is no separate keep-alive.
//!
//! TLS is rustls with the ring provider and the same Mozilla root bundle the
//! HTTP transport trusts.

use std::io::{self, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tungstenite::client::IntoClientRequest;
use tungstenite::protocol::WebSocketConfig;
use tungstenite::WebSocket;

use super::message::{Message, Outgoing};
use crate::error::CoreError;
use crate::http::{CONNECT_TIMEOUT, MAX_RESPONSE_BYTES};

/// How long a single write may stall before the socket counts as lost.
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);

/// How long [`SyncSocket::close`] waits for the server to answer the close.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(2);

/// The shortest read timeout: the OS refuses a zero one.
const MIN_WAIT: Duration = Duration::from_millis(1);

/// One open sync socket.
pub struct SyncSocket {
    ws: WebSocket<Stream>,
}

impl std::fmt::Debug for SyncSocket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SyncSocket").finish_non_exhaustive()
    }
}

impl SyncSocket {
    /// Open the socket at `url`: `wss://<host>:<port>/ws`, or `ws://` for a
    /// server without TLS.
    ///
    /// # Errors
    ///
    /// [`CoreError::InvalidInput`] for an unusable URL;
    /// [`CoreError::HttpStatus`] when the server refuses the upgrade;
    /// [`CoreError::Transport`] when it cannot be reached or the TLS or
    /// WebSocket handshake fails.
    pub fn connect(url: &str) -> Result<Self, CoreError> {
        let parsed = url::Url::parse(url).map_err(|_| CoreError::InvalidInput)?;
        let secure = match parsed.scheme() {
            "wss" => true,
            "ws" => false,
            _ => return Err(CoreError::InvalidInput),
        };
        if !parsed.username().is_empty() || parsed.password().is_some() {
            return Err(CoreError::InvalidInput);
        }
        let host = parsed.host().ok_or(CoreError::InvalidInput)?;
        let port = parsed
            .port_or_known_default()
            .ok_or(CoreError::InvalidInput)?;
        let tcp = connect_tcp(&host, port)?;
        tcp.set_read_timeout(Some(CONNECT_TIMEOUT))
            .and_then(|()| tcp.set_write_timeout(Some(WRITE_TIMEOUT)))
            .and_then(|()| tcp.set_nodelay(true))
            .map_err(|_| CoreError::Transport)?;
        let stream = if secure {
            Stream::Tls(Box::new(tls_stream(&host, tcp)?))
        } else {
            Stream::Plain(tcp)
        };

        let request = parsed
            .as_str()
            .into_client_request()
            .map_err(|_| CoreError::InvalidInput)?;
        let limit = usize::try_from(MAX_RESPONSE_BYTES).ok();
        let config = WebSocketConfig::default().max_message_size(limit);
        let (ws, _) = tungstenite::client::client_with_config(request, stream, Some(config))
            .map_err(|error| match error {
                tungstenite::HandshakeError::Failure(tungstenite::Error::Http(response)) => {
                    CoreError::HttpStatus {
                        status: response.status().as_u16(),
                    }
                }
                _ => CoreError::Transport,
            })?;
        Ok(Self { ws })
    }

    /// The next message, waiting at most `timeout`; `None` when none came.
    ///
    /// A message type this client does not use comes back as
    /// [`Message::Other`] and an unreadable one as [`Message::Malformed`]:
    /// neither ends the session.
    ///
    /// # Errors
    ///
    /// [`CoreError::Transport`] when the socket is gone, whether the network
    /// dropped it or the server closed it; open a new one.
    pub fn read(&mut self, timeout: Duration) -> Result<Option<Message>, CoreError> {
        let deadline = Instant::now() + timeout;
        loop {
            let wait = deadline
                .saturating_duration_since(Instant::now())
                .max(MIN_WAIT);
            self.ws
                .get_ref()
                .tcp()
                .set_read_timeout(Some(wait))
                .map_err(|_| CoreError::Transport)?;
            match self.ws.read() {
                Ok(tungstenite::Message::Text(text)) => {
                    return Ok(Some(Message::parse(text.as_str())));
                }
                // Pings are answered by tungstenite itself; the server sends
                // nothing else that is not text.
                Ok(_) => {}
                Err(tungstenite::Error::Io(error)) if timed_out(&error) => {}
                Err(_) => return Err(CoreError::Transport),
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
        }
    }

    /// Send one frame.
    ///
    /// # Errors
    ///
    /// [`CoreError::Transport`] when the socket is gone.
    pub fn send(&mut self, message: &Outgoing<'_>) -> Result<(), CoreError> {
        self.ws
            .send(tungstenite::Message::text(message.to_text()))
            .map_err(|_| CoreError::Transport)
    }

    /// Close the socket, waiting briefly for the server to agree. The server
    /// tells the others this participant left; the room itself stays until
    /// [`super::room::leave_room`].
    ///
    /// # Errors
    ///
    /// [`CoreError::Transport`] when the socket was already gone.
    pub fn close(mut self) -> Result<(), CoreError> {
        self.ws.close(None).map_err(|_| CoreError::Transport)?;
        let deadline = Instant::now() + CLOSE_TIMEOUT;
        while Instant::now() < deadline {
            let wait = deadline
                .saturating_duration_since(Instant::now())
                .max(MIN_WAIT);
            let _ = self.ws.get_ref().tcp().set_read_timeout(Some(wait));
            match self.ws.read() {
                Ok(_) => {}
                Err(tungstenite::Error::Io(error)) if timed_out(&error) => {}
                Err(_) => break,
            }
        }
        Ok(())
    }
}

/// A read timeout is `WouldBlock` on Unix and `TimedOut` on Windows.
fn timed_out(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    )
}

fn connect_tcp(host: &url::Host<&str>, port: u16) -> Result<TcpStream, CoreError> {
    let addresses: Vec<SocketAddr> = match host {
        url::Host::Domain(name) => (*name, port)
            .to_socket_addrs()
            .map_err(|_| CoreError::Transport)?
            .collect(),
        url::Host::Ipv4(ip) => vec![SocketAddr::new(IpAddr::V4(*ip), port)],
        url::Host::Ipv6(ip) => vec![SocketAddr::new(IpAddr::V6(*ip), port)],
    };
    addresses
        .iter()
        .find_map(|address| TcpStream::connect_timeout(address, CONNECT_TIMEOUT).ok())
        .ok_or(CoreError::Transport)
}

fn tls_stream(
    host: &url::Host<&str>,
    tcp: TcpStream,
) -> Result<rustls::StreamOwned<rustls::ClientConnection, TcpStream>, CoreError> {
    let roots = rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|_| CoreError::Transport)?
        .with_root_certificates(roots)
        .with_no_client_auth();
    let name = match host {
        url::Host::Domain(name) => rustls::pki_types::ServerName::try_from((*name).to_owned())
            .map_err(|_| CoreError::InvalidInput)?,
        url::Host::Ipv4(ip) => rustls::pki_types::ServerName::from(IpAddr::V4(*ip)),
        url::Host::Ipv6(ip) => rustls::pki_types::ServerName::from(IpAddr::V6(*ip)),
    };
    let connection =
        rustls::ClientConnection::new(Arc::new(config), name).map_err(|_| CoreError::Transport)?;
    Ok(rustls::StreamOwned::new(connection, tcp))
}

/// The connection under the WebSocket: plain TCP or TLS over it.
enum Stream {
    Plain(TcpStream),
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>),
}

impl Stream {
    fn tcp(&self) -> &TcpStream {
        match self {
            Self::Plain(tcp) => tcp,
            Self::Tls(tls) => tls.get_ref(),
        }
    }
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Plain(tcp) => tcp.read(buf),
            Self::Tls(tls) => tls.read(buf),
        }
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Self::Plain(tcp) => tcp.write(buf),
            Self::Tls(tls) => tls.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Plain(tcp) => tcp.flush(),
            Self::Tls(tls) => tls.flush(),
        }
    }
}
