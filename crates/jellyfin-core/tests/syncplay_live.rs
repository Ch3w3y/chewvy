//! Two members of one SyncPlay group on a real server, each with its own
//! socket: both must be told the same thing at the same server-clock instant
//! for a queue, a pause, a play and a seek.
//!
//! Run against a throwaway server only:
//!
//! ```text
//! JELLYFIN_LIVE_URL=http://127.0.0.1:18096 \
//! JELLYFIN_LIVE_ADMIN_TOKEN_FILE=/path/to/admin-token \
//! cargo test -p jellyfin-core --test syncplay_live -- --ignored --nocapture
//! ```
//!
//! It signs in as `syncplay-host` (SyncPlay access `CreateAndJoinGroups`),
//! `syncplay-guest` (`JoinGroups`) and `syncplay-none` (`None`), each with
//! its own name as password. Missing users are created with the admin token;
//! the server then records this test's device name against that token.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::time::{Duration, Instant};

use jellyfin_core::auth::sign_in_password;
use jellyfin_core::browse::Client;
use jellyfin_core::socket::Socket;
use jellyfin_core::syncplay::clock::{self, ClockSample, UtcTime};
use jellyfin_core::syncplay::message::{Command, CommandKind, GroupUpdate, Message, Update};
use jellyfin_core::syncplay::PlayerReport;
use jellyfin_core::{media_browser_header, DeviceInfo, Http, JfError, SecretString};

const WAIT: Duration = Duration::from_secs(10);

/// One signed-in device with its socket.
struct Member {
    name: &'static str,
    device: DeviceInfo,
    token: SecretString,
    user_id: String,
    socket: Socket,
    clock: ClockSample,
}

impl Member {
    fn sign_in(http: &Http, name: &'static str) -> Self {
        let device = device(name);
        let auth = sign_in_password(http, &device, name, name).expect("sign-in");
        let token = SecretString::new(&auth.access_token);
        let user_id = auth.user.expect("a user").id;
        let socket = Socket::connect(http.base(), &device, &token).expect("socket");
        let mut member = Self {
            name,
            device,
            token,
            user_id,
            socket,
            clock: ClockSample {
                offset_ticks: 0,
                round_trip_ticks: 0,
            },
        };
        let samples: Vec<ClockSample> = (0..8)
            .map(|_| member.client(http).clock_sample().expect("clock sample"))
            .collect();
        member.clock = clock::best(&samples).expect("a sample");
        member
    }

    fn client<'a>(&'a self, http: &'a Http) -> Client<'a> {
        Client::new(http, &self.device, &self.token, &self.user_id).expect("client")
    }

    /// Read until `pick` accepts a message, printing everything on the way.
    fn wait_for<T>(&mut self, what: &str, mut pick: impl FnMut(&Message) -> Option<T>) -> T {
        let deadline = Instant::now() + WAIT;
        while Instant::now() < deadline {
            if let Some(message) = self
                .socket
                .read(Duration::from_millis(250))
                .expect("socket")
            {
                println!("  [{}] {}", self.name, describe(&message));
                if let Some(found) = pick(&message) {
                    return found;
                }
            }
        }
        panic!("{}: no {what} within {WAIT:?}", self.name);
    }

    fn command(&mut self, kind: CommandKind) -> Command {
        self.wait_for(&format!("{kind:?}"), |message| match message {
            Message::Command(command) if command.command == kind => Some(command.clone()),
            _ => None,
        })
    }

    fn report_ready(&self, http: &Http, playlist_item_id: &str, position: Duration) {
        let report = PlayerReport {
            when: self.clock.to_server(UtcTime::now()),
            position,
            is_playing: false,
            playlist_item_id,
        };
        self.client(http).syncplay_ready(&report).expect("ready");
    }
}

fn device(name: &str) -> DeviceInfo {
    DeviceInfo {
        app: "Chewvy".to_owned(),
        app_version: env!("CARGO_PKG_VERSION").to_owned(),
        device: format!("{name} live test"),
        device_id: format!("chewvy-live-{name}"),
    }
}

