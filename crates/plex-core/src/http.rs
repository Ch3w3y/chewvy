//! Blocking HTTP transport (ureq 3.3.0 + rustls/ring) with typed, token-free
//! errors.
//!
//! Design rules carried from the layer contract in `lib.rs`:
//!
//! * blocking calls only ever run on a worker thread owned by the caller (the
//!   UI talks to this through `auth::AuthWorker` / channels);
//! * the token is attached as the `X-Plex-Token` **header** by
//!   [`ClientInfo::headers_with_token`]; there is no function here that accepts
//!   a token for a query string, and [`Http::send`] asserts the final URL query
//!   does not carry one;
//! * no caller-supplied message can enter an error: the mapping in
//!   [`map_ureq_error`] produces the closed [`CoreError`] variants, and ureq's
//!   own `Display` never includes request header values.
//!
//! Root certificates: ureq 3.3.0 is used with its default `rustls` feature,
//! which bundles Mozilla's `webpki-roots`. Loading the *system* store through
//! ureq would need `RootCerts::PlatformVerifier`, i.e. the `platform-verifier`
//! ureq feature and the `rustls-platform-verifier` crate, which is **not** in
//! the locked dependency graph. The pinned `rustls-native-certs` crate is
//! therefore not wired into this transport (it matters for `*.plex.direct`
//! certificates, docs/PLEX_AUTH.md §5 item 5).

use std::time::Duration;

use crate::client_info::{ClientInfo, TOKEN_HEADER};
use crate::error::CoreError;
use crate::secret::SecretString;

/// ureq 3.3.0 does not export a `Response` alias; it is an `http::Response`
/// carrying ureq's lazy [`ureq::Body`].
type HttpResponse = ureq::http::Response<ureq::Body>;

/// Upper bound on a response body we are willing to buffer (Plex JSON answers
/// for auth/account calls are far below this).
pub const MAX_RESPONSE_BYTES: u64 = 2 * 1024 * 1024;

/// Default connect timeout.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Default whole-call timeout (a pin poll must never block the pairing loop
/// longer than its own 1 s tick plus slack).
pub const CALL_TIMEOUT: Duration = Duration::from_secs(20);

/// A status code plus the (bounded) response body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    /// HTTP status.
    pub status: u16,
    /// Response body bytes, capped at [`MAX_RESPONSE_BYTES`].
    pub body: Vec<u8>,
}

impl Response {
    /// Parse the body as JSON. A parse failure never echoes the body.
    pub fn json<T: serde::de::DeserializeOwned>(&self) -> Result<T, CoreError> {
        serde_json::from_slice(&self.body).map_err(|_| CoreError::ResponseShape)
    }

    /// True when the status is 2xx.
    #[must_use]
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// One transport to one Plex host (plex.tv, or a LAN PMS at :32400).
#[derive(Clone)]
pub struct Http {
    agent: ureq::Agent,
    base: String,
    client: ClientInfo,
}

impl std::fmt::Debug for Http {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Deliberately does not print the agent or the client identifier: the
        // identifier is device-persistent, so keep it out of ambient Debug.
        f.debug_struct("Http")
            .field("base", &self.base)
            .finish_non_exhaustive()
    }
}

impl Http {
    /// Build a transport for `base` (e.g. `https://plex.tv`) with the default
    /// timeouts.
    pub fn new(base: &str, client: &ClientInfo) -> Result<Self, CoreError> {
        Self::with_timeouts(base, client, CONNECT_TIMEOUT, CALL_TIMEOUT)
    }

