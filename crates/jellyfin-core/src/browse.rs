//! Browsing one Jellyfin library as an authorized user.
//!
//! Every endpoint here is a read, and the request shapes are the ones the
//! 12.4 capture in `tests/fixtures/live-12.4/` was taken with: the token
//! always travels in the `MediaBrowser` `Authorization` header
//! ([`crate::header`]) and the user is named by a `userId` query parameter,
//! which is how Jellyfin scopes watch state — not by a per-user port or token
//! as Plex does.
//!
//! Two shapes are worth knowing before reading the calls:
//!
//! * every list endpoint answers a `QueryResult<BaseItemDto>` envelope, so
//!   [`Client::views`], [`Client::seasons`] and friends all return the same
//!   [`QueryResult`] of the same [`Item`];
//! * the query string is assembled with `url`'s form encoding, so the `,`
//!   that separates the values of `fields` and `includeItemTypes` reaches the
//!   server as `%2C`, which it decodes before splitting. The separator must be
//!   a comma: a 12.4 server ignores a `|`-joined `includeItemTypes` and answers
//!   every row of the view, folders and episodes included.

use crate::device::DeviceInfo;
use crate::dto::{ImageInfo, Item, PlaybackInfo, QueryResult, SearchHintResult, User};
use crate::error::JfError;
use crate::http::Http;
use crate::playback::{
    Delivery, SessionBody, ACTIVE_ENCODINGS_PATH, SESSIONS_PLAYING_PATH, SESSIONS_PROGRESS_PATH,
    SESSIONS_STOPPED_PATH,
};
use crate::{media_browser_header, SecretString};

/// The `Fields` the captured browse requests ask for: the detail text, the
/// credits, the playable versions and the layout hints — and nothing else.
pub const ITEM_FIELDS: &str =
    "Overview,Genres,People,MediaSources,MediaStreams,ProviderIds,Taglines,DateCreated,PrimaryImageAspectRatio";

/// The `EnableImageTypes` the captured requests ask for. `Logo` is Jellyfin's
/// name for what Plex calls `clearLogo`; `Thumb` is the episode preview.
pub const ENABLE_IMAGE_TYPES: &str = "Primary,Backdrop,Logo,Thumb";

/// The body of the PlaybackInfo request: this client's `DeviceProfile`, and
/// which of the server's deliveries it will accept this time.
///
/// Because no `PlayMethod`, `AutoOpenLiveStream` or `NowPlayingQueue` is sent,
/// the call asks for nothing the server has to remember — which is why a
/// reading call is allowed to be a `POST`. The stream it describes only starts
/// encoding when its URL is fetched.
///
/// The flags belong to the request and not to the profile: a server binds them
/// from `Jellyfin.Api/Models/MediaInfoDtos/PlaybackInfoDto.cs` and defaults
/// each to `true`, so a client that wants the server's stream instead of the
/// file, or new streams instead of copies, says `false` about the rest.
#[derive(serde::Serialize)]
#[serde(rename_all = "PascalCase")]
struct PlaybackInfoRequest<'p> {
    device_profile: &'p crate::playback::DeviceProfile,
    enable_direct_play: bool,
    enable_direct_stream: bool,
    enable_transcoding: bool,
    allow_video_stream_copy: bool,
    allow_audio_stream_copy: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_streaming_bitrate: Option<u64>,
}

/// `GET /Users/Me`, the current user (Plex's `GET /` root, insofar as a
/// Jellyfin server has one per token).
pub const ME_PATH: &str = "/Users/Me";

/// `GET /UserViews` — the library sections.
pub const USER_VIEWS_PATH: &str = "/UserViews";

/// `GET /Items` — the rows of a view, and `GET /Items/{id}` — one item.
pub const ITEMS_PATH: &str = "/Items";

/// `GET /Shows/{id}/Seasons`, `…/Episodes` and `GET /Shows/NextUp`.
pub const SHOWS_PATH: &str = "/Shows";

/// `GET /UserItems/Resume` — continue watching.
pub const RESUME_PATH: &str = "/UserItems/Resume";