fn describe(message: &Message) -> String {
    match message {
        Message::Command(command) => format!(
            "{:?} at {} position {:?} (sent {})",
            command.command,
            command.when.to_wire(),
            command.position(),
            command.emitted_at.to_wire()
        ),
        Message::GroupUpdate(GroupUpdate { update, .. }) => match update {
            Update::PlayQueue(queue) => format!(
                "PlayQueue {} entries, start {} ticks",
                queue.playlist.len(),
                queue.start_position_ticks
            ),
            Update::GroupJoined(group) => {
                format!(
                    "GroupJoined {:?} {:?}",
                    group.group_name, group.participants
                )
            }
            other => format!("{other:?}"),
        },
        other => format!("{other:?}"),
    }
}

/// Create the three test users when sign-in shows they are missing.
fn ensure_users(http: &Http) {
    let names = ["syncplay-host", "syncplay-guest", "syncplay-none"];
    let missing = names
        .iter()
        .any(|name| sign_in_password(http, &device(name), name, name).is_err());
    if !missing {
        return;
    }
    let path = std::env::var("JELLYFIN_LIVE_ADMIN_TOKEN_FILE").expect("admin token file");
    let token = SecretString::new(std::fs::read_to_string(path).expect("admin token").trim());
    let admin = media_browser_header(&device("admin"), Some(&token));
    let users: Vec<serde_json::Value> = http
        .get("/Users", &[], Some(&admin))
        .and_then(|response| response.json())
        .expect("users");
    for (name, access) in names
        .iter()
        .zip(["CreateAndJoinGroups", "JoinGroups", "None"])
    {
        let existing = users.iter().find(|user| user["Name"] == *name).cloned();
        let mut user = existing.unwrap_or_else(|| {
            let body = serde_json::json!({"Name": name, "Password": name}).to_string();
            http.post("/Users/New", &[], Some(&body), Some(&admin))
                .and_then(|response| response.json())
                .expect("new user")
        });
        let mut policy = user["Policy"].take();
        policy["SyncPlayAccess"] = access.into();
        policy["EnableAllFolders"] = true.into();
        let id = user["Id"].as_str().expect("user id").to_owned();
        http.post(
            &format!("/Users/{id}/Policy"),
            &[],
            Some(&policy.to_string()),
            Some(&admin),
        )
        .expect("policy");
    }
}

