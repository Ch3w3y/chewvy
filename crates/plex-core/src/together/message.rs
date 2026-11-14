//! The Syncplay messages a Watch Together socket carries, both ways.
//!
//! Every frame is one JSON object with a single key naming its type
//! (`Hello`, `List`, `Set`, `State`, `Error`). Reading a frame never fails:
//! a type this client does not use is [`Message::Other`] and a frame that
//! is not the shape its type promises is [`Message::Malformed`]; either way
//! the caller carries on reading.
//!
//! Plex packs its own data into two Syncplay strings: the user name is the
//! JSON text of a [`UserName`], and the file name the JSON text of the
//! room's source URI with an ads flag ([`file_name`]).

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

/// The protocol version Plex's own web app announces.
pub const PROTOCOL_VERSION: &str = "1.6.4";

/// Who a participant is, as Plex writes it into the Syncplay user name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserName {
    /// The device's client identifier; what tells two devices of one user
    /// apart.
    pub device_identifier: String,
    /// The device's name, for display.
    pub device_name: String,
    /// The plex.tv user id, as text.
    #[serde(rename = "userID")]
    pub user_id: String,
}

impl UserName {
    /// The Syncplay user name: this value as JSON text.
    #[must_use]
    pub fn to_wire(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// Read a Syncplay user name. The server appends underscores to a name
    /// that is already in the room, so those are stripped first.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        serde_json::from_str(raw.trim_end_matches('_')).ok()
    }
}

/// The Syncplay file name for a room's source URI.
#[must_use]
pub fn file_name(source_uri: &str) -> String {
    json!({ "ads": { "playing": false }, "uri": source_uri }).to_string()
}

/// The source URI inside a Syncplay file name; `None` for any other text.
#[must_use]
pub fn file_uri(name: &str) -> Option<String> {
    #[derive(Deserialize)]
    struct File {
        uri: String,
    }
    serde_json::from_str::<File>(name).ok().map(|file| file.uri)
}

/// One frame from the server.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    /// The answer to this client's `Hello`: it is in the room.
    Hello(Hello),
    /// The answer to a `List`: everyone in the room.
    List(Vec<Participant>),
    /// Someone joined, left, chose a file or changed readiness.
    Set(Set),
    /// The group's state, sent about once a second; answer every one.
    State(State),
    /// The server refused something; its own words.
    Error {
        /// The server's message.
        message: String,
    },
    /// A message type this client does not use.
    Other {
        /// The type as the server named it.
        kind: String,
    },
    /// A frame that is not the shape its type promises. Never echoes the
    /// payload.
    Malformed {
        /// The type, or empty when the frame named none.
        kind: String,
    },
}

/// The server's `Hello`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hello {
    /// This client's user name as the server holds it, perhaps with
    /// underscores added (see [`UserName::parse`]).
    pub user: String,
    /// The room joined.
    pub room: String,
    /// The protocol version the server speaks.
    pub version: String,
    /// The server's own version.
    pub real_version: Option<String>,
}

/// One entry of a `List` answer.
#[derive(Debug, Clone, PartialEq)]
pub struct Participant {
    /// The room.
    pub room: String,
    /// The Syncplay user name (see [`UserName::parse`]).
    pub user: String,
    /// Where this participant's player is, in seconds.
    pub position: Option<f64>,
    /// The file name it has chosen (see [`file_uri`]).
    pub file: Option<String>,
    /// Whether it says it is ready; `None` before it has said.
    pub is_ready: Option<bool>,
}

/// A `Set` from the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Set {
    /// Something about one participant.
    User {
        /// The Syncplay user name.
        user: String,
        /// The room.
        room: Option<String>,
        /// A join or a leave; `None` when the message is about a file.
        event: Option<UserEvent>,
        /// The file name it has chosen.
        file: Option<String>,
    },
    /// A participant's readiness.
    Ready {
        /// The Syncplay user name.
        user: String,
        /// `None` before it has said.
        is_ready: Option<bool>,
        /// Whether a person, not the player, changed it.
        manually_initiated: bool,
    },
    /// A setting this client does not use (`playlistChange`, …).
    Other {
        /// The setting as the server named it.
        kind: String,
    },
}

/// A join or a leave.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserEvent {
    /// The participant came in.
    Joined,
    /// The participant went.
    Left,
}

/// A `State`, either way.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct State {
    /// The latency exchange.
    #[serde(default)]
    pub ping: Ping,
    /// The group's state; absent from some states.
    pub playstate: Option<PlayState>,
    /// The change counters; see [`super::exchange`].
    #[serde(rename = "ignoringOnTheFly")]
    pub ignoring_on_the_fly: Option<IgnoringOnTheFly>,
}

/// The server's half of the latency exchange.
#[derive(Debug, Clone, Copy, PartialEq, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ping {
    /// The server's send time, to be echoed back.
    pub latency_calculation: Option<f64>,
    /// The server's measured round trip to this client, in seconds.
    pub server_rtt: Option<f64>,
    /// This client's last `clientLatencyCalculation`, echoed back.
    pub client_latency_calculation: Option<f64>,
}

/// Where a player is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayState {
    /// Seconds into the item.
    pub position: f64,
    /// Whether it is paused.
    pub paused: bool,
    /// Whether this state is a jump rather than steady play.
    #[serde(default)]
    pub do_seek: bool,
    /// The Syncplay user name of whoever set it; always `null` from a
    /// client.
    #[serde(default)]
    pub set_by: Option<String>,
}

