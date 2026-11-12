//! Playback position and watched state on a Plex Media Server - the two calls
//! this client can make that change what the server remembers.
//!
//! Position goes to [`TIMELINE_PATH`]. The server turns those reports into an
//! item's resume point (`viewOffset`), its "On Deck" shelf, and - past its own
//! per-library completion threshold - the watched flag. Marking watched is the
//! separate [`SCROBBLE_PATH`] call a client makes for a user action rather than
//! for playback; the client never edits library metadata or deletes anything.
//!
//! Both endpoints are addressed by rating key and answered with a body nothing
//! here reads, so only the status is interesting here. Times are
//! **milliseconds**, the same unit
//! `GET /library/metadata/{ratingKey}` reports `duration` and `viewOffset` in;
//! sending a position in seconds sets the resume point a few seconds in and is
//! the difference between "continues where you left off" and "starts over".
//!
//! Playback reports are **opt-in** and off by default, per the accepted plan's
//! mutation policy: [`ProgressUpdate::new`] leaves [`ProgressUpdate::enabled`]
//! `false` and [`report_timeline`] answers [`Outcome::NotEnabled`] **before** it
//! touches a connection, so a client that never opts in sends zero requests. That
//! claim is measured against recorded loopback traffic by
//! `crates/plex-core/tests/library_read_only.rs`, and the "no other write exists"
//! claim by `crates/plex-core/tests/mutation_surface.rs`.
//!
//! Sources (accessed 2026-11-10): Plex's
//! own [PMS reference](https://developer.plex.tv/pms/) (OpenAPI 3.1.0,
//! "Plex Media Server" 1.2.3), which names the two paths, the `time`/`duration`
//! millisecond unit, the `stopped|buffering|playing|paused` states, the ~10 s
//! report cadence, and - for the scrobble - "does respond to the GET verb but
//! applications should use PUT" while declaring the operation itself as `PUT`
//! (and `/timeline` as `POST`); python-plexapi, which sends both as `GET`
//! (<https://github.com/pkkid/python-plexapi/blob/master/plexapi/base.py#L1012-L1028>,
//! <https://github.com/pkkid/python-plexapi/blob/master/plexapi/mixins/played_unplayed.py>);
//! and Plex Media Server request lines quoted by server owners, which show
//! `GET /:/timeline?...` arriving with millisecond times and answered `200`.
//! This client sends `GET`, following the working clients; whether the
//! operator's PMS answers `GET` on both routes is a real-server smoke test.

use std::time::Duration;

use crate::error::CoreError;
use crate::http::Response;
use crate::server::Server;

/// `GET /:/timeline`: one playback position.
pub const TIMELINE_PATH: &str = "/:/timeline";
/// `GET /:/scrobble`: mark one item watched.
pub const SCROBBLE_PATH: &str = "/:/scrobble";
/// The library plugin both endpoints file their answer under, sent as
/// [`QUERY_IDENTIFIER`] on every call in this module. Plex's own parameter list
/// for `/timeline` omits it, but python-plexapi and logged clients send it there
/// too, so this module addresses both calls the same way.
pub const LIBRARY_IDENTIFIER: &str = "com.plexapp.plugins.library";

/// Query key for the library plugin ([`LIBRARY_IDENTIFIER`]).
pub const QUERY_IDENTIFIER: &str = "identifier";
/// Query key for the item's rating key on `/timeline` (`/scrobble` names the
/// same value `key`; Plex's own description of that parameter says so).
pub const QUERY_RATING_KEY: &str = "ratingKey";
/// Optional query key for the item's library path (`/library/metadata/{id}`),
/// which the Plex clients that maintain a play queue also send to `/timeline`.
pub const QUERY_KEY: &str = "key";
/// Query key for the play state ([`PlayState::wire`]).
pub const QUERY_STATE: &str = "state";
/// Query key for the position in milliseconds.
pub const QUERY_TIME: &str = "time";
/// Query key for the total duration in milliseconds.
pub const QUERY_DURATION: &str = "duration";

/// Longest rating key this module will put in a query (defense in depth, same
/// bound as the read paths in [`crate::library`]).
pub const MAX_RATING_KEY_LEN: usize = 256;

