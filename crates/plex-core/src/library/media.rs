//! The `Media` element, its `Part[]`, the `Stream[]` inside a part, and the
//! item-level `Marker[]` / `Chapter[]` ranges.
//!
//! Shapes come from the captured real answers in `tests/fixtures/real/`, which
//! are the only shapes this module claims: `Media[0]` scalars
//! (`videoResolution`, `videoCodec`, `videoProfile`, `audioCodec`,
//! `audioChannels`, `container`, `bitrate`), `Part[0].Stream[]`
//! (`streamType`, `codec`, `displayTitle`, `language`, `selected`),
//! `Marker[]` (`type`, `startTimeOffset`, `endTimeOffset`) and `Chapter[]`
//! (`index`, `startTimeOffset`, `endTimeOffset`, `thumb`). Anything a captured
//! answer does not name is left out rather than guessed.
//!
//! `Marker`/`Chapter` provenance is deliberate: **every** captured answer that
//! carries them puts them on the item (`Metadata`) element and none nests them
//! under `Media`. A movie answer can carry a `Marker` with no `Chapter` array
//! at all, so a missing chapter list is reported as empty, not invented.
//!
//! Offsets and durations are the server's own recorded numbers (milliseconds
//! in every captured answer; a credits `Marker` at 5 845 074 ms inside a 6 081
//! 194 ms movie). This module converts nothing and makes no play/transcode
//! decision from them.

use serde::Deserialize;

use super::container::clean;

/// One `Media` version of an item: the summary a detail screen shows as
/// quality badges, plus the parts this version offers.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Media {
    /// The server's own `id` for this version (not the item's `ratingKey`).
    pub id: Option<u64>,
    /// Runtime as recorded; milliseconds in every captured answer.
    pub duration: Option<u64>,
    /// Overall bitrate as recorded (kbps in the captured answers).
    pub bitrate: Option<u64>,
    /// Vertical resolution label as recorded, e.g. `1080` / `4K`.
    pub video_resolution: Option<String>,
    /// Video codec as recorded, e.g. `h264` / `hevc`.
    pub video_codec: Option<String>,
    /// Video profile as recorded, e.g. `high`.
    pub video_profile: Option<String>,
    /// Audio codec as recorded, e.g. `aac` / `dca`.
    pub audio_codec: Option<String>,
    /// Channel count as recorded, e.g. `2` / `5`.
    pub audio_channels: Option<u64>,
    /// Container as recorded on the version, e.g. `mkv`.
    pub container: Option<String>,
    /// The parts this version offers (one captured item has one part).
    pub parts: Vec<MediaPart>,
}

/// One `Part[]` of a [`Media`]: where the server can serve the file from, and
/// what streams it carries.
///
/// A part's on-disk file path is **not** modelled: no captured answer is a
/// reason to carry a server filesystem path through this client.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MediaPart {
    /// Path of the part on the server (`/library/parts/{id}/{ts}/file.mkv`).
    pub key: Option<String>,
    /// Container / file extension as recorded.
    pub container: Option<String>,
    /// Delivery protocol as recorded, e.g. `http`.
    ///
    /// No captured answer names a protocol on a `Part` (one names `mp4` on a
    /// `Media` under `Extras[]`, which this model does not read), so a real
    /// server reports `None` here; the field is kept because this project's
    /// older mock shapes put it on the part.
    pub protocol: Option<String>,
    /// Byte size as recorded.
    pub size: Option<u64>,
    /// Part runtime as recorded; milliseconds in the captured answers.
    pub duration: Option<u64>,
    /// The streams of this part, in the order the server listed them.
    pub streams: Vec<Stream>,
}

