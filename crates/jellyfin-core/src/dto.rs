//! The parsed shape of a Jellyfin browse answer.
//!
//! Every key here is a key the 12.4 capture in `tests/fixtures/live-12.4/`
//! answers with, in the server's own casing (`PascalCase`), and every Rust
//! field is one the plex-app UI reads for Plex through `plex_core::library` —
//! a title to render, an id to navigate with, a position to resume from. Jellyfin
//! names one flat `BaseItemDto` for a view, a movie, a series, a season and an
//! episode, so one [`Item`] models all five and the fields that do not apply to
//! a kind are simply `None` (the server omits them).
//!
//! Casing is **pinned, not matched**: serde matches these names case
//! sensitively, so the same DTOs do not parse the camelCase answers a
//! first-run (wizard-not-finished) server produces. That is deliberate — see
//! `docs/JELLYFIN.md` §3.1 — and it means a
//! camelCase answer is a loud [`crate::error::JfError::ResponseShape`] on an
//! item detail, and a quiet empty list on an envelope whose only arrays are
//! defaulted.
//!
//! Fields named by the live media never appear here if the capture does not
//! name them at all: no fixture carries `Overview`, `OfficialRating`, a
//! `Logo` image tag, or a non-video [`MediaStream`] (the generated fixture
//! media are silent video-only files), so those names come from the `Fields`
//! list the captured requests themselves sent.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::Deserialize;

use crate::ticks;

/// The `QueryResult<T>` envelope every list endpoint answers with.
///
/// `Items` is defaulted rather than required: an envelope the client cannot
/// fill has to yield an empty list plus the server's own count, not an error
/// for the whole page.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct QueryResult<T> {
    /// The rows of this page.
    #[serde(default)]
    pub items: Vec<T>,
    /// Rows on the server for this query, which is larger than `items` once a
    /// limit is in play.
    pub total_record_count: Option<u64>,
    /// Offset of this page inside the query.
    pub start_index: Option<u64>,
}

/// The `UserDto` of `GET /Users/Me` (and of the authenticate endpoints).
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct User {
    /// The user's id — the `userId` every browse query needs.
    pub id: String,
    /// Display name, for the sources card.
    pub name: Option<String>,
    /// The server that issued this identity, which must match the id from
    /// [`crate::server::public_info`] for the token to be the right one.
    pub server_id: Option<String>,
    /// Whether this user has a password (the password sign-in fallback needs
    /// one, Quick Connect does not).
    pub has_password: Option<bool>,
}

impl User {
    /// The user id as it goes into a `userId` query parameter.
    #[must_use]
    pub fn user_id(&self) -> &str {
        &self.id
    }
}

/// One credited person of `People[]` (`BaseItemPersonDto`).
///
/// The captured servers answered `"People": []` for every fixture item, so
/// these names are the ones the browse requests ask for by `Fields=People`,
/// not names read out of a live row.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Person {
    /// Person name.
    pub name: Option<String>,
    /// Character or crew role (`Theo O'Dell`, `Director`, …).
    pub role: Option<String>,
    /// What this person is credited as (`Actor`, `Director`, …).
    #[serde(rename = "Type")]
    pub kind: Option<String>,
    /// The person's own `Primary` image tag; the artwork URL needs it.
    pub primary_image_tag: Option<String>,
}

/// One playback position or watch state of an item, `UserData`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UserData {
    /// Where this user stopped, in 100 ns ticks; `0` for a folder row.
    pub playback_position_ticks: Option<i64>,
    /// Fraction watched, as the server computes it.
    pub played_percentage: Option<f64>,
    /// How often this user finished it.
    pub play_count: Option<u64>,
    /// The server's own watched flag.
    pub played: Option<bool>,
    /// The user's favourite flag.
    pub is_favorite: Option<bool>,
    /// Unwatched leaves below this row, for a series or season.
    pub unplayed_item_count: Option<u64>,
}

impl UserData {
    /// [`Self::playback_position_ticks`] as a [`Duration`].
    ///
    /// A folder row (`Season`, `Series`) answers `0` here even when it is
    /// half-watched: the position belongs to the episode inside it, so a
    /// continue-watching card uses [`UserData::played_percentage`] for a
    /// folder and this only for an episode.
    #[must_use]
    pub fn position(&self) -> Option<Duration> {
        self.playback_position_ticks
            .and_then(ticks::ticks_to_duration)
    }
}

