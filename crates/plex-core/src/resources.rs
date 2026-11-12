//! Server discovery: `GET plex.tv/api/v2/resources` and the selection rules.
//!
//! Everything here comes from the field list recorded in `docs/PLEX_AUTH.md`
//! §1 item 6 (the `MyPlexResource` / `ResourceConnection` attribute lists of
//! the retained python-plexapi `myplex.py` copy, L1458-1482 and L1599-1625),
//! which is the only field list this project has for this call. The call itself
//! is `GET plex.tv/api/v2/resources?includeHttps=1&includeRelay=1&includeIPv6=1`
//! (recorded at `docs/PLEX_AUTH.md` §1 item 6, line 32 of that file) with
//! the **account** token in the `X-Plex-Token` header (§1 item 7, §3).
//!
//! ## Wire fields this client reads (the verified set)
//!
//! `accessToken`, `clientIdentifier`, `device`, `name`, `provides`, `presence`,
//! `owned`, `httpsRequired`, `dnsRebindingProtection`, `natLoopbackSupported`,
//! `relay`, and per-connection `address`, `port`, `protocol`, `uri`, `local`,
//! `relay`, `IPv6`, `httpuri`. Nothing else is read.
//!
//! ## Wire fields this client refuses to read
//!
//! `ownedBy`, `machineIdentifier`, `httpsEnabled`, `reachable`, `secure`.
//! `docs/PLEX_AUTH.md` §1 item 6 records them as absent from the retained
//! source: "`ownedBy`, `machineIdentifier`, `httpsEnabled`, `reachable`,
//! connection `secure` DO NOT EXIST (grep 0 matches)". That source is the
//! python-plexapi copy (a zero-match grep of that file), **not** a PlexApp HAR;
//! no captured source supports these names. This module documents those
//! names but never maps them to a field, a flag or a decision; the guardrail
//! test `m3_resources_field_guard_is_the_verified_set_only`
//! (`tests/resources_selection.rs`) scans this file's text: every
//! `#[serde(rename = "…")]` name must be in the verified set (or one of the
//! three documented container wrappers), and none of the forbidden names may
//! appear on a non-comment line. A server's machine identifier is captured in
//! `crate::server`, from the connected server itself, never from a plex.tv row.
//!
//! ## Semantics carried from the recorded docs
//!
//! * `provides == "server"` and `owned` (docs/PLEX_AUTH.md §1 item 6; the
//!   retained reference selects `servers = provides == 'server' and owned`,
//!   `myplex.py` L897); a shared (`owned = 0`) row is **not** treated as owned.
//!   Such a row is kept as an [`Excluded`] entry with the connections and the
//!   `accessToken` it actually recorded, so a UI can offer an explicit,
//!   user-confirmed [`crate::server::connect_shared`] against **what plex.tv
//!   published** rather than against an invented `host:32400`. Whether Plex
//!   authorizes an account token on a shared server is **[U]**
//!   (`docs/PLEX_AUTH.md` §5 gap 7, recorded in `docs/CAPABILITIES.md`).
//! * Candidate order local > remote > relay, `https` > `http`, IPv4 > IPv6 is
//!   the reference default recorded in docs/PLEX_AUTH.md §1 item 6
//!   (`myplex.py` L1452-1454, `DEFAULT_LOCATION_ORDER`/`DEFAULT_SCHEME_ORDER`/
//!   `DEFAULT_IP_ORDER`).
//! * `httpsRequired` is a recorded row attribute (`myplex.py` L1464). Why a PMS
//!   needs TLS on a `*.plex.direct` certificate, and whether an `http` fallback
//!   to such a host works at all, is docs/PLEX_AUTH.md §5 gap 5: **[U]**.
//!   This client never silently downgrades an `https`
//!   connection; it *lists* the recorded `httpuri` as a clearly-labelled
//!   fallback candidate after the `https` one, and a failure names the cause.
//! * `dnsRebindingProtection`/`allowedIps` host allow-listing is **not
//!   implemented**: `allowedIps` is not in the verified field set, so a server
//!   that rejects a client for rebinding reasons is reported as an unreachable
//!   candidate with the real status, not guessed around.
//! * `natLoopbackSupported` and `presence` are carried as facts about the row;
//!   nothing in `docs/PLEX_AUTH.md` gives them a client-side rule.
//!
//! ## Not implemented here (open in `docs/CAPABILITIES.md`)
//!
//! Managed-user / Plex Home scoping of `resources` (gaps §5 items 7–8) is
//! **not implemented**: this client only asks for the resources of the account
//! its token belongs to, and does not read `plex.tv/api/home/users*`. 429
//! handling is **not implemented** beyond surfacing the status (gap §5 item 4).

use std::time::Duration;

use serde::de::Visitor;
use serde::Deserialize;

