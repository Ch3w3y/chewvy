//! The messages a session's socket carries, as far as SyncPlay needs them.
//!
//! Every frame is a JSON text frame `{MessageType, MessageId, Data}`. The
//! socket carries far more than SyncPlay (library changes, remote control,
//! scheduled tasks), so reading a frame never fails: a type this client does
//! not use is [`Message::Other`], and a known type whose `Data` is not the
//! shape the server's API description gives is [`Message::Malformed`]. Either
//! way the caller carries on reading.

use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;

use super::clock::UtcTime;
use super::{GroupInfo, GroupState};
use crate::ticks;

/// What a client sends to keep its socket open.
pub const KEEP_ALIVE: &str = r#"{"MessageType":"KeepAlive"}"#;

/// One frame from the server.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    /// The server drops a socket it has not heard from for `timeout`; the
    /// client answers by sending [`KEEP_ALIVE`] at half that interval.
    ForceKeepAlive {
        /// The server's idle limit.
        timeout: Duration,
    },
    /// The server's echo of a [`KEEP_ALIVE`].
    KeepAlive,
    /// A playback command for the whole group.
    Command(Command),
    /// A change to the group: members, state or queue.
    GroupUpdate(GroupUpdate),
    /// A message type SyncPlay does not use.
    Other {
        /// The `MessageType` as the server named it.
        message_type: String,
    },
    /// A frame that is not the shape its type promises. Never echoes the
    /// payload.
    Malformed {
        /// The `MessageType`, or empty when the frame named none.
        message_type: String,
    },
}

/// What a group's members are told to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum CommandKind {
    /// Be playing at [`Command::position_ticks`] when the server's clock
    /// reads [`Command::when`].
    Unpause,
    /// Be paused at [`Command::position_ticks`].
    Pause,
    /// Leave playback.
    Stop,
    /// Move to [`Command::position_ticks`], stay paused and report `Ready`.
    Seek,
}

/// `SyncPlayCommand`: one command, scheduled on the server's clock.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Command {
    /// The group the command is for.
    pub group_id: String,
    /// The queue entry it applies to; all zeros before a queue is set.
    pub playlist_item_id: String,
    /// The server-clock instant at which the command takes effect.
    pub when: UtcTime,
    /// The position at [`Command::when`], in ticks.
    pub position_ticks: Option<i64>,
    /// What to do.
    pub command: CommandKind,
    /// The server-clock instant the message was sent.
    pub emitted_at: UtcTime,
}

impl Command {
    /// [`Command::position_ticks`] as a duration; `None` when absent or
    /// negative.
    #[must_use]
    pub fn position(&self) -> Option<Duration> {
        self.position_ticks.and_then(ticks::ticks_to_duration)
    }
}

/// `SyncPlayGroupUpdate`: something about the group changed.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupUpdate {
    /// The group the update is about.
    pub group_id: String,
    /// What changed.
    pub update: Update,
}

/// The `Type` of a group update, with its `Data`.
#[derive(Debug, Clone, PartialEq)]
pub enum Update {
    /// This session is now in the group (sent to the one who joined).
    GroupJoined(GroupInfo),
    /// Another user joined; the user's name.
    UserJoined(String),
    /// Another user left; the user's name.
    UserLeft(String),
    /// This session has left the group; the group id, hyphenated.
    GroupLeft(String),
    /// A request came from a session in no group.
    NotInGroup(String),
    /// The group asked for does not exist (a `Join` still answers `204`).
    GroupDoesNotExist(String),
    /// This user cannot see an item in the group's queue.
    LibraryAccessDenied(String),
    /// The group's state moved.
    StateUpdate(StateUpdate),
    /// The group's queue, sent on a new queue and to a member who joins.
    PlayQueue(PlayQueue),
    /// An update type this client does not use.
    Other {
        /// The `Type` as the server named it.
        kind: String,
    },
}

