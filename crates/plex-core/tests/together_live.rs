//! Watch Together against Plex's live service, two participants from one
//! process: an account creates a room for an item and invites a second
//! user, both join the room's sync server, play, pause and seek reach the
//! other side, and both leave.
//!
//! Ignored by default. It reads everything from files named by environment
//! variables, so no value appears on a command line, and skips when any is
//! unset:
//!
//! * `CHEWVY_TOGETHER_OWNER_TOKEN_FILE`, `CHEWVY_TOGETHER_OWNER_CLIENT_ID_FILE`:
//!   the account that creates the room;
//! * `CHEWVY_TOGETHER_GUEST_TOKEN_FILE`, `CHEWVY_TOGETHER_GUEST_CLIENT_ID_FILE`,
//!   `CHEWVY_TOGETHER_GUEST_USER_ID_FILE`: the user invited, a member of the
//!   owner's Plex Home;
//! * `CHEWVY_TOGETHER_MACHINE_ID_FILE`: the server holding the item;
//! * `CHEWVY_TOGETHER_RATING_KEY`: the item's rating key.
//!
//! It prints counts and timings only, never an id or a token.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::print_stdout)]

use std::time::{Duration, Instant};

use plex_core::client_info::{ClientInfo, Identity};
use plex_core::http::Http;
use plex_core::secret::SecretString;
use plex_core::together::exchange::{Exchange, Reading};
use plex_core::together::message::{Message, Outgoing, UserName};
use plex_core::together::people::{friends, home_users, COMMUNITY_BASE};
use plex_core::together::room::{
    create_room, leave_room, list_rooms, source_uri, NewRoom, Room, TOGETHER_BASE,
};
use plex_core::together::socket::SyncSocket;

const PLEX_TV: &str = "https://plex.tv";

fn read_var(name: &str) -> Option<String> {
    let path = std::env::var(name).ok()?;
    let text = std::fs::read_to_string(path).ok()?;
    Some(text.trim().to_owned())
}

struct Account {
    token: SecretString,
    client_id: String,
    together: Http,
}

fn account(token_var: &str, client_var: &str) -> Option<Account> {
    let token = SecretString::new(read_var(token_var)?);
    let client_id = read_var(client_var)?;
    let client = ClientInfo::with_identity(
        &Identity {
            client_identifier: client_id.clone(),
        },
        env!("CARGO_PKG_VERSION"),
    );
    Some(Account {
        together: Http::new(TOGETHER_BASE, &client).unwrap(),
        token,
        client_id,
    })
}

/// One participant's socket, exchange and simulated player.
struct Peer {
    name: &'static str,
    socket: SyncSocket,
    exchange: Exchange,
    source_uri: String,
    position: f64,
    paused: bool,
    at: Instant,
    play_pause_pending: bool,
    seek_pending: bool,
    said_file: bool,
}

impl Peer {
    fn join(name: &'static str, room: &Room, user: &UserName) -> Self {
        let mut socket = SyncSocket::connect(&room.syncplay_url().expect("a sync server"))
            .expect("the sync server answers");
        socket
            .send(&Outgoing::Hello {
                room: &room.id,
                user,
            })
            .unwrap();
        Self {
            name,
            socket,
            exchange: Exchange::new(&user.device_identifier),
            source_uri: room.source_uri.clone(),
            position: 0.0,
            paused: true,
            at: Instant::now(),
            play_pause_pending: false,
            seek_pending: false,
            said_file: false,
        }
    }

    fn now_position(&self) -> f64 {
        if self.paused {
            self.position
        } else {
            self.position + self.at.elapsed().as_secs_f64()
        }
    }

    fn set(&mut self, position: f64, paused: bool) {
        self.position = position;
        self.paused = paused;
        self.at = Instant::now();
    }

    fn play(&mut self, play: bool) {
        self.set(self.now_position(), !play);
        self.play_pause_pending = true;
    }

    fn seek(&mut self, to: f64) {
        self.set(to, self.paused);
        self.seek_pending = true;
    }