/// One `MediaSourceInfo` of `MediaSources[]` — a version that can be played.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct MediaSource {
    /// The id `mediaSourceId` in
    /// [`crate::browse::stream_path`]. For a `File` source the captured
    /// servers answer the item's own id here, but it is a separate id and is
    /// read from this field, never assumed.
    pub id: Option<String>,
    /// Where the bytes live on the server, as its own answer named them
    /// (`/media/movies/Aurora Test (2008)/Aurora Test (2008).mkv`). A name for
    /// a human: no URL is built from it, and it is never sent back.
    pub path: Option<String>,
    /// Source container (`mkv`), as recorded; `null` on a PlaybackInfo answer.
    pub container: Option<String>,
    /// Size in bytes.
    pub size: Option<u64>,
    /// Overall bitrate in bits/s, for the quality menu.
    pub bitrate: Option<u64>,
    /// Length of this source in ticks.
    pub run_time_ticks: Option<i64>,
    /// Where the bytes live (`File` on every captured answer).
    pub protocol: Option<String>,
    /// Whether the server thinks this client can play the source as it sits.
    pub supports_direct_play: Option<bool>,
    /// Whether the source can be streamed without remuxing.
    pub supports_direct_stream: Option<bool>,
    /// Whether the server offers to transcode it — the fallback when this
    /// client cannot play the container or codec at all.
    pub supports_transcoding: Option<bool>,
    /// Whether playback must open the source through the server first.
    pub requires_opening: Option<bool>,
    /// The server's own stream of this source (`/videos/{id}/master.m3u8?…`),
    /// when it offers one for the profile and flags it was asked with. The
    /// server writes the caller's token into it as `ApiKey`; that is cut out
    /// as the answer is read, so the token never rests in a value that can be
    /// logged, and the request is authorised by the header like every other.
    #[serde(default, deserialize_with = "without_api_key")]
    pub transcoding_url: Option<String>,
    /// The tracks inside this source.
    #[serde(default)]
    pub media_streams: Vec<MediaStream>,
}

/// A server-written URL with any `ApiKey`/`api_key` query pair removed.
fn without_api_key<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let url = Option::<String>::deserialize(deserializer)?;
    Ok(url.map(|url| {
        let Some((path, query)) = url.split_once('?') else {
            return url;
        };
        let kept: Vec<&str> = query
            .split('&')
            .filter(|pair| {
                let key = pair.split('=').next().unwrap_or_default();
                !key.eq_ignore_ascii_case("ApiKey") && !key.eq_ignore_ascii_case("api_key")
            })
            .collect();
        format!("{path}?{}", kept.join("&"))
    }))
}

impl MediaSource {
    /// [`Self::run_time_ticks`] as a [`Duration`].
    #[must_use]
    pub fn runtime(&self) -> Option<Duration> {
        self.run_time_ticks.and_then(ticks::ticks_to_duration)
    }
}

/// One `MediaStreamInfo` of `MediaSources[n].MediaStreams[]`.
///
/// Only the names the fixtures actually record are modelled. The generated
/// fixture media carry a single `Video` stream each, so no fixture names
/// `Language`, `Channels` or a subtitle `Type`; a milestone that lists audio
/// and subtitle tracks has to pin those names against a real answer first.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct MediaStream {
    /// Codec as the server probed it (`h264`, `hevc`, `av1`).
    pub codec: Option<String>,
    /// `Video`, `Audio`, `Subtitle`, `EmbeddedImage` — stored verbatim,
    /// because the open set is the server's.
    #[serde(rename = "Type")]
    pub kind: Option<String>,
    /// The label the server built for this track (`1080p HEVC SDR`).
    pub display_title: Option<String>,
    /// Track index inside the source.
    pub index: Option<u64>,
    /// The server's default-track flag, for the preselected audio track.
    pub is_default: Option<bool>,
    /// Whether the server says this track must be used.
    pub is_forced: Option<bool>,
    /// Whether a subtitle track is a text track (renderable) or bitmap.
    pub is_text_subtitle: Option<bool>,
    /// Width of a video track.
    pub width: Option<u64>,
    /// Height of a video track.
    pub height: Option<u64>,
}