use crate::error::CoreError;
use crate::http::Http;
use crate::secret::SecretString;

/// The discovery endpoint (docs/PLEX_AUTH.md §1 item 6).
pub const RESOURCES_PATH: &str = "/api/v2/resources";

/// The three include flags recorded with the endpoint (docs/PLEX_AUTH.md
/// §1 item 6). The retained reference `key` (L1417/L1421/L1449) shows only
/// `includeHttps=1&includeRelay=1`; `includeIPv6=1` is the plan-specified form
/// by this client and is **[U]** against a live plex.tv answer.
pub const RESOURCES_QUERY: [(&str, &str); 3] = [
    ("includeHttps", "1"),
    ("includeRelay", "1"),
    ("includeIPv6", "1"),
];

/// A probe needs no new endpoint: docs/PLEX_AUTH.md §1 item 6 records only
/// that a resource's reachability comes from **probing its connections**
/// (`myplex.py` L1590-1597, `_connect` L2483-2508), which is what
/// `crate::server` does; see that module for which request a probe sends.
/// A probe must
/// fail fast: an unreachable candidate is tried in order and the whole server
/// stays browsable through its next candidate.
pub const PROBE_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

/// Whole-call bound for one connection probe.
pub const PROBE_CALL_TIMEOUT: Duration = Duration::from_secs(8);

// ---------------------------------------------------------------- wire shapes

/// XML-derived JSON encodes these attributes as `0`/`1`, and captured fixtures
/// also show `true`/`false` and `"1"`/`"0"`. `Flag` accepts all of them and
/// defaults an **absent** attribute to false, which is the conservative reading
/// for `owned` (a row whose ownership cannot be proven is not ours).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Flag(bool);

impl Flag {
    fn is_set(self) -> bool {
        self.0
    }
}

impl<'de> Deserialize<'de> for Flag {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct FlagVisitor;

        impl Visitor<'_> for FlagVisitor {
            type Value = Flag;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a boolean, 0/1 or \"0\"/\"1\"")
            }

            fn visit_bool<E>(self, value: bool) -> Result<Flag, E> {
                Ok(Flag(value))
            }

            fn visit_i64<E>(self, value: i64) -> Result<Flag, E> {
                Ok(Flag(value != 0))
            }

            fn visit_u64<E>(self, value: u64) -> Result<Flag, E> {
                Ok(Flag(value != 0))
            }

            fn visit_f64<E>(self, value: f64) -> Result<Flag, E> {
                Ok(Flag(value != 0.0))
            }

            fn visit_str<E>(self, value: &str) -> Result<Flag, E>
            where
                E: serde::de::Error,
            {
                match value {
                    "1" | "true" | "TRUE" | "yes" => Ok(Flag(true)),
                    "0" | "false" | "FALSE" | "no" | "" => Ok(Flag(false)),
                    other => Err(E::invalid_value(
                        serde::de::Unexpected::Str(other),
                        &"a Plex boolean attribute",
                    )),
                }
            }

            fn visit_none<E>(self) -> Result<Flag, E> {
                Ok(Flag(false))
            }

            fn visit_unit<E>(self) -> Result<Flag, E> {
                Ok(Flag(false))
            }
        }

        deserializer.deserialize_any(FlagVisitor)
    }
}

/// One row of the `resources` answer. `#[serde(rename)]` below is the whole
/// set of names this client reads; the guardrail test asserts it.
#[derive(Deserialize)]
struct ResourceWire {
    #[serde(rename = "accessToken", default)]
    access_token: Option<String>,
    #[serde(rename = "clientIdentifier", default)]
    client_identifier: Option<String>,
    #[serde(default)]
    device: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    provides: Option<String>,
    #[serde(default)]
    presence: Flag,
    #[serde(default)]
    owned: Flag,
    #[serde(rename = "httpsRequired", default)]
    https_required: Flag,
    #[serde(rename = "dnsRebindingProtection", default)]
    dns_rebinding_protection: Flag,
    #[serde(rename = "natLoopbackSupported", default)]
    nat_loopback_supported: Flag,
    #[serde(default)]
    relay: Flag,
    #[serde(default)]
    connections: Vec<ConnectionWire>,
}

/// One entry of a row's `connections` array.
#[derive(Deserialize)]
struct ConnectionWire {
    #[serde(default)]
    address: Option<String>,
    #[serde(default)]
    port: Option<u16>,
    #[serde(default)]
    protocol: Option<String>,
    #[serde(default)]
    uri: Option<String>,
    #[serde(default)]
    local: Flag,
    #[serde(default)]
    relay: Flag,
    #[serde(rename = "IPv6", default)]
    ip_v6: Flag,
    #[serde(rename = "httpuri", default)]
    httpuri: Option<String>,
}

