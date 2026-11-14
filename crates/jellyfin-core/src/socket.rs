//! The session socket: the server's channel to one signed-in device.
//!
//! `GET /socket` upgraded to a WebSocket, authorized by the same
//! `MediaBrowser` header as every other call ([`crate::header`]): the device
//! id and token travel in that header, never in the URL. A 12.4 server
//! answers `403` to a handshake without a valid token.
//!
//! This is a blocking session with no thread of its own. The caller reads
//! with a timeout; each [`Socket::read`] also sends the keep-alive the server
//! asked for when it is due, so a caller that reads at least that often
//! keeps the socket open. Keep-alive frames are handled here and never
//! returned.
//!
//! TLS is rustls with the ring provider and the same Mozilla root bundle the
//! HTTP transport trusts, so a server that answers one answers the other.

use std::io::{self, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tungstenite::client::IntoClientRequest;
use tungstenite::protocol::WebSocketConfig;
use tungstenite::WebSocket;

use crate::device::DeviceInfo;
use crate::error::JfError;
use crate::header::{media_browser_header, AUTHORIZATION_HEADER};
use crate::http::{CONNECT_TIMEOUT, MAX_RESPONSE_BYTES};
use crate::syncplay::message::{Message, KEEP_ALIVE};
use crate::SecretString;

/// The socket's path on the server.
pub const SOCKET_PATH: &str = "/socket";

/// How long a single write may stall before the socket counts as lost.
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);

/// How long [`Socket::close`] waits for the server to answer the close.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(2);

/// The shortest read timeout: the OS refuses a zero one.
const MIN_WAIT: Duration = Duration::from_millis(1);

/// The shortest keep-alive interval, whatever the server asks for.
const MIN_KEEP_ALIVE: Duration = Duration::from_secs(1);

/// One open session socket.
pub struct Socket {
    ws: WebSocket<Stream>,
    /// Set by the server's `ForceKeepAlive`; nothing is sent before it.
    keep_alive_every: Option<Duration>,
    last_sent: Instant,
}

impl std::fmt::Debug for Socket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Socket")
            .field("keep_alive_every", &self.keep_alive_every)
            .finish_non_exhaustive()
    }
}

impl Socket {
    /// Open the socket of the session `token` and `device` identify, on the
    /// server at `base`.
    ///
    /// # Errors
    ///
    /// [`JfError::InvalidInput`] for an unusable base; [`JfError::AuthRejected`]
    /// when the server refuses the token; [`JfError::HttpStatus`] for any
    /// other refusal; [`JfError::Transport`] when the server cannot be
    /// reached or the TLS or WebSocket handshake fails.
    pub fn connect(base: &str, device: &DeviceInfo, token: &SecretString) -> Result<Self, JfError> {
        if token.is_empty() {
            return Err(JfError::InvalidInput);
        }
        let base = crate::server::normalize_base(base)?;
        let url = url::Url::parse(&base).map_err(|_| JfError::InvalidInput)?;
        let secure = url.scheme() == "https";
        let host = url.host().ok_or(JfError::InvalidInput)?;
        let port = url.port_or_known_default().ok_or(JfError::InvalidInput)?;
        let tcp = connect_tcp(&host, port)?;
        tcp.set_read_timeout(Some(CONNECT_TIMEOUT))
            .and_then(|()| tcp.set_write_timeout(Some(WRITE_TIMEOUT)))
            .and_then(|()| tcp.set_nodelay(true))
            .map_err(|_| JfError::Transport)?;
        let stream = if secure {
            Stream::Tls(Box::new(tls_stream(&host, tcp)?))
        } else {
            Stream::Plain(tcp)
        };

        let scheme = if secure { "wss" } else { "ws" };
        let authority = url.host_str().ok_or(JfError::InvalidInput)?;
        let mut request = format!("{scheme}://{authority}:{port}{SOCKET_PATH}")
            .into_client_request()
            .map_err(|_| JfError::InvalidInput)?;
        let mut authorization =
            tungstenite::http::HeaderValue::from_str(&media_browser_header(device, Some(token)))
                .map_err(|_| JfError::InvalidInput)?;
        authorization.set_sensitive(true);
        request
            .headers_mut()
            .insert(AUTHORIZATION_HEADER, authorization);

        let limit = usize::try_from(MAX_RESPONSE_BYTES).ok();
        let config = WebSocketConfig::default().max_message_size(limit);
        let (ws, _) = tungstenite::client::client_with_config(request, stream, Some(config))
            .map_err(|error| match error {
                tungstenite::HandshakeError::Failure(tungstenite::Error::Http(response)) => {
                    JfError::from_status(response.status().as_u16())
                }
                _ => JfError::Transport,
            })?;
        Ok(Self {
            ws,
            keep_alive_every: None,
            last_sent: Instant::now(),
        })
    }