/// One `BaseItemDto` row: a library view, a movie, a series, a season or an
/// episode.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Item {
    /// The id every other endpoint and every artwork URL addresses this item
    /// by. Required: a row without it cannot be opened, so it is a shape
    /// error rather than an item with a lost title.
    pub id: String,
    /// Display title (`Aurora Test`, `Season 1`).
    pub name: Option<String>,
    /// `Movie`, `Series`, `Season`, `Episode`, `CollectionFolder` — stored
    /// verbatim, like Plex's `type`.
    #[serde(rename = "Type")]
    pub kind: Option<String>,
    /// The kind of a library view (`movies`, `tvshows`), which is what decides
    /// whether a view opens as movies or as shows.
    pub collection_type: Option<String>,
    /// Year for a movie.
    pub production_year: Option<i64>,
    /// Length in ticks; absent on every folder row the capture contains.
    pub run_time_ticks: Option<i64>,
    /// Number of this row inside its parent: the episode number of an episode,
    /// the season number of a season.
    pub index_number: Option<u64>,
    /// The season number an episode belongs to.
    pub parent_index_number: Option<u64>,
    /// Show title of an episode (Plex's `grandparentTitle`).
    pub series_name: Option<String>,
    /// Show id of an episode or season (Plex's `grandparentRatingKey`).
    pub series_id: Option<String>,
    /// Season title of an episode (Plex's `parentTitle`).
    pub season_name: Option<String>,
    /// Season id of an episode (Plex's `parentRatingKey`).
    pub season_id: Option<String>,
    /// Blurb (Plex's `summary`). Named by the captured `Fields` list, never by
    /// a fixture row, because the fixture media have no overview.
    pub overview: Option<String>,
    /// Certification (Plex's `contentRating`), e.g. `PG`. Same caveat as
    /// [`Item::overview`]: named by the request, absent from these rows.
    pub official_rating: Option<String>,
    /// Genres, for the detail meta line.
    #[serde(default)]
    pub genres: Vec<String>,
    /// Cast and crew, for the detail credits row.
    #[serde(default)]
    pub people: Vec<Person>,
    /// The image tags this item carries, keyed by image type (`Primary`,
    /// `Logo`, `Thumb`, …). An absent key means the item has no such image,
    /// which is why artwork URLs are only built from tags that are here.
    #[serde(default)]
    pub image_tags: BTreeMap<String, String>,
    /// Backdrop image tags, in the server's own order.
    #[serde(default)]
    pub backdrop_image_tags: Vec<String>,
    /// Aspect ratio of the `Primary` image, for card layout.
    pub primary_image_aspect_ratio: Option<f64>,
    /// Total leaves below this row (Plex's `leafCount`).
    pub recursive_item_count: Option<u64>,
    /// Direct children of this row, which is what a view reports.
    pub child_count: Option<u64>,
    /// This user's position and watch state for this row.
    pub user_data: Option<UserData>,
    /// Playable versions of this item; empty on a folder row.
    #[serde(default)]
    pub media_sources: Vec<MediaSource>,
}

impl Item {
    /// [`Item::run_time_ticks`] as a [`Duration`].
    #[must_use]
    pub fn runtime(&self) -> Option<Duration> {
        self.run_time_ticks.and_then(ticks::ticks_to_duration)
    }

    /// Where this user stopped, as a [`Duration`]; see
    /// [`UserData::position`] for what a `0` on a folder row means.
    #[must_use]
    pub fn position(&self) -> Option<Duration> {
        self.user_data.as_ref().and_then(UserData::position)
    }

    /// Watched percentage, which is the only progress a folder row reports.
    #[must_use]
    pub fn played_percentage(&self) -> Option<f64> {
        self.user_data
            .as_ref()
            .and_then(|user_data| user_data.played_percentage)
    }

    /// Unwatched leaves below this row, for a series or season.
    #[must_use]
    pub fn unplayed_item_count(&self) -> Option<u64> {
        self.user_data
            .as_ref()
            .and_then(|user_data| user_data.unplayed_item_count)
    }