/// The answer shape: the retained source parses XML (`MyPlexResource` rows with
/// `ResourceConnection` children). How plex.tv renders the same data as JSON is
/// **[U]** (docs/PLEX_AUTH.md §5 gap 8 area: no Plex-published spec was
/// located). Both plausible JSON renderings are accepted so a real answer does
/// not fail; anything else is [`CoreError::ResponseShape`], which never echoes
/// the body. The accepted shapes are exercised only by mock fixtures
/// (`tests/resources_selection.rs`), which are this project's own construction.
#[derive(Deserialize)]
#[serde(untagged)]
enum ResourcesWire {
    /// A bare JSON array of rows.
    List(Vec<ResourceWire>),
    /// The XML-derived shape, rendered as `{"MediaContainer": {"media-server": [...]}}`.
    Wrapped {
        #[serde(rename = "MediaContainer")]
        container: ContainerWire,
    },
}

#[derive(Deserialize)]
struct ContainerWire {
    #[serde(rename = "media-server", default)]
    kebab: Vec<ResourceWire>,
    #[serde(rename = "mediaServer", default)]
    camel: Vec<ResourceWire>,
}

// ------------------------------------------------------------------ public set

/// The transport of one candidate connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Protocol {
    /// `https` — ordered first (docs/PLEX_AUTH.md §1 item 6,
    /// `DEFAULT_SCHEME_ORDER = ['https', 'http']`, `myplex.py` L1453).
    Https,
    /// `http` — only tried after every `https` candidate.
    Http,
}

/// Where a connection is expected to reach the server from.
///
/// Declaration order is the probing order:
/// local > remote > relay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Reachability {
    /// `local == 1` on this connection.
    Local,
    /// `local == 0`, `relay == 0`: the public address of the server.
    Remote,
    /// `relay == 1` on this connection (tried last).
    Relay,
}

/// Address family of a connection, from the verified `IPv6` attribute.
///
/// The attribute is authoritative when present. A candidate whose address is
/// literally IPv6 (`:` in it) but whose `IPv6` attribute was absent/0 is still
/// ranked IPv6, so this client cannot be talked into preferring an IPv6 socket
/// it has no evidence for. `docs/PLEX_AUTH.md` §5 gap 5 (`*.plex.direct`
/// TLS vs self-signed PMS, pinning) is unresolved and is a real-server gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AddressFamily {
    /// Ranked first (order: ipv4 > ipv6).
    Ipv4,
    /// Ranked after IPv4.
    Ipv6,
}

/// One entry of a server row's `connections` array.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Connection {
    /// `address` as recorded.
    pub address: String,
    /// `port` as recorded.
    pub port: u16,
    /// Normalized `protocol`.
    pub protocol: Protocol,
    /// `uri` as recorded (scheme://host:port), the base this client uses for
    /// `https` candidates.
    pub uri: String,
    /// `local`.
    pub local: bool,
    /// `relay` on this connection.
    pub relay: bool,
    /// `IPv6`.
    pub ip_v6: bool,
    /// `httpuri`, the documented HTTP fallback base of an `https` connection.
    /// Listed after the `https` candidate, never used silently.
    pub httpuri: Option<String>,
}

impl Connection {
    /// Where this connection is expected to reach the server from.
    #[must_use]
    pub fn reachability(&self) -> Reachability {
        if self.relay {
            Reachability::Relay
        } else if self.local {
            Reachability::Local
        } else {
            Reachability::Remote
        }
    }

    /// Address family used for ranking (see [`AddressFamily`]).
    #[must_use]
    pub fn family(&self) -> AddressFamily {
        if self.ip_v6 || self.address.contains(':') {
            AddressFamily::Ipv6
        } else {
            AddressFamily::Ipv4
        }
    }

    /// The base URL a request to this connection would use.
    #[must_use]
    pub fn base_url(&self) -> &str {
        &self.uri
    }
}

/// A request target derived from a connection: the connection itself plus the
/// base URL and transport to use for it.
///
/// A `https` connection contributes two candidates: its `uri` (`https`) and,
/// only after every `https` candidate of every connection has been tried, its
/// recorded `httpuri` (`http`). The reference derives `httpuri` as
/// `http://{address}:{port}` for every connection (`myplex.py` L1625, cited in
/// docs/PLEX_AUTH.md §1 item 6). Whether an `http` fallback to a
/// `httpsRequired` host works is **[U]** (docs/PLEX_AUTH.md §5 gap 5), so it
/// is last and it is always reported as a fallback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// The connection this candidate came from.
    pub connection: Connection,
    /// Base URL for requests on this candidate.
    pub base_url: String,
    /// Transport used on this candidate.
    pub protocol: Protocol,
    /// Reachability class used for ordering.
    pub reachability: Reachability,
    /// Address family class used for ordering.
    pub family: AddressFamily,
    /// True when this candidate is the recorded `httpuri` fallback of an
    /// `https` connection.
    pub fallback: bool,
}