/// The interval a playback loop should report on. `/:/timeline` is designed to
/// be hit repeatedly while playing: Plex's own reference asks clients to hit it
/// on every play-state change and, absent a change, "every 10 seconds on a
/// LAN/WAN, and every 20 seconds over cellular". A desktop LAN client is the 10 s
/// case. This is still a client-side choice, not something a server enforces.
pub const TIMELINE_TICK: Duration = Duration::from_secs(10);

/// Where in an item's playback a report belongs. The wire labels are what a
/// server logs (`state=playing`, `state=paused`, `state=stopped`).
///
/// Plex enumerates a fourth state, `buffering`; this client does not track
/// mpv's buffering well enough to report it honestly, so it stays unsent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayState {
    /// Playback is running.
    Playing,
    /// Playback is paused.
    Paused,
    /// Playback stopped or ended.
    Stopped,
}

impl PlayState {
    /// The wire label for this state.
    #[must_use]
    pub fn wire(self) -> &'static str {
        match self {
            Self::Playing => "playing",
            Self::Paused => "paused",
            Self::Stopped => "stopped",
        }
    }
}

/// One playback position report, and the **opt-in** that gates it.
///
/// `enabled` is a field of the update rather than a parameter of
/// [`report_timeline`] on purpose: there is no call site that can send a report
/// without carrying the user's explicit opt-in in the value being passed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgressUpdate {
    /// The item's rating key.
    pub rating_key: String,
    /// Where playback is.
    pub state: PlayState,
    /// Position in milliseconds; `None` omits `time` (a state change with no
    /// position to move).
    pub time_ms: Option<u64>,
    /// Total duration in milliseconds when the player knows it.
    pub duration_ms: Option<u64>,
    /// The item's library path, sent as [`QUERY_KEY`] when the caller has it.
    pub item_key: Option<String>,
    /// The user's opt-in. `false` - the constructor default - means no request.
    pub enabled: bool,
}

impl ProgressUpdate {
    /// A **disabled** report at an unknown position: the only state a freshly
    /// built update has. Turning it on is the user's decision, via
    /// [`ProgressUpdate::opt_in`].
    #[must_use]
    pub fn new(rating_key: impl Into<String>, state: PlayState) -> Self {
        Self {
            rating_key: rating_key.into(),
            state,
            time_ms: None,
            duration_ms: None,
            item_key: None,
            enabled: false,
        }
    }

    /// The explicit opt-in: the **only** way this value becomes reportable.
    #[must_use]
    pub fn opt_in(mut self) -> Self {
        self.enabled = true;
        self
    }

    /// The observed position and duration, in milliseconds, from the player's
    /// own reported values. `None` omits either.
    #[must_use]
    pub fn with_position_ms(mut self, time_ms: Option<u64>, duration_ms: Option<u64>) -> Self {
        self.time_ms = time_ms;
        self.duration_ms = duration_ms;
        self
    }

    /// The item's library path (e.g. `/library/metadata/9`), when the caller
    /// resolved it while selecting the item.
    #[must_use]
    pub fn with_item_key(mut self, item_key: impl Into<String>) -> Self {
        self.item_key = Some(item_key.into());
        self
    }

    /// Why this report may **not** be sent, or `None` when it may.
    ///
    /// Checks in the order a caller must care about: the opt-in first (a disabled
    /// report is refused before any validation, so the default case cannot reach a
    /// connection), then the item address, then the numbers.
    #[must_use]
    pub fn refusal(&self) -> Option<&'static str> {
        if !self.enabled {
            return Some("progress reporting is off (opt-in required)");
        }
        if let Some(reason) = rating_key_refusal(&self.rating_key) {
            return Some(reason);
        }
        if let (Some(time), Some(duration)) = (self.time_ms, self.duration_ms) {
            if time > duration {
                return Some("reported position is beyond the reported duration");
            }
        }
        None
    }

    /// The query map for this report, in wire order: the item address, then the
    /// state, then the numbers the server is being asked to remember.
    #[must_use]
    pub fn timeline_query(&self) -> Vec<(String, String)> {
        let mut query = vec![
            (QUERY_RATING_KEY.to_owned(), self.rating_key.clone()),
            (QUERY_IDENTIFIER.to_owned(), LIBRARY_IDENTIFIER.to_owned()),
        ];
        if let Some(key) = &self.item_key {
            query.push((QUERY_KEY.to_owned(), key.clone()));
        }
        query.push((QUERY_STATE.to_owned(), self.state.wire().to_owned()));
        if let Some(time) = self.time_ms {
            query.push((QUERY_TIME.to_owned(), time.to_string()));
        }
        if let Some(duration) = self.duration_ms {
            query.push((QUERY_DURATION.to_owned(), duration.to_string()));
        }
        query
    }
}