    /// Handle whatever arrives in `wait`.
    fn pump(&mut self, clock: Instant, wait: Duration) {
        let Some(message) = self.socket.read(wait).expect("the socket stays up") else {
            return;
        };
        match message {
            Message::Hello(_) => self.socket.send(&Outgoing::List).unwrap(),
            Message::State(state) => {
                let now = clock.elapsed().as_secs_f64();
                let reading = Reading {
                    position: self.now_position(),
                    paused: self.paused,
                    play_pause_pending: self.play_pause_pending,
                    seek_pending: self.seek_pending,
                };
                let answer = self.exchange.on_state(&state, reading, now);
                if answer.sent_change {
                    self.play_pause_pending = false;
                    self.seek_pending = false;
                }
                if let Some(target) = &answer.follow {
                    let off = (self.now_position() - target.position).abs();
                    if target.do_seek || target.paused != self.paused || off > 1.5 {
                        self.set(target.position, target.paused);
                    }
                }
                self.socket.send(&Outgoing::State(&answer.reply)).unwrap();
                if !self.said_file {
                    self.said_file = true;
                    let uri = self.source_uri.clone();
                    self.socket
                        .send(&Outgoing::File { source_uri: &uri })
                        .unwrap();
                    self.socket.send(&Outgoing::Ready(true)).unwrap();
                }
            }
            Message::Error { .. } => panic!("{}: the sync server refused something", self.name),
            _ => {}
        }
    }
}

/// Pump both peers until `done` holds or `limit` passes; the time it took.
fn until(
    peers: &mut [&mut Peer; 2],
    clock: Instant,
    limit: Duration,
    done: impl Fn(&Peer, &Peer) -> bool,
) -> Option<Duration> {
    let started = Instant::now();
    while started.elapsed() < limit {
        for peer in peers.iter_mut() {
            peer.pump(clock, Duration::from_millis(20));
        }
        if done(peers[0], peers[1]) {
            return Some(started.elapsed());
        }
    }
    None
}

fn run(owner: &Account, guest: &Account, guest_id: u64, room: &Room) -> Result<(), String> {
    let guest_rooms = list_rooms(&guest.together, &guest.token).map_err(|e| e.to_string())?;
    if !guest_rooms.iter().any(|r| r.id == room.id) {
        return Err("the invited user does not list the room".to_owned());
    }
    let owner_user = UserName {
        device_identifier: owner.client_id.clone(),
        device_name: "Chewvy live check A".to_owned(),
        user_id: room
            .created_by
            .ok_or("the room names no creator")?
            .to_string(),
    };
    let guest_user = UserName {
        device_identifier: guest.client_id.clone(),
        device_name: "Chewvy live check B".to_owned(),
        user_id: guest_id.to_string(),
    };
    let clock = Instant::now();
    let mut a = Peer::join("A", room, &owner_user);
    let mut b = Peer::join("B", room, &guest_user);
    let mut peers = [&mut a, &mut b];
    until(&mut peers, clock, Duration::from_secs(3), |_, _| false);

    peers[0].play(true);
    let took = until(&mut peers, clock, Duration::from_secs(5), |_, b| !b.paused)
        .ok_or("B did not follow A's play")?;
    println!("play from A reached B in {:.2} s", took.as_secs_f64());
    until(&mut peers, clock, Duration::from_secs(3), |_, _| false);

    peers[1].play(false);
    let took = until(&mut peers, clock, Duration::from_secs(5), |a, _| a.paused)
        .ok_or("A did not follow B's pause")?;
    println!("pause from B reached A in {:.2} s", took.as_secs_f64());
    let offset = (peers[0].now_position() - peers[1].now_position()).abs();
    println!("paused offset {offset:.3} s");

    peers[0].seek(120.0);
    let took = until(&mut peers, clock, Duration::from_secs(5), |_, b| {
        (b.now_position() - 120.0).abs() < 0.5
    })
    .ok_or("B did not follow A's seek")?;
    println!("seek from A reached B in {:.2} s", took.as_secs_f64());

    peers[1].play(true);
    let took = until(&mut peers, clock, Duration::from_secs(5), |a, _| !a.paused)
        .ok_or("A did not follow B's play")?;
    println!("play from B reached A in {:.2} s", took.as_secs_f64());
    until(&mut peers, clock, Duration::from_secs(3), |_, _| false);
    let offset = (peers[0].now_position() - peers[1].now_position()).abs();
    println!(
        "playing offset {offset:.3} s, round trips {:.3} / {:.3} s",
        peers[0].exchange.round_trip(),
        peers[1].exchange.round_trip()
    );
    if offset > 1.0 {
        return Err(format!("playing offset {offset:.3} s"));
    }
    a.socket.close().map_err(|e| e.to_string())?;
    b.socket.close().map_err(|e| e.to_string())?;
    Ok(())
}