/// Order the candidates are tried in: local > remote > relay, then
/// `https` > `http`, then IPv4 > IPv6, then recorded order (stable).
///
/// Candidates that cannot produce a usable base URL are dropped here, and
/// candidates whose transport is forbidden are filtered out:
///
/// * when `https_required` is true, every `http` candidate is dropped — an
///   `http` request to a server that requires TLS is not attempted;
/// * a row with `dns_rebinding_protection` keeps its candidates, because the
///   rebinding host allow-list (`allowedIps`) is outside the verified field set
///   and this client does not invent it.
#[must_use]
pub fn candidate_order(connections: &[Connection], https_required: bool) -> Vec<Candidate> {
    let mut candidates: Vec<Candidate> = Vec::new();
    for connection in connections {
        // `push_candidate` derives the transport from the URL it is given and
        // marks an `http` URL that came from an `https` connection as a
        // fallback, so the two call sites differ only in where the URL came
        // from.
        push_candidate(&mut candidates, connection, connection.uri.clone(), false);
        if let Some(httpuri) = &connection.httpuri {
            push_candidate(&mut candidates, connection, httpuri.clone(), true);
        }
    }
    let mut ordered: Vec<Candidate> = candidates
        .into_iter()
        .filter(|candidate| !https_required || candidate.protocol == Protocol::Https)
        .collect();
    ordered.sort_by(|left, right| {
        (left.reachability, left.protocol, left.family, left.fallback).cmp(&(
            right.reachability,
            right.protocol,
            right.family,
            right.fallback,
        ))
    });
    ordered
}

fn push_candidate(
    candidates: &mut Vec<Candidate>,
    connection: &Connection,
    url: String,
    recorded_as_fallback: bool,
) {
    let base_url = url.trim_end_matches('/');
    let Ok(parsed) = base_url.parse::<url::Url>() else {
        return;
    };
    if !matches!(parsed.scheme(), "https" | "http") {
        return;
    }
    let protocol = if parsed.scheme() == "https" {
        Protocol::Https
    } else {
        Protocol::Http
    };
    // A candidate is never quieter than the transport it actually uses: an
    // `https` connection's `httpuri` is an `http` candidate, and is marked.
    let fallback = recorded_as_fallback
        || (connection.protocol == Protocol::Https && protocol == Protocol::Http);
    candidates.push(Candidate {
        connection: connection.clone(),
        base_url: base_url.to_owned(),
        protocol,
        reachability: connection.reachability(),
        family: connection.family(),
        fallback,
    });
}

/// A server row of the `resources` answer, with its **own** access token.
///
/// The token is a [`SecretString`]: it prints as `[redacted]`, and the only
/// way to move it onto a request is [`ServerResource::token`], which hands
/// back another `SecretString` for `X-Plex-Token`. `Debug` never prints it.
#[derive(Clone, PartialEq, Eq)]
pub struct ServerResource {
    client_identifier: String,
    device: Option<String>,
    name: Option<String>,
    provides: Option<String>,
    present: bool,
    owned: bool,
    https_required: bool,
    dns_rebinding_protection: bool,
    nat_loopback_supported: bool,
    relay_offered: bool,
    connections: Vec<Connection>,
    access_token: Option<SecretString>,
}

impl std::fmt::Debug for ServerResource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerResource")
            .field("client_identifier", &self.client_identifier)
            .field("name", &self.name)
            .field("device", &self.device)
            .field("provides", &self.provides)
            .field("present", &self.present)
            .field("owned", &self.owned)
            .field("https_required", &self.https_required)
            .field("dns_rebinding_protection", &self.dns_rebinding_protection)
            .field("nat_loopback_supported", &self.nat_loopback_supported)
            .field("relay_offered", &self.relay_offered)
            .field("connections", &self.connections)
            .field("access_token", &crate::secret::REDACTED)
            .field("has_access_token", &self.access_token.is_some())
            .finish()
    }
}

impl ServerResource {
    /// `clientIdentifier` as recorded. It is a device identifier, not a secret
    /// (docs/PLEX_AUTH.md §3), and it is how a row is matched to a
    /// connection attempt.
    #[must_use]
    pub fn client_identifier(&self) -> &str {
        &self.client_identifier
    }

    /// `name`, the label a UI shows.
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// `device`.
    #[must_use]
    pub fn device(&self) -> Option<&str> {
        self.device.as_deref()
    }

