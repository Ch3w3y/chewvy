//! Playing an item: as it sits when the server says this client can take it,
//! and as the server's own HLS conversion of it when it cannot.
//!
//! Four things live here, and none of them opens a connection:
//!
//! * [`DeviceProfile`] — what the client tells a server it can decode, in the
//!   shape `POST /Items/{id}/PlaybackInfo` wants. A server answers
//!   `SupportsDirectPlay` *for the profile it was sent*, so a profile that
//!   overstates the player buys a stream the player then cannot show.
//! * [`Delivery`] — which of the server's ways of handing an item over the
//!   client asks for.
//! * [`resolve`] — picking the source this client will ask for, and naming the
//!   reason out loud when there is none.
//! * [`SessionBody`] — the bodies of the three `POST /Sessions/…` calls that
//!   keep an item on "Continue Watching".
//!
//! The *values* are the player build's, so the app supplies them; this module
//! owns the wire shape, as everywhere else in this crate.

use std::time::Duration;

use serde::Serialize;

use crate::browse::stream_path;
use crate::dto::{Item, MediaSource, PlaybackInfo};
use crate::ticks;

/// `POST /Sessions/Playing`: the item has started.
pub const SESSIONS_PLAYING_PATH: &str = "/Sessions/Playing";
/// `POST /Sessions/Playing/Progress`: where in the item the player is.
pub const SESSIONS_PROGRESS_PATH: &str = "/Sessions/Playing/Progress";
/// `POST /Sessions/Playing/Stopped`: the player has left the item.
pub const SESSIONS_STOPPED_PATH: &str = "/Sessions/Playing/Stopped";

/// `DELETE /Videos/ActiveEncodings`: end the encode behind one play session.
pub const ACTIVE_ENCODINGS_PATH: &str = "/Videos/ActiveEncodings";

/// The play method a session report names for the file as it sits.
const DIRECT_PLAY: &str = "DirectPlay";
/// The play method for the server's own stream of it, whatever it copied.
const TRANSCODE: &str = "Transcode";

/// Which of the server's ways of handing an item over the client asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    /// Whatever the server decides for the profile: the file as it sits when
    /// it can, its own conversion when it cannot.
    ServerChoice,
    /// The server's stream with the original video and audio kept wherever
    /// the profile's stream container and codecs allow.
    Remux,
    /// Video and audio re-encoded to fit these limits.
    Transcode {
        /// Bits per second for the whole stream.
        max_bitrate: u64,
        /// The widest frame to encode.
        max_width: u32,
        /// The tallest frame to encode.
        max_height: u32,
    },
}

impl Delivery {
    /// Whether the server may answer with the file as it sits.
    #[must_use]
    pub fn allows_direct(self) -> bool {
        self == Self::ServerChoice
    }

    /// Whether the server may copy the original streams into its own.
    #[must_use]
    pub fn allows_copy(self) -> bool {
        !matches!(self, Self::Transcode { .. })
    }

    /// The bitrate the server must fit the stream under, when one is asked.
    #[must_use]
    pub fn max_bitrate(self) -> Option<u64> {
        match self {
            Self::Transcode { max_bitrate, .. } => Some(max_bitrate),
            Self::ServerChoice | Self::Remux => None,
        }
    }
}

/// What the client can decode, as `PlaybackInfo` asks for it.
///
/// An empty profile is not a small one: the server reads an empty
/// `DirectPlayProfiles` as "every container, every codec", which is a bigger
/// claim than this player has evidence for. A client that only plays files as
/// they sit therefore has to say what it has actually been seen decoding.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct DeviceProfile {
    /// The name the server's log records this profile under.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The bitrate ceiling the server should hold a *static* (direct-played)
    /// source under, in bits per second. `None` leaves the server's own default
    /// in charge — 8 Mbit/s in a stock Jellyfin `DeviceProfile`, which would
    /// refuse a Blu-ray remux on bandwidth alone.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_static_bitrate: Option<u64>,
    /// The ceiling for any stream, static ones included. A 12.4 server answers
    /// `SupportsDirectPlay=false` for every source when this is absent, whatever
    /// `max_static_bitrate` says.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_streaming_bitrate: Option<u64>,
    /// The rows of the matrix: containers, and the codecs this player has been
    /// seen decoding out of each.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub direct_play_profiles: Vec<DirectPlayProfile>,
    /// Ceilings one codec has been measured against, whatever container it sits
    /// in — a source over a ceiling is not direct play for this client.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub codec_profiles: Vec<CodecProfile>,
    /// The streams the server may make for this client out of a source it
    /// cannot hand over as it sits. With none, the server offers no conversion.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub transcoding_profiles: Vec<TranscodingProfile>,
}

