//! `jellyfin-core`: the display-free Jellyfin client core.
//!
//! Mirrors the shape of `plex-core` (blocking HTTP, closed typed errors,
//! secrets inert in `Debug`/`Display`) while following Jellyfin's own
//! semantics, researched in docs/JELLYFIN.md:
//!
//! * authorization is the `MediaBrowser` `Authorization` header
//!   ([`header`]); the access token never travels in a URL,
//! * browser-style sign-in is Quick Connect ([`quickconnect`]), with password
//!   [`auth::sign_in_password`] as the fallback for the case where Quick
//!   Connect is disabled on the server,
//! * servers are validated by identity ([`server::public_info`]) plus the
//!   cheap [`server::ping`], and found on the LAN via UDP 7359
//!   ([`discovery`]),
//! * a library is browsed through [`browse`] with the row shapes of [`dto`]
//!   (one `Item` for a view, a movie, a series, a season and an episode, as
//!   the server answers one flat type for all five),
//! * an item is played through [`playback`]: the client's own
//!   [`playback::DeviceProfile`] decides what a server offers, and only a
//!   source the server hands over untouched is asked for,
//! * a group of players is kept in step by the server's SyncPlay
//!   ([`syncplay`]), whose commands arrive over the session socket
//!   ([`socket`]), authorized by the same header,
//! * positions are 100 ns ticks ([`ticks`]).
//!
//! The one deliberate deviation from the plex-core "no secrets in URLs"
//! rule: the Quick Connect *secret* is a query parameter on
//! `/QuickConnect/Connect` because that is how the server API is designed
//! (docs/JELLYFIN.md "Quick Connect"). It is a short-lived single-device
//! pairing secret, not the bearer token.

pub mod auth;
pub mod browse;
pub mod device;
pub mod discovery;
pub mod dto;
pub mod error;
pub mod header;
pub mod http;
pub mod playback;
pub mod quickconnect;
pub mod server;
pub mod socket;
pub mod syncplay;
pub mod ticks;

pub use browse::{
    image_path, stream_path, validate_id, Client, ImageKind, ItemKind, ENABLE_IMAGE_TYPES,
    ITEM_FIELDS,
};
pub use device::DeviceInfo;
pub use dto::{
    ImageInfo, Item, MediaSource, MediaStream, Person, PlaybackInfo, QueryResult, SearchHint,
    SearchHintResult, User, UserData,
};
pub use error::JfError;
pub use header::media_browser_header;
pub use http::{Http, Response};
pub use playback::{resolve, DeviceProfile, PlayTarget};

/// A bearer token handed out by `Authenticate*`; inert in `Debug`.
pub type SecretString = plex_core::secret::SecretString;