    /// Label for a UI: `name`, else `device`, else the identifier.
    ///
    /// All three are **server-supplied text**, so the label this client renders
    /// is passed through [`crate::log::sanitize`]: bounded in length and
    /// flattened to one line with control characters neutralised. A hostile or
    /// malformed row can therefore not inject a newline (a forged log record),
    /// an ANSI escape or an arbitrarily long string into UI or log output.
    /// [`ServerResource::name`] and [`ServerResource::device`] still hand back
    /// what the server sent, for a UI that renders it in a safe widget.
    #[must_use]
    pub fn display_name(&self) -> String {
        let raw = self
            .name
            .as_deref()
            .or(self.device.as_deref())
            .unwrap_or(&self.client_identifier);
        crate::log::sanitize(raw, crate::log::MAX_LABEL_LEN)
    }

    /// `provides` as recorded (`"server"` for a PMS).
    #[must_use]
    pub fn provides(&self) -> Option<&str> {
        self.provides.as_deref()
    }

    /// `presence`.
    #[must_use]
    pub fn is_present(&self) -> bool {
        self.present
    }

    /// `owned` — **the only ownership signal this client uses**. A shared
    /// (`owned = 0`) row is not owned, and `ownedBy` is never consulted.
    #[must_use]
    pub fn is_owned(&self) -> bool {
        self.owned
    }

    /// `httpsRequired` as recorded (`myplex.py` L1464; docs/PLEX_AUTH.md
    /// §1 item 6). What Plex requires TLS for is §5 gap 5 `[U]`; this client
    /// only uses the flag to drop `http` candidates.
    #[must_use]
    pub fn requires_https(&self) -> bool {
        self.https_required
    }

    /// `dnsRebindingProtection` (informational; see the module docs).
    #[must_use]
    pub fn has_dns_rebinding_protection(&self) -> bool {
        self.dns_rebinding_protection
    }

    /// `natLoopbackSupported` (informational; see the module docs).
    #[must_use]
    pub fn supports_nat_loopback(&self) -> bool {
        self.nat_loopback_supported
    }

    /// `relay` on the row: this server offers a relay connection.
    #[must_use]
    pub fn offers_relay(&self) -> bool {
        self.relay_offered
    }

    /// `connections` in the order the server recorded them.
    #[must_use]
    pub fn connections(&self) -> &[Connection] {
        &self.connections
    }

    /// The candidates to try, in probing order (see [`candidate_order`]).
    #[must_use]
    pub fn candidates(&self) -> Vec<Candidate> {
        candidate_order(&self.connections, self.https_required)
    }

    /// True when this row carried a usable `accessToken`.
    #[must_use]
    pub fn has_access_token(&self) -> bool {
        self.access_token.as_ref().is_some_and(|t| !t.is_empty())
    }

    /// Length of the token without revealing any character.
    #[must_use]
    pub fn token_len(&self) -> usize {
        self.access_token.as_ref().map_or(0, SecretString::len)
    }

    /// Clone the row's **own** access token for a request to this server.
    ///
    /// This is the token that must be used for that server's
    /// PMS calls; the account token used to read `resources` is a different
    /// value and must never reach a PMS host. That separation is asserted by
    /// `tests/server_connect.rs::the_account_token_never_reaches_an_owned_server_host`
    /// (executed: the mock PMS records every request the transport makes).
    #[must_use]
    pub fn token(&self) -> Option<SecretString> {
        self.access_token.clone()
    }

    /// True for a `provides == "server"` row this client may browse.
    #[must_use]
    pub fn is_server(&self) -> bool {
        self.provides.as_deref() == Some("server")
    }

    fn from_wire(wire: ResourceWire) -> Option<Self> {
        let client_identifier = wire.client_identifier.filter(|v| !v.is_empty())?;
        let connections = wire
            .connections
            .into_iter()
            .filter_map(|entry| {
                let address = entry.address.unwrap_or_default();
                let uri = entry.uri.filter(|v| !v.is_empty())?;
                let protocol = match entry.protocol.as_deref() {
                    Some(value) if value.eq_ignore_ascii_case("https") => Protocol::Https,
                    Some(value) if value.eq_ignore_ascii_case("http") => Protocol::Http,
                    _ => return None,
                };
                Some(Connection {
                    address,
                    port: entry.port.unwrap_or(0),
                    protocol,
                    uri,
                    local: entry.local.is_set(),
                    relay: entry.relay.is_set(),
                    ip_v6: entry.ip_v6.is_set(),
                    httpuri: entry.httpuri.filter(|v| !v.is_empty()),
                })
            })
            .collect();
        Some(Self {
            client_identifier,
            device: wire.device.filter(|v| !v.is_empty()),
            name: wire.name.filter(|v| !v.is_empty()),
            provides: wire.provides.filter(|v| !v.is_empty()),
            present: wire.presence.is_set(),
            owned: wire.owned.is_set(),
            https_required: wire.https_required.is_set(),
            dns_rebinding_protection: wire.dns_rebinding_protection.is_set(),
            nat_loopback_supported: wire.nat_loopback_supported.is_set(),
            relay_offered: wire.relay.is_set(),
            connections,
            access_token: wire
                .access_token
                .filter(|v| !v.is_empty())
                .map(SecretString::new),
        })
    }

