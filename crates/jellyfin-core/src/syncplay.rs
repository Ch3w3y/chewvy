//! SyncPlay: the server's own way of keeping several players in step.
//!
//! A group lives on the server. Members ask it for changes over REST (the
//! calls here, all on [`Client`]); the server decides, and tells every
//! member — the one who asked included — over each member's session socket
//! ([`crate::socket`], with the frames of [`message`]). A client therefore
//! never acts on its own button while in a group: it asks, then follows what
//! the socket says.
//!
//! Commands are scheduled on the server's clock ([`clock`]). The behaviour a
//! 12.4 server showed, and that a client has to follow, is written up in
//! docs/JELLYFIN.md, "SyncPlay".
//!
//! Every call answers `403` with an empty body both when the user has no
//! SyncPlay access and when this session is in no group (for the calls that
//! need one); that arrives as [`JfError::AuthRejected`] with `status: 403`.
//! A rejected token is `401`.

pub mod clock;
pub mod message;

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::browse::{validate_id, Client};
use crate::error::JfError;
use crate::ticks;
use clock::{ClockSample, UtcTime};

/// `GET /SyncPlay/List`, `GET /SyncPlay/{id}` and the `POST /SyncPlay/…`
/// requests.
pub const SYNCPLAY_PATH: &str = "/SyncPlay";

/// `GET /GetUtcTime`: the server's clock, for [`ClockSample`].
pub const UTC_TIME_PATH: &str = "/GetUtcTime";

/// Where a group is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum GroupState {
    /// Nothing queued, or stopped.
    Idle,
    /// Holding until every member has reported `Ready`.
    Waiting,
    /// Paused by a member.
    Paused,
    /// Playing.
    Playing,
}

/// `GroupInfoDto`: one group as the server lists it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GroupInfo {
    /// The group's id (32 hex digits).
    pub group_id: String,
    /// The name its creator gave it.
    pub group_name: String,
    /// Where the group is.
    pub state: GroupState,
    /// The names of the users in it, each once however many devices they
    /// joined with.
    pub participants: Vec<String>,
    /// When this answer was made, on the server's clock.
    pub last_updated_at: UtcTime,
}

/// `GET /GetUtcTime`: the two server timestamps of one exchange.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UtcTimeAnswer {
    /// When the server received the request.
    pub request_reception_time: UtcTime,
    /// When the server sent the answer.
    pub response_transmission_time: UtcTime,
}