/// One `Stream[]` of a [`MediaPart`].
///
/// `stream_type` is stored verbatim as the captured answers record it, where
/// `1` is the video stream, `2` an audio stream and `3` a subtitle stream;
/// this client keeps the number rather than assuming a closed vocabulary.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Stream {
    /// The server's own stream `id`.
    pub id: Option<u64>,
    /// The stream's position in its own type (video/audio/subtitle numbering).
    pub index: Option<u64>,
    /// Stream kind as recorded (`1` video, `2` audio, `3` subtitle).
    pub stream_type: Option<u64>,
    /// Codec as recorded, e.g. `h264`, `aac`, `srt`.
    pub codec: Option<String>,
    /// Short label the server offers, e.g. `1080p` or
    /// `English (AAC Stereo)`.
    pub display_title: Option<String>,
    /// Language name as recorded, e.g. `English`.
    pub language: Option<String>,
    /// BCP-47 style tag as recorded, e.g. `en` (`languageTag` in the captured
    /// answers).
    pub language_tag: Option<String>,
    /// Channel count, present on audio streams only in the captured answers.
    pub channels: Option<u64>,
    /// `true` when the server marked this stream selected.
    pub selected: bool,
    /// `true` when the server marked this stream default.
    pub default: bool,
    /// `true` for a forced (signs and foreign-dialogue) subtitle.
    pub forced: bool,
    /// The stream's own title, e.g. `Signs & Songs`.
    pub title: Option<String>,
    /// Server path of a sidecar file (`/library/streams/{id}`); absent for
    /// streams inside the media file.
    pub key: Option<String>,
}

/// One item-level `Marker[]` range (intro / credits).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Marker {
    /// What the range marks, as recorded (`credits`, `intro`, …).
    pub marker_type: Option<String>,
    /// Range start as recorded (milliseconds in the captured answers).
    pub start_offset_ms: Option<u64>,
    /// Range end as recorded; the captured credits marker ends exactly at the
    /// item's own `Media` duration.
    pub end_offset_ms: Option<u64>,
}

/// One item-level `Chapter[]` range.
///
/// Captured chapter thumbs are server paths
/// (`/library/media/{key}/chapterImages/{index}`), recorded verbatim.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Chapter {
    /// 1-based chapter number as recorded.
    pub index: Option<u64>,
    /// Chapter start as recorded (milliseconds).
    pub start_offset_ms: Option<u64>,
    /// Chapter end as recorded; consecutive chapters share the boundary.
    pub end_offset_ms: Option<u64>,
    /// Thumb path for this chapter, when the server recorded one.
    pub thumb: Option<String>,
}

/// `type` cannot be a field name in a struct, so it is renamed on the wire
/// types only.
#[derive(Deserialize)]
pub(crate) struct WireMarker {
    #[serde(rename = "type", default)]
    marker_type: Option<String>,
    #[serde(rename = "startTimeOffset", default)]
    start_offset_ms: Option<u64>,
    #[serde(rename = "endTimeOffset", default)]
    end_offset_ms: Option<u64>,
}