/// One stream the server may make for this client: a segment container, the
/// codecs it may hold, and how it is delivered.
///
/// The server copies a source stream whose codec is listed here and encodes to
/// the first listed codec otherwise, so the lists are claims about the player
/// exactly as a [`DirectPlayProfile`] row is.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct TranscodingProfile {
    /// The segment container (`ts`).
    pub container: String,
    #[serde(rename = "Type")]
    pub kind: ProfileKind,
    /// Comma-separated; the first is what the server encodes to.
    pub video_codec: String,
    /// Comma-separated; the first is what the server encodes to.
    pub audio_codec: String,
    /// How the stream is delivered (`hls`).
    pub protocol: String,
    /// What the stream is for (`Streaming`, as opposed to a download).
    pub context: String,
    /// The most audio channels the server should send, as it reads them.
    pub max_audio_channels: String,
}

impl TranscodingProfile {
    /// A video stream delivered as HLS for watching now.
    #[must_use]
    pub fn hls(container: &str, video_codec: &str, audio_codec: &str, channels: u8) -> Self {
        Self {
            container: container.to_owned(),
            kind: ProfileKind::Video,
            video_codec: video_codec.to_owned(),
            audio_codec: audio_codec.to_owned(),
            protocol: "hls".to_owned(),
            context: "Streaming".to_owned(),
            max_audio_channels: channels.to_string(),
        }
    }
}

/// One row of the matrix: a container this client opens, and the codecs it has
/// been seen decoding out of it.
///
/// `container`, `video_codec` and `audio_codec` are comma-separated because
/// that is how the server splits them (`ContainerProfile.SplitValue`), and an
/// omitted codec list means that type of stream is accepted in any codec —
/// which is why an audio-only row needs no `video_codec` at all.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct DirectPlayProfile {
    pub container: String,
    #[serde(rename = "Type")]
    pub kind: ProfileKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub video_codec: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_codec: Option<String>,
}

/// Which kind of stream a profile row speaks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ProfileKind {
    #[serde(rename = "Audio")]
    Audio,
    #[serde(rename = "Video")]
    Video,
}

/// One codec's measured ceiling, in the shape a server compares a stream to.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct CodecProfile {
    #[serde(rename = "Type")]
    pub kind: ProfileKind,
    /// Comma-separated codec names, as the server splits them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub codec: Option<String>,
    /// The containers this ceiling was measured in, when it matters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container: Option<String>,
    /// The limits it has been measured against.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub conditions: Vec<ProfileCondition>,
}

/// One limit, in the shape a server compares a stream against.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct ProfileCondition {
    /// How the value is compared.
    pub condition: ConditionKind,
    /// Which stream property the value limits.
    pub property: ProfileProperty,
    /// The limit, as the server reads it: a number in the property's unit.
    pub value: String,
    /// Whether a stream that does not name this property fails the row.
    ///
    /// `false`, because a probe that reports no frame rate is a probe that did
    /// not report one, not a source this client cannot play.
    pub is_required: bool,
}

impl ProfileCondition {
    /// A stream property that must not exceed `value` — and that does not
    /// disqualify a source which does not report it at all.
    #[must_use]
    pub fn at_most(property: ProfileProperty, value: impl std::fmt::Display) -> Self {
        Self {
            condition: ConditionKind::LessThanEqual,
            property,
            value: value.to_string(),
            is_required: false,
        }
    }
}

/// How a [`ProfileCondition`] compares a stream to its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ConditionKind {
    /// The property must not exceed the value.
    #[serde(rename = "LessThanEqual")]
    LessThanEqual,
}

/// The stream property a [`ProfileCondition`] limits.
///
/// Names are the server's own `ProfileConditionValue` entries, quoted from
/// `MediaBrowser.Model/Dlna/ProfileConditionValue.cs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ProfileProperty {
    /// Video width, in pixels.
    Width,
    /// Video height, in pixels.
    Height,
    /// Frames per second.
    VideoFramerate,
    /// Bits per sample in the decoded picture.
    VideoBitDepth,
}

/// A source this client can ask for, as the resolve step left it.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayTarget {
    /// The item, as its server ids it.
    pub item_id: String,
    /// The source to ask for, in the `mediaSourceId` the stream URL and every
    /// session report carry.
    pub media_source_id: String,
    /// The `PlaySessionId` the server named, when it named one.
    pub play_session_id: Option<String>,
    /// The stream path, server root not included — [`crate::Http`] holds the
    /// root, and a token never belongs in a URL.
    pub path: String,
    /// Whether that path is the server's own stream of the source rather than
    /// the file as it sits.
    pub transcoding: bool,
    /// The container the source says it is.
    pub container: Option<String>,
    /// The video stream's codec, when the source names one.
    pub video_codec: Option<String>,
    /// The audio stream's codec, when the source names one.
    pub audio_codec: Option<String>,
    /// The source's length, when the server named one.
    pub duration: Option<Duration>,
    /// Where the server says this user stopped, when it says anywhere.
    pub resume_at: Option<Duration>,
}