    fn from_host(host: &str) -> Self {
        Self {
            client_identifier: host.to_owned(),
            device: None,
            name: None,
            provides: Some("server".to_owned()),
            present: true,
            owned: false,
            https_required: false,
            dns_rebinding_protection: false,
            nat_loopback_supported: false,
            relay_offered: false,
            connections: Vec::new(),
            access_token: None,
        }
    }
}

/// Why a row of the `resources` answer is not in the selectable list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExclusionReason {
    /// `provides != "server"` (docs/PLEX_AUTH.md §1 item 6).
    NotAServer,
    /// The row carried no usable `accessToken`.
    MissingAccessToken,
    /// The row carried no connection with a usable `uri` + `protocol`.
    NoUsableConnection,
    /// The row is this install's own device row (`clientIdentifier` equals our
    /// own `X-Plex-Client-Identifier`).
    OwnDeviceRow,
}

impl ExclusionReason {
    /// Machine-readable code for a UI/CLI.
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::NotAServer => "not-a-server",
            Self::MissingAccessToken => "missing-access-token",
            Self::NoUsableConnection => "no-usable-connection",
            Self::OwnDeviceRow => "own-device-row",
        }
    }

    /// Actionable text a UI can show. Built only from fixed strings plus the
    /// label, which is sanitized here as well (a label derived from a server row
    /// is untrusted text): no token, no response body, one line, bounded length.
    #[must_use]
    pub fn message(self, label: &str) -> String {
        let label = crate::log::sanitize(label, crate::log::MAX_LABEL_LEN);
        let label = label.as_str();
        match self {
            Self::NotAServer => format!("{label} is not a media server, it cannot be browsed"),
            Self::MissingAccessToken => format!(
                "{label} did not provide its own access token, so this client cannot contact it"
            ),
            Self::NoUsableConnection => format!(
                "{label} advertised no connection this client can address (check the server's \
                 published address/port and whether https is required)"
            ),
            Self::OwnDeviceRow => format!("{label} is this client's own device row"),
        }
    }
}

/// A row that was seen but not selected, with the reason.
///
/// An excluded row is **not thrown away**: its label, flags and recorded
/// connections are kept, so a UI can say exactly *which* row was excluded, why,
/// and what that row actually advertised. This is what keeps a shared
/// (`owned = 0`) server honest: the address it really publishes stays available
/// instead of a UI having to invent one (see [`shared_resource_from_host`],
/// which is for a host the **user** typed, not for a row plex.tv answered).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Excluded {
    /// Why this row is not selectable.
    pub reason: ExclusionReason,
    /// `clientIdentifier` when the row carried one.
    pub client_identifier: Option<String>,
    /// Sanitized `name`/`device` label for the UI.
    pub label: String,
    /// `provides` as recorded.
    pub provides: Option<String>,
    /// `owned` as recorded.
    pub owned: bool,
    /// `httpsRequired` as recorded.
    pub https_required: bool,
    /// The connections this row recorded, in recorded order.
    connections: Vec<Connection>,
    /// This row's **own** `accessToken`, when it carried one.
    access_token: Option<SecretString>,
}

impl Excluded {
    /// What this row advertised as addresses, exactly as recorded.
    ///
    /// A UI offering a **user-confirmed** connect to a shared server must use
    /// these connections rather than a hand-built `host:32400` guess.
    #[must_use]
    pub fn connections(&self) -> &[Connection] {
        &self.connections
    }

    /// Candidates derived from these connections, in probing order.
    #[must_use]
    pub fn candidates(&self) -> Vec<Candidate> {
        candidate_order(&self.connections, self.https_required)
    }

    /// True when the row carried a usable `accessToken` of its own.
    #[must_use]
    pub fn has_access_token(&self) -> bool {
        self.access_token
            .as_ref()
            .is_some_and(|token| !token.is_empty())
    }

    /// Length of that token without revealing any character.
    #[must_use]
    pub fn token_len(&self) -> usize {
        self.access_token.as_ref().map_or(0, SecretString::len)
    }

    /// This row's own `accessToken`, when it carried one.
    ///
    /// Kept so that an **explicit, user-confirmed** connection (see
    /// [`crate::server::connect_shared`]) can use the credential plex.tv
    /// recorded for this row instead of a credential this client guessed. No
    /// call in this crate sends it to that server automatically: an
    /// `Excluded` row is, by definition, a row this client does not browse.
    #[must_use]
    pub fn token(&self) -> Option<SecretString> {
        self.access_token.clone()
    }
}