/// What `Ready` and `Buffering` tell the group about this player.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerReport<'a> {
    /// The server-clock instant [`PlayerReport::position`] was read at.
    pub when: UtcTime,
    /// Where the player is.
    pub position: Duration,
    /// Whether the player is playing. The server advances a playing
    /// player's position by the time since [`PlayerReport::when`].
    pub is_playing: bool,
    /// The queue entry the player has loaded.
    pub playlist_item_id: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct NewGroupBody<'a> {
    group_name: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct JoinBody<'a> {
    group_id: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct NewQueueBody<'a> {
    playing_queue: &'a [&'a str],
    playing_item_position: u32,
    start_position_ticks: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct SeekBody {
    position_ticks: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct ReportBody<'a> {
    when: UtcTime,
    position_ticks: i64,
    is_playing: bool,
    playlist_item_id: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct PingBody {
    ping: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct IgnoreWaitBody {
    ignore_wait: bool,
}

impl Client<'_> {
    /// The groups this user can join: `GET /SyncPlay/List`.
    ///
    /// # Errors
    ///
    /// As every call here; see the module notes for `403`.
    pub fn syncplay_groups(&self) -> Result<Vec<GroupInfo>, JfError> {
        self.get_json(&format!("{SYNCPLAY_PATH}/List"), &[])
    }

    /// One group: `GET /SyncPlay/{id}`.
    ///
    /// # Errors
    ///
    /// [`JfError::InvalidInput`] for an id this client cannot address; a
    /// group that no longer exists is `HttpStatus { status: 404 }`.
    pub fn syncplay_group(&self, group_id: &str) -> Result<GroupInfo, JfError> {
        validate_id(group_id)?;
        self.get_json(&format!("{SYNCPLAY_PATH}/{group_id}"), &[])
    }

    /// Start a group and join it: `POST /SyncPlay/New`.
    ///
    /// A 12.4 server answers the new group; the API also allows an empty
    /// `204`, which is `None`. The socket's `GroupJoined` says the same
    /// either way.
    ///
    /// # Errors
    ///
    /// As [`Client::syncplay_groups`].
    pub fn syncplay_new_group(&self, name: &str) -> Result<Option<GroupInfo>, JfError> {
        let response = self.syncplay_post("New", &NewGroupBody { group_name: name })?;
        if response.body.is_empty() {
            Ok(None)
        } else {
            response.json().map(Some)
        }
    }

    /// Join a group: `POST /SyncPlay/Join`. A group that does not exist
    /// still answers `204`; the socket then says `GroupDoesNotExist`.
    ///
    /// # Errors
    ///
    /// As [`Client::syncplay_group`].
    pub fn syncplay_join(&self, group_id: &str) -> Result<(), JfError> {
        validate_id(group_id)?;
        self.syncplay_post("Join", &JoinBody { group_id }).map(drop)
    }

    /// Leave the group: `POST /SyncPlay/Leave`.
    ///
    /// # Errors
    ///
    /// As [`Client::syncplay_groups`].
    pub fn syncplay_leave(&self) -> Result<(), JfError> {
        self.syncplay_post_empty("Leave")
    }

    /// Replace the group's queue and start it from `start` in the entry at
    /// `playing_index`: `POST /SyncPlay/SetNewQueue`. The group waits for
    /// every member's `Ready`, then plays.
    ///
    /// # Errors
    ///
    /// [`JfError::InvalidInput`] for an empty queue or an id this client
    /// cannot address; otherwise as [`Client::syncplay_groups`].
    pub fn syncplay_set_new_queue(
        &self,
        item_ids: &[&str],
        playing_index: u32,
        start: Duration,
    ) -> Result<(), JfError> {
        if item_ids.is_empty() {
            return Err(JfError::InvalidInput);
        }
        for id in item_ids {
            validate_id(id)?;
        }
        let body = NewQueueBody {
            playing_queue: item_ids,
            playing_item_position: playing_index,
            start_position_ticks: ticks::duration_to_ticks(start),
        };
        self.syncplay_post("SetNewQueue", &body).map(drop)
    }

    /// Ask the group to play: `POST /SyncPlay/Unpause`.
    ///
    /// # Errors
    ///
    /// As [`Client::syncplay_groups`].
    pub fn syncplay_unpause(&self) -> Result<(), JfError> {
        self.syncplay_post_empty("Unpause")
    }

    /// Ask the group to pause: `POST /SyncPlay/Pause`.
    ///
    /// # Errors
    ///
    /// As [`Client::syncplay_groups`].
    pub fn syncplay_pause(&self) -> Result<(), JfError> {
        self.syncplay_post_empty("Pause")
    }

    /// Ask the group to stop: `POST /SyncPlay/Stop`.
    ///
    /// # Errors
    ///
    /// As [`Client::syncplay_groups`].
    pub fn syncplay_stop(&self) -> Result<(), JfError> {
        self.syncplay_post_empty("Stop")
    }

    /// Ask the group to move to `position`: `POST /SyncPlay/Seek`. The group
    /// waits for every member's `Ready` at the new position.
    ///
    /// # Errors
    ///
    /// As [`Client::syncplay_groups`].
    pub fn syncplay_seek(&self, position: Duration) -> Result<(), JfError> {
        let body = SeekBody {
            position_ticks: ticks::duration_to_ticks(position),
        };
        self.syncplay_post("Seek", &body).map(drop)
    }

    /// Tell the group this player has stalled: `POST /SyncPlay/Buffering`.
    /// The group pauses everyone and waits.
    ///
    /// # Errors
    ///
    /// As [`Client::syncplay_set_new_queue`].
    pub fn syncplay_buffering(&self, report: &PlayerReport<'_>) -> Result<(), JfError> {
        self.syncplay_report("Buffering", report)
    }

    /// Tell the group this player can play: `POST /SyncPlay/Ready`.
    ///
    /// The server compares the reported position with the group's; too far
    /// apart (0.6 s was, 0.4 s was not), it answers this member alone with a
    /// `Seek` and keeps waiting.
    ///
    /// # Errors
    ///
    /// As [`Client::syncplay_set_new_queue`].
    pub fn syncplay_ready(&self, report: &PlayerReport<'_>) -> Result<(), JfError> {
        self.syncplay_report("Ready", report)
    }

    /// Report this device's round trip to the server, which it uses to
    /// schedule commands: `POST /SyncPlay/Ping`.
    ///
    /// # Errors
    ///
    /// As [`Client::syncplay_groups`].
    pub fn syncplay_ping(&self, round_trip: Duration) -> Result<(), JfError> {
        let ping = u64::try_from(round_trip.as_millis()).unwrap_or(u64::MAX);
        self.syncplay_post("Ping", &PingBody { ping }).map(drop)
    }

    /// Whether the group should stop waiting for this member:
    /// `POST /SyncPlay/SetIgnoreWait`.
    ///
    /// # Errors
    ///
    /// As [`Client::syncplay_groups`].
    pub fn syncplay_set_ignore_wait(&self, ignore_wait: bool) -> Result<(), JfError> {
        self.syncplay_post("SetIgnoreWait", &IgnoreWaitBody { ignore_wait })
            .map(drop)
    }

    /// The server's clock: `GET /GetUtcTime`.
    ///
    /// # Errors
    ///
    /// Transport, status and shape errors.
    pub fn utc_time(&self) -> Result<UtcTimeAnswer, JfError> {
        self.get_json(UTC_TIME_PATH, &[])
    }

    /// One clock exchange, timed on this device's clock.
    ///
    /// # Errors
    ///
    /// As [`Client::utc_time`].
    pub fn clock_sample(&self) -> Result<ClockSample, JfError> {
        let sent = UtcTime::now();
        let answer = self.utc_time()?;
        let received = UtcTime::now();
        Ok(ClockSample::new(
            sent,
            answer.request_reception_time,
            answer.response_transmission_time,
            received,
        ))
    }

    fn syncplay_report(&self, path: &str, report: &PlayerReport<'_>) -> Result<(), JfError> {
        validate_id(report.playlist_item_id)?;
        let body = ReportBody {
            when: report.when,
            position_ticks: ticks::duration_to_ticks(report.position),
            is_playing: report.is_playing,
            playlist_item_id: report.playlist_item_id,
        };
        self.syncplay_post(path, &body).map(drop)
    }

    fn syncplay_post(
        &self,
        path: &str,
        body: &impl Serialize,
    ) -> Result<crate::http::Response, JfError> {
        let body = serde_json::to_string(body).map_err(|_| JfError::InvalidInput)?;
        self.http.post(
            &format!("{SYNCPLAY_PATH}/{path}"),
            &[],
            Some(&body),
            Some(&self.authorization()),
        )
    }

    fn syncplay_post_empty(&self, path: &str) -> Result<(), JfError> {
        self.http
            .post(
                &format!("{SYNCPLAY_PATH}/{path}"),
                &[],
                None,
                Some(&self.authorization()),
            )
            .map(drop)
    }
}