    /// Build a transport with explicit timeouts (used by the mock-server tests
    /// and by the pin poll path).
    pub fn with_timeouts(
        base: &str,
        client: &ClientInfo,
        connect: Duration,
        overall: Duration,
    ) -> Result<Self, CoreError> {
        let base = base.trim_end_matches('/');
        let parsed = url::Url::parse(base).map_err(|_| CoreError::InvalidInput)?;
        if !matches!(parsed.scheme(), "https" | "http") {
            return Err(CoreError::InvalidInput);
        }
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_connect(Some(connect))
            .timeout_global(Some(overall))
            // No redirect is ever followed, with or without a token attached.
            // ureq-proto's redirect handling drops `Authorization`, `Cookie`
            // and `Content-Length` but keeps custom headers, so following a
            // redirect would re-send `X-Plex-Token` to the redirect target - a
            // different host, possibly over http, even for a candidate that was
            // https. `max_redirects(0)` returns the 3xx as-is and `send` below
            // turns it into a typed error.
            .max_redirects(0)
            .tls_config(ureq::tls::TlsConfig::builder().build())
            .build()
            .into();
        Ok(Self {
            agent,
            base: base.to_owned(),
            client: client.clone(),
        })
    }

    /// The host this transport talks to (scheme + host + port).
    #[must_use]
    pub fn base(&self) -> &str {
        &self.base
    }

    /// A clone of this transport with poll-shaped timeouts, re-attaching
    /// `client` for the identity headers.
    ///
    /// The PIN poll path (see `auth::POLL_HTTP_TIMEOUT`) needs a bound much
    /// tighter than [`CALL_TIMEOUT`]: a poll that outlives its tick both
    /// delays cancellation and hides a dead pin until the whole pairing budget
    /// has elapsed.
    pub fn with_poll_timeouts(
        &self,
        client: &ClientInfo,
        connect: Duration,
        overall: Duration,
    ) -> Result<Self, CoreError> {
        Self::with_timeouts(&self.base, client, connect, overall)
    }

    /// GET with an optional query map and optional token header.
    pub fn get(
        &self,
        path: &str,
        query: &[(&str, &str)],
        token: Option<&SecretString>,
    ) -> Result<Response, CoreError> {
        self.send("GET", path, query, token, None)
    }

    /// POST with an optional JSON body, query map and optional token header.
    pub fn post(
        &self,
        path: &str,
        query: &[(&str, &str)],
        body: Option<&str>,
        token: Option<&SecretString>,
    ) -> Result<Response, CoreError> {
        self.send("POST", path, query, token, body)
    }

    /// DELETE with an optional query map and optional token header.
    pub fn delete(
        &self,
        path: &str,
        query: &[(&str, &str)],
        token: Option<&SecretString>,
    ) -> Result<Response, CoreError> {
        self.send("DELETE", path, query, token, None)
    }

    /// The single request path every verb goes through.
    ///
    /// Guards, all of which are asserted by `tests/headers.rs` and the mock
    /// server test: the identity headers are always sent, the token only ever
    /// arrives as a header, and a query key named `X-Plex-Token` is refused
    /// before the request leaves the process.
    pub fn send(
        &self,
        method: &str,
        path: &str,
        query: &[(&str, &str)],
        token: Option<&SecretString>,
        body: Option<&str>,
    ) -> Result<Response, CoreError> {
        if query.iter().any(|(key, _)| *key == TOKEN_HEADER) {
            // Defense in depth: no call site can put the token in a query.
            return Err(CoreError::InvalidInput);
        }
        let mut url = self
            .base
            .parse::<url::Url>()
            .map_err(|_| CoreError::InvalidInput)?
            .join(path)
            .map_err(|_| CoreError::InvalidInput)?;
        if !query.is_empty() {
            let mut pairs = url.query_pairs_mut();
            for (key, value) in query {
                pairs.append_pair(key, value);
            }
        }
        if url.query().is_some_and(|q| q.contains(TOKEN_HEADER)) {
            // Second guard, on the URL that would actually go on the wire.
            return Err(CoreError::InvalidInput);
        }
        let url = url.as_str().to_owned();
        let headers = headers_for(&self.client, token);
        let response = match self.call(method, &url, &headers, body) {
            // See [`dropped_unanswered`]: sent once more, on a new connection.
            Err(error) if dropped_unanswered(&error) => self.call(method, &url, &headers, body),
            answer => answer,
        }
        .map_err(map_ureq_error)?;

        let status = response.status().as_u16();
        if (300..400).contains(&status) {
            // See the `max_redirects(0)` note in [`Http::with_timeouts`]: a
            // redirect is refused **before** any body is read, so no call site
            // can treat a redirect answer as data, and this client never
            // re-presents its token to a redirect target.
            return Err(CoreError::HttpStatus { status });
        }
        let body_bytes = read_body(response)?;
        Ok(Response {
            status,
            body: body_bytes,
        })
    }