/// `GET /Shows/NextUp` — the next unwatched episode per show.
pub const NEXT_UP_PATH: &str = "/Shows/NextUp";

/// `GET /Search/Hints` — the search box.
pub const SEARCH_HINTS_PATH: &str = "/Search/Hints";

/// The upper bound on an id accepted into a request path. Jellyfin ids are
/// GUIDs (32 hex characters, or 36 hyphenated), so this only has to be far
/// enough above that to reject a runaway string.
const MAX_ID_LEN: usize = 64;

/// Validate one Jellyfin id before it becomes part of a request path.
///
/// Anything empty, long, or carrying a character that could extend a path
/// into a different route (`/`, `?`, `#`, a space) is refused rather than
/// sent.
///
/// # Errors
///
/// [`JfError::InvalidInput`] for an id this client cannot address.
pub fn validate_id(id: &str) -> Result<(), JfError> {
    let addressable = !id.is_empty()
        && id.len() <= MAX_ID_LEN
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
    if addressable {
        Ok(())
    } else {
        Err(JfError::InvalidInput)
    }
}

/// Which kind of rows a view request asks the server for.
///
/// The two kinds this client presents as browsable views; Jellyfin's own
/// `IncludeItemTypes` accepts far more, and the answer's `Type` is stored
/// verbatim rather than parsed into this enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    /// A movie.
    Movie,
    /// A series.
    Series,
    /// An episode, which a season's listing is made of.
    Episode,
}

impl ItemKind {
    /// The name the server's `IncludeItemTypes` takes.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Movie => "Movie",
            Self::Series => "Series",
            Self::Episode => "Episode",
        }
    }
}

/// An image type of the `/Items/{id}/Images/{type}` route, named as the
/// server's own `ImageType` names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageKind {
    /// Poster / episode preview — Plex's `thumb`.
    Primary,
    /// Wide background — Plex's `art` / `background`.
    Backdrop,
    /// Title cut-out — Plex's `clearLogo`.
    Logo,
    /// Small preview — the other Plex `thumb` role.
    Thumb,
}

impl ImageKind {
    /// The name this type has in a URL and in an `ImageTags` map.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Primary => "Primary",
            Self::Backdrop => "Backdrop",
            Self::Logo => "Logo",
            Self::Thumb => "Thumb",
        }
    }

    /// The tag of this type an item carries, if it carries one. Backdrops are
    /// a list on an item, every other type is a map entry.
    pub(crate) fn tag(self, item: &Item) -> Option<&str> {
        match self {
            Self::Backdrop => item.backdrop_image_tags.first().map(String::as_str),
            other => item.image_tags.get(other.as_str()).map(String::as_str),
        }
    }

    /// The image index this route addresses: backdrops are indexed by their
    /// position in `BackdropImageTags`, everything else has one image.
    const fn index_suffix(self) -> &'static str {
        match self {
            Self::Backdrop => "/0",
            _ => "",
        }
    }
}

/// The artwork path of one image, in the shape the server routes:
/// `/Items/{id}/Images/{type}?maxWidth=&tag=`, with a `/0` index for the
/// first backdrop.
///
/// A path, not a URL: prepend [`Http::base`] (or any host the artwork cache
/// already knows) to fetch it. `None` for an id this client cannot address —
/// a tag is only ever taken from an item the server named, so the caller
/// already has it.
#[must_use]
pub fn image_path(item_id: &str, kind: ImageKind, tag: &str, max_width: u32) -> Option<String> {
    validate_id(item_id).ok()?;
    Some(format!(
        "{route}?maxWidth={max_width}&tag={tag}",
        route = image_route(item_id, kind),
        tag = crate::header::escape(tag),
    ))
}

/// The same route without the requested size: `/Items/{id}/Images/{type}?tag=`.
///
/// For a caller that fetches at a size it decides later — an artwork cache
/// holding one address per item and asking for whatever width a card is drawn
/// at — and appends `&maxWidth=` itself. A path, not a URL, as
/// [`image_path`]'s is.
#[must_use]
pub fn image_address(item_id: &str, kind: ImageKind, tag: &str) -> Option<String> {
    validate_id(item_id).ok()?;
    Some(format!(
        "{route}?tag={tag}",
        route = image_route(item_id, kind),
        tag = crate::header::escape(tag),
    ))
}

