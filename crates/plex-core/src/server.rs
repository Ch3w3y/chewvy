//! Connecting to one Plex Media Server, and the identity that follows.
//!
//! The rule this module exists to enforce: every authenticated call to
//! a PMS uses **that server's own `accessToken`** from its `api/v2/resources`
//! row (docs/PLEX_AUTH.md §1 item 6: the retained reference contacts "each
//! resource" with "its OWN accessToken" L1594, not the account token). The
//! account token that read `resources` is never sent to a PMS host, and one PMS
//! host never sees another server's token. The single exception is the explicit,
//! caller-supplied [`connect_shared`] path for a **shared** (`owned = 0`) or
//! manually addressed server; whether Plex accepts the account token there is
//! **[U]** (docs/PLEX_AUTH.md §5 gap 7).
//!
//! ## What a probe is (and what the record does *not* say)
//!
//! docs/PLEX_AUTH.md §1 item 6 records only that a resource's reachability
//! comes from **probing its connections**: the retained python-plexapi copy
//! builds a `PlexServer`/`PlexClient` directly on each connection URI with that
//! row's `accessToken` and keeps the first one that answers (`myplex.py`
//! L1590-1597 threaded probe, `_connect` L2483-2508, `_chooseConnection`
//! L2511-2525). **Which HTTP request that constructor issues is [U]** —
//! `plexapi/server.py` is not in the retained copy, so this client does **not**
//! claim that Plex documents a probe request. What it does instead, and why:
//!
//! * **step 1, no credential:** `GET /` on that candidate's own base URL with a
//!   short timeout. Any HTTP status at all means something answers at that
//!   address/port; only a transport failure is [`AttemptKind::Unreachable`]. No
//!   token is presented before a host has answered, so a stale or taken LAN
//!   address never sees one.
//! * **step 2, that row's own token:** `GET [IDENTITY_PATH]` on the same base,
//!   which both shows the server accepts this credential and captures the
//!   identifier. 401/422 there is [`AttemptKind::TokenRejected`].
//! * Statuses decide; probe bodies are read only for `/identity`. `GET
//!   /api/v2/resources` on a PMS is deliberately **not** used as a probe: that
//!   answer would list other devices' access tokens, which this client has no
//!   reason to read (docs/PLEX_AUTH.md §3: only plex.tv needs the account
//!   token).
//!
//! ## Machine identifier
//!
//! Captured **only after a candidate of that server answered**, from that server
//! itself (`GET /identity` with its own token). docs/PLEX_AUTH.md §1 item 6
//! records `machineIdentifier` as a `PlexServer` attribute "learned after
//! connecting" (`SharedServer` has it, L1377) and records it as **absent** from
//! the `resources` answer ("DO NOT EXIST (grep 0 matches)" — a zero-match grep
//! of the retained python-plexapi copy, not a PlexApp HAR capture), so no
//! identifier is read from a plex.tv row. The `/identity` answer shape is
//! **[U]** until observed against a real server: parsing accepts
//! the two shapes this project's fixtures take, and an answer with no usable
//! identifier is [`AttemptKind::BadShape`], after which the next candidate is
//! tried.
//!
//! ## Ordering, timeouts and what is *not* implemented
//!
//! Candidate order comes from `crate::resources::candidate_order` (local >
//! remote > relay, https > http, ipv4 > ipv6; docs/PLEX_AUTH.md §1 item 6,
//! `myplex.py` L1452-1454), one pass, no retry of the same candidate,
//! `httpsRequired` servers never given an `http` attempt. The probe timeout is
//! short so an offline server cannot stall the list. Redirects are **not
//! followed** anywhere in this crate (see `crate::http`): a token is never
//! re-presented to a redirect target. A PMS whose TLS certificate is not
//! verifiable (self-signed, `*.plex.direct`) fails step 1/step 2 as
//! [`AttemptKind::Unreachable`]: docs/PLEX_AUTH.md §5 gap 5 is **[U]** and
//! this client adds no insecure-TLS or pinning option of its own. 429 rules
//! (`docs/PLEX_AUTH.md` §5 gap 4) are **not implemented**: the status is
//! surfaced to the caller. Relay transport (`docs/PLEX_AUTH.md` §5 gap 6) is
//! **not implemented**: a relay candidate is ordered last and probed like any
//! other candidate, but the relay handshake itself is not modelled, so a
//! relay-only server is reported as unreachable with its real status.
//! Managed-user / Plex Home scoping (§5 gaps 7–8) is **not implemented**. All of
//! these are recorded as open in `docs/CAPABILITIES.md`.

