//! Rooms on `together.plex.tv`.
//!
//! Every call is authorised by the account token in `X-Plex-Token` plus the
//! usual `X-Plex-*` headers, and goes through an [`Http`] built for
//! [`TOGETHER_BASE`]. Both the creator and every invitee see a room in
//! [`list_rooms`] until they [`leave_room`] or it ends.
//!
//! The service does not check the room's item: it accepts a source URI for
//! a server nobody in the room can reach. Offering only people who can play
//! the item is the caller's job.

use serde::{Deserialize, Deserializer, Serialize};

use crate::error::CoreError;
use crate::http::{Http, Response};
use crate::secret::SecretString;

/// Where the rooms live.
pub const TOGETHER_BASE: &str = "https://together.plex.tv";

/// `GET` lists rooms, `POST` creates one; `/rooms/<id>` is one room.
pub const ROOMS_PATH: &str = "/rooms";

/// The library provider every source URI names.
pub const LIBRARY_PROVIDER: &str = "com.plexapp.plugins.library";

/// One room, as the service answers it. Every field but the id may be
/// missing from an answer, or `null`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Room {
    /// The room's id; also the name of its Syncplay room.
    pub id: String,
    /// The title its creator gave it.
    #[serde(default, deserialize_with = "or_default")]
    pub title: String,
    /// The item, as [`source_uri`] builds it.
    #[serde(default, deserialize_with = "or_default")]
    pub source_uri: String,
    /// The plex.tv user id of whoever created it.
    pub created_by: Option<u64>,
    /// When the room opened, in Unix seconds.
    pub starts_at: Option<i64>,
    /// When the room ends, in Unix seconds.
    pub ends_at: Option<i64>,
    /// When the room last changed, in Unix seconds.
    pub updated_at: Option<i64>,
    /// The Syncplay server's host name.
    #[serde(default, deserialize_with = "or_default")]
    pub syncplay_host: String,
    /// The Syncplay server's port.
    pub syncplay_port: Option<u16>,
    /// The people in the room, as the service lists them.
    #[serde(default, deserialize_with = "or_default")]
    pub users: Vec<RoomUser>,
}

/// One person invited to a room.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RoomUser {
    /// The plex.tv user id.
    pub id: u64,
    /// The plex.tv user uuid.
    #[serde(default, deserialize_with = "or_default")]
    pub uuid: String,
    /// The name to show.
    #[serde(default, deserialize_with = "or_default")]
    pub title: String,
    /// The account's user name; empty for a managed Home user, which has
    /// none.
    #[serde(default, deserialize_with = "or_default")]
    pub username: String,
    /// The avatar's URL.
    pub thumb: Option<String>,
}

impl Room {
    /// The room's sync socket, `wss://<host>:<port>/ws`; `None` when the
    /// answer named no server.
    #[must_use]
    pub fn syncplay_url(&self) -> Option<String> {
        let port = self.syncplay_port?;
        let host = self.syncplay_host.as_str();
        let plain = !host.is_empty()
            && host
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
        plain.then(|| format!("wss://{host}:{port}/ws"))
    }
}

/// The item a source URI names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    /// The Plex Media Server's machine identifier.
    pub machine_identifier: String,
    /// The item's rating key on that server.
    pub rating_key: String,
}

/// `server://<machineIdentifier>/com.plexapp.plugins.library/library/metadata/<ratingKey>`.
///
/// # Errors
///
/// [`CoreError::InvalidInput`] when either part is empty or is not a single
/// path segment.
pub fn source_uri(machine_identifier: &str, rating_key: &str) -> Result<String, CoreError> {
    if !is_segment(machine_identifier) || !is_segment(rating_key) {
        return Err(CoreError::InvalidInput);
    }
    Ok(format!(
        "server://{machine_identifier}/{LIBRARY_PROVIDER}/library/metadata/{rating_key}"
    ))
}