/// The route of one image type, without a query: the `/0` index is a backdrop's
/// position in `BackdropImageTags`.
fn image_route(item_id: &str, kind: ImageKind) -> String {
    format!(
        "/Items/{item_id}/Images/{image_type}{index}",
        image_type = kind.as_str(),
        index = kind.index_suffix(),
    )
}

/// The static direct-stream path of one media source,
/// `/Videos/{id}/stream?static=true&mediaSourceId=…`.
///
/// `static=true` is the request for the source as it sits on disk: no
/// transcoding, no remux, so the server answers the source container by byte
/// range (the capture's `video_stream_range.status` records `206
/// video/x-matroska`). The returned path carries its own query — append it to
/// [`Http::base`] and send it as-is; the token stays in the header.
#[must_use]
pub fn stream_path(item_id: &str, media_source_id: &str) -> Option<String> {
    validate_id(item_id).ok()?;
    validate_id(media_source_id).ok()?;
    Some(format!(
        "/Videos/{item_id}/stream?static=true&mediaSourceId={media_source_id}"
    ))
}

/// One authorized user on one server: the transport, the device identity, the
/// token and the user id the browse queries are scoped by.
///
/// Built once after sign-in (or after [`Client::me`] supplies the id) and
/// borrowed by every call.
#[derive(Debug, Clone)]
pub struct Client<'a> {
    http: &'a Http,
    device: &'a DeviceInfo,
    token: &'a SecretString,
    user_id: &'a str,
}

impl<'a> Client<'a> {
    /// Pair a transport and identity with a token and a user id.
    ///
    /// # Errors
    ///
    /// [`JfError::InvalidInput`] for a user id this client cannot address.
    pub fn new(
        http: &'a Http,
        device: &'a DeviceInfo,
        token: &'a SecretString,
        user_id: &'a str,
    ) -> Result<Self, JfError> {
        validate_id(user_id)?;
        Ok(Self {
            http,
            device,
            token,
            user_id,
        })
    }

    /// The user id these calls are scoped by.
    #[must_use]
    pub fn user_id(&self) -> &str {
        self.user_id
    }

    /// The current user of this token: `GET /Users/Me`.
    ///
    /// # Errors
    ///
    /// Transport, status and shape errors; [`JfError::AuthRejected`] for a
    /// `401`/`403` (a token the server no longer accepts).
    pub fn me(&self) -> Result<User, JfError> {
        self.get_json(ME_PATH, &[])
    }

    /// The library sections this user sees: `GET /UserViews?userId=…`.
    ///
    /// # Errors
    ///
    /// As [`Client::me`].
    pub fn views(&self) -> Result<QueryResult<Item>, JfError> {
        let query = [("userId", self.user_id)];
        self.get_json(USER_VIEWS_PATH, &query)
    }

    /// The rows of one view: `GET /Items?userId=…&parentId=…&recursive=true
    /// &includeItemTypes=…&sortBy=SortName&fields=…&enableImageTypes=…`.
    ///
    /// `kinds` is `IncludeItemTypes` joined the way the server splits it (on
    /// `,`); an empty slice leaves the filter off and the server answers
    /// whatever the view holds.
    ///
    /// # Errors
    ///
    /// As [`Client::me`], plus [`JfError::InvalidInput`] for an unaddressable
    /// view id.
    pub fn view_items(
        &self,
        view_id: &str,
        kinds: &[ItemKind],
    ) -> Result<QueryResult<Item>, JfError> {
        validate_id(view_id)?;
        let kinds = kinds
            .iter()
            .map(|kind| kind.as_str())
            .collect::<Vec<_>>()
            .join(",");
        let mut query: Vec<(&str, &str)> = vec![
            ("userId", self.user_id),
            ("parentId", view_id),
            ("recursive", "true"),
        ];
        if !kinds.is_empty() {
            query.push(("includeItemTypes", &kinds));
        }
        query.push(("sortBy", "SortName"));
        query.push(("fields", ITEM_FIELDS));
        query.push(("enableImageTypes", ENABLE_IMAGE_TYPES));
        self.get_json(ITEMS_PATH, &query)
    }

