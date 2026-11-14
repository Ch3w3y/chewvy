//! Watching together, as the shell runs it: the group this device is in, the
//! follower that keeps the player in step with it, the panel on a detail
//! page and the short notices.
//!
//! The group session runs on the worker's side ([`crate::together::jellyfin`]);
//! this end hears [`News`] and answers with [`Ask`]s. The follower runs here,
//! right after the player is ticked, because both live on the window's
//! thread.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::player;
use crate::source::{self, Origin};
use crate::together::clock::{local_now, GroupClock, GroupTime};
use crate::together::follow::{Action, Command, Follower, Reading};
use crate::together::plex::{self, Pulse};
use crate::together::{Ask, Listed, News, Person, Report, Room, State};
use crate::ui::player::together::{Badge, Guest, Guests};
use crate::ui::together::{self as view, Choice, PanelView};
use crate::watch::Cmd;

use super::Ui;

/// How long a notice stays up.
const NOTICE: Duration = Duration::from_secs(4);
/// How often the follower runs while a group plays, whatever else redraws
/// the window: the HDR window draws for itself, and this window would
/// otherwise wake only four times a second.
const TICK: Duration = Duration::from_millis(50);
/// How often the distance from the group is written to the log.
const SYNC_LOG: Duration = Duration::from_secs(1);

/// The group this device is in.
pub(super) struct Group {
    /// The source the group is on; empty for a Plex room.
    server: String,
    /// The Plex room's id, for a room.
    pub(super) room: Option<String>,
    name: String,
    members: Vec<String>,
    state: State,
}

/// What a panel is about.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Subject {
    /// A Jellyfin server's groups.
    Jellyfin(String),
    /// A Plex title's rooms.
    Title { rating_key: String, title: String },
    /// Every Plex room on the account.
    Rooms,
}

/// The watch-together panel, while it is open.
pub(super) struct Panel {
    pub(super) subject: Subject,
    /// The button that opened it, which the panel sits under.
    pub(super) anchor: egui::Rect,
    /// The server's groups, once it listed them, or why it would not.
    pub(super) groups: Option<Result<Vec<Listed>, String>>,
    /// Why starting or joining did not work.
    pub(super) refusal: Option<String>,
}

/// Everything the shell keeps about watching together.
#[derive(Default)]
pub(super) struct Together {
    pub(super) group: Option<Group>,
    /// A start or join was asked for and the server has not answered yet.
    pub(super) joining: bool,
    pub(super) panel: Option<Panel>,
    /// The Plex rooms this account is in or invited to, once listed.
    pub(super) rooms: Option<Result<Vec<Room>, String>>,
    /// Who a Plex room can invite, once listed.
    pub(super) people: Option<Result<Vec<Person>, String>>,
    /// Who is ticked to invite.
    pub(super) ticked: Vec<u64>,
    clock: GroupClock,
    /// Keeps the playing item in step; made when a player opens in a group.
    follower: Option<Follower>,
    /// The group's latest command, for a player that opens after it came.
    pub(super) command: Option<Command>,
    /// The group's title to open on the next frame, and where.
    open: Option<(String, f64)>,
    notices: VecDeque<(String, Instant)>,
    logged: Option<Instant>,
}

impl Together {
    pub(super) fn notice(&mut self, text: impl Into<String>) {
        let text = text.into();
        // The HDR window covers this one, so it is said there as well.
        player::with(|p| {
            if p.output().is_hdr_window() {
                log::debug!("together: notice shown in the HDR window");
                p.show_text(&text, NOTICE);
            }
        });
        self.notices.push_back((text, Instant::now()));
    }

    pub(super) fn now(&self) -> Option<GroupTime> {
        self.clock.group_time(local_now())
    }

    /// What the player shows of the group.
    pub(super) fn badge(&self) -> Option<Badge> {
        self.group.as_ref().map(|group| Badge {
            name: group.name.clone(),
            members: group.members.clone(),
            state: group.state.words(),
            guests: group.room.as_deref().map(|room| self.guests(room)),
        })
    }