/// The result of one `api/v2/resources` read, split by the milestone M3 rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerList {
    selectable: Vec<ServerResource>,
    excluded: Vec<Excluded>,
}

impl ServerList {
    /// Servers this client may connect to and browse.
    #[must_use]
    pub fn servers(&self) -> &[ServerResource] {
        &self.selectable
    }

    /// Rows that were seen and why they are not browsable.
    #[must_use]
    pub fn excluded(&self) -> &[Excluded] {
        &self.excluded
    }

    /// The selectable row with this `clientIdentifier`.
    #[must_use]
    pub fn find(&self, client_identifier: &str) -> Option<&ServerResource> {
        self.selectable
            .iter()
            .find(|server| server.client_identifier() == client_identifier)
    }

    fn from_rows(rows: Vec<ServerResource>, our_identifier: &str) -> Self {
        let mut selectable = Vec::new();
        let mut excluded = Vec::new();
        for row in rows {
            let label = row.display_name();
            let reason = if row.client_identifier() == our_identifier {
                Some(ExclusionReason::OwnDeviceRow)
            } else if !row.is_server() {
                Some(ExclusionReason::NotAServer)
            } else if !row.has_access_token() {
                Some(ExclusionReason::MissingAccessToken)
            } else if row.candidates().is_empty() {
                Some(ExclusionReason::NoUsableConnection)
            } else {
                None
            };
            match reason {
                None => selectable.push(row),
                Some(reason) => excluded.push(Excluded {
                    reason,
                    client_identifier: Some(row.client_identifier().to_owned()),
                    label,
                    provides: row.provides().map(str::to_owned),
                    owned: row.is_owned(),
                    https_required: row.requires_https(),
                    connections: row.connections().to_vec(),
                    access_token: row.token(),
                }),
            }
        }
        Self {
            selectable,
            excluded,
        }
    }
}

/// Parse an `api/v2/resources` body into the split [`ServerList`].
///
/// `our_identifier` is this install's `X-Plex-Client-Identifier`; a row that
/// carries it is our own device row and is excluded.
pub fn parse_resources(body: &[u8], our_identifier: &str) -> Result<ServerList, CoreError> {
    let wire: ResourcesWire = serde_json::from_slice(body).map_err(|_| CoreError::ResponseShape)?;
    let rows = match wire {
        ResourcesWire::List(rows) => rows,
        ResourcesWire::Wrapped { container } => {
            let mut rows = container.kebab;
            rows.extend(container.camel);
            rows
        }
    };
    let servers = rows
        .into_iter()
        .filter_map(ServerResource::from_wire)
        .collect::<Vec<_>>();
    Ok(ServerList::from_rows(servers, our_identifier))
}

/// `GET api/v2/resources?includeHttps=1&includeRelay=1&includeIPv6=1`.
///
/// The `token` here is the **account** token from sign-in: this is the only
/// call here that uses it, and it goes to plex.tv only. The
/// returned rows each carry their own `accessToken`, which is what every
/// later call to that server uses.
///
/// 401/422 becomes [`CoreError::AuthRejected`], which the caller must handle by
/// clearing the stored token and pairing again (`crate::api::clear_token_on_auth_rejection`).
/// 429 is surfaced as [`CoreError::HttpStatus`] because the 429 rules are an
/// open question (docs/PLEX_AUTH.md §5 gap 4), not a retry policy we invent.
pub fn fetch_resources(
    http: &Http,
    account_token: &SecretString,
    our_identifier: &str,
) -> Result<ServerList, CoreError> {
    let response = http.get(RESOURCES_PATH, &RESOURCES_QUERY, Some(account_token))?;
    if !response.is_success() {
        return Err(CoreError::from_status(response.status));
    }
    parse_resources(&response.body, our_identifier)
}

