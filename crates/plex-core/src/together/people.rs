//! The people a room can invite: the account's Home users and its friends.
//!
//! Both answer with numeric plex.tv user ids, which is what
//! [`super::room::NewRoom::users`] and [`super::room::invite`] take. Neither
//! list says whether someone can reach a given server.

use serde::{Deserialize, Deserializer};
use serde_json::json;

use super::room::{checked, or_default};
use crate::error::CoreError;
use crate::http::Http;
use crate::secret::SecretString;

/// `GET` on plex.tv: the account's Home users, the account itself included.
pub const HOME_USERS_PATH: &str = "/api/v2/home/users";

/// Where the friends list is asked for.
pub const COMMUNITY_BASE: &str = "https://community.plex.tv";

/// The community GraphQL endpoint, `POST` only.
pub const COMMUNITY_PATH: &str = "/api";

/// The friends query, as Plex's own web app sends it.
pub const FRIENDS_QUERY: &str =
    "query GetAllFriends { allFriendsV2 { user { avatar displayName id username idRaw } createdAt } }";

/// One member of the account's Plex Home.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct HomeUser {
    /// The plex.tv user id.
    pub id: u64,
    /// The plex.tv user uuid.
    #[serde(default, deserialize_with = "or_default")]
    pub uuid: String,
    /// The name to show.
    #[serde(default, deserialize_with = "or_default")]
    pub title: String,
    /// A managed user with content restrictions.
    #[serde(default, deserialize_with = "or_default")]
    pub restricted: bool,
    /// The Home's administrator, normally the account itself.
    #[serde(default, deserialize_with = "or_default")]
    pub admin: bool,
    /// The avatar's URL.
    pub thumb: Option<String>,
}

/// One friend of the account.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Friend {
    /// The plex.tv user id (`idRaw`).
    #[serde(rename = "idRaw", deserialize_with = "number_or_text")]
    pub id: u64,
    /// The friend's uuid (`id` in the answer).
    #[serde(rename = "id", default, deserialize_with = "or_default")]
    pub uuid: String,
    /// The account's user name.
    #[serde(default, deserialize_with = "or_default")]
    pub username: String,
    /// The name to show.
    #[serde(default, deserialize_with = "or_default")]
    pub display_name: String,
    /// The avatar's URL.
    pub avatar: Option<String>,
}

#[derive(Deserialize)]
struct HomeUsers {
    users: Vec<HomeUser>,
}

#[derive(Deserialize)]
struct FriendsAnswer {
    data: FriendsData,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FriendsData {
    all_friends_v2: Vec<FriendEntry>,
}

#[derive(Deserialize)]
struct FriendEntry {
    user: Friend,
}

/// The account's Home users: `GET /api/v2/home/users` on an [`Http`] for
/// plex.tv.
///
/// # Errors
///
/// [`CoreError::AuthRejected`] for a refused token, [`CoreError::HttpStatus`]
/// for any other refusal, [`CoreError::Transport`] when plex.tv cannot be
/// reached and [`CoreError::ResponseShape`] for any other answer.
pub fn home_users(http: &Http, token: &SecretString) -> Result<Vec<HomeUser>, CoreError> {
    let response = checked(http.get(HOME_USERS_PATH, &[], Some(token))?)?;
    Ok(response.json::<HomeUsers>()?.users)
}

/// The account's friends: [`FRIENDS_QUERY`] posted to an [`Http`] for
/// [`COMMUNITY_BASE`]. The query changes nothing; GraphQL just takes it as a
/// `POST`.
///
/// # Errors
///
/// As [`home_users`]; an answer carrying GraphQL errors instead of data is
/// [`CoreError::ResponseShape`].
pub fn friends(http: &Http, token: &SecretString) -> Result<Vec<Friend>, CoreError> {
    let body = json!({ "query": FRIENDS_QUERY, "variables": {} }).to_string();
    let response = checked(http.post(COMMUNITY_PATH, &[], Some(&body), Some(token))?)?;
    let answer: FriendsAnswer = response.json()?;
    Ok(answer
        .data
        .all_friends_v2
        .into_iter()
        .map(|entry| entry.user)
        .collect())
}

/// GraphQL may hand a numeric id back as a string.
fn number_or_text<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Id {
        Number(u64),
        Text(String),
    }
    match Id::deserialize(deserializer)? {
        Id::Number(id) => Ok(id),
        Id::Text(text) => text.parse().map_err(serde::de::Error::custom),
    }
}