use std::time::Duration;

use serde::Deserialize;

use crate::client_info::ClientInfo;
use crate::error::CoreError;
use crate::http::{Http, Response};
use crate::resources::{self, AddressFamily, Candidate, Protocol, Reachability, ServerResource};
use crate::secret::SecretString;

/// The route used to read a connected server's identifier.
///
/// **[U]** shape (see the module docs); verified against a real server in
/// real-server verification.
pub const IDENTITY_PATH: &str = "/identity";

/// The route used for the credential-free liveness probe (see the module docs:
/// the request a probe sends is this client's design, not a documented Plex
/// probe route).
pub const PROBE_PATH: &str = "/";

/// Connect bound for the identifier read.
pub const IDENTITY_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

/// Whole-call bound for the identifier read.
pub const IDENTITY_CALL_TIMEOUT: Duration = Duration::from_secs(8);

/// Default PMS port (docs/PLEX_AUTH.md §1 item 7: 32400 "… primary port").
pub const DEFAULT_PMS_PORT: u16 = 32400;

/// A server's stable machine identifier.
///
/// `docs/PLEX_AUTH.md` §1 item 6 records `machineIdentifier` as a
/// `PlexServer` attribute; it is device-persistent and is treated as sensitive
/// in the sense that it must not appear in logs or `Debug` output. It is therefore held in a [`SecretString`] so `Debug`
/// and `Display` print [`crate::secret::REDACTED`], and compared with
/// [`MachineIdentifier::matches`] instead of printed. It is *not* a token: it is
/// never sent as a credential.
///
/// `Hash` is deliberately **not** derived: [`SecretString`] implements only
/// `Clone` + a constant-time `PartialEq`/`Eq` and never exposes its contents,
/// so hashing this wrapper would either require reading the secret or hash
/// the redaction placeholder (which would make every identifier collide).
/// Identity comparison goes through [`MachineIdentifier::matches`].
#[derive(Clone, PartialEq, Eq)]
pub struct MachineIdentifier(SecretString);

impl MachineIdentifier {
    /// True when `value` is this identifier (no rendering of either).
    #[must_use]
    pub fn matches(&self, value: &str) -> bool {
        self.0.matches(value)
    }

    /// Length of the identifier without revealing it.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// True when the identifier is empty (a capture that yielded nothing).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl std::fmt::Debug for MachineIdentifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MachineIdentifier")
            .field("value", &crate::secret::REDACTED)
            .field("len", &self.0.len())
            .finish()
    }
}

#[derive(Deserialize)]
struct IdentityWire {
    #[serde(rename = "machineIdentifier", default)]
    flat: Option<String>,
    #[serde(rename = "MediaContainer", default)]
    container: Option<IdentityContainer>,
}

#[derive(Deserialize)]
struct IdentityContainer {
    #[serde(rename = "machineIdentifier", default)]
    nested: Option<String>,
}

fn capture_machine_identifier(response: &Response) -> Result<MachineIdentifier, AttemptKind> {
    if !response.is_success() {
        return Err(AttemptKind::HttpStatus(response.status));
    }
    let wire: IdentityWire =
        serde_json::from_slice(&response.body).map_err(|_| AttemptKind::BadShape)?;
    let value = wire
        .flat
        .or_else(|| wire.container.and_then(|c| c.nested))
        .filter(|v| !v.is_empty())
        .ok_or(AttemptKind::BadShape)?;
    Ok(MachineIdentifier(SecretString::new(value)))
}

/// Why one candidate connection did not work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttemptKind {
    /// Transport failure: nothing answered at that address/port.
    Unreachable,
    /// The candidate refused to build (a `uri` this client will not address).
    InvalidUrl,
    /// The server answered with this status.
    HttpStatus(u16),
    /// 401/422 on the probe: the token this client presented was not accepted.
    TokenRejected(u16),
    /// `GET /identity` did not yield a machine identifier.
    BadShape,
}

impl AttemptKind {
    /// Actionable wording for one attempt (fixed strings only).
    #[must_use]
    pub fn message(self) -> &'static str {
        match self {
            Self::Unreachable => "no answer at this address/port (server offline or blocked)",
            Self::InvalidUrl => "the advertised connection uri is not usable",
            Self::HttpStatus(_) => "the server answered with an error status",
            Self::TokenRejected(_) => "the server rejected this server's own access token",
            Self::BadShape => "the connected server did not report its machine identifier",
        }
    }
}

