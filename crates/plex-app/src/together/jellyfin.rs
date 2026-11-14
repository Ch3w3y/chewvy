//! A Jellyfin SyncPlay group, as this device takes part in it.
//!
//! One thread per joined group owns the session socket. It turns the server's
//! messages into [`News`] for the screens, the group's commands as
//! [`Command`]s on the group's clock, and makes the calls the screens and the
//! follower ask for ([`Ask`]). What a command's `When` means differs by
//! command (see [`Link::command`]); docs/JELLYFIN.md, "SyncPlay", is the
//! record of how a 12.4 server behaves.

use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use jellyfin_core::socket::Socket;
use jellyfin_core::syncplay::clock::{self as wire_clock, ClockSample, UtcTime};
use jellyfin_core::syncplay::message::{self as wire, CommandKind, GroupUpdate, Message, Update};
use jellyfin_core::syncplay::{GroupInfo, GroupState, PlayerReport};
use jellyfin_core::JfError;

use super::clock::{local_now, GroupTime, Sample};
use super::follow::{Command, Request};
use super::{Ask, Listed, News, State};
use crate::source::jellyfin_id;
use crate::sources::browse::{self, Server};
use crate::watch::Ev;

/// Clock samples taken on joining; the group clock keeps the one with the
/// shortest round trip.
const BURST: usize = 8;
/// How often the clock is measured again while in a group, so a drifting
/// clock is followed.
const RESAMPLE: Duration = Duration::from_secs(60);
/// How long one socket read waits, which is also the longest an ask waits to
/// be sent.
const READ: Duration = Duration::from_millis(20);
/// How long leaving waits for the server to confirm before closing anyway.
const LEAVE_WAIT: Duration = Duration::from_secs(2);
/// A socket that drops again this soon after a rejoin is not tried again.
const REJOIN_SPACING: Duration = Duration::from_secs(30);
/// How long the worker waits for a session to leave its group: on closing
/// the app, and before another group is started or joined.
pub(crate) const CLOSE_WAIT: Duration = Duration::from_secs(3);

/// One ask from the screens: listing, starting and joining happen here,
/// everything else goes to the group this device is in.
pub(crate) fn handle(
    group: &mut Option<Session>,
    jellyfin: &browse::Jellyfin,
    ask: Ask,
    ev: &Sender<Ev>,
) {
    if group.as_ref().is_some_and(|session| !session.alive()) {
        *group = None;
    }
    let missing = || {
        tell(
            ev,
            News::Left {
                why: Some("That server is not available right now.".into()),
            },
        );
    };
    match ask {
        Ask::Groups { server } => match jellyfin.server(&server) {
            Some(found) => list(found, ev),
            None => tell(
                ev,
                News::Groups {
                    server,
                    groups: Err("That server is not available right now.".into()),
                },
            ),
        },
        Ask::Start { server } | Ask::Join { server, .. } if jellyfin.server(&server).is_none() => {
            missing();
        }
        Ask::Start { server } => {
            if let Some(old) = group.take() {
                old.close(CLOSE_WAIT);
            }
            *group = jellyfin
                .server(&server)
                .and_then(|found| Session::open(found, None, ev));
        }
        Ask::Join { server, group: id } => {
            if let Some(old) = group.take() {
                old.close(CLOSE_WAIT);
            }
            *group = jellyfin
                .server(&server)
                .and_then(|found| Session::open(found, Some(id), ev));
        }
        Ask::Leave => {
            if let Some(session) = group.take() {
                session.close(CLOSE_WAIT);
            }
        }
        other => {
            if let Some(session) = group.as_ref() {
                session.ask(other);
            }
        }
    }
}

/// One group this device is in, or joining, as the worker holds it.
pub(crate) struct Session {
    asks: Sender<Ask>,
    /// Answers (or hangs up) once the session's thread has finished.
    done: Receiver<()>,
}