/// The server and item a source URI names; `None` for any other shape.
#[must_use]
pub fn parse_source_uri(uri: &str) -> Option<Source> {
    let rest = uri.strip_prefix("server://")?;
    let (machine_identifier, rest) = rest.split_once('/')?;
    let rating_key = rest
        .strip_prefix(LIBRARY_PROVIDER)?
        .strip_prefix("/library/metadata/")?;
    (is_segment(machine_identifier) && is_segment(rating_key)).then(|| Source {
        machine_identifier: machine_identifier.to_owned(),
        rating_key: rating_key.to_owned(),
    })
}

fn is_segment(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
        && value != "."
        && value != ".."
}

/// What [`create_room`] sends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewRoom<'a> {
    /// The item, from [`source_uri`].
    pub source_uri: &'a str,
    /// The room's title.
    pub title: &'a str,
    /// The plex.tv user ids to invite; `None` for a room with only its
    /// creator, who can [`invite`] people later.
    pub users: Option<&'a [u64]>,
}

#[derive(Serialize)]
struct InviteBody<'a> {
    users: &'a [u64],
}

#[derive(Deserialize)]
struct RoomList {
    #[serde(default, deserialize_with = "or_default")]
    rooms: Vec<Room>,
}

/// Every room this account is in: `GET /rooms`.
///
/// # Errors
///
/// [`CoreError::AuthRejected`] for a refused token, [`CoreError::HttpStatus`]
/// for any other refusal, [`CoreError::Transport`] when the service cannot
/// be reached and [`CoreError::ResponseShape`] for an answer that is not a
/// room list.
pub fn list_rooms(http: &Http, token: &SecretString) -> Result<Vec<Room>, CoreError> {
    let response = checked(http.get(ROOMS_PATH, &[], Some(token))?)?;
    Ok(response.json::<RoomList>()?.rooms)
}

/// One room: `GET /rooms/<id>`.
///
/// # Errors
///
/// As [`list_rooms`], and [`CoreError::InvalidInput`] for an id that is not
/// one path segment.
pub fn room(http: &Http, token: &SecretString, id: &str) -> Result<Room, CoreError> {
    let response = checked(http.get(&room_path(id)?, &[], Some(token))?)?;
    response.json()
}

/// Create a room: `POST /rooms`, answered `201` with the room.
///
/// # Errors
///
/// As [`list_rooms`].
pub fn create_room(
    http: &Http,
    token: &SecretString,
    new: &NewRoom<'_>,
) -> Result<Room, CoreError> {
    let body = serde_json::to_string(new).map_err(|_| CoreError::InvalidInput)?;
    let response = checked(http.post(ROOMS_PATH, &[], Some(&body), Some(token))?)?;
    response.json()
}

/// Invite more people: `POST /rooms/<id>/invite`. An invitee then sees the
/// room in their own [`list_rooms`].
///
/// # Errors
///
/// As [`room`], and [`CoreError::InvalidInput`] for an empty list.
pub fn invite(http: &Http, token: &SecretString, id: &str, users: &[u64]) -> Result<(), CoreError> {
    if users.is_empty() {
        return Err(CoreError::InvalidInput);
    }
    let path = format!("{}/invite", room_path(id)?);
    let body = serde_json::to_string(&InviteBody { users }).map_err(|_| CoreError::InvalidInput)?;
    checked(http.post(&path, &[], Some(&body), Some(token))?)?;
    Ok(())
}

/// Leave a room: `DELETE /rooms/<id>`, answered `204`.
///
/// This takes the caller out of the room, creator or not; the room stays for
/// whoever is left until they leave too or it ends.
///
/// # Errors
///
/// As [`room`].
pub fn leave_room(http: &Http, token: &SecretString, id: &str) -> Result<(), CoreError> {
    checked(http.delete(&room_path(id)?, &[], Some(token))?)?;
    Ok(())
}

/// A field the service may send as `null` reads as its default.
pub(super) fn or_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Option::unwrap_or_default)
}

fn room_path(id: &str) -> Result<String, CoreError> {
    if is_segment(id) {
        Ok(format!("{ROOMS_PATH}/{id}"))
    } else {
        Err(CoreError::InvalidInput)
    }
}

pub(super) fn checked(response: Response) -> Result<Response, CoreError> {
    if response.is_success() {
        Ok(response)
    } else {
        Err(CoreError::from_status(response.status))
    }
}
