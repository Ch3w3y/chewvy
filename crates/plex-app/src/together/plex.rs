//! A Plex Watch Together room, as this device takes part in it.
//!
//! One thread per joined room owns the sync socket and the
//! [`Exchange`]. The server sends the group's state about once a second and
//! this thread answers every one with the player's latest reading
//! ([`report`]); what the exchange says the group wants becomes a
//! [`Command`] for the follower ([`Steer::follow`]), and the person's own
//! play, pause and seek become a change the next answer carries
//! ([`Steer::request`]). There is no shared clock: a state's position already
//! allows for the time it spent on the way, so a play starts from "now" on
//! this device's own clock. docs/PLEX_WATCH_TOGETHER.md is the record of how
//! the service behaves.

use std::collections::BTreeMap;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use plex_core::error::CoreError;
use plex_core::together::exchange::{self, Exchange, Target};
use plex_core::together::message::{Message, Outgoing, Set, State as Wire, UserEvent, UserName};
use plex_core::together::room::{self, NewRoom, Room};
use plex_core::together::socket::SyncSocket;

use super::clock::{local_now, GroupTime, Sample};
use super::follow::{Command, Request};
use super::rooms::{refusal, Account};
use super::{Ask, News, State};
use crate::watch::Ev;

/// How long one socket read waits, which is also the longest an ask waits.
const READ: Duration = Duration::from_millis(20);
/// A playing group whose state is this far from where this device last
/// pointed the follower is followed again: the same band the follower
/// leaves alone, so jitter in the state's own timing is not chased.
const RESYNC: f64 = 0.1;
/// A paused group's position moving this far is a move to follow.
const PAUSED_MOVE: f64 = 0.5;
/// The longest wait between attempts to reach the sync server again.
const RETRY_MAX: Duration = Duration::from_secs(8);
/// How long the connection may stay lost before the room is given up on.
const GIVE_UP: Duration = Duration::from_secs(120);

/// The player's latest reading, as the window's thread last took it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Pulse {
    /// Seconds into the item.
    pub position: f64,
    /// Paused.
    pub paused: bool,
    /// Seeking or waiting for data: the position is not where it is going.
    pub stalled: bool,
    /// The local time ([`local_now`]) it was read at.
    pub at: f64,
}

/// The reading the room's thread answers with: the window's thread writes it
/// on every follower tick, and a state can arrive at any moment in between.
static PLAYER: Mutex<Option<Pulse>> = Mutex::new(None);

/// Record the player's reading for the room; `None` once no player follows
/// a room.
pub(crate) fn report(pulse: Option<Pulse>) {
    if let Ok(mut slot) = PLAYER.lock() {
        *slot = pulse;
    }
}

fn latest() -> Option<Pulse> {
    PLAYER.lock().ok().and_then(|slot| *slot)
}

/// Where this device last pointed the follower.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Goal {
    position: f64,
    paused: bool,
    /// Local time the position applied at.
    at: f64,
}

impl Goal {
    fn position_at(&self, now: f64) -> f64 {
        if self.paused {
            self.position
        } else {
            self.position + (now - self.at).max(0.0)
        }
    }

    fn command(&self) -> Command {
        if self.paused {
            Command::PauseAt {
                position: self.position,
            }
        } else {
            Command::PlayAt {
                at: GroupTime(self.at),
                position: self.position,
            }
        }
    }
}

/// The person's own change, until an answer carries it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Pending {
    goal: Goal,
    play_pause: bool,
    seek: bool,
}

/// Turns the group's states into follower commands and the person's own
/// changes into what the next answer reports. Pure: the caller passes the
/// time.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Steer {
    goal: Option<Goal>,
    pending: Option<Pending>,
}

impl Steer {
    /// The group's state from one answer: the command that brings the
    /// follower there, when it needs one.
    pub(crate) fn follow(&mut self, target: &Target, now: f64) -> Option<Command> {
        let goal = Goal {
            position: target.position,
            paused: target.paused,
            at: now,
        };
        let command = match self.goal {
            _ if target.do_seek && target.paused => Some(Command::SeekTo {
                position: goal.position,
            }),
            Some(last) if !target.do_seek && last.paused == target.paused => {
                let off = (last.position_at(now) - target.position).abs();
                let band = if target.paused { PAUSED_MOVE } else { RESYNC };
                (off > band).then(|| goal.command())
            }
            _ => Some(goal.command()),
        };
        if command.is_some() {
            self.goal = Some(goal);
        }
        command
    }

