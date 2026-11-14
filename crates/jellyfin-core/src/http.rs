//! Blocking HTTP transport for one Jellyfin server (ureq 3.3.0 + rustls/ring,
//! same stack and timeouts as `plex_core::http`).
//!
//! Differences from the Plex transport, both forced by Jellyfin's API:
//!
//! * authorization travels as a caller-visible `Authorization: MediaBrowser`
//!   header built by [`crate::header::media_browser_header`] — this transport
//!   takes the finished header value, it never assembles tokens itself;
//! * the only secret ever allowed into a URL is the short-lived Quick Connect
//!   `secret` query parameter (see [`Http::get`] docs); the bearer token is
//!   header-only, and the `api_key` legacy parameter is refused outright.

use std::time::Duration;

use crate::error::JfError;

/// ureq 3.3.0's response type (an `http::Response` with ureq's lazy body).
type HttpResponse = ureq::http::Response<ureq::Body>;

/// Upper bound on a buffered response body (server JSON answers stay far
/// below this).
pub const MAX_RESPONSE_BYTES: u64 = 2 * 1024 * 1024;

/// Default connect timeout.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Default whole-call timeout.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(20);

/// Query keys whose value would be a bearer secret in the URL. Only the Quick
/// Connect `secret` may carry one, and [`Http::send`] refuses these.
const FORBIDDEN_QUERY_KEYS: [&str; 2] = ["api_key", "access_token"];

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
    pub fn json<T: serde::de::DeserializeOwned>(&self) -> Result<T, JfError> {
        serde_json::from_slice(&self.body).map_err(|_| JfError::ResponseShape)
    }

    /// True when the status is 2xx.
    #[must_use]
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// One transport to one Jellyfin server (`http://host:8096`).
#[derive(Clone)]
pub struct Http {
    agent: ureq::Agent,
    base: String,
}

impl std::fmt::Debug for Http {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Http")
            .field("base", &self.base)
            .finish_non_exhaustive()
    }
}

impl Http {
    /// Build a transport for `base` with the default timeouts. `base` is
    /// normalised by [`crate::server::normalize_base`].
    pub fn new(base: &str) -> Result<Self, JfError> {
        Self::with_timeouts(base, CONNECT_TIMEOUT, CALL_TIMEOUT)
    }

    /// Build a transport with explicit timeouts (mock-server tests, LAN
    /// probes).
    pub fn with_timeouts(
        base: &str,
        connect: Duration,
        overall: Duration,
    ) -> Result<Self, JfError> {
        let base = crate::server::normalize_base(base)?;
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_connect(Some(connect))
            .timeout_global(Some(overall))
            // No redirect is followed: a 3xx is surfaced as an `HttpStatus`
            // error before any body is read, so the Authorization header (and
            // its token) is never re-presented to an unknown target.
            .max_redirects(0)
            .tls_config(ureq::tls::TlsConfig::builder().build())
            .build()
            .into();
        Ok(Self { agent, base })
    }

    /// The normalised host this transport talks to.
    #[must_use]
    pub fn base(&self) -> &str {
        &self.base
    }

    /// GET with an optional query map and optional `Authorization` value.
    pub fn get(
        &self,
        path: &str,
        query: &[(&str, &str)],
        authorization: Option<&str>,
    ) -> Result<Response, JfError> {
        self.send("GET", path, query, authorization, None)
    }

    /// POST with an optional JSON body (already serialised), query map and
    /// optional `Authorization` value.
    pub fn post(
        &self,
        path: &str,
        query: &[(&str, &str)],
        body: Option<&str>,
        authorization: Option<&str>,
    ) -> Result<Response, JfError> {
        self.send("POST", path, query, authorization, body)
    }

    /// DELETE with an optional query map and optional `Authorization` value.
    pub fn delete(
        &self,
        path: &str,
        query: &[(&str, &str)],
        authorization: Option<&str>,
    ) -> Result<Response, JfError> {
        self.send("DELETE", path, query, authorization, None)
    }