impl Session {
    /// Start a group on `server` (`group` is `None`), or join `group`, on a
    /// thread of its own. `None` when no thread could be started; the
    /// screens have been told.
    pub(crate) fn open(
        server: Arc<Server>,
        group: Option<String>,
        ev: &Sender<Ev>,
    ) -> Option<Self> {
        let (asks, inbox) = mpsc::channel();
        let (finished, done) = mpsc::channel();
        let news = ev.clone();
        let started = std::thread::Builder::new()
            .name("jf-together".into())
            .spawn(move || {
                run(&server, group.as_deref(), &inbox, &news);
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
        Some(Self { asks, done })
    }

    /// Whether the session is still running: it ends by itself when the
    /// group is gone or the connection could not be restored.
    pub(crate) fn alive(&self) -> bool {
        matches!(self.done.try_recv(), Err(TryRecvError::Empty))
    }

    /// Pass one ask on to the group.
    pub(crate) fn ask(&self, ask: Ask) {
        let _ = self.asks.send(ask);
    }

    /// Leave the group, waiting at most `wait` for the session to finish, so
    /// its last news reaches the screens before anything that follows.
    pub(crate) fn close(self, wait: Duration) {
        let _ = self.asks.send(Ask::Leave);
        let _ = self.done.recv_timeout(wait);
    }
}

/// List the groups on `server` for the panel, on a thread of its own.
pub(crate) fn list(server: Arc<Server>, ev: &Sender<Ev>) {
    let news = ev.clone();
    let id = server.id().to_owned();
    let named = id.clone();
    let started = std::thread::Builder::new()
        .name("jf-groups".into())
        .spawn(move || {
            let groups = server
                .api()
                .and_then(|api| api.syncplay_groups())
                .map(|groups| groups.into_iter().map(listed).collect())
                .map_err(refusal);
            tell(&news, News::Groups { server: id, groups });
        });
    if started.is_err() {
        tell(
            ev,
            News::Groups {
                server: named,
                groups: Err("The groups could not be listed.".into()),
            },
        );
    }
}

/// The session: socket, clock and the group as far as this device knows it.
struct Link<'a> {
    server: &'a Server,
    socket: Socket,
    ev: &'a Sender<Ev>,
    /// The group's id, once the server said this device is in it.
    group: Option<String>,
    /// The queue entry the group plays, once a queue arrived.
    entry: Option<String>,
    members: Vec<String>,
    /// Leave was asked for, so the server's confirmation ends nothing early.
    leaving: bool,
    sampled: Instant,
    rejoined: Option<Instant>,
}

fn run(server: &Server, group: Option<&str>, asks: &Receiver<Ask>, ev: &Sender<Ev>) {
    let socket = match server.socket() {
        Ok(socket) => socket,
        Err(error) => {
            let why = format!("Could not connect to the server: {}", refusal(error));
            return tell(ev, News::Left { why: Some(why) });
        }
    };
    let mut link = Link {
        server,
        socket,
        ev,
        group: None,
        entry: None,
        members: Vec::new(),
        leaving: false,
        sampled: Instant::now(),
        rejoined: None,
    };
    let entered = match group {
        Some(id) => link.api().and_then(|api| api.syncplay_join(id)),
        None => {
            let name = match server.user_name() {
                Some(user) => format!("{user}'s group"),
                None => "Watch together".to_owned(),
            };
            link.api()
                .and_then(|api| api.syncplay_new_group(&name))
                .map(drop)
        }
    };
    if let Err(error) = entered {
        let _ = link.socket.close();
        return tell(
            ev,
            News::Left {
                why: Some(refusal(error)),
            },
        );
    }
    link.sample(BURST);
    loop {
        let ended = match link.socket.read(READ) {
            Ok(Some(message)) => link.message(message).err(),
            Ok(None) => None,
            Err(_) => link.rejoin().err(),
        };
        if let Some(why) = ended {
            let _ = link.socket.close();
            return tell(ev, News::Left { why: Some(why) });
        }
        loop {
            match asks.try_recv() {
                Ok(Ask::Leave) | Err(TryRecvError::Disconnected) => return link.leave(),
                Ok(ask) => link.ask(ask),
                Err(TryRecvError::Empty) => break,
            }
        }
        if link.sampled.elapsed() >= RESAMPLE {
            link.sample(1);
        }
    }
}

impl Link<'_> {
    fn api(&self) -> Result<jellyfin_core::browse::Client<'_>, JfError> {
        self.server.api()
    }

    fn tell(&self, news: News) {
        tell(self.ev, news);
    }

    /// Measure the clock `count` times and tell the server the round trip,
    /// which it allows for when it schedules a start.
    fn sample(&mut self, count: usize) {
        self.sampled = Instant::now();
        let samples: Vec<ClockSample> = (0..count).filter_map(|_| self.one_sample()).collect();
        for sample in &samples {
            self.tell(News::Clock(Sample {
                offset: sample.offset_seconds(),
                round_trip: sample.round_trip().as_secs_f64(),
            }));
        }
        if let (Some(best), Ok(api)) = (wire_clock::best(&samples), self.api()) {
            log::debug!(
                "together: clock offset {:+.4} s, round trip {:.4} s",
                best.offset_seconds(),
                best.round_trip().as_secs_f64()
            );
            let _ = api.syncplay_ping(best.round_trip());
        }
    }

    /// One time exchange, timed on the clock every reading uses.
    fn one_sample(&self) -> Option<ClockSample> {
        let api = self.api().ok()?;
        let sent = local_utc();
        let answer = api.utc_time().ok()?;
        let received = local_utc();
        Some(ClockSample::new(
            sent,
            answer.request_reception_time,
            answer.response_transmission_time,
            received,
        ))
    }

    /// One frame from the server; `Err` with the reason when it ends the
    /// session.
    fn message(&mut self, message: Message) -> Result<(), String> {
        match message {
            Message::Command(command) => {
                self.command(&command);
                Ok(())
            }
            Message::GroupUpdate(GroupUpdate { update, .. }) => self.update(update),
            _ => Ok(()),
        }
    }

    fn update(&mut self, update: Update) -> Result<(), String> {
        match update {
            Update::GroupJoined(info) => {
                self.group = Some(info.group_id.clone());
                self.members.clone_from(&info.participants);
                self.tell(News::Joined {
                    server: self.server.id().to_owned(),
                    room: None,
                    name: info.group_name.clone(),
                    members: info.participants.clone(),
                    state: state(info.state, ""),
                });
            }
            Update::UserJoined(name) => self.members_changed(format!("{name} joined")),
            Update::UserLeft(name) => self.members_changed(format!("{name} left")),
            Update::GroupLeft(_) if self.leaving => {}
            Update::GroupLeft(_) => return Err("You are no longer in the group.".into()),
            Update::NotInGroup(_) if self.group.is_some() && !self.leaving => {
                return Err("The group has ended.".into());
            }
            Update::GroupDoesNotExist(_) => return Err("That group no longer exists.".into()),
            Update::LibraryAccessDenied(_) if self.group.is_none() => {
                return Err(
                    "You can't open what this group is playing, so you can't join it.".into(),
                );
            }
            Update::LibraryAccessDenied(_) => {
                self.tell(News::Notice(
                    "Not everyone in the group can open that title.".into(),
                ));
            }
            Update::StateUpdate(update) => {
                let now = state(update.state, &update.reason);
                log::debug!(
                    "together: state {:?} ({}) at {:.3}",
                    update.state,
                    update.reason,
                    local_now()
                );
                self.tell(News::State(now));
                match (update.state, update.reason.as_str()) {
                    (GroupState::Paused, "Pause") => {
                        self.tell(News::Notice("The group paused".into()));
                    }
                    (GroupState::Playing, "Unpause") => {
                        self.tell(News::Notice("The group is playing".into()));
                    }
                    _ => {}
                }
            }
            Update::PlayQueue(queue) => {
                let Some(playing) = queue.playing_item() else {
                    self.entry = None;
                    return Ok(());
                };
                self.entry = Some(playing.playlist_item_id.clone());
                let position = wire_clock::ticks_to_seconds(queue.start_position_ticks).max(0.0);
                log::debug!(
                    "together: queue {} from {position:.3} ({})",
                    playing.item_id,
                    queue.reason
                );
                self.tell(News::Queue {
                    item: jellyfin_id(self.server.id(), &playing.item_id),
                    position,
                });
            }
            Update::NotInGroup(_) | Update::Other { .. } => {}
        }
        Ok(())
    }

    /// Someone came or went: the server's own list is the one to show, since
    /// it names a user once however many devices they joined with.
    fn members_changed(&mut self, notice: String) {
        let listed = self
            .group
            .as_deref()
            .and_then(|id| self.api().ok()?.syncplay_group(id).ok());
        if let Some(group) = listed {
            self.members = group.participants;
        }
        self.tell(News::Members {
            members: self.members.clone(),
            notice,
        });
    }

    /// A command for the queue entry the group plays. What its `When` means
    /// depends on the command.
    fn command(&self, command: &wire::Command) {
        // Before a queue there is nothing to play: the `Stop` that follows
        // every join, for one.
        if self.entry.as_deref() != Some(command.playlist_item_id.as_str()) {
            return;
        }
        let position = command.position().map_or(0.0, |at| at.as_secs_f64());
        let follow = match command.command {
            // `When` is the scheduled start, a second ahead.
            CommandKind::Unpause => Command::PlayAt {
                at: group_time(command.when),
                position,
            },
            // `When` is when the server handled it; the position is the
            // group's at that moment, so it is where to stop.
            CommandKind::Pause => Command::PauseAt { position },
            // `When` is when the seek was asked for, kept on a re-send: not a
            // schedule.
            CommandKind::Seek => {
                self.tell(News::Notice(format!(
                    "The group moved to {}",
                    crate::player::clock(position)
                )));
                Command::SeekTo { position }
            }
            // The position is where the group stopped.
            CommandKind::Stop => Command::Stop,
        };
        log::debug!(
            "together: {:?} position {position:.3} when {:.3} received {:.3}",
            command.command,
            group_time(command.when).0,
            local_now()
        );
        self.tell(News::Command(follow));
    }

    /// The socket dropped, which takes this device out of the group: one
    /// new socket and one join. `Err` when that fails too.
    fn rejoin(&mut self) -> Result<(), String> {
        const DROPPED: &str = "The connection to the group dropped.";
        let recent = self
            .rejoined
            .is_some_and(|at| at.elapsed() < REJOIN_SPACING);
        let Some(group) = self.group.clone().filter(|_| !recent) else {
            return Err(DROPPED.into());
        };
        self.rejoined = Some(Instant::now());
        log::info!("together: the socket dropped; joining the group again");
        let socket = self.server.socket().map_err(|_| DROPPED.to_owned())?;
        self.socket = socket;
        self.api()
            .and_then(|api| api.syncplay_join(&group))
            .map_err(|_| DROPPED.to_owned())?;
        self.tell(News::Notice("Reconnected to the group".into()));
        Ok(())
    }

    /// One ask from the screens or the follower.
    fn ask(&mut self, ask: Ask) {
        let Ok(api) = self.api() else {
            return;
        };
        let sent = match ask {
            Ask::Queue { item, start } => api.syncplay_set_new_queue(&[&item], 0, seconds(start)),
            Ask::Request(Request::Pause) => api.syncplay_pause(),
            Ask::Request(Request::Unpause) => api.syncplay_unpause(),
            Ask::Request(Request::Seek { position }) => api.syncplay_seek(seconds(position)),
            Ask::Ready(report) | Ask::Buffering(report) => {
                let Some(entry) = self.entry.as_deref() else {
                    return;
                };
                let player = PlayerReport {
                    when: utc(report.at),
                    position: seconds(report.position),
                    is_playing: report.playing,
                    playlist_item_id: entry,
                };
                log::debug!(
                    "together: report {} position {:.3} at {:.3} playing {}",
                    if matches!(ask, Ask::Ready(_)) {
                        "ready"
                    } else {
                        "buffering"
                    },
                    report.position,
                    report.at.0,
                    report.playing
                );
                if matches!(ask, Ask::Ready(_)) {
                    api.syncplay_ready(&player)
                } else {
                    api.syncplay_buffering(&player)
                }
            }
            _ => Ok(()),
        };
        if let Err(error) = sent {
            log::debug!("together: the group refused an ask: {error}");
            self.tell(News::Notice(format!(
                "The group did not take that: {}",
                refusal(error)
            )));
        }
    }

    /// Leave the group, wait briefly for the server to say so, and close.
    fn leave(mut self) {
        self.leaving = true;
        if self.group.is_some() {
            if let Ok(api) = self.api() {
                let _ = api.syncplay_leave();
            }
            let until = Instant::now() + LEAVE_WAIT;
            while Instant::now() < until {
                match self.socket.read(READ) {
                    Ok(Some(Message::GroupUpdate(GroupUpdate {
                        update: Update::GroupLeft(_),
                        ..
                    })))
                    | Err(_) => break,
                    _ => {}
                }
            }
        }
        let ev = self.ev;
        let _ = self.socket.close();
        tell(ev, News::Left { why: None });
    }
}