    /// A play, pause or seek the person made, with the player at `here`:
    /// the command that makes it on this device at once. The next answer
    /// tells the group.
    pub(crate) fn request(&mut self, request: Request, here: f64, now: f64) -> Command {
        let paused = self.goal.is_none_or(|goal| goal.paused);
        let (position, paused, seek) = match request {
            Request::Pause => (here, true, false),
            Request::Unpause => (here, false, false),
            Request::Seek { position } => (position.max(0.0), paused, true),
        };
        let goal = Goal {
            position,
            paused,
            at: now,
        };
        let earlier = self.pending.take();
        self.pending = Some(Pending {
            goal,
            play_pause: !seek || earlier.is_some_and(|p| p.play_pause),
            seek: seek || earlier.is_some_and(|p| p.seek),
        });
        self.goal = Some(goal);
        if seek && paused {
            Command::SeekTo { position }
        } else {
            goal.command()
        }
    }

    /// What to answer a state with: the person's change while one is on its
    /// way, else the player's reading, else where the group was last seen.
    pub(crate) fn reading(&self, player: Option<Pulse>, now: f64) -> exchange::Reading {
        if let Some(pending) = self.pending {
            return exchange::Reading {
                position: pending.goal.position_at(now),
                paused: pending.goal.paused,
                play_pause_pending: pending.play_pause,
                seek_pending: pending.seek,
            };
        }
        let (position, paused) = match (player, self.goal) {
            // Mid-seek the player still reads where it was; report where it
            // is going, or the group would be pulled back there.
            (Some(pulse), Some(goal)) if pulse.stalled => (goal.position_at(now), goal.paused),
            (Some(pulse), _) => {
                let moved = if pulse.paused {
                    0.0
                } else {
                    (now - pulse.at).max(0.0)
                };
                (pulse.position + moved, pulse.paused)
            }
            (None, Some(goal)) => (goal.position_at(now), goal.paused),
            (None, None) => (0.0, true),
        };
        exchange::Reading {
            position,
            paused,
            play_pause_pending: false,
            seek_pending: false,
        }
    }

    /// The answer carried the change.
    pub(crate) fn sent(&mut self) {
        self.pending = None;
    }
}

/// How a session gets into its room.
pub(crate) enum Entry {
    /// Make a room for an item and join it.
    Create {
        rating_key: String,
        title: String,
        people: Vec<u64>,
    },
    /// Join a room this account is in.
    Join(String),
}

/// One room this device is in, or joining, as the worker holds it.
pub(crate) struct Session {
    asks: Sender<Ask>,
    done: Receiver<()>,
    /// The room's id, once known.
    room: Arc<OnceLock<String>>,
}

impl Session {
    /// Join a room on a thread of its own; `refresh` asks for the room list
    /// to be read again once a new room exists. `None` when no thread could
    /// start; the screens have been told.
    pub(crate) fn open(
        account: Arc<Account>,
        entry: Entry,
        refresh: Option<Sender<()>>,
        ev: &Sender<Ev>,
    ) -> Option<Self> {
        let (asks, inbox) = mpsc::channel();
        let (finished, done) = mpsc::channel();
        let room = Arc::new(OnceLock::new());
        let named = Arc::clone(&room);
        let news = ev.clone();
        let started = std::thread::Builder::new()
            .name("plex-together".into())
            .spawn(move || {
                run(&account, entry, &named, refresh.as_ref(), &inbox, &news);
                report(None);
                let _ = finished.send(());
            });
        if started.is_err() {
            tell(
                ev,
                News::Left {
                    why: Some("Watching together could not start.".into()),
                },
            );
            return None;
        }
        Some(Self { asks, done, room })
    }

    /// Whether the session is still running.
    pub(crate) fn alive(&self) -> bool {
        matches!(self.done.try_recv(), Err(TryRecvError::Empty))
    }

    /// The room's id, once the session has one.
    pub(crate) fn room(&self) -> Option<&str> {
        self.room.get().map(String::as_str)
    }

    /// Pass one ask on to the room.
    pub(crate) fn ask(&self, ask: Ask) {
        let _ = self.asks.send(ask);
    }

    /// Leave the room, waiting at most `wait` for the socket to close.
    pub(crate) fn close(self, wait: Duration) {
        let _ = self.asks.send(Ask::Leave);
        let _ = self.done.recv_timeout(wait);
    }
}