    /// The tag of one image type this item actually carries, `None` when the
    /// item has no such image. Backdrops come from
    /// [`Item::backdrop_image_tags`] (first entry), everything else from
    /// [`Item::image_tags`].
    #[must_use]
    pub fn image_tag(&self, kind: crate::browse::ImageKind) -> Option<&str> {
        kind.tag(self)
    }

    /// The artwork path of an image this item carries; `None` when it does not
    /// carry one. See [`crate::browse::image_path`].
    #[must_use]
    pub fn image_path(&self, kind: crate::browse::ImageKind, max_width: u32) -> Option<String> {
        let tag = self.image_tag(kind)?;
        crate::browse::image_path(&self.id, kind, tag, max_width)
    }

    /// True for the two kinds that hold something playable.
    #[must_use]
    pub fn is_playback_item(&self) -> bool {
        matches!(self.kind.as_deref(), Some("Movie") | Some("Episode"))
    }
}

/// One row of `GET /Search/Hints` (`SearchHintDto`).
///
/// The captured server answers both `ItemId` and `Id` with the same value;
/// both are kept, and [`SearchHint::item_id`] prefers `ItemId` because that is
/// the field the server names for the row's own item.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SearchHint {
    /// The id of the item this hint points at.
    pub item_id: Option<String>,
    /// The same id under a second name, as the server sends it.
    pub id: Option<String>,
    /// Display title.
    pub name: Option<String>,
    /// `Movie`, `Series`, `Episode`, …
    #[serde(rename = "Type")]
    pub kind: Option<String>,
    /// Show title, present on an episode hint and absent on a series hint.
    pub series: Option<String>,
    /// Episode number of an episode hint.
    pub index_number: Option<u64>,
    /// Season number of an episode hint.
    pub parent_index_number: Option<u64>,
    /// Length in ticks.
    pub run_time_ticks: Option<i64>,
    /// The `Primary` image tag of the hinted item — under this name, not in an
    /// `ImageTags` map as [`Item`] carries it.
    pub primary_image_tag: Option<String>,
    /// Aspect ratio of that image.
    pub primary_image_aspect_ratio: Option<f64>,
}

impl SearchHint {
    /// The id to open, preferring `ItemId` and falling back to `Id`.
    #[must_use]
    pub fn opened_id(&self) -> Option<&str> {
        self.item_id.as_deref().or(self.id.as_deref())
    }

    /// [`SearchHint::run_time_ticks`] as a [`Duration`].
    #[must_use]
    pub fn runtime(&self) -> Option<Duration> {
        self.run_time_ticks.and_then(ticks::ticks_to_duration)
    }

    /// The artwork path of the hinted item, `None` when it carries no
    /// `Primary` tag. A hint's image is always addressed as `Primary`.
    #[must_use]
    pub fn image_path(&self, max_width: u32) -> Option<String> {
        crate::browse::image_path(
            self.opened_id()?,
            crate::browse::ImageKind::Primary,
            self.primary_image_tag.as_deref()?,
            max_width,
        )
    }
}

/// The answer of `GET /Search/Hints`, which wraps its rows in `SearchHints`
/// rather than `Items`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SearchHintResult {
    /// The hints themselves.
    #[serde(default)]
    pub search_hints: Vec<SearchHint>,
    /// How many hints the server found.
    pub total_record_count: Option<u64>,
}

/// One entry of `GET /Items/{id}/Images` (`ImageInfo`).
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ImageInfo {
    /// `Primary`, `Backdrop`, `Logo`, …, as recorded.
    pub image_type: Option<String>,
    /// The tag its artwork URL needs.
    pub image_tag: Option<String>,
    /// Width and height the server probed.
    pub width: Option<u64>,
    /// Height the server probed.
    pub height: Option<u64>,
    /// Size in bytes.
    pub size: Option<u64>,
}

/// The answer of `POST /Items/{id}/PlaybackInfo`.
///
/// Only the ids and capabilities this client needs: the session id (which a
/// later milestone reports progress against) and the sources it can build a
/// static URL for.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PlaybackInfo {
    /// The server's playback session id for this answer.
    pub play_session_id: Option<String>,
    /// The sources the server considers for this item.
    pub media_sources: Vec<MediaSource>,
}
