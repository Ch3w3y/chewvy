//! Watching together: keeping this player in step with other people's.
//!
//! Backend-neutral: a group session (on any server) turns its messages into
//! [`follow::Command`]s, keeps a [`clock::GroupClock`], and applies what the
//! [`follow::Follower`] answers to the player and the group. The session for
//! a Jellyfin server is [`jellyfin`], the one for a Plex Watch Together room
//! [`plex`], and [`rooms`] keeps the Plex rooms and routes each [`Ask`] to the
//! session it is for; the screens and the worker exchange [`Ask`] and
//! [`News`] with them.

pub mod clock;
pub mod follow;
pub(crate) mod jellyfin;
pub(crate) mod plex;
pub(crate) mod rooms;

use clock::{GroupTime, Sample};
use follow::{Command, Request};

/// Where a group is, as the panel says it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// Nothing queued yet, or stopped.
    Idle,
    /// Holding until every member is ready.
    Waiting,
    /// Holding for a member whose player stalled.
    CatchingUp,
    /// Paused by a member.
    Paused,
    /// Playing.
    Playing,
}

impl State {
    /// The state in plain words.
    #[must_use]
    pub fn words(self) -> &'static str {
        match self {
            Self::Idle => "Nothing playing yet",
            Self::Waiting => "Waiting for everyone to be ready",
            Self::CatchingUp => "Waiting for someone to catch up",
            Self::Paused => "Paused",
            Self::Playing => "Playing",
        }
    }
}

/// One group a server offers to join.
#[derive(Clone, Debug, PartialEq)]
pub struct Listed {
    /// The server's id for the group.
    pub id: String,
    /// The name its creator gave it.
    pub name: String,
    /// Who is in it.
    pub members: Vec<String>,
    /// Where it is.
    pub state: State,
}

/// One Plex Watch Together room this account is in or is invited to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Room {
    /// The service's id for it.
    pub id: String,
    /// The title it was given: the title it plays.
    pub title: String,
    /// The item's rating key, when it is on the connected Plex server.
    pub rating_key: Option<String>,
    /// The server the item is on, when it is another one this account has.
    pub elsewhere: Option<Elsewhere>,
    /// Who else is invited, by name, its creator first.
    pub people: Vec<String>,
    /// The plex.tv ids of everyone invited, this account included.
    pub ids: Vec<u64>,
}

/// Another of the account's Plex servers, which a room's title is on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Elsewhere {
    /// The server's machine identifier.
    pub machine: String,
    /// The server's name.
    pub name: String,
}

/// Someone a Plex room can invite.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Person {
    /// The plex.tv user id.
    pub id: u64,
    /// The name to show.
    pub name: String,
    /// A member of this account's Plex Home, rather than a friend.
    pub home: bool,
}

/// Where this device's player is, for the group.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Report {
    /// Seconds into the item.
    pub position: f64,
    /// The group time the position was read at.
    pub at: GroupTime,
    /// Whether the player is playing.
    pub playing: bool,
}

/// What the screens, and this device's player, ask of a group. Each passes
/// through the worker to the session.
#[derive(Clone, Debug, PartialEq)]
pub enum Ask {
    /// List the groups on a server, for the panel.
    Groups {
        /// The source's id.
        server: String,
    },
    /// Start a group on a server, named after the signed-in user, and join it.
    Start {
        /// The source's id.
        server: String,
    },
    /// Join a group a server listed.
    Join {
        /// The source's id.
        server: String,
        /// The group's id.
        group: String,
    },
    /// Play this item for the whole group, from `start` seconds.
    Queue {
        /// The item, as its server ids it.
        item: String,
        /// Seconds into it.
        start: f64,
    },
    /// A play, pause or seek, for everyone.
    Request(Request),
    /// This device can play from where it is.
    Ready(Report),
    /// This device's player has stalled.
    Buffering(Report),
    /// Leave the group.
    Leave,
    /// List the Plex rooms now, rather than at the next look.
    Rooms,
    /// List who a Plex room can invite.
    People,
    /// Make a Plex room for an item, inviting `people`, and join it.
    Create {
        /// The item's rating key on the connected Plex server.
        rating_key: String,
        /// The item's title, which names the room.
        title: String,
        /// The plex.tv ids to invite.
        people: Vec<u64>,
    },
    /// Join a Plex room.
    Enter {
        /// The room's id.
        room: String,
        /// The machine identifier of the server its title is on, when that
        /// is not the connected one: the app connects to it first.
        server: Option<String>,
    },
    /// Invite more people to a Plex room.
    Invite {
        /// The room's id.
        room: String,
        /// The plex.tv ids to invite.
        people: Vec<u64>,
    },
    /// Take a Plex room off this account's list: the service's way of
    /// leaving it for good.
    Remove {
        /// The room's id.
        room: String,
    },
}

/// What a group session tells the screens.
#[derive(Clone, Debug, PartialEq)]
pub enum News {
    /// The groups a server listed, or why it would not list them.
    Groups {
        /// The source's id.
        server: String,
        /// The groups, or the refusal in words.
        groups: Result<Vec<Listed>, String>,
    },
    /// A new measurement of the group's clock.
    Clock(Sample),
    /// This device is in a group.
    Joined {
        /// The source's id; empty for a Plex room.
        server: String,
        /// The Plex room's id, for a room.
        room: Option<String>,
        /// The group's name.
        name: String,
        /// Who is in it.
        members: Vec<String>,
        /// Where it is.
        state: State,
    },
    /// Someone joined, left or became ready.
    Members {
        /// Who is in the group now.
        members: Vec<String>,
        /// Who came or went, in words; empty when nobody did.
        notice: String,
    },
    /// The group moved to another state.
    State(State),
    /// The group's title: load it, paused at `position`, and say when ready.
    Queue {
        /// The item's browse id ([`crate::source::jellyfin_id`], or a Plex
        /// rating key).
        item: String,
        /// Seconds into it.
        position: f64,
    },
    /// What the group wants the player to do.
    Command(Command),
    /// Something worth a short notice.
    Notice(String),
    /// The Plex rooms this account is in or invited to, or why they could not
    /// be listed.
    Rooms(Result<Vec<Room>, String>),
    /// Who a Plex room can invite, Home first, or why they could not be
    /// listed.
    People(Result<Vec<Person>, String>),
    /// This device is out of the group; `why` when it did not ask to leave.
    Left {
        /// The reason, in words.
        why: Option<String>,
    },
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "tests may unwrap; product code keeps these lints on"
)]
mod tests;

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "tests may unwrap; product code keeps these lints on"
)]
mod room_tests;