/// A server the user addressed directly (`ip:port` / `host:port`).
///
/// This is for a host **the user typed** (a LAN address, or a server with no
/// `resources` row at all): there is no row, so there is **no resource token**,
/// and the caller must hand over the credential it wants used through
/// [`crate::server::connect_shared`], which is the only path here
/// that sends a token it did not get from a `resources` row to a PMS. Whether
/// Plex accepts the account token on a server that is shared rather than owned
/// is **[U]**: docs/PLEX_AUTH.md §1 item 6 records that a `resources` row is
/// contacted with **its own** `accessToken` (L1594) and §3 records the account
/// token as the account-wide bearer secret for plex.tv calls; shared-server
/// authorization is §5 gap 7 and is not yet verified against a real server.
///
/// A shared row that plex.tv **did** answer is handled differently: its recorded
/// connections and its own `accessToken` stay available on the [`Excluded`]
/// entry (`Excluded::connections`, `Excluded::token`), so a delegated connect
/// uses what the server published instead of the synthetic candidates built
/// here. The two connections built here are this client's own construction for a
/// bare `host:port` (documented as such, not a Plex answer), and the ordering in
/// [`candidate_order`] still puts `https` before `http`.
///
/// Explicitly **not** implemented, and not guessed at: Plex Home / managed-user
/// scoping (`docs/PLEX_AUTH.md` §5 gaps 7–8). A managed or guest account
/// may see fewer servers and fewer libraries than its token would allow; this
/// client does not read `plex.tv/api/home/users*` and does not infer scopes.
pub fn shared_resource_from_host(host_and_port: &str) -> Result<ServerResource, CoreError> {
    let host_and_port = host_and_port.trim();
    if host_and_port.is_empty()
        || host_and_port.contains('/')
        || host_and_port.contains(' ')
        || host_and_port.contains('@')
    {
        return Err(CoreError::InvalidInput);
    }
    // A bracketed IPv6 literal owns its closing bracket: `[2001:db8::1]` with no
    // port must default to 32400. Splitting it at the last `:` would read `1]` as
    // port text and refuse an address the user typed correctly.
    let (host, port) = if host_and_port.starts_with('[') {
        let Some(end) = host_and_port.find(']') else {
            return Err(CoreError::InvalidInput);
        };
        let rest = &host_and_port[end + 1..];
        let port = match rest {
            "" => 32400,
            rest => {
                let Some(port) = rest.strip_prefix(':') else {
                    return Err(CoreError::InvalidInput);
                };
                let Ok(port) = port.parse::<u16>() else {
                    return Err(CoreError::InvalidInput);
                };
                port
            }
        };
        (host_and_port[..=end].to_owned(), port)
    } else {
        match host_and_port.rsplit_once(':') {
            Some((host, port)) => {
                let Ok(port) = port.parse::<u16>() else {
                    return Err(CoreError::InvalidInput);
                };
                (host.to_owned(), port)
            }
            None => (host_and_port.to_owned(), 32400),
        }
    };
    let address = normalize_host(&host)?;
    if port == 0 {
        return Err(CoreError::InvalidInput);
    }
    // An IPv6 literal is only a valid URL host when bracketed. Bracketing it
    // here is what keeps the diagnostic honest: without it the URIs built below
    // are unparseable, `push_candidate` drops every candidate, and the user is
    // left with a "0 tried" failure that names nothing. A bare `x:y:32400` is
    // also inherently ambiguous (IPv6 without a port); this client reads the
    // last `:` as the port separator, which is what a user typing `::1:32400`
    // means.
    let host_text = if address.contains(':') {
        format!("[{address}]")
    } else {
        address.clone()
    };
    let ip_v6 = address.contains(':');
    let mut server = ServerResource::from_host(&host_text);
    // docs/PLEX_AUTH.md §1 item 7: 32400 is Plex's published primary port.
    server.connections.push(Connection {
        address: host_text.clone(),
        port,
        protocol: Protocol::Http,
        uri: format!("http://{host_text}:{port}"),
        local: true,
        relay: false,
        ip_v6,
        httpuri: None,
    });
    server.connections.push(Connection {
        address: host_text.clone(),
        port,
        protocol: Protocol::Https,
        uri: format!("https://{host_text}:{port}"),
        local: true,
        relay: false,
        ip_v6,
        httpuri: None,
    });
    Ok(server)
}

/// Validate a typed host and normalize an IPv6 literal to its bracketed form.
///
/// Only what this client can actually address is accepted: no scheme, path,
/// userinfo or whitespace (checked by the caller), a bracketed literal only in
/// full, and an unbracketed literal made of hex digits and `:` only.
fn normalize_host(host: &str) -> Result<String, CoreError> {
    if host.is_empty() {
        return Err(CoreError::InvalidInput);
    }
    if let Some(inner) = host.strip_prefix('[') {
        let inner = inner.strip_suffix(']').ok_or(CoreError::InvalidInput)?;
        if inner.is_empty() || !inner.contains(':') {
            return Err(CoreError::InvalidInput);
        }
        // The bracketed form is the canonical IPv6 literal, so it takes the
        // same alphabet as the unbracketed one: hex digits, `:` and `.` (for
        // IPv4-mapped literals such as `[::ffff:192.0.2.1]`). Accepting only
        // decimal digits here would reject the form this client itself prints.
        if !inner
            .chars()
            .all(|c| c.is_ascii_hexdigit() || c == ':' || c == '.')
        {
            return Err(CoreError::InvalidInput);
        }
        return Ok(inner.to_owned());
    }
    if host.contains(':') {
        if !host
            .chars()
            .all(|c| c.is_ascii_hexdigit() || c == ':' || c == '.')
        {
            return Err(CoreError::InvalidInput);
        }
        return Ok(host.to_owned());
    }
    Ok(host.to_owned())
}