/// `StateUpdate`: the group's state and the request that moved it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct StateUpdate {
    /// The new state.
    pub state: GroupState,
    /// The request that caused it (`Ready`, `Buffer`, `Seek`, `Pause`, …).
    pub reason: String,
}

/// `PlayQueue`: the group's queue and where it starts.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PlayQueue {
    /// Why the queue was sent (`NewPlaylist`, `SetCurrentItem`, …).
    pub reason: String,
    /// When the queue last changed, on the server's clock.
    pub last_update: UtcTime,
    /// The entries, in play order.
    pub playlist: Vec<QueueItem>,
    /// The index into [`PlayQueue::playlist`] of the entry to play.
    pub playing_item_index: i32,
    /// Where in that entry to start, in ticks. For a member who joins a
    /// playing group this is the group's position at the join.
    pub start_position_ticks: i64,
    /// Whether the group was playing when the queue was sent.
    pub is_playing: bool,
    /// `Sorted` or `Shuffle`.
    pub shuffle_mode: String,
    /// `RepeatNone`, `RepeatOne` or `RepeatAll`.
    pub repeat_mode: String,
}

impl PlayQueue {
    /// The entry to play, if the index names one.
    #[must_use]
    pub fn playing_item(&self) -> Option<&QueueItem> {
        usize::try_from(self.playing_item_index)
            .ok()
            .and_then(|index| self.playlist.get(index))
    }
}

/// One entry of a group's queue.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct QueueItem {
    /// The library item.
    pub item_id: String,
    /// This entry's id; `Ready` and `Buffering` name it.
    pub playlist_item_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Envelope {
    message_type: String,
    #[serde(default)]
    data: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawUpdate {
    group_id: String,
    #[serde(rename = "Type")]
    kind: String,
    #[serde(default)]
    data: Value,
}

impl Message {
    /// Read one text frame. Never fails; see the module notes.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let Ok(envelope) = serde_json::from_str::<Envelope>(text) else {
            return Self::Malformed {
                message_type: String::new(),
            };
        };
        let data = &envelope.data;
        let parsed = match envelope.message_type.as_str() {
            "ForceKeepAlive" => u64::deserialize(data).map(|seconds| Self::ForceKeepAlive {
                timeout: Duration::from_secs(seconds),
            }),
            "KeepAlive" => Ok(Self::KeepAlive),
            "SyncPlayCommand" => Command::deserialize(data).map(Self::Command),
            "SyncPlayGroupUpdate" => RawUpdate::deserialize(data)
                .and_then(GroupUpdate::from_raw)
                .map(Self::GroupUpdate),
            _ => {
                return Self::Other {
                    message_type: envelope.message_type,
                }
            }
        };
        parsed.unwrap_or(Self::Malformed {
            message_type: envelope.message_type,
        })
    }
}

impl GroupUpdate {
    fn from_raw(raw: RawUpdate) -> Result<Self, serde_json::Error> {
        let data = &raw.data;
        let text = || String::deserialize(data);
        let update = match raw.kind.as_str() {
            "GroupJoined" => Update::GroupJoined(GroupInfo::deserialize(data)?),
            "UserJoined" => Update::UserJoined(text()?),
            "UserLeft" => Update::UserLeft(text()?),
            "GroupLeft" => Update::GroupLeft(text()?),
            "NotInGroup" => Update::NotInGroup(text()?),
            "GroupDoesNotExist" => Update::GroupDoesNotExist(text()?),
            "LibraryAccessDenied" => Update::LibraryAccessDenied(text()?),
            "StateUpdate" => Update::StateUpdate(StateUpdate::deserialize(data)?),
            "PlayQueue" => Update::PlayQueue(PlayQueue::deserialize(data)?),
            other => Update::Other {
                kind: other.to_owned(),
            },
        };
        Ok(Self {
            group_id: raw.group_id,
            update,
        })
    }
}