    /// Who the Plex room `room` can still invite.
    fn guests(&self, room: &str) -> Guests {
        let invited = self
            .rooms
            .as_ref()
            .and_then(|rooms| rooms.as_ref().ok())
            .and_then(|rooms| rooms.iter().find(|listed| listed.id == room))
            .map_or(&[][..], |listed| &listed.ids);
        match &self.people {
            None => Guests::Finding,
            Some(Err(why)) => Guests::Unknown(why.clone()),
            Some(Ok(people)) => Guests::Listed(
                people
                    .iter()
                    .filter(|person| !invited.contains(&person.id))
                    .map(|person| Guest {
                        id: person.id,
                        name: person.name.clone(),
                        home: person.home,
                        ticked: self.ticked.contains(&person.id),
                    })
                    .collect(),
            ),
        }
    }

    /// Whether this device is in a group, or joining one.
    pub(super) fn active(&self) -> bool {
        self.group.is_some() || self.joining
    }
}

impl Ui {
    /// One piece of news from the group session.
    pub(super) fn apply_together(&mut self, news: News) {
        let together = &mut self.together;
        match news {
            News::Groups { server, groups } => {
                if let Some(panel) = together
                    .panel
                    .as_mut()
                    .filter(|p| p.subject == Subject::Jellyfin(server.clone()))
                {
                    panel.groups = Some(groups);
                }
            }
            News::Rooms(rooms) => together.rooms = Some(rooms),
            News::People(people) => together.people = Some(people),
            News::Clock(sample) => together.clock.add(sample),
            News::Joined {
                server,
                room,
                name,
                members,
                state,
            } => {
                together.joining = false;
                together.ticked.clear();
                let in_room = room.is_some();
                together.group = Some(Group {
                    server,
                    room,
                    name,
                    members,
                    state,
                });
                if in_room {
                    // The room's panel in the player offers who else to invite.
                    self.send(Cmd::Together(Ask::People));
                }
            }
            News::Members { members, notice } => {
                if let Some(group) = together.group.as_mut() {
                    group.members = members;
                }
                if !notice.is_empty() {
                    together.notice(notice);
                }
            }
            News::State(state) => {
                if let Some(group) = together.group.as_mut() {
                    group.state = state;
                }
            }
            News::Queue { item, position } => {
                let command = Command::SeekTo { position };
                together.command = Some(command);
                let loaded = player::with(|p| p.request().rating_key == item).unwrap_or(false);
                match together.follower.as_mut().filter(|_| loaded) {
                    Some(follower) => follower.command(command),
                    None => together.open = Some((item, position)),
                }
            }
            News::Command(command) => {
                together.command = Some(command);
                if let Some(follower) = together.follower.as_mut() {
                    follower.command(command);
                }
            }
            News::Notice(text) => together.notice(text),
            News::Left { why } => {
                self.release_player();
                let together = &mut self.together;
                together.group = None;
                together.joining = false;
                // The next group's clock is its own.
                together.clock = GroupClock::default();
                if let Some(why) = why {
                    match together.panel.as_mut() {
                        Some(panel) => panel.refusal = Some(why),
                        None => together.notice(why),
                    }
                }
            }
        }
    }

    /// Start a title: for the whole group when it is on the group's server,
    /// otherwise here alone, leaving any group first.
    pub(super) fn start_title(
        &mut self,
        rating_key: String,
        title: String,
        start: Option<f64>,
        ctx: &egui::Context,
    ) {
        if let Some(group) = &self.together.group {
            if group.room.is_some() && self.room_title() == Some(rating_key.as_str()) {
                // A room plays its one title: open it where the room is.
                let start = self.together.follower_position();
                self.status = format!("Opening “{title}” for the room…");
                self.send(Cmd::Play {
                    rating_key,
                    title,
                    start,
                    quality: None,
                    wakeup: ctx.clone(),
                });
                return;
            }
            if let (Origin::Jellyfin(server), id) = source::split(&rating_key) {
                if server == group.server {
                    self.status = format!("Starting “{title}” for the group…");
                    self.send(Cmd::Together(Ask::Queue {
                        item: id.to_owned(),
                        start: start.unwrap_or(0.0),
                    }));
                    return;
                }
            }
            self.leave_group(Some("You left the group to play this on your own"));
        }
        self.status = format!("Opening “{title}”…");
        self.send(Cmd::Play {
            rating_key,
            title,
            start,
            quality: None,
            wakeup: ctx.clone(),
        });
    }

