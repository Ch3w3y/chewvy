//! Watch Together: Plex's own service for watching one item in step with
//! other people.
//!
//! Three parts, each usable on its own by a caller that owns a thread:
//!
//! * [`room`]: the rooms on `together.plex.tv`. A room names one item by its
//!   source URI, lists who is invited, and says which sync server to use. It
//!   lasts three hours.
//! * [`people`]: who can be invited, the account's Home users and its
//!   friends.
//! * [`socket`], [`message`] and [`exchange`]: the sync server. It runs stock
//!   Syncplay; every client sends `Hello` with the room id, and the server
//!   then sends the group's state once a second. A client answers each
//!   state with its own player's reading, and [`exchange::Exchange`] decides
//!   what that answer says and what the player should follow.
//!
//! The sync socket carries no token: knowing the room id is what lets a
//! client in. The behaviour the service showed is written up in
//! docs/PLEX_WATCH_TOGETHER.md.

pub mod exchange;
pub mod message;
pub mod people;
pub mod room;
pub mod socket;