/// What a progress attempt became.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// No request was made: the user has not opted in.
    NotEnabled {
        /// The documented reason, always the same one.
        reason: &'static str,
    },
    /// The request was sent and the server answered. Both progress endpoints
    /// answer with an empty body, so the status is the whole result; a `4xx` or
    /// `5xx` arrives here rather than as an error.
    Reported {
        /// Response status.
        status: u16,
    },
}

impl Outcome {
    /// Machine-readable id.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::NotEnabled { .. } => "not-enabled-no-request".to_owned(),
            Self::Reported { status } => format!("reported-http-{status}"),
        }
    }

    /// True when this outcome means a request left the process.
    #[must_use]
    pub fn made_a_request(&self) -> bool {
        matches!(self, Self::Reported { .. })
    }
}

/// The rating-key rule [`ProgressUpdate::refusal`] applies, factored out so
/// [`mark_watched`] checks exactly the same thing before addressing an item.
fn rating_key_refusal(rating_key: &str) -> Option<&'static str> {
    if rating_key.is_empty() || rating_key.len() > MAX_RATING_KEY_LEN {
        return Some("rating key empty or longer than the bound");
    }
    if !rating_key
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '%'))
    {
        return Some("rating key contains a character this query builder refuses");
    }
    None
}

/// Send one playback position to [`TIMELINE_PATH`].
///
/// A disabled or invalid update is answered locally, **without any network
/// call**: [`ProgressUpdate::refusal`] is checked first and, if it refuses, this
/// returns [`Outcome::NotEnabled`] without opening a transport. That is what
/// makes "off by default" testable as zero requests rather than as a suppressed
/// response.
///
/// # Errors
///
/// [`CoreError::Transport`] from the request itself. A non-2xx answer is not an
/// error here: it arrives as [`Outcome::Reported`] so a UI can show the status
/// the server gave.
pub fn report_timeline(server: &Server, update: &ProgressUpdate) -> Result<Outcome, CoreError> {
    if let Some(reason) = update.refusal() {
        return Ok(Outcome::NotEnabled { reason });
    }
    let query = update.timeline_query();
    let query: Vec<(&str, &str)> = query
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    let response: Response = server.get(TIMELINE_PATH, &query)?;
    Ok(Outcome::Reported {
        status: response.status,
    })
}

/// Mark one item watched with [`SCROBBLE_PATH`], as the answer to a user action.
///
/// This is **not** gated by [`ProgressUpdate::enabled`]: that switch is about
/// sending playback positions to the server continuously, while marking an item
/// watched is an explicit request a person made. Servers also set the flag
/// themselves from timeline reports once their own completion threshold is
/// passed, so a client that never calls this still tracks watched state.
///
/// Rating keys are addressed by the `key` parameter here, not `ratingKey`.
///
/// # Errors
///
/// [`CoreError::InvalidInput`] for an unusable rating key, [`CoreError::Transport`]
/// from the request itself.
pub fn mark_watched(server: &Server, rating_key: &str) -> Result<u16, CoreError> {
    if rating_key_refusal(rating_key).is_some() {
        return Err(CoreError::InvalidInput);
    }
    let response = server.get(
        SCROBBLE_PATH,
        &[("key", rating_key), (QUERY_IDENTIFIER, LIBRARY_IDENTIFIER)],
    )?;
    Ok(response.status)
}