#[derive(Deserialize)]
pub(crate) struct WireChapter {
    #[serde(rename = "index", default)]
    index: Option<u64>,
    #[serde(rename = "startTimeOffset", default)]
    start_offset_ms: Option<u64>,
    #[serde(rename = "endTimeOffset", default)]
    end_offset_ms: Option<u64>,
    #[serde(rename = "thumb", default)]
    thumb: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct WireStream {
    #[serde(rename = "id", default)]
    id: Option<u64>,
    #[serde(rename = "index", default)]
    index: Option<u64>,
    #[serde(rename = "streamType", default)]
    stream_type: Option<u64>,
    #[serde(rename = "codec", default)]
    codec: Option<String>,
    #[serde(rename = "displayTitle", default)]
    display_title: Option<String>,
    #[serde(rename = "language", default)]
    language: Option<String>,
    #[serde(rename = "languageTag", default)]
    language_tag: Option<String>,
    #[serde(rename = "channels", default)]
    channels: Option<u64>,
    #[serde(rename = "selected", default)]
    selected: Option<bool>,
    #[serde(rename = "default", default)]
    default: Option<bool>,
    #[serde(rename = "forced", default)]
    forced: Option<bool>,
    #[serde(rename = "title", default)]
    title: Option<String>,
    #[serde(rename = "key", default)]
    key: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct WirePart {
    #[serde(rename = "key", default)]
    key: Option<String>,
    #[serde(rename = "container", default)]
    container: Option<String>,
    #[serde(rename = "protocol", default)]
    protocol: Option<String>,
    #[serde(rename = "size", default)]
    size: Option<u64>,
    #[serde(rename = "duration", default)]
    duration: Option<u64>,
    #[serde(rename = "Stream", default)]
    streams: Vec<WireStream>,
}

#[derive(Deserialize)]
pub(crate) struct WireMedia {
    #[serde(rename = "id", default)]
    id: Option<u64>,
    #[serde(rename = "duration", default)]
    duration: Option<u64>,
    #[serde(rename = "bitrate", default)]
    bitrate: Option<u64>,
    #[serde(rename = "videoResolution", default)]
    video_resolution: Option<String>,
    #[serde(rename = "videoCodec", default)]
    video_codec: Option<String>,
    #[serde(rename = "videoProfile", default)]
    video_profile: Option<String>,
    #[serde(rename = "audioCodec", default)]
    audio_codec: Option<String>,
    #[serde(rename = "audioChannels", default)]
    audio_channels: Option<u64>,
    #[serde(rename = "container", default)]
    container: Option<String>,
    #[serde(rename = "Part", default)]
    parts: Vec<WirePart>,
}

impl WireMedia {
    fn into_media(self) -> Media {
        Media {
            id: self.id,
            duration: self.duration,
            bitrate: self.bitrate,
            video_resolution: clean(self.video_resolution),
            video_codec: clean(self.video_codec),
            video_profile: clean(self.video_profile),
            audio_codec: clean(self.audio_codec),
            audio_channels: self.audio_channels,
            container: clean(self.container),
            parts: self.parts.into_iter().map(WirePart::into_part).collect(),
        }
    }
}

impl WirePart {
    pub(crate) fn into_part(self) -> MediaPart {
        MediaPart {
            key: clean(self.key),
            container: clean(self.container),
            protocol: clean(self.protocol),
            size: self.size,
            duration: self.duration,
            streams: self
                .streams
                .into_iter()
                .map(WireStream::into_stream)
                .collect(),
        }
    }
}

impl WireStream {
    pub(crate) fn into_stream(self) -> Stream {
        Stream {
            id: self.id,
            index: self.index,
            stream_type: self.stream_type,
            codec: clean(self.codec),
            display_title: clean(self.display_title),
            language: clean(self.language),
            language_tag: clean(self.language_tag),
            channels: self.channels,
            selected: self.selected.unwrap_or(false),
            default: self.default.unwrap_or(false),
            forced: self.forced.unwrap_or(false),
            title: clean(self.title),
            key: clean(self.key),
        }
    }
}

impl WireMarker {
    pub(crate) fn into_marker(self) -> Marker {
        Marker {
            marker_type: clean(self.marker_type),
            start_offset_ms: self.start_offset_ms,
            end_offset_ms: self.end_offset_ms,
        }
    }
}

impl WireChapter {
    pub(crate) fn into_chapter(self) -> Chapter {
        Chapter {
            index: self.index,
            start_offset_ms: self.start_offset_ms,
            end_offset_ms: self.end_offset_ms,
            thumb: clean(self.thumb),
        }
    }
}

pub(crate) fn media_from(media: Vec<WireMedia>) -> Vec<Media> {
    media.into_iter().map(WireMedia::into_media).collect()
}

/// One part attached directly to an item row (this project's older mock shape).
pub(crate) fn part_from(part: WirePart) -> MediaPart {
    part.into_part()
}

pub(crate) fn markers_from(markers: Vec<WireMarker>) -> Vec<Marker> {
    markers.into_iter().map(WireMarker::into_marker).collect()
}

pub(crate) fn chapters_from(chapters: Vec<WireChapter>) -> Vec<Chapter> {
    chapters
        .into_iter()
        .map(WireChapter::into_chapter)
        .collect()
}
