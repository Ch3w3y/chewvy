//! `plex-core`: the display-free Plex client core layer.
//!
//! Milestone M2 fills `client_info`, `http`, `error`, `secret`, `log`, `auth`,
//! `api` (account + signout) and `tokens`. Milestone M3 adds `resources`
//! (server discovery + selection rules), `server` (connecting to one server and
//! capturing its identifier) and `library` (read-only section/item/search and
//! metadata models). The layer contract that later changes must not break:
//!
//! * no UI, no display server dependency, no windowing/OpenGL/libmpv code here;
//! * blocking HTTP work runs on a background thread owned by the caller and
//!   reports to the UI through channels, never from a UI callback;
//! * Plex tokens travel only in the `X-Plex-Token` header, never in query
//!   strings, logs, argv, error `Display` output or UI text;
//! * typed errors (thiserror) with the token redacted before formatting;
//! * no code path anywhere in this crate accepts a Plex account password.

#![deny(unsafe_code)]
#![warn(missing_docs)]

/// Stable per-install client identity and the `X-Plex-*` header builder
/// (Product, Client-Identifier, Version, Platform, Platform-Version, Device,
/// Device-Name, Provides, Language, Accept) plus header-only `X-Plex-Token`.
pub mod client_info;
/// Typed core errors; every variant is closed so no secret can reach
/// `Display`/`Debug`.
pub mod error;
/// Blocking HTTP transport (ureq 3.3.0 + rustls/ring) with typed,
/// token-redacted errors and bounded timeouts.
pub mod http;
/// Log capture backend used to *prove* tokens never reach log output, plus the
/// secret scrubbing helper used before rendering foreign error text.
pub mod log;
/// A secret string wrapper that is inert in `Debug`/`Display`/logs/errors.
pub mod secret;

/// The XDG Secret Service (`org.freedesktop.secrets`) token backend; re-
/// exported from [`tokens`] so storage has a single import site.
pub mod secret_service;

/// Plex PIN browser-authorization state machine: create pin, poll,
/// pending -> claimed | expired | cancelled | network_error | rejected(401/422),
/// bounded backoff, no blind retry, no password path.
pub mod auth;

/// Token storage abstraction: XDG Secret Service over zbus 5.19.0 first, a
/// `0600` file fallback only behind explicit, recorded in-app consent.
pub mod tokens;

/// `api/v2` account (`GET /api/v2/user`) and logout
/// (`DELETE /api/v2/users/signout`) calls. Server discovery and connection
/// probing live in `resources` + `server`.
pub mod api;

/// Server discovery: the `api/v2/resources` model restricted to the verified
/// field set, the selection rules (`provides == "server"` + `owned`), and the
/// candidate order local > remote > relay, https > http, ipv4 > ipv6.
pub mod resources;

/// Connecting to one Plex Media Server through one candidate connection,
/// the per-attempt failure records a UI shows, and the machine identifier that
/// is only ever read from a server that answered.
pub mod server;

/// Library sections, items, search results and metadata models parsed from Plex
/// JSON, read-only. The direct-play decision against the tested capability
/// matrix is milestone M5/M6.
pub mod library;

/// **Opt-in** playback position and watched-state reports: the only two requests
/// this client makes that change what a server remembers, off by default. A
/// disabled update performs no transport at all.
pub mod progress;