/// One participant, as the room lists them.
struct Member {
    name: String,
    ready: Option<bool>,
}

/// The session: the room, its socket and what this device knows of the group.
struct Link<'a> {
    account: &'a Account,
    room: Room,
    url: String,
    user: UserName,
    socket: SyncSocket,
    exchange: Exchange,
    steer: Steer,
    /// Everyone in the room, by their Syncplay user name.
    members: BTreeMap<String, Member>,
    /// The title has been handed to the screens to open.
    queued: bool,
    /// This connection has told the room the file and readiness.
    said_file: bool,
    paused: Option<bool>,
    ev: &'a Sender<Ev>,
}

/// Why the session ended: `None` when the person asked to leave.
type Ended = Option<String>;

fn run(
    account: &Account,
    entry: Entry,
    named: &OnceLock<String>,
    refresh: Option<&Sender<()>>,
    asks: &Receiver<Ask>,
    ev: &Sender<Ev>,
) {
    let found = match entry {
        Entry::Create {
            rating_key,
            title,
            people,
        } => create(account, &rating_key, &title, &people),
        Entry::Join(id) => room::room(&account.together, &account.token, &id).map_err(refusal),
    };
    let room = match found {
        Ok(room) => room,
        Err(why) => return tell(ev, News::Left { why: Some(why) }),
    };
    let on = room::parse_source_uri(&room.source_uri).map(|source| source.machine_identifier);
    if on.is_none() || on != account.machine {
        // The rating key means something on that server only.
        return tell(
            ev,
            News::Left {
                why: Some("This room's title is on a server Chewvy is not connected to.".into()),
            },
        );
    }
    let _ = named.set(room.id.clone());
    if let Some(refresh) = refresh {
        let _ = refresh.send(());
    }
    let Some(url) = room.syncplay_url() else {
        return tell(
            ev,
            News::Left {
                why: Some("The room names no sync server.".into()),
            },
        );
    };
    let user = UserName {
        device_identifier: account.client.client_identifier.clone(),
        device_name: account.client.device_name.clone(),
        user_id: account.me.map(|id| id.to_string()).unwrap_or_default(),
    };
    let socket = match hello(&url, &room.id, &user) {
        Ok(socket) => socket,
        Err(_) => {
            return tell(
                ev,
                News::Left {
                    why: Some("Could not reach the room's sync server.".into()),
                },
            );
        }
    };
    let mut link = Link {
        account,
        exchange: Exchange::new(&user.device_identifier),
        room,
        url,
        user,
        socket,
        steer: Steer::default(),
        members: BTreeMap::new(),
        queued: false,
        said_file: false,
        paused: None,
        ev,
    };
    // There is no shared clock: the group's time is this device's own.
    tell(
        ev,
        News::Clock(Sample {
            offset: 0.0,
            round_trip: 0.0,
        }),
    );
    tell(
        ev,
        News::Joined {
            server: String::new(),
            room: Some(link.room.id.clone()),
            name: link.room.title.clone(),
            members: Vec::new(),
            state: State::Idle,
        },
    );
    let why = link.serve(asks);
    let _ = link.socket.close();
    tell(ev, News::Left { why });
}

/// Make the room, titled after the item.
fn create(
    account: &Account,
    rating_key: &str,
    title: &str,
    people: &[u64],
) -> Result<Room, String> {
    let machine = account
        .machine
        .as_deref()
        .ok_or("Connect to a Plex server first.")?;
    let source = room::source_uri(machine, rating_key)
        .map_err(|_| "This title cannot be shared.".to_owned())?;
    let new = NewRoom {
        source_uri: &source,
        title,
        users: (!people.is_empty()).then_some(people),
    };
    room::create_room(&account.together, &account.token, &new).map_err(refusal)
}

/// Open the sync socket and say hello.
fn hello(url: &str, room: &str, user: &UserName) -> Result<SyncSocket, CoreError> {
    let mut socket = SyncSocket::connect(url)?;
    socket.send(&Outgoing::Hello { room, user })?;
    Ok(socket)
}

