//! The state exchange: what to answer each `State` with, and what the group
//! wants the player to do.
//!
//! The server sends the group's state about once a second and a client
//! answers every one; a client never speaks otherwise, so a local change
//! leaves on the next answer. [`Exchange`] keeps the little state this needs
//! and does exactly what Plex's own web app does:
//!
//! * **Latency.** Each answer carries the client's clock as
//!   `clientLatencyCalculation`; the server echoes it on its next state, and
//!   the difference is the round trip. Half of it is how far a playing
//!   group has moved on by the time its state arrives.
//! * **A local change.** When the user plays, pauses or seeks, the answer
//!   reports the player's new state and raises `ignoringOnTheFly.client`,
//!   so the server takes it as a change rather than drift. Until the server
//!   acknowledges it the group's state is not followed, so a state sent
//!   before the change cannot undo it.
//! * **A change the server imposes.** A state carrying
//!   `ignoringOnTheFly.server` is another participant's change. The answer
//!   echoes that state back with the counter, which is how the server learns
//!   the client has it; a reading taken before the player caught up would
//!   otherwise count as a change of its own.
//!
//! The caller owns the clock: [`Exchange::on_state`] takes the time in
//! seconds from any fixed origin, and no other time is read.

use super::message::{IgnoringOnTheFly, PlayState, ReplyPing, State, StateReply, UserName};

/// The local player's reading when a `State` arrives.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reading {
    /// Seconds into the item.
    pub position: f64,
    /// Whether the player is paused.
    pub paused: bool,
    /// The user played or paused since the last answer that carried a
    /// change.
    pub play_pause_pending: bool,
    /// The user sought since the last answer that carried a change.
    pub seek_pending: bool,
}

impl Reading {
    fn pending(&self) -> bool {
        self.play_pause_pending || self.seek_pending
    }
}

/// The group's state for the player to follow.
#[derive(Debug, Clone, PartialEq)]
pub struct Target {
    /// Where the group is now, in seconds: the state's position, moved on by
    /// half the round trip when playing.
    pub position: f64,
    /// Whether the group is paused.
    pub paused: bool,
    /// Whether the change was a jump: move there even when close.
    pub do_seek: bool,
    /// Who set it, when the server said and the name reads.
    pub set_by: Option<UserName>,
}

/// What to do about one `State`.
#[derive(Debug, Clone, PartialEq)]
pub struct Answer {
    /// Send this back now.
    pub reply: StateReply,
    /// The group's state, when there is one to follow; `None` while this
    /// client's own change is on its way, and for this client's own state.
    pub follow: Option<Target>,
    /// The reply carries the pending play, pause or seek; clear it. When
    /// `false` with a change pending (the state was the server's), keep it
    /// for the next answer.
    pub sent_change: bool,
}

/// The exchange for one device in one room.
#[derive(Debug, Clone, PartialEq)]
pub struct Exchange {
    device_identifier: String,
    round_trip: f64,
    ignoring_client: u32,
    ignoring_server: u32,
}

impl Exchange {
    /// A new exchange for the device whose [`UserName::device_identifier`]
    /// is `device_identifier`; states it set are never followed.
    #[must_use]
    pub fn new(device_identifier: &str) -> Self {
        Self {
            device_identifier: device_identifier.to_owned(),
            round_trip: 0.0,
            ignoring_client: 0,
            ignoring_server: 0,
        }
    }

    /// The last measured round trip to the server, in seconds.
    #[must_use]
    pub fn round_trip(&self) -> f64 {
        self.round_trip
    }

    /// Take one `State` from the server, with the player's `reading` and the
    /// caller's clock `now`, in seconds, read as the state arrived: the
    /// round trip is measured to that instant. Send the reply straight away.
    pub fn on_state(&mut self, state: &State, reading: Reading, now: f64) -> Answer {
        if let Some(sent) = state.ping.client_latency_calculation {
            let round_trip = now - sent;
            if round_trip.is_finite() && round_trip >= 0.0 {
                self.round_trip = round_trip;
            }
        }

        let counters = state.ignoring_on_the_fly.unwrap_or_default();
        let acknowledged = counters.client.unwrap_or(0);
        self.ignoring_server = counters.server.unwrap_or(0);
        let echo = self.ignoring_server > 0;
        if echo || acknowledged == self.ignoring_client {
            self.ignoring_client = 0;
        }
        let sent_change = !echo && reading.pending();
        if sent_change {
            self.ignoring_client += 1;
        }

        let follow = state.playstate.as_ref().and_then(|group| {
            let own = group
                .set_by
                .as_deref()
                .and_then(UserName::parse)
                .is_some_and(|user| user.device_identifier == self.device_identifier);
            (!own && self.ignoring_client == 0 && !reading.pending()).then(|| Target {
                position: group.position
                    + if group.paused {
                        0.0
                    } else {
                        self.round_trip / 2.0
                    },
                paused: group.paused,
                do_seek: group.do_seek,
                set_by: group.set_by.as_deref().and_then(UserName::parse),
            })
        });

        let playstate = match (&state.playstate, &follow) {
            (Some(group), _) if echo => PlayState {
                position: group.position,
                paused: group.paused,
                do_seek: false,
                set_by: None,
            },
            // The player is about to move there; report where it is going.
            (_, Some(target)) if target.do_seek || target.paused != reading.paused => PlayState {
                position: target.position,
                paused: target.paused,
                do_seek: false,
                set_by: None,
            },
            _ => PlayState {
                position: reading.position,
                paused: reading.paused,
                do_seek: sent_change && reading.seek_pending,
                set_by: None,
            },
        };

        let counter = |value: u32| (value > 0).then_some(value);
        let ignoring = IgnoringOnTheFly {
            client: counter(self.ignoring_client),
            server: counter(self.ignoring_server),
        };
        let reply = StateReply {
            ping: ReplyPing {
                client_latency_calculation: now,
                client_rtt: self.round_trip,
                server_rtt: state.ping.server_rtt.unwrap_or(0.0),
                latency_calculation: state.ping.latency_calculation.unwrap_or(0.0),
            },
            playstate,
            ignoring_on_the_fly: (ignoring != IgnoringOnTheFly::default()).then_some(ignoring),
        };
        Answer {
            reply,
            follow,
            sent_change,
        }
    }
}