/// A refusal in words the panel can show as they are.
fn refusal(error: JfError) -> String {
    match error {
        JfError::AuthRejected { status: 403 } => {
            "You don't have permission to watch together on this server.".into()
        }
        JfError::AuthRejected { .. } => {
            "The server no longer accepts this device's sign-in.".into()
        }
        JfError::Transport => "The server could not be reached.".into(),
        other => format!("The server answered with an error ({other})."),
    }
}

fn listed(group: GroupInfo) -> Listed {
    Listed {
        state: state(group.state, ""),
        id: group.group_id,
        name: group.group_name,
        members: group.participants,
    }
}

/// The server's state, and the request that moved it there.
fn state(state: GroupState, reason: &str) -> State {
    match state {
        GroupState::Idle => State::Idle,
        GroupState::Waiting if reason == "Buffer" => State::CatchingUp,
        GroupState::Waiting => State::Waiting,
        GroupState::Paused => State::Paused,
        GroupState::Playing => State::Playing,
    }
}

/// The local clock every reading is taken on, as an instant on the wire.
fn local_utc() -> UtcTime {
    UtcTime((local_now() * 1e7).round() as i64)
}

/// A server instant as group time: the group's clock is the server's.
fn group_time(at: UtcTime) -> GroupTime {
    GroupTime(at.0 as f64 / 1e7)
}

/// Group time as a server instant.
fn utc(at: GroupTime) -> UtcTime {
    UtcTime((at.0 * 1e7).round() as i64)
}

fn seconds(position: f64) -> Duration {
    Duration::from_secs_f64(position.max(0.0))
}

fn tell(ev: &Sender<Ev>, news: News) {
    let _ = ev.send(Ev::Together(news));
}