impl Link<'_> {
    fn tell(&self, news: News) {
        tell(self.ev, news);
    }

    /// Run the room until the person leaves or it cannot go on.
    fn serve(&mut self, asks: &Receiver<Ask>) -> Ended {
        loop {
            if let Err(why) = self.step(asks) {
                return why;
            }
        }
    }

    /// One read, then the asks waiting.
    fn step(&mut self, asks: &Receiver<Ask>) -> Result<(), Ended> {
        match self.socket.read(READ) {
            Ok(Some(message)) => self.message(message).map_err(Some)?,
            Ok(None) => {}
            Err(_) => self.reconnect(asks)?,
        }
        if self.say_file().is_err() {
            self.reconnect(asks)?;
        }
        loop {
            match asks.try_recv() {
                Ok(Ask::Leave) | Err(TryRecvError::Disconnected) => return Err(None),
                Ok(Ask::Request(request)) => self.request(request),
                Ok(_) => {}
                Err(TryRecvError::Empty) => break,
            }
        }
        if self.ended() {
            return Err(Some("The room has ended.".into()));
        }
        Ok(())
    }

    /// One frame from the server; `Err` with the reason when it ends the
    /// session.
    fn message(&mut self, message: Message) -> Result<(), String> {
        match message {
            Message::Hello(_) => {
                let _ = self.socket.send(&Outgoing::List);
            }
            Message::List(participants) => {
                self.members.clear();
                for participant in participants {
                    let name = self.name(&participant.user);
                    self.members.insert(
                        participant.user,
                        Member {
                            name,
                            ready: participant.is_ready,
                        },
                    );
                }
                self.members_changed(None);
            }
            Message::Set(Set::User { user, event, .. }) => match event {
                Some(UserEvent::Joined) => {
                    let name = self.name(&user);
                    let notice = format!("{name} joined");
                    self.members.insert(user, Member { name, ready: None });
                    self.members_changed(Some(notice));
                }
                Some(UserEvent::Left) => {
                    if let Some(member) = self.members.remove(&user) {
                        self.members_changed(Some(format!("{} left", member.name)));
                    }
                }
                None => {}
            },
            Message::Set(Set::Ready { user, is_ready, .. }) => {
                if let Some(member) = self.members.get_mut(&user) {
                    member.ready = is_ready;
                    self.members_changed(None);
                }
            }
            Message::State(state) => self.state(&state),
            Message::Error { message } => {
                log::info!("together: the sync server refused something");
                return Err(format!("The room refused this device: {message}"));
            }
            Message::Set(Set::Other { .. }) | Message::Other { .. } | Message::Malformed { .. } => {
            }
        }
        Ok(())
    }

    /// Answer one state at once, then follow it.
    fn state(&mut self, state: &Wire) {
        let now = local_now();
        let reading = self.steer.reading(latest(), now);
        let answer = self.exchange.on_state(state, reading, now);
        if answer.sent_change {
            self.steer.sent();
        }
        if self.socket.send(&Outgoing::State(&answer.reply)).is_err() {
            // The next read finds the socket gone and reconnects.
            return;
        }
        if let (Some(group), Some(pulse)) = (&state.playstate, latest()) {
            // How far the player is from the group's state as it arrived.
            let ahead = if group.paused {
                0.0
            } else {
                self.exchange.round_trip() / 2.0
            };
            log::debug!(
                "together: state at {now:.3} group {:.3} paused {} player {:.3} paused {} stalled {}",
                group.position + ahead,
                group.paused,
                reading.position,
                pulse.paused,
                pulse.stalled
            );
        }
        if !self.queued {
            self.queued = true;
            let position = answer.follow.as_ref().map_or(0.0, |t| t.position);
            self.tell(News::Queue {
                item: self.rating_key(),
                position,
            });
        }
        let Some(target) = answer.follow else {
            return;
        };
        let Some(command) = self.steer.follow(&target, now) else {
            return;
        };
        log::debug!(
            "together: follow {command:?} at {now:.3} (round trip {:.3})",
            self.exchange.round_trip()
        );
        // Asked before the new state is taken: it is told from the old one.
        let notice = self.change_notice(&target);
        if self.paused != Some(target.paused) {
            self.paused = Some(target.paused);
            self.tell(News::State(if target.paused {
                State::Paused
            } else {
                State::Playing
            }));
        }
        if let Some(notice) = notice {
            self.tell(News::Notice(notice));
        }
        self.tell(News::Command(command));
    }

    /// Who changed the group, and how, when someone did.
    fn change_notice(&self, target: &Target) -> Option<String> {
        let by = target.set_by.as_ref()?;
        // The state a room is found in on joining is not a change.
        let changed = target.do_seek || self.paused.is_some_and(|was| was != target.paused);
        if !changed || by.device_identifier == self.user.device_identifier {
            return None;
        }
        let name = self.display(by);
        Some(if target.do_seek {
            format!("{name} moved to {}", crate::player::clock(target.position))
        } else if target.paused {
            format!("{name} paused")
        } else {
            format!("{name} played")
        })
    }

    /// The person's own play, pause or seek.
    fn request(&mut self, request: Request) {
        let now = local_now();
        let here = self.steer.reading(latest(), now).position;
        let command = self.steer.request(request, here, now);
        let paused = !matches!(command, Command::PlayAt { .. });
        if self.paused != Some(paused) {
            self.paused = Some(paused);
            self.tell(News::State(if paused {
                State::Paused
            } else {
                State::Playing
            }));
        }
        log::debug!("together: asking {request:?} at {now:.3}");
        self.tell(News::Command(command));
    }

    /// Once the player has the title open, tell the room which file and
    /// that this device is ready. `Err` when the socket is gone.
    fn say_file(&mut self) -> Result<(), ()> {
        if self.said_file || latest().is_none() {
            return Ok(());
        }
        self.said_file = true;
        let source = self.room.source_uri.clone();
        self.socket
            .send(&Outgoing::File {
                source_uri: &source,
            })
            .and_then(|()| self.socket.send(&Outgoing::Ready(true)))
            .map_err(|_| ())
    }

    /// The socket dropped: try again until it is back, the room has ended or
    /// the person leaves. `Err` ends the session.
    fn reconnect(&mut self, asks: &Receiver<Ask>) -> Result<(), Ended> {
        log::info!("together: the sync socket dropped; reconnecting");
        self.tell(News::Notice("Reconnecting to the room…".into()));
        let started = Instant::now();
        let mut wait = Duration::from_secs(1);
        loop {
            if self.ended() {
                return Err(Some("The room has ended.".into()));
            }
            if let Ok(socket) = hello(&self.url, &self.room.id, &self.user) {
                self.socket = socket;
                self.exchange = Exchange::new(&self.user.device_identifier);
                self.said_file = false;
                self.tell(News::Notice("Reconnected to the room".into()));
                return Ok(());
            }
            if started.elapsed() > GIVE_UP {
                return Err(Some("The connection to the room dropped.".into()));
            }
            let until = Instant::now() + wait;
            while Instant::now() < until {
                match asks.recv_timeout(READ * 5) {
                    Ok(Ask::Leave) | Err(mpsc::RecvTimeoutError::Disconnected) => return Err(None),
                    Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
            }
            wait = (wait * 2).min(RETRY_MAX);
        }
    }

    /// Whether the room is over: past its end, or gone from the service.
    fn ended(&mut self) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX));
        if self.room.ends_at.is_some_and(|end| now >= end) {
            // The service may have extended it; ask before giving up.
            match room::room(&self.account.together, &self.account.token, &self.room.id) {
                Ok(fresh) if fresh.ends_at.is_some_and(|end| now < end) => {
                    self.room = fresh;
                    return false;
                }
                Err(CoreError::Transport) => return false,
                _ => return true,
            }
        }
        false
    }

    /// The room's item on the connected server.
    fn rating_key(&self) -> String {
        room::parse_source_uri(&self.room.source_uri)
            .map(|source| source.rating_key)
            .unwrap_or_default()
    }

    /// A participant's name to show.
    fn name(&self, wire: &str) -> String {
        UserName::parse(wire).map_or_else(|| "Someone".to_owned(), |user| self.display(&user))
    }

    fn display(&self, user: &UserName) -> String {
        let invited = self
            .room
            .users
            .iter()
            .find(|person| person.id.to_string() == user.user_id)
            .map(|person| person.title.clone())
            .filter(|title| !title.is_empty());
        invited.unwrap_or_else(|| user.device_name.clone())
    }

    fn members_changed(&self, notice: Option<String>) {
        let members = self
            .members
            .values()
            .map(|member| match member.ready {
                Some(true) => member.name.clone(),
                _ => format!("{} (getting ready)", member.name),
            })
            .collect();
        self.tell(News::Members {
            members,
            notice: notice.unwrap_or_default(),
        });
    }
}

fn tell(ev: &Sender<Ev>, news: News) {
    let _ = ev.send(Ev::Together(news));
}