    /// Put one request on the wire.
    fn call(
        &self,
        method: &str,
        url: &str,
        headers: &[crate::client_info::Header],
        body: Option<&str>,
    ) -> Result<HttpResponse, ureq::Error> {
        match method {
            // ureq 3.3.0 models a request that has not been given a body as a
            // different typestate than one that has, so each verb is built and
            // sent inside its own arm instead of sharing a `RequestBuilder`.
            // `RequestBuilder<WithoutBody>` (GET/DELETE) is sent with `call()`;
            // `send_empty()` only exists on `RequestBuilder<WithBody>`.
            "GET" => {
                let mut request = self.agent.get(url);
                for header in headers {
                    request = request.header(&header.name, &header.value);
                }
                request.call()
            }
            "DELETE" => {
                let mut request = self.agent.delete(url);
                for header in headers {
                    request = request.header(&header.name, &header.value);
                }
                request.call()
            }
            "POST" => {
                let mut request = self.agent.post(url);
                for header in headers {
                    request = request.header(&header.name, &header.value);
                }
                match body {
                    Some(text) => request
                        .header("Content-Type", "application/json")
                        .send(text.to_owned()),
                    None => request.send_empty(),
                }
            }
            // Mapped to `CoreError::InvalidInput`, like a URL that cannot be sent.
            _ => Err(ureq::Error::BadUri(String::new())),
        }
    }
}

/// Assemble the header set for one request.
///
/// `pub` (not `pub(crate)`) so the integration-style tests in `tests/` — the
/// only test shape that can also run the mock Plex server — can assert the
/// token appears exactly once and only as a header before it is handed to the
/// transport.
pub fn headers_for(
    client: &ClientInfo,
    token: Option<&SecretString>,
) -> Vec<crate::client_info::Header> {
    match token {
        Some(token) => client.headers_with_token(token),
        None => client.headers(),
    }
}

fn read_body(mut response: HttpResponse) -> Result<Vec<u8>, CoreError> {
    use std::io::Read as _;
    let mut buf = Vec::new();
    response
        .body_mut()
        .with_config()
        .limit(MAX_RESPONSE_BYTES)
        .reader()
        .read_to_end(&mut buf)
        .map_err(|_| CoreError::ResponseShape)?;
    Ok(buf)
}

/// Whether the peer closed the connection without answering.
///
/// A server lets go of a kept-alive connection it has left idle (Plex's room
/// service after 20 s), and ureq 3.3.0 reuses a pooled connection whatever its
/// age. A request that leaves just as the server closes is never read and
/// comes back as this error, so [`Http::send`] sends it once more; the pool
/// then finds the connection closed and opens a new one. An answer of any
/// status is never sent again.
fn dropped_unanswered(error: &ureq::Error) -> bool {
    use std::io::ErrorKind::{BrokenPipe, ConnectionAborted, ConnectionReset, UnexpectedEof};
    matches!(
        error,
        ureq::Error::Io(io) if matches!(
            io.kind(),
            BrokenPipe | ConnectionAborted | ConnectionReset | UnexpectedEof
        )
    )
}

fn map_ureq_error(error: ureq::Error) -> CoreError {
    match error {
        // ureq reports the status, not the response body: nothing token-bearing
        // can ride along in this mapping.
        ureq::Error::StatusCode(status) => CoreError::from_status(status),
        ureq::Error::BadUri(_) => CoreError::InvalidInput,
        _ => CoreError::Transport,
    }
}