impl PlayTarget {
    /// The stream URL for a server root: the path, and no token in it.
    #[must_use]
    pub fn stream_url(&self, base: &str) -> String {
        format!("{}{}", base.trim_end_matches('/'), self.path)
    }

    /// The codecs being played, for a track line or a status message.
    #[must_use]
    pub fn codecs(&self) -> Vec<&str> {
        self.video_codec
            .as_deref()
            .into_iter()
            .chain(self.audio_codec.as_deref())
            .collect()
    }
}

impl std::fmt::Display for PlayTarget {
    /// What is about to play, in the words a status line uses. The ids mean
    /// nothing to someone watching, so they stay out of the sentence.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let codecs = match self.codecs().is_empty() {
            true => "this item".to_owned(),
            false => self.codecs().join(" + "),
        };
        write!(
            f,
            "{codecs} in {}",
            self.container.as_deref().unwrap_or("an unknown container")
        )
    }
}

/// Why an item cannot be played.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DirectPlayRefusal {
    /// The item has no media source at all.
    NoSources,
    /// A source exists, but the server neither hands it over as it sits nor
    /// offers a stream of its own.
    NotOffered {
        /// The container, when the source names one.
        container: Option<String>,
        /// The video codec, when the source names one.
        video_codec: Option<String>,
        /// The audio codec, when the source names one.
        audio_codec: Option<String>,
    },
    /// The direct source names no id to stream it by.
    NoSourceId,
}

impl std::fmt::Display for DirectPlayRefusal {
    /// An honest sentence for an error line: what the server would have to do,
    /// with no promise that it does it.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoSources => {
                f.write_str("This Jellyfin item has no playable version on the server.")
            }
            Self::NoSourceId => {
                f.write_str("This Jellyfin item's own version has no id to ask for.")
            }
            Self::NotOffered {
                container,
                video_codec,
                audio_codec,
            } => write!(
                f,
                "Jellyfin would have to convert {} to play it here, and did not offer \
                 to: transcoding may be turned off for this user.",
                describe(
                    container.as_deref(),
                    video_codec.as_deref(),
                    audio_codec.as_deref()
                )
            ),
        }
    }
}

/// The words a refusal names an item by.
fn describe(container: Option<&str>, video: Option<&str>, audio: Option<&str>) -> String {
    let mut parts = Vec::new();
    if let Some(video) = video {
        parts.push(format!("{video} video"));
    }
    if let Some(audio) = audio {
        parts.push(format!("{audio} audio"));
    }
    if parts.is_empty() {
        parts.push("this item".to_owned());
    }
    match container {
        Some(container) => format!("{} in {container}", parts.join(", ")),
        None => parts.join(", "),
    }
}

/// Pick the source this client will play: the first one the server says it can
/// hand over untouched when `delivery` allows that, else the first one it
/// offers a stream of its own for.
///
/// `item` is the item the caller read, for the position its server says the
/// user stopped at; `info` is the answer to this client's own `PlaybackInfo`,
/// whose sources carry the server's decision for *this* `DeviceProfile` and
/// `delivery`. The server's flags decide — this function does not second-guess
/// them with a codec list of its own.
pub fn resolve(
    item: &Item,
    info: &PlaybackInfo,
    delivery: Delivery,
) -> Result<PlayTarget, DirectPlayRefusal> {
    let direct = info
        .media_sources
        .iter()
        .find(|source| delivery.allows_direct() && is_direct(source));
    let streamed = || {
        info.media_sources
            .iter()
            .find(|source| source.transcoding_url.is_some())
    };
    let source = direct
        .or_else(streamed)
        .ok_or_else(|| match info.media_sources.first() {
            // The reason names the file the user asked for, not the verdict:
            // "it cannot be played" is only useful with the shape that made
            // it so.
            Some(source) => DirectPlayRefusal::NotOffered {
                container: source.container.clone(),
                video_codec: codec(source, "Video"),
                audio_codec: codec(source, "Audio"),
            },
            // Nothing was offered at all, so there is no shape to name.
            None => DirectPlayRefusal::NoSources,
        })?;
    let media_source_id = source
        .id
        .as_deref()
        .filter(|id| !id.trim().is_empty())
        .unwrap_or_default();
    let path = match (direct, source.transcoding_url.as_deref()) {
        (None, Some(url)) => capped(url, delivery),
        _ => stream_path(&item.id, media_source_id).ok_or(DirectPlayRefusal::NoSourceId)?,
    };
    Ok(PlayTarget {
        item_id: item.id.clone(),
        media_source_id: media_source_id.to_owned(),
        play_session_id: info
            .play_session_id
            .as_deref()
            .filter(|id| !id.trim().is_empty())
            .map(str::to_owned),
        path,
        transcoding: direct.is_none(),
        container: source.container.clone(),
        video_codec: codec(source, "Video"),
        audio_codec: codec(source, "Audio"),
        duration: source.runtime().or_else(|| item.runtime()),
        // `0` is what a server answers for an item nobody has started, and a
        // player that is told to resume at zero is told to seek to where it
        // already is.
        resume_at: item.position().filter(|at| !at.is_zero()),
    })
}