#[test]
#[ignore = "needs a throwaway Jellyfin server: JELLYFIN_LIVE_URL and JELLYFIN_LIVE_ADMIN_TOKEN_FILE"]
fn two_members_receive_the_same_commands() {
    let base = std::env::var("JELLYFIN_LIVE_URL").expect("JELLYFIN_LIVE_URL");
    let http = Http::new(&base).expect("server address");
    ensure_users(&http);

    let mut host = Member::sign_in(&http, "syncplay-host");
    let mut guest = Member::sign_in(&http, "syncplay-guest");
    for member in [&host, &guest] {
        println!(
            "{}: clock offset {:+.4} s, round trip {:?}",
            member.name,
            member.clock.offset_seconds(),
            member.clock.round_trip()
        );
        member
            .client(&http)
            .syncplay_ping(member.clock.round_trip())
            .expect("ping");
    }

    let item = host
        .client(&http)
        .search_items("F01", 1)
        .expect("search")
        .items
        .into_iter()
        .next()
        .expect("an item named F01");
    println!("queue: {:?} ({})", item.name, item.id);

    println!("host creates a group");
    let group = host
        .client(&http)
        .syncplay_new_group("Chewvy live test")
        .expect("new group")
        .expect("the new group");
    host.wait_for("GroupJoined", |message| {
        matches!(
            message,
            Message::GroupUpdate(GroupUpdate {
                update: Update::GroupJoined(_),
                ..
            })
        )
        .then_some(())
    });
    let listed = guest.client(&http).syncplay_groups().expect("list");
    assert!(listed
        .iter()
        .any(|listed| listed.group_id == group.group_id));

    println!("guest joins");
    guest
        .client(&http)
        .syncplay_join(&group.group_id)
        .expect("join");
    let joined = guest.wait_for("GroupJoined", |message| match message {
        Message::GroupUpdate(GroupUpdate {
            update: Update::GroupJoined(joined),
            ..
        }) => Some(joined.participants.len()),
        _ => None,
    });
    assert_eq!(joined, 2, "both members are named");
    host.wait_for("UserJoined", |message| {
        matches!(
            message,
            Message::GroupUpdate(GroupUpdate {
                update: Update::UserJoined(_),
                ..
            })
        )
        .then_some(())
    });

    println!("host sets the queue");
    host.client(&http)
        .syncplay_set_new_queue(&[&item.id], 0, Duration::ZERO)
        .expect("queue");
    let mut entries = Vec::new();
    for member in [&mut host, &mut guest] {
        entries.push(member.wait_for("PlayQueue", |message| {
            match message {
                Message::GroupUpdate(GroupUpdate {
                    update: Update::PlayQueue(queue),
                    ..
                }) => queue
                    .playing_item()
                    .map(|entry| entry.playlist_item_id.clone()),
                _ => None,
            }
        }));
    }
    assert_eq!(entries[0], entries[1]);
    let entry = entries.remove(0);

    println!("both report ready");
    host.report_ready(&http, &entry, Duration::ZERO);
    guest.report_ready(&http, &entry, Duration::ZERO);
    let start = same_command(&mut host, &mut guest, CommandKind::Unpause);
    let lead = start.when.seconds_since(start.emitted_at);
    assert!(lead > 0.0, "a start is scheduled ahead: {lead}");

    std::thread::sleep(Duration::from_secs(2));
    println!("host pauses");
    host.client(&http).syncplay_pause().expect("pause");
    let paused = same_command(&mut host, &mut guest, CommandKind::Pause);
    let played = paused.position().expect("a position");
    assert!(played > Duration::from_millis(500), "{played:?}");

    println!("guest plays");
    guest.client(&http).syncplay_unpause().expect("unpause");
    same_command(&mut host, &mut guest, CommandKind::Unpause);

    println!("guest seeks to 30 s");
    guest
        .client(&http)
        .syncplay_seek(Duration::from_secs(30))
        .expect("seek");
    let seek = same_command(&mut host, &mut guest, CommandKind::Seek);
    assert_eq!(seek.position(), Some(Duration::from_secs(30)));
    host.report_ready(&http, &entry, Duration::from_secs(30));
    guest.report_ready(&http, &entry, Duration::from_secs(30));
    same_command(&mut host, &mut guest, CommandKind::Unpause);

    println!("both leave");
    for member in [&mut guest, &mut host] {
        member.client(&http).syncplay_leave().expect("leave");
        member.wait_for("GroupLeft", |message| {
            matches!(
                message,
                Message::GroupUpdate(GroupUpdate {
                    update: Update::GroupLeft(_),
                    ..
                })
            )
            .then_some(())
        });
    }
    host.socket.close().expect("close");
    guest.socket.close().expect("close");

    println!("a user without SyncPlay access is refused");
    let refused = Member::sign_in(&http, "syncplay-none");
    assert_eq!(
        refused.client(&http).syncplay_groups().map(drop),
        Err(JfError::AuthRejected { status: 403 })
    );
    refused.socket.close().expect("close");
}

/// Both members receive `kind` with the same instant and position.
fn same_command(host: &mut Member, guest: &mut Member, kind: CommandKind) -> Command {
    let at_host = host.command(kind);
    let at_guest = guest.command(kind);
    assert_eq!(at_host.when, at_guest.when, "{kind:?}: the same instant");
    assert_eq!(
        at_host.position_ticks, at_guest.position_ticks,
        "{kind:?}: the same position"
    );
    assert_eq!(at_host.playlist_item_id, at_guest.playlist_item_id);
    println!(
        "  both: {kind:?} at {} position {:?}",
        at_host.when.to_wire(),
        at_host.position()
    );
    at_host
}