/// One candidate that was tried, in the order it was tried.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attempt {
    /// Base URL actually contacted.
    pub base_url: String,
    /// Transport used.
    pub protocol: Protocol,
    /// local / remote / relay class of the candidate.
    pub reachability: Reachability,
    /// Address family used for the ordering position.
    pub family: AddressFamily,
    /// True when this candidate was a server's recorded `httpuri` fallback.
    pub fallback: bool,
    /// What happened.
    pub kind: AttemptKind,
}

impl Attempt {
    /// One line a UI can show. Contains the address/port the user needs to fix
    /// (a LAN address is not a credential) and never any token. The base URL
    /// came from a server-supplied `uri`/`httpuri`, so it is rendered through
    /// [`crate::log::sanitize`]: bounded in length, flattened to one line, with
    /// control characters neutralised. `Attempt::base_url` still holds what the
    /// row recorded.
    #[must_use]
    pub fn message(&self) -> String {
        let status = match self.kind {
            AttemptKind::HttpStatus(status) | AttemptKind::TokenRejected(status) => {
                format!(" http {status}")
            }
            _ => String::new(),
        };
        let fallback = if self.fallback {
            " [http fallback]"
        } else {
            ""
        };
        format!(
            "{} via {} {}{}{}: {}",
            crate::log::sanitize(&self.base_url, crate::log::MAX_LABEL_LEN),
            self.protocol_label(),
            self.family_label(),
            fallback,
            status,
            self.kind.message()
        )
    }

    fn protocol_label(&self) -> &'static str {
        match self.protocol {
            Protocol::Https => "https",
            Protocol::Http => "http",
        }
    }

    fn family_label(&self) -> &'static str {
        match self.family {
            AddressFamily::Ipv4 => "ipv4",
            AddressFamily::Ipv6 => "ipv6",
        }
    }
}

/// Every candidate for one server failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectFailure {
    label: String,
    client_identifier: String,
    https_required: bool,
    attempts: Vec<Attempt>,
}

impl ConnectFailure {
    /// Server label the UI already knows.
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    /// `clientIdentifier` of the row that could not be reached.
    #[must_use]
    pub fn client_identifier(&self) -> &str {
        &self.client_identifier
    }

    /// The attempts, in the order they were tried.
    #[must_use]
    pub fn attempts(&self) -> &[Attempt] {
        &self.attempts
    }

    /// True when this server's row said `httpsRequired`.
    #[must_use]
    pub fn requires_https(&self) -> bool {
        self.https_required
    }

    /// A short actionable summary for a UI: what failed, on which addresses,
    /// and the one thing the user can do about it. Built from [`Attempt`] lines
    /// and fixed strings; the label and the candidate URLs are sanitized server
    /// text, no token and no response body can appear, and
    /// [`crate::log::describe`] can be used on top of it for any foreign error
    /// text so a token cannot ride along.
    #[must_use]
    pub fn message(&self) -> String {
        let mut out = format!(
            "cannot connect to {} ({} tried)",
            crate::log::sanitize(&self.label, crate::log::MAX_LABEL_LEN),
            self.attempts.len()
        );
        if self.https_required {
            out.push_str(" — this server requires https (httpsRequired), so http was not tried");
        }
        for attempt in &self.attempts {
            out.push_str("\n  - ");
            out.push_str(&attempt.message());
        }
        if self
            .attempts
            .iter()
            .any(|a| a.kind == AttemptKind::Unreachable)
            && !self.https_required
        {
            out.push_str("\n  server offline, asleep, or firewalled from this machine");
        }
        out
    }
}

impl std::fmt::Display for ConnectFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `message()` is built from fixed strings, a sanitized label and
        // sanitized candidate URLs (`crate::log::sanitize`): it can never render
        // a token and can never carry a newline/ANSI sequence supplied by a
        // hostile server, so `log::describe` on it is safe.
        f.write_str(&self.message())
    }
}

impl std::error::Error for ConnectFailure {}

impl ConnectFailure {
    fn new(resource: &ServerResource, attempts: Vec<Attempt>) -> Self {
        Self {
            label: resource.display_name(),
            client_identifier: resource.client_identifier().to_owned(),
            https_required: resource.requires_https(),
            attempts,
        }
    }
}