    /// Open the detail page's panel for `server`, asking it for its groups.
    pub(super) fn open_together(&mut self, server: String, anchor: egui::Rect) {
        self.together.panel = Some(Panel {
            subject: Subject::Jellyfin(server.clone()),
            anchor,
            groups: None,
            refusal: None,
        });
        let here = self
            .together
            .group
            .as_ref()
            .is_some_and(|g| g.room.is_none() && g.server == server);
        if !here && !self.together.joining {
            self.send(Cmd::Together(Ask::Groups { server }));
        }
    }

    /// Leave the group and give the player back; `notice` says so.
    pub(super) fn leave_group(&mut self, notice: Option<&str>) {
        if !self.together.active() {
            return;
        }
        self.send(Cmd::Together(Ask::Leave));
        self.release_player();
        self.together.group = None;
        self.together.joining = false;
        if let Some(text) = notice {
            self.together.notice(text);
        }
    }

    /// A player just opened: in a group it opens paused and its controls ask
    /// the group, until the follower takes over on its first reading. The
    /// detail page's panel closes, as the page itself does.
    pub(super) fn player_opened(&mut self) {
        self.together.panel = None;
        if self.together.group.is_some() {
            player::with(|p| {
                p.set_paused(true);
                p.follow_group(true);
            });
        }
        self.together.follower = None;
    }

    /// Every frame, before anything else draws: open the group's title when
    /// a queue named one, forget the follower of a player that closed, and
    /// draw the panel and the notices.
    pub(super) fn together_frame(&mut self, ctx: &egui::Context) {
        if !player::is_active() && self.together.follower.take().is_some() {
            plex::report(None);
        }
        if let Some((item, position)) = self.together.open.take() {
            if self.together.group.is_some() {
                if player::is_active() {
                    self.end_playback();
                }
                self.handoff = Some("Opening the group's title…".into());
                self.send(Cmd::Play {
                    rating_key: item,
                    title: String::new(),
                    start: Some(position),
                    quality: None,
                    wakeup: ctx.clone(),
                });
            }
        }
        self.together_panel(ctx);
        let together = &mut self.together;
        while together
            .notices
            .front()
            .is_some_and(|(_, at)| at.elapsed() >= NOTICE)
        {
            together.notices.pop_front();
        }
        if let Some((text, at)) = together.notices.back() {
            view::notice(ctx, text);
            ctx.request_repaint_after(NOTICE.saturating_sub(at.elapsed()));
        }
    }

    /// Keep the playing item in step with the group. Runs right after the
    /// player's own tick, so the reading is the one the previous frame's
    /// actions produced.
    pub(super) fn follow_group(&mut self, ctx: &egui::Context) {
        if self.together.group.is_none() || !player::is_active() {
            return;
        }
        let Some(now) = self.together.now() else {
            return;
        };
        let Some((reading, requests)) =
            player::with(|p| (Reading::from(p.snapshot()), p.take_requests()))
        else {
            return;
        };
        for request in requests {
            log::debug!("together: asking {request:?} at {:.3}", now.0);
            self.send(Cmd::Together(Ask::Request(request)));
        }
        let together = &mut self.together;
        let follower = together.follower.get_or_insert_with(|| {
            let mut follower = Follower::new(reading.speed);
            if let Some(command) = together.command {
                follower.command(command);
            }
            follower
        });
        let actions = follower.tick(reading, now);
        let expected = follower.expected(now);
        if together.group.as_ref().is_some_and(|g| g.room.is_some()) && reading.loaded {
            plex::report(Some(Pulse {
                position: reading.position,
                paused: reading.paused,
                stalled: reading.stalled,
                at: now.0,
            }));
        }
        let starts_at = follower.starts_at().filter(|at| *at > now);
        let mut paused = reading.paused;
        let mut asks = Vec::new();
        let mut stop = false;
        for action in actions {
            log::debug!(
                "together: {action:?} at {:.3} from {:.3}",
                now.0,
                reading.position
            );
            match action {
                Action::SetPaused(on) => {
                    paused = on;
                    player::with(|p| p.set_paused(on));
                }
                Action::SeekTo(position) => {
                    player::with(|p| p.align(position));
                }
                Action::SetSpeed(speed) => {
                    player::with(|p| p.set_speed(speed));
                }
                Action::Ready { position } | Action::Buffering { position } => {
                    let report = Report {
                        position,
                        at: now,
                        playing: !paused,
                    };
                    asks.push(if matches!(action, Action::Ready { .. }) {
                        Ask::Ready(report)
                    } else {
                        Ask::Buffering(report)
                    });
                }
                Action::Request(request) => asks.push(Ask::Request(request)),
                Action::Stop => stop = true,
            }
        }
        for ask in asks {
            self.send(Cmd::Together(ask));
        }
        self.log_sync(now, reading, expected);
        if stop {
            // The group stopped: this player closes, the group stays.
            self.end_playback();
            self.reset_output();
            return;
        }
        let mut wait = TICK;
        if let Some(at) = starts_at {
            wait = wait.min(Duration::from_secs_f64((at.0 - now.0).max(0.0)));
        }
        ctx.request_repaint_after(wait);
    }