    /// The next message, waiting at most `timeout`; `None` when none came.
    ///
    /// A message type SyncPlay does not use comes back as
    /// [`Message::Other`] and an unreadable one as [`Message::Malformed`]:
    /// neither ends the session.
    ///
    /// # Errors
    ///
    /// [`JfError::Transport`] when the socket is gone, whether the network
    /// dropped it or the server closed it; open a new one.
    pub fn read(&mut self, timeout: Duration) -> Result<Option<Message>, JfError> {
        let deadline = Instant::now() + timeout;
        loop {
            self.keep_alive_if_due()?;
            let now = Instant::now();
            let wait = deadline
                .saturating_duration_since(now)
                .min(self.until_keep_alive(now))
                .max(MIN_WAIT);
            self.ws
                .get_ref()
                .tcp()
                .set_read_timeout(Some(wait))
                .map_err(|_| JfError::Transport)?;
            match self.ws.read() {
                Ok(tungstenite::Message::Text(text)) => match Message::parse(text.as_str()) {
                    Message::ForceKeepAlive { timeout } => {
                        self.keep_alive_every = Some((timeout / 2).max(MIN_KEEP_ALIVE));
                        self.send_keep_alive()?;
                    }
                    Message::KeepAlive => {}
                    message => return Ok(Some(message)),
                },
                // Pings are answered by tungstenite itself; the server sends
                // nothing else that is not text.
                Ok(_) => {}
                Err(tungstenite::Error::Io(error)) if timed_out(&error) => {}
                Err(_) => return Err(JfError::Transport),
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
        }
    }

    /// Close the socket, waiting briefly for the server to agree. The
    /// server forgets nothing on close: leave a SyncPlay group first.
    ///
    /// # Errors
    ///
    /// [`JfError::Transport`] when the socket was already gone.
    pub fn close(mut self) -> Result<(), JfError> {
        self.ws.close(None).map_err(|_| JfError::Transport)?;
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

    fn keep_alive_if_due(&mut self) -> Result<(), JfError> {
        if self.until_keep_alive(Instant::now()).is_zero() {
            self.send_keep_alive()?;
        }
        Ok(())
    }

    /// Time left before the next keep-alive is due; forever before the
    /// server has asked for any.
    fn until_keep_alive(&self, now: Instant) -> Duration {
        self.keep_alive_every.map_or(Duration::MAX, |every| {
            (self.last_sent + every).saturating_duration_since(now)
        })
    }

    fn send_keep_alive(&mut self) -> Result<(), JfError> {
        self.ws
            .send(tungstenite::Message::text(KEEP_ALIVE))
            .map_err(|_| JfError::Transport)?;
        self.last_sent = Instant::now();
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

fn connect_tcp(host: &url::Host<&str>, port: u16) -> Result<TcpStream, JfError> {
    let addresses: Vec<SocketAddr> = match host {
        url::Host::Domain(name) => (*name, port)
            .to_socket_addrs()
            .map_err(|_| JfError::Transport)?
            .collect(),
        url::Host::Ipv4(ip) => vec![SocketAddr::new(IpAddr::V4(*ip), port)],
        url::Host::Ipv6(ip) => vec![SocketAddr::new(IpAddr::V6(*ip), port)],
    };
    addresses
        .iter()
        .find_map(|address| TcpStream::connect_timeout(address, CONNECT_TIMEOUT).ok())
        .ok_or(JfError::Transport)
}

fn tls_stream(
    host: &url::Host<&str>,
    tcp: TcpStream,
) -> Result<rustls::StreamOwned<rustls::ClientConnection, TcpStream>, JfError> {
    let roots = rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|_| JfError::Transport)?
        .with_root_certificates(roots)
        .with_no_client_auth();
    let name = match host {
        url::Host::Domain(name) => rustls::pki_types::ServerName::try_from((*name).to_owned())
            .map_err(|_| JfError::InvalidInput)?,
        url::Host::Ipv4(ip) => rustls::pki_types::ServerName::from(IpAddr::V4(*ip)),
        url::Host::Ipv6(ip) => rustls::pki_types::ServerName::from(IpAddr::V6(*ip)),
    };
    let connection =
        rustls::ClientConnection::new(Arc::new(config), name).map_err(|_| JfError::Transport)?;
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