/// A server this client is connected to through one candidate connection.
///
/// The candidate host is fixed before the token is presented, and the token is
/// only ever presented to that host after it answered a credential-free probe.
/// A host that answers but is not the row's server is a real-server
/// verification question: `*.plex.direct` TLS is the mechanism
/// Plex documents for this (docs/PLEX_AUTH.md §5 gap 5), its exact behaviour
/// is **[U]**, and this client adds no invented identity pinning of its own.
///
/// The token held here is **this server's own** `accessToken` (or, for the
/// delegated [`connect_shared`] path, the account token that Plex records as
/// valid on that server). Every call made through this value carries it in the
/// `X-Plex-Token` header only.
#[derive(Clone)]
pub struct Server {
    client_identifier: String,
    label: String,
    base_url: String,
    protocol: Protocol,
    reachability: Reachability,
    fallback: bool,
    machine_identifier: MachineIdentifier,
    http: Http,
    token: SecretString,
    delegated: bool,
}

impl std::fmt::Debug for Server {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Server")
            .field("client_identifier", &self.client_identifier)
            .field("label", &self.label)
            .field("base_url", &self.base_url)
            .field("protocol", &self.protocol)
            .field("reachability", &self.reachability)
            .field("fallback", &self.fallback)
            .field("machine_identifier", &self.machine_identifier)
            .field("token", &crate::secret::REDACTED)
            .field("delegated_token", &self.delegated)
            .finish()
    }
}

impl Server {
    /// `clientIdentifier` of the row this server came from.
    #[must_use]
    pub fn client_identifier(&self) -> &str {
        &self.client_identifier
    }

    /// UI label (`name` / `device` / identifier).
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    /// The base URL this client actually reached the server on.
    #[must_use]
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Transport in use.
    #[must_use]
    pub fn protocol(&self) -> Protocol {
        self.protocol
    }

    /// Which candidate class succeeded.
    #[must_use]
    pub fn reachability(&self) -> Reachability {
        self.reachability
    }

    /// True when the connection in use is a server's recorded `httpuri`
    /// fallback (a downgrade the user should be told about).
    #[must_use]
    pub fn using_http_fallback(&self) -> bool {
        self.fallback
    }

    /// The identifier captured from this server after connecting.
    #[must_use]
    pub fn machine_identifier(&self) -> &MachineIdentifier {
        &self.machine_identifier
    }

    /// Length of the held token without revealing characters.
    #[must_use]
    pub fn token_len(&self) -> usize {
        self.token.len()
    }

    /// Clone this server's token for a call that cannot go through [`Server::get`].
    #[must_use]
    pub fn token(&self) -> SecretString {
        self.token.clone()
    }

    /// The token held here came from a caller rather than from this server's
    /// `resources` row ([`connect_shared`]).
    #[must_use]
    pub fn uses_delegated_token(&self) -> bool {
        self.delegated
    }

    /// `GET` on this server with **its own** token, header only.
    ///
    /// The only transport path a `Server` performs: the read paths, and the two
    /// state-changing progress calls of [`crate::progress`] (`/:/timeline`,
    /// `/:/scrobble`), are all `GET`, so `Server` exposes no `POST`/`PUT`/
    /// `PATCH`/`DELETE` wrapper at all.
    pub fn get(&self, path: &str, query: &[(&str, &str)]) -> Result<Response, CoreError> {
        self.http.get(path, query, Some(&self.token))
    }

    /// Liveness check on this server: `GET /` with **no** credential, status
    /// only.
    ///
    /// A UI that only wants "still reachable?" gets it without presenting any
    /// token again, and without asking a PMS for the `resources` list (whose
    /// answer would carry other devices' access tokens).
    pub fn probe(&self) -> Result<u16, CoreError> {
        self.http
            .get(PROBE_PATH, &[], None)
            .map(|response| response.status)
    }
}

/// True when the host produced an HTTP answer of **any** status, as opposed to
/// a transport failure (nothing listened at that address/port).
fn host_answered(result: &Result<Response, CoreError>) -> bool {
    matches!(
        result,
        // A real PMS answers the token-less probe of `/` with 401.
        Ok(_)
            | Err(CoreError::HttpStatus { .. })
            | Err(CoreError::AuthRejected { .. })
            | Err(CoreError::ResponseShape)
    )
}

/// Map a call failure on the authenticated step to one attempt reason.
fn classify_call(error: CoreError) -> AttemptKind {
    match error {
        CoreError::HttpStatus { status } | CoreError::AuthRejected { status } => {
            if matches!(status, 401 | 422) {
                AttemptKind::TokenRejected(status)
            } else {
                AttemptKind::HttpStatus(status)
            }
        }
        CoreError::InvalidInput => AttemptKind::InvalidUrl,
        CoreError::ResponseShape => AttemptKind::BadShape,
        _ => AttemptKind::Unreachable,
    }
}