/// The `ignoringOnTheFly` counters. A side leaves out a counter it has at
/// zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct IgnoringOnTheFly {
    /// Raised by a client that changed its own player.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client: Option<u32>,
    /// Raised by the server when it imposes a change.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server: Option<u32>,
}

/// The client's answer to a [`State`].
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StateReply {
    /// The client's half of the latency exchange.
    pub ping: ReplyPing,
    /// The player's state, or the server's echoed.
    pub playstate: PlayState,
    /// Left out when both counters are zero.
    #[serde(rename = "ignoringOnTheFly", skip_serializing_if = "Option::is_none")]
    pub ignoring_on_the_fly: Option<IgnoringOnTheFly>,
}

/// The client's half of the latency exchange.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplyPing {
    /// The client's clock now, in seconds; the server echoes it back.
    pub client_latency_calculation: f64,
    /// The client's measured round trip, in seconds.
    pub client_rtt: f64,
    /// The server's `serverRtt`, echoed.
    pub server_rtt: f64,
    /// The server's `latencyCalculation`, echoed.
    pub latency_calculation: f64,
}

/// One frame to the server.
#[derive(Debug, Clone, PartialEq)]
pub enum Outgoing<'a> {
    /// Join a room; the first frame on a socket.
    Hello {
        /// The room id.
        room: &'a str,
        /// Who is joining.
        user: &'a UserName,
    },
    /// Ask who is in the room.
    List,
    /// Say which item this client has open.
    File {
        /// The room's source URI.
        source_uri: &'a str,
    },
    /// Say whether this client is ready to play.
    Ready(bool),
    /// Answer a [`State`].
    State(&'a StateReply),
}

impl Outgoing<'_> {
    /// The frame's text.
    #[must_use]
    pub fn to_text(&self) -> String {
        let value = match self {
            Self::Hello { room, user } => json!({
                "Hello": {
                    "room": { "name": room },
                    "username": user.to_wire(),
                    "version": PROTOCOL_VERSION,
                }
            }),
            Self::List => json!({ "List": {} }),
            Self::File { source_uri } => {
                json!({ "Set": { "file": { "name": file_name(source_uri) } } })
            }
            Self::Ready(ready) => json!({ "Set": { "ready": { "isReady": ready } } }),
            Self::State(reply) => json!({ "State": reply }),
        };
        value.to_string()
    }
}

impl Message {
    /// Read one text frame. Never fails; see the module notes.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let Ok(Value::Object(object)) = serde_json::from_str::<Value>(text) else {
            return malformed("");
        };
        let mut entries = object.into_iter();
        let (Some((kind, body)), None) = (entries.next(), entries.next()) else {
            return malformed("");
        };
        let parsed = match kind.as_str() {
            "Hello" => parse_hello(&body).map(Self::Hello),
            "List" => parse_list(&body).map(Self::List),
            "Set" => parse_set(&body).map(Self::Set),
            "State" => State::deserialize(&body).ok().map(Self::State),
            "Error" => body
                .get("message")
                .and_then(Value::as_str)
                .map(|message| Self::Error {
                    message: message.to_owned(),
                }),
            _ => return Self::Other { kind },
        };
        parsed.unwrap_or_else(|| malformed(&kind))
    }
}

fn malformed(kind: &str) -> Message {
    Message::Malformed {
        kind: kind.to_owned(),
    }
}

fn text(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn name_of(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(|inner| text(inner, "name"))
}

fn parse_hello(body: &Value) -> Option<Hello> {
    Some(Hello {
        user: text(body, "username")?,
        room: name_of(body, "room")?,
        version: text(body, "version").unwrap_or_default(),
        real_version: text(body, "realversion"),
    })
}

fn parse_list(body: &Value) -> Option<Vec<Participant>> {
    let mut out = Vec::new();
    for (room, users) in body.as_object()? {
        for (user, entry) in users.as_object()? {
            out.push(Participant {
                room: room.clone(),
                user: user.clone(),
                position: entry.get("position").and_then(Value::as_f64),
                file: name_of(entry, "file"),
                is_ready: entry.get("isReady").and_then(Value::as_bool),
            });
        }
    }
    Some(out)
}

fn parse_set(body: &Value) -> Option<Set> {
    let object: &Map<String, Value> = body.as_object()?;
    let mut entries = object.iter();
    let (Some((kind, value)), None) = (entries.next(), entries.next()) else {
        return None;
    };
    match kind.as_str() {
        "user" => {
            let mut users = value.as_object()?.iter();
            let (Some((user, about)), None) = (users.next(), users.next()) else {
                return None;
            };
            let event = about.get("event").and_then(|event| {
                if event.get("joined").and_then(Value::as_bool) == Some(true) {
                    Some(UserEvent::Joined)
                } else if event.get("left").and_then(Value::as_bool) == Some(true) {
                    Some(UserEvent::Left)
                } else {
                    None
                }
            });
            Some(Set::User {
                user: user.clone(),
                room: name_of(about, "room"),
                event,
                file: name_of(about, "file"),
            })
        }
        "ready" => Some(Set::Ready {
            user: text(value, "username")?,
            is_ready: value.get("isReady").and_then(Value::as_bool),
            manually_initiated: value
                .get("manuallyInitiated")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        }),
        other => Some(Set::Other {
            kind: other.to_owned(),
        }),
    }
}