    /// One item, with its versions: `GET /Items/{id}?userId=…`.
    ///
    /// # Errors
    ///
    /// As [`Client::view_items`].
    pub fn item(&self, item_id: &str) -> Result<Item, JfError> {
        validate_id(item_id)?;
        let query = [("userId", self.user_id)];
        let path = format!("{ITEMS_PATH}/{item_id}");
        self.get_json(&path, &query)
    }

    /// The seasons of a series: `GET /Shows/{id}/Seasons?userId=…` — the
    /// captured shape, which names no `fields`, so a season row carries its
    /// numbers and its watch state and no blurb.
    ///
    /// # Errors
    ///
    /// As [`Client::view_items`].
    pub fn seasons(&self, series_id: &str) -> Result<QueryResult<Item>, JfError> {
        validate_id(series_id)?;
        let query = [("userId", self.user_id)];
        let path = format!("{SHOWS_PATH}/{series_id}/Seasons");
        self.get_json(&path, &query)
    }

    /// The episodes of a series, optionally of one season:
    /// `GET /Shows/{id}/Episodes?userId=…&season=…&fields=…`.
    ///
    /// # Errors
    ///
    /// As [`Client::view_items`].
    pub fn episodes(
        &self,
        series_id: &str,
        season: Option<u32>,
    ) -> Result<QueryResult<Item>, JfError> {
        validate_id(series_id)?;
        let season = season.map(|season| season.to_string());
        let mut query: Vec<(&str, &str)> = vec![("userId", self.user_id)];
        if let Some(season) = &season {
            query.push(("season", season));
        }
        query.push(("fields", ITEM_FIELDS));
        let path = format!("{SHOWS_PATH}/{series_id}/Episodes");
        self.get_json(&path, &query)
    }

    /// Continue watching: `GET /UserItems/Resume?userId=…&fields=…`.
    ///
    /// Note what the capture shows this answering: `Season` and `Series` rows
    /// whose `PlaybackPositionTicks` is `0` while `PlayedPercentage` says
    /// a third watched. The position lives on the unwatched episode inside, so
    /// a resume card uses the item id to fetch it, not the row's own position.
    ///
    /// # Errors
    ///
    /// As [`Client::me`].
    pub fn resume(&self) -> Result<QueryResult<Item>, JfError> {
        let query = [("userId", self.user_id), ("fields", ITEM_FIELDS)];
        self.get_json(RESUME_PATH, &query)
    }

    /// The next unwatched episode per show: `GET /Shows/NextUp?userId=…&fields=…`.
    ///
    /// # Errors
    ///
    /// As [`Client::me`].
    pub fn next_up(&self) -> Result<QueryResult<Item>, JfError> {
        let query = [("userId", self.user_id), ("fields", ITEM_FIELDS)];
        self.get_json(NEXT_UP_PATH, &query)
    }

    /// The images a server holds for one item: `GET /Items/{id}/Images`.
    ///
    /// # Errors
    ///
    /// As [`Client::view_items`].
    pub fn images(&self, item_id: &str) -> Result<Vec<ImageInfo>, JfError> {
        validate_id(item_id)?;
        let path = format!("{ITEMS_PATH}/{item_id}/Images");
        self.get_json(&path, &[])
    }

    /// The search box: `GET /Search/Hints?userId=…&searchTerm=…&limit=…`.
    ///
    /// # Errors
    ///
    /// As [`Client::me`].
    pub fn search_hints(&self, search_term: &str, limit: u32) -> Result<SearchHintResult, JfError> {
        let limit = limit.to_string();
        let query = [
            ("userId", self.user_id),
            ("searchTerm", search_term),
            ("limit", &limit),
        ];
        self.get_json(SEARCH_HINTS_PATH, &query)
    }