fn attempt_for(candidate: &Candidate, kind: AttemptKind) -> Attempt {
    Attempt {
        base_url: candidate.base_url.clone(),
        protocol: candidate.protocol,
        reachability: candidate.reachability,
        family: candidate.family,
        fallback: candidate.fallback,
        kind,
    }
}

/// One candidate connection: probe it without a credential, then read the
/// identifier with **this** token (see the module docs for why the probe carries
/// none).
fn try_candidate(
    candidate: &Candidate,
    client: &ClientInfo,
    token: &SecretString,
) -> Result<MachineIdentifier, AttemptKind> {
    let probe_http = Http::with_timeouts(
        &candidate.base_url,
        client,
        resources::PROBE_CONNECT_TIMEOUT,
        resources::PROBE_CALL_TIMEOUT,
    )
    .map_err(|_| AttemptKind::InvalidUrl)?;

    let probe = probe_http.get(PROBE_PATH, &[], None);
    if !host_answered(&probe) {
        return Err(AttemptKind::Unreachable);
    }

    let identity_http = Http::with_timeouts(
        &candidate.base_url,
        client,
        IDENTITY_CONNECT_TIMEOUT,
        IDENTITY_CALL_TIMEOUT,
    )
    .map_err(|_| AttemptKind::InvalidUrl)?;
    let response = identity_http
        .get(IDENTITY_PATH, &[], Some(token))
        .map_err(classify_call)?;
    capture_machine_identifier(&response)
}

/// Try every candidate of `resource` in probing order, once each, and stop
/// at the first candidate that answers and reports its identifier.
///
/// `delegated` records where the credential came from, so a UI can say so: the
/// [`connect`] path passes `false` (the token came from this server's own row),
/// [`connect_shared`] passes `true` (the caller supplied it).
fn connect_with(
    resource: &ServerResource,
    client: &ClientInfo,
    token: &SecretString,
    delegated: bool,
) -> Result<Server, ConnectFailure> {
    let mut attempts = Vec::new();
    for candidate in resource.candidates() {
        match try_candidate(&candidate, client, token) {
            Err(kind) => attempts.push(attempt_for(&candidate, kind)),
            Ok(machine_identifier) => match Http::new(&candidate.base_url, client) {
                Err(_) => attempts.push(attempt_for(&candidate, AttemptKind::InvalidUrl)),
                Ok(http) => {
                    return Ok(Server {
                        client_identifier: resource.client_identifier().to_owned(),
                        label: resource.display_name(),
                        base_url: candidate.base_url.clone(),
                        protocol: candidate.protocol,
                        reachability: candidate.reachability,
                        fallback: candidate.fallback,
                        machine_identifier,
                        http,
                        token: token.clone(),
                        delegated,
                    });
                }
            },
        }
    }
    Err(ConnectFailure::new(resource, attempts))
}

/// Connect to a server row using its **own** `accessToken`.
///
/// Tries every candidate of `resource.candidates()` in probing order, once
/// each, and returns as soon as one probe plus identifier capture succeeds.
/// Nothing is retried: a failed candidate is recorded and left failed.
pub fn connect(resource: &ServerResource, client: &ClientInfo) -> Result<Server, ConnectFailure> {
    match resource.token() {
        Some(token) => connect_with(resource, client, &token, false),
        None => Err(ConnectFailure::new(resource, Vec::new())),
    }
}

/// Connect to a server the user addressed directly, or one that is **shared**
/// with them (`owned = 0`), using the account token as the delegated credential.
///
/// This is the only case in milestone M3 where the account token goes to a PMS,
/// and it requires the caller to hand it over explicitly. The owned path must
/// never use it; that is asserted by
/// `tests/server_connect.rs::the_account_token_never_reaches_an_owned_server_host`,
/// which arms a mock PMS and asserts the recorded requests never carry it.
/// Plex Home / managed-user scope limits are **not** modelled
/// (`docs/PLEX_AUTH.md` §5 gaps 7–8).
pub fn connect_shared(
    resource: &ServerResource,
    client: &ClientInfo,
    account_token: &SecretString,
) -> Result<Server, ConnectFailure> {
    connect_with(resource, client, account_token, true)
}