    /// Once a second: how far the player is from where the group is.
    fn log_sync(&mut self, now: GroupTime, reading: Reading, expected: Option<f64>) {
        if self
            .together
            .logged
            .is_some_and(|at| at.elapsed() < SYNC_LOG)
        {
            return;
        }
        self.together.logged = Some(Instant::now());
        if let Some(expected) = expected.filter(|_| reading.loaded) {
            log::debug!(
                "together: sync at {:.3} position {:.3} expected {expected:.3} error {:+.3} paused {} stalled {} speed {:.2}",
                now.0,
                reading.position,
                reading.position - expected,
                reading.paused,
                reading.stalled,
                reading.speed
            );
        }
    }

    /// Give the player back: the person's own speed, and their own controls.
    fn release_player(&mut self) {
        plex::report(None);
        let together = &mut self.together;
        let restore = together.follower.take().map(Follower::leave);
        player::with(|p| {
            for action in restore.into_iter().flatten() {
                if let Action::SetSpeed(speed) = action {
                    p.set_speed(speed);
                }
            }
            p.follow_group(false);
        });
        together.command = None;
        together.open = None;
    }

    /// The watch-together panel, while it is open.
    fn together_panel(&mut self, ctx: &egui::Context) {
        let Some(panel) = &self.together.panel else {
            return;
        };
        let anchor = panel.anchor;
        let subject = panel.subject.clone();
        let choice = {
            let view = self.panel_view();
            view::panel(ctx, anchor, &view)
        };
        let Some(choice) = choice else {
            return;
        };
        if matches!(
            choice,
            Choice::Start | Choice::Join(_) | Choice::Create | Choice::Enter(_)
        ) {
            self.together.joining = true;
            if let Some(panel) = self.together.panel.as_mut() {
                panel.refusal = None;
            }
        }
        match (choice, subject) {
            (Choice::Start, Subject::Jellyfin(server)) => {
                self.send(Cmd::Together(Ask::Start { server }));
            }
            (Choice::Join(group), Subject::Jellyfin(server)) => {
                self.send(Cmd::Together(Ask::Join { server, group }));
            }
            (Choice::Leave, _) => {
                self.leave_group(None);
                self.together.panel = None;
            }
            (Choice::Close, _) => self.together.panel = None,
            (choice, _) => self.room_choice(choice),
        }
    }

    /// What the open panel shows.
    fn panel_view(&self) -> PanelView<'_> {
        let together = &self.together;
        let Some(panel) = &together.panel else {
            return PanelView::Joining;
        };
        // Only the group this panel is about shows as joined: a title from
        // one service never offers the other's.
        let group = together
            .group
            .as_ref()
            .filter(|group| self.panel_is_for(group, &panel.subject));
        match (group, together.joining, &panel.subject) {
            (Some(group), _, _) => PanelView::In {
                name: &group.name,
                members: &group.members,
                state: group.state.words(),
                invite: self.invite_view(group),
            },
            (None, true, _) => PanelView::Joining,
            (None, false, Subject::Jellyfin(_)) => PanelView::Choose {
                groups: panel.groups.as_ref(),
                refusal: panel.refusal.as_deref(),
            },
            (None, false, subject) => self.rooms_view(subject, panel.refusal.as_deref()),
        }
    }

    /// Whether `group` is what a panel about `subject` shows.
    fn panel_is_for(&self, group: &Group, subject: &Subject) -> bool {
        match subject {
            Subject::Jellyfin(server) => group.room.is_none() && &group.server == server,
            Subject::Title { rating_key, .. } => {
                group.room.is_some() && self.room_title() == Some(rating_key.as_str())
            }
            Subject::Rooms => group.room.is_some(),
        }
    }
}