#[test]
#[ignore = "talks to Plex's live service; needs the CHEWVY_TOGETHER_* files"]
fn two_participants_follow_each_other_in_a_live_room() {
    let (Some(owner), Some(guest), Some(guest_id), Some(machine), Ok(rating_key)) = (
        account(
            "CHEWVY_TOGETHER_OWNER_TOKEN_FILE",
            "CHEWVY_TOGETHER_OWNER_CLIENT_ID_FILE",
        ),
        account(
            "CHEWVY_TOGETHER_GUEST_TOKEN_FILE",
            "CHEWVY_TOGETHER_GUEST_CLIENT_ID_FILE",
        ),
        read_var("CHEWVY_TOGETHER_GUEST_USER_ID_FILE").and_then(|id| id.parse::<u64>().ok()),
        read_var("CHEWVY_TOGETHER_MACHINE_ID_FILE"),
        std::env::var("CHEWVY_TOGETHER_RATING_KEY"),
    ) else {
        println!("skipped: the CHEWVY_TOGETHER_* variables are not all set");
        return;
    };

    let owner_client = ClientInfo::with_identity(
        &Identity {
            client_identifier: owner.client_id.clone(),
        },
        env!("CARGO_PKG_VERSION"),
    );
    let home =
        home_users(&Http::new(PLEX_TV, &owner_client).unwrap(), &owner.token).expect("home users");
    println!("home users: {}", home.len());
    assert!(
        home.iter().any(|user| user.id == guest_id),
        "the guest is in the Home"
    );
    let friend_count = friends(
        &Http::new(COMMUNITY_BASE, &owner_client).unwrap(),
        &owner.token,
    )
    .expect("friends")
    .len();
    println!("friends: {friend_count}");

    let uri = source_uri(&machine, &rating_key).unwrap();
    let room = create_room(
        &owner.together,
        &owner.token,
        &NewRoom {
            source_uri: &uri,
            title: "Chewvy live check",
            users: Some(&[guest_id]),
        },
    )
    .expect("the room is created");
    assert_eq!(room.source_uri, uri);
    println!(
        "room open for {} s with {} invited",
        room.ends_at.unwrap_or(0) - room.starts_at.unwrap_or(0),
        room.users.len()
    );

    // Whatever happens in the room, both leave it afterwards.
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run(&owner, &guest, guest_id, &room)
    }));

    let left_guest = leave_room(&guest.together, &guest.token, &room.id);
    let left_owner = leave_room(&owner.together, &owner.token, &room.id);
    let guest_rooms = list_rooms(&guest.together, &guest.token).expect("guest rooms");
    let owner_rooms = list_rooms(&owner.together, &owner.token).expect("owner rooms");
    println!(
        "after leaving: owner lists {}, guest lists {}",
        owner_rooms.len(),
        guest_rooms.len()
    );
    outcome.expect("the session ran").unwrap();
    left_guest.expect("the guest leaves");
    left_owner.expect("the owner leaves");
    assert!(owner_rooms.is_empty() && guest_rooms.is_empty());
}