/// The server's stream URL, held to the frame size a transcode was asked at.
///
/// The server sizes a transcode by its bitrate alone (a 20 Mbit/s ask of a 4K
/// source was answered at 3840x2160), so the frame the caller named has to ride
/// on the URL, where the stream endpoint reads `MaxWidth` and `MaxHeight`.
fn capped(url: &str, delivery: Delivery) -> String {
    match delivery {
        Delivery::Transcode {
            max_width,
            max_height,
            ..
        } => format!("{url}&MaxWidth={max_width}&MaxHeight={max_height}"),
        Delivery::ServerChoice | Delivery::Remux => url.to_owned(),
    }
}

/// Whether the server says this source reaches the player as it sits.
fn is_direct(source: &MediaSource) -> bool {
    source.supports_direct_play == Some(true) || source.supports_direct_stream == Some(true)
}

/// The codec of the first stream of `kind` that names one.
fn codec(source: &MediaSource, kind: &str) -> Option<String> {
    source
        .media_streams
        .iter()
        .find(|stream| {
            stream
                .kind
                .as_deref()
                .is_some_and(|stream_kind| stream_kind.eq_ignore_ascii_case(kind))
        })
        .and_then(|stream| stream.codec.as_deref())
        .filter(|codec| !codec.is_empty())
        .map(str::to_owned)
}

/// What a session report says: one item, one position, one play method.
///
/// The method is the server's `PlayMethod`: the file as it sits, or its own
/// stream of it — which a server lists as a transcode even when it copied
/// every stream into it.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionBody<'a> {
    /// The item being played.
    pub item_id: &'a str,
    /// The source being played, as the resolve step named it.
    pub media_source_id: Option<&'a str>,
    /// The session the server named for this playback.
    pub play_session_id: Option<&'a str>,
    /// Where the player is.
    pub position: Duration,
    /// Whether the player is paused at that position.
    pub paused: bool,
    /// The item's length, when it has one.
    pub duration: Option<Duration>,
    /// Whether the server's own stream is playing rather than the file.
    pub transcoding: bool,
}

impl<'a> SessionBody<'a> {
    /// The body of `POST /Sessions/Playing`.
    #[must_use]
    pub fn playing(&self) -> String {
        self.body("Playing")
    }

    /// The body of `POST /Sessions/Playing/Progress`.
    #[must_use]
    pub fn progress(&self) -> String {
        self.body("Progress")
    }

    /// The body of `POST /Sessions/Playing/Stopped`.
    #[must_use]
    pub fn stopped(&self) -> String {
        self.body("Stopped")
    }

    /// The JSON, in the shape every session endpoint reads. Only the client
    /// knows an item has reached its end, so the report says so when the
    /// position has got there; `Playing` never claims it.
    fn body(&self, event: &str) -> String {
        #[derive(Serialize)]
        #[serde(rename_all = "PascalCase")]
        struct Wire<'b> {
            item_id: &'b str,
            #[serde(skip_serializing_if = "Option::is_none")]
            media_source_id: Option<&'b str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            play_session_id: Option<&'b str>,
            position_ticks: i64,
            #[serde(skip_serializing_if = "Option::is_none")]
            runtime_ticks: Option<i64>,
            can_seek: bool,
            is_paused: bool,
            is_playing: bool,
            play_method: &'static str,
            event_name: &'b str,
            #[serde(skip_serializing_if = "std::ops::Not::not")]
            completed: bool,
        }
        let completed = event != "Playing" && self.duration.is_some_and(|end| self.position >= end);
        let wire = Wire {
            item_id: self.item_id,
            media_source_id: self.media_source_id,
            play_session_id: self.play_session_id,
            position_ticks: ticks::duration_to_ticks(self.position),
            runtime_ticks: self.duration.map(ticks::duration_to_ticks),
            can_seek: true,
            is_paused: self.paused,
            is_playing: event != "Stopped" && !self.paused,
            play_method: if self.transcoding {
                TRANSCODE
            } else {
                DIRECT_PLAY
            },
            event_name: event,
            completed,
        };
        serde_json::to_string(&wire).unwrap_or_default()
    }
}