    /// Ask the server which sources an item has *for this client*:
    /// `POST /Items/{id}/PlaybackInfo?userId=…`.
    ///
    /// The answer is a fact about the pair (item, profile): `SupportsDirectPlay`
    /// says what *the sent* [`DeviceProfile`] can take, so the profile a caller
    /// hands in is the claim being tested, not a formality.
    ///
    /// This is a **`POST` that reads**: the body carries only the profile, so
    /// no play method, no `AutoOpenLiveStream` and no playing queue is
    /// registered server-side. Jellyfin has no read-equivalent `GET` in the
    /// captured flow, which is why the mutation surface has one more `POST`
    /// than Plex would.
    ///
    /// # Errors
    ///
    /// As [`Client::view_items`].
    pub fn playback_info(
        &self,
        item_id: &str,
        profile: &crate::playback::DeviceProfile,
        delivery: Delivery,
    ) -> Result<PlaybackInfo, JfError> {
        validate_id(item_id)?;
        let body = serde_json::to_string(&PlaybackInfoRequest {
            device_profile: profile,
            enable_direct_play: delivery.allows_direct(),
            enable_direct_stream: delivery.allows_direct(),
            enable_transcoding: true,
            allow_video_stream_copy: delivery.allows_copy(),
            allow_audio_stream_copy: delivery.allows_copy(),
            max_streaming_bitrate: delivery.max_bitrate(),
        })
        .map_err(|_| JfError::InvalidInput)?;
        let query = [("userId", self.user_id)];
        let path = format!("{ITEMS_PATH}/{item_id}/PlaybackInfo");
        self.http
            .post(&path, &query, Some(&body), Some(&self.authorization()))?
            .json()
    }

    /// Report that an item has started: `POST /Sessions/Playing`.
    ///
    /// # Errors
    ///
    /// As [`Client::view_items`].
    pub fn report_playing(&self, body: &SessionBody<'_>) -> Result<(), JfError> {
        self.report(SESSIONS_PLAYING_PATH, &body.playing())
    }

    /// Report where in the item the player is: `POST …/Progress`.
    ///
    /// # Errors
    ///
    /// As [`Client::view_items`].
    pub fn report_progress(&self, body: &SessionBody<'_>) -> Result<(), JfError> {
        self.report(SESSIONS_PROGRESS_PATH, &body.progress())
    }

    /// Report that the player has left the item: `POST …/Stopped`.
    ///
    /// # Errors
    ///
    /// As [`Client::view_items`].
    pub fn report_stopped(&self, body: &SessionBody<'_>) -> Result<(), JfError> {
        self.report(SESSIONS_STOPPED_PATH, &body.stopped())
    }

    /// End the encode behind one play session:
    /// `DELETE /Videos/ActiveEncodings?deviceId=…&playSessionId=…`.
    ///
    /// A server keeps encoding ahead of a stream nobody reads any more until
    /// it is told to stop, or hears a `Stopped` report — which this client
    /// only sends when the user has turned reporting on.
    ///
    /// # Errors
    ///
    /// As [`Client::view_items`].
    pub fn stop_encoding(&self, play_session_id: &str) -> Result<(), JfError> {
        let query = [
            ("deviceId", self.device.device_id.as_str()),
            ("playSessionId", play_session_id),
        ];
        self.http
            .delete(ACTIVE_ENCODINGS_PATH, &query, Some(&self.authorization()))
            .map(|_| ())
    }

    /// One session report. A server that is not told playback stopped keeps
    /// the item on "Continue Watching"; it answers `204` with no body, so
    /// nothing is parsed out of the answer.
    fn report(&self, path: &str, body: &str) -> Result<(), JfError> {
        self.http
            .post(path, &[], Some(body), Some(&self.authorization()))
            .map(|_| ())
    }

    /// The `MediaBrowser` header value for this identity and token.
    fn authorization(&self) -> String {
        media_browser_header(self.device, Some(self.token))
    }

    /// The single read path every browse call goes through: `GET`, then parse
    /// the answer as the type the endpoint promises.
    fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<T, JfError> {
        self.http
            .get(path, query, Some(&self.authorization()))
            .and_then(|response| response.json())
    }
}