    /// The single request path every verb goes through.
    ///
    /// # Errors
    ///
    /// [`JfError::InvalidInput`] for unknown verbs or a bearer query key
    /// (`api_key`/`access_token`) in the query; [`JfError::HttpStatus`] for
    /// 3xx (never followed) and error statuses mapped by
    /// [`JfError::from_status`]; [`JfError::Transport`] /
    /// [`JfError::ResponseShape`] for transport-level failures.
    pub fn send(
        &self,
        method: &str,
        path: &str,
        query: &[(&str, &str)],
        authorization: Option<&str>,
        body: Option<&str>,
    ) -> Result<Response, JfError> {
        if query
            .iter()
            .any(|(key, _)| FORBIDDEN_QUERY_KEYS.contains(key))
        {
            // Defense in depth against the legacy in-URL bearer token.
            return Err(JfError::InvalidInput);
        }
        let mut url = url::Url::parse(&self.base)
            .map_err(|_| JfError::InvalidInput)?
            .join(path)
            .map_err(|_| JfError::InvalidInput)?;
        if !query.is_empty() {
            let mut pairs = url.query_pairs_mut();
            for (key, value) in query {
                pairs.append_pair(key, value);
            }
        }
        let url = url.as_str().to_owned();
        let response = match self.call(method, &url, authorization, body) {
            // See [`dropped_unanswered`]: sent once more, on a new connection.
            Err(error) if dropped_unanswered(&error) => {
                self.call(method, &url, authorization, body)
            }
            answer => answer,
        }
        .map_err(map_ureq_error)?;

        let status = response.status().as_u16();
        if (300..400).contains(&status) {
            return Err(JfError::HttpStatus { status });
        }
        if status >= 400 {
            // The body of an error answer is not read: a Jellyfin error body
            // is a message list that could carry server text we refuse to
            // route into errors.
            return Err(JfError::from_status(status));
        }
        Ok(Response {
            status,
            body: read_body(response)?,
        })
    }

    /// Put one request on the wire.
    fn call(
        &self,
        method: &str,
        url: &str,
        authorization: Option<&str>,
        body: Option<&str>,
    ) -> Result<HttpResponse, ureq::Error> {
        match method {
            // ureq 3.3.0 types a bodyless request differently from one with a
            // body, so each verb is built and sent in its own arm (mirrors
            // `plex_core::http::Http::send`).
            "GET" => {
                let mut request = self.agent.get(url);
                if let Some(value) = authorization {
                    request = request.header(crate::header::AUTHORIZATION_HEADER, value);
                }
                request.call()
            }
            "POST" => {
                let mut request = self.agent.post(url);
                if let Some(value) = authorization {
                    request = request.header(crate::header::AUTHORIZATION_HEADER, value);
                }
                match body {
                    Some(text) => request
                        .header("Content-Type", "application/json")
                        .send(text.to_owned()),
                    None => request.send_empty(),
                }
            }
            "DELETE" => {
                let mut request = self.agent.delete(url);
                if let Some(value) = authorization {
                    request = request.header(crate::header::AUTHORIZATION_HEADER, value);
                }
                request.call()
            }
            // Mapped to `JfError::InvalidInput`, like a URL that cannot be sent.
            _ => Err(ureq::Error::BadUri(String::new())),
        }
    }
}

fn read_body(mut response: HttpResponse) -> Result<Vec<u8>, JfError> {
    use std::io::Read as _;
    let mut buf = Vec::new();
    response
        .body_mut()
        .with_config()
        .limit(MAX_RESPONSE_BYTES)
        .reader()
        .read_to_end(&mut buf)
        .map_err(|_| JfError::ResponseShape)?;
    Ok(buf)
}

/// Whether the peer closed the connection without answering.
///
/// A server, or the proxy in front of it, lets go of a kept-alive connection
/// it has left idle, and ureq 3.3.0 reuses a pooled connection whatever its
/// age. A request that leaves just as the far side closes is never read and
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

fn map_ureq_error(error: ureq::Error) -> JfError {
    match error {
        // ureq surfaces the status, not headers or body, so nothing
        // token-bearing can ride into the error.
        ureq::Error::StatusCode(status) => JfError::from_status(status),
        ureq::Error::BadUri(_) => JfError::InvalidInput,
        _ => JfError::Transport,
    }
}
