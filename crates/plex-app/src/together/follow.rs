//! The sync follower: keeps this player in step with a group.
//!
//! It turns the group's commands and the player's readings into the actions
//! that bring this player to where the group is. It knows nothing of any
//! server: a session turns its messages into [`Command`]s, applies the
//! [`Action`]s to the player and sends the group what they ask for.

use super::clock::GroupTime;
use crate::player::Snapshot;

/// A start this far from the scheduled position seeks there first; closer, the
/// speed nudge catches up without the picture jumping.
const START_SEEK: f64 = 0.5;
/// Drift below this is jitter in how positions are read; chasing it would hunt.
const DRIFT_IGNORE: f64 = 0.1;
/// A nudged speed goes back to normal inside this, so it does not overshoot.
const DRIFT_DONE: f64 = 0.05;
/// Beyond this a nudge would take too long to catch up (40 s at 5%), so seek.
const DRIFT_SEEK: f64 = 2.0;
/// How far a nudge moves the speed: mpv keeps the pitch, and 5% passes unnoticed.
const NUDGE: f64 = 0.05;
/// A stall shorter than this is a hiccup the cache rides out; reporting it
/// would pause everyone.
const STALL_GRACE: f64 = 0.5;
/// A seek counts as landed when the position is this close to where it went.
const LANDED: f64 = 0.5;
/// A seek that has not landed in this long is given up on and may be sent again.
const SEEK_PATIENCE: f64 = 5.0;
/// A jump between two readings larger than this, that the follower did not
/// ask for, is someone seeking in the player itself.
const USER_JUMP: f64 = 1.0;
/// How long a change asked of the group may go unanswered before the player
/// goes back to where the group is: a server answers in well under a second,
/// so silence this long means the request was lost or refused.
const REQUEST_PATIENCE: f64 = 4.0;
/// A speed this far from the one last set was changed in the player itself.
const SPEED_CHANGED: f64 = 0.01;

/// What the group wants this player to do.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
    /// Be playing, at `position` when the group clock reads `at`.
    PlayAt {
        /// Group time the position applies to; playback starts then.
        at: GroupTime,
        /// Seconds into the item at `at`.
        position: f64,
    },
    /// Be paused at `position`.
    PauseAt {
        /// Seconds into the item.
        position: f64,
    },
    /// Move to `position`, stay paused, and say when ready.
    SeekTo {
        /// Seconds into the item.
        position: f64,
    },
    /// Leave playback.
    Stop,
}

/// What the player reports each tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reading {
    /// Seconds into the item; meaningless until `loaded`.
    pub position: f64,
    /// Paused.
    pub paused: bool,
    /// Not advancing although it should: waiting for the cache, or a seek or
    /// the first frame not yet shown.
    pub stalled: bool,
    /// The item has reached a first position.
    pub loaded: bool,
    /// Playback speed.
    pub speed: f64,
    /// Held on the last frame at the end of the item.
    pub ended: bool,
}

impl From<&Snapshot> for Reading {
    fn from(snapshot: &Snapshot) -> Self {
        Self {
            position: snapshot.position.unwrap_or(0.0),
            paused: snapshot.paused,
            stalled: snapshot.stalled,
            loaded: snapshot.position.is_some(),
            speed: snapshot.speed,
            ended: snapshot.eof,
        }
    }
}

/// A change made in the player itself (the HDR window has its own keys) that
/// the group should make for everyone.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Request {
    /// Pause the group.
    Pause,
    /// Play the group.
    Unpause,
    /// Move the group to `position`.
    Seek {
        /// Seconds into the item.
        position: f64,
    },
}

/// What to do to the player, or to tell the group.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    /// Pause or play the player.
    SetPaused(bool),
    /// Seek the player, exactly, to this many seconds into the item.
    SeekTo(f64),
    /// Set the player's speed.
    SetSpeed(f64),
    /// Tell the group this device can play from `position`.
    Ready {
        /// Seconds into the item.
        position: f64,
    },
    /// Tell the group this device has stalled at `position`.
    Buffering {
        /// Seconds into the item.
        position: f64,
    },
    /// Ask the group for a change made in the player itself.
    Request(Request),
    /// Leave playback.
    Stop,
}

/// Where the group wants the player.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Goal {
    /// No command yet.
    Idle,
    Play {
        at: GroupTime,
        position: f64,
    },
    Pause {
        position: f64,
    },
    Stopped,
}

/// A seek the follower sent that has not landed yet.
#[derive(Clone, Copy, Debug)]
struct Seek {
    position: f64,
    sent: GroupTime,
}

/// The previous reading, to tell a seek in the player from playback.
#[derive(Clone, Copy, Debug)]
struct Last {
    position: f64,
    at: GroupTime,
    moving: bool,
}

/// Keeps one player in step with a group.
#[derive(Debug)]
pub struct Follower {
    goal: Goal,
    /// The goal changed since the last tick: line up with it once.
    fresh: bool,
    /// The speed the user had chosen, given back on leaving.
    user_speed: f64,
    /// The speed last set on the player.
    speed: f64,
    /// -1, 0 or 1: slowing down, normal, or catching up.
    nudge: f64,
    /// The pause state last set on the player, or accepted from the user.
    paused: Option<bool>,
    seek: Option<Seek>,
    stalled_since: Option<GroupTime>,
    /// `Buffering` was sent; `Ready` is owed on recovery.
    buffering: bool,
    /// A group seek asked for `Ready` once it lands.
    ready_owed: bool,
    /// The group seek has been sent a second time.
    resettled: bool,
    /// A start still to be checked once the player is seen moving: a player
    /// that has just opened can hold its start position for a second
    /// without saying it is stalled.
    start_check: bool,
    last: Option<Last>,
    /// The user changed the player itself, at this time: leave it alone until
    /// the group answers, or for [`REQUEST_PATIENCE`].
    deferring: Option<GroupTime>,
}

impl Follower {
    /// A follower for a player the user had at `user_speed`.
    #[must_use]
    pub fn new(user_speed: f64) -> Self {
        Self {
            goal: Goal::Idle,
            fresh: false,
            user_speed,
            speed: user_speed,
            nudge: 0.0,
            paused: None,
            seek: None,
            stalled_since: None,
            buffering: false,
            ready_owed: false,
            resettled: false,
            start_check: false,
            last: None,
            deferring: None,
        }
    }

    /// Take the group's latest command; the next [`Follower::tick`] acts on it.
    pub fn command(&mut self, command: Command) {
        self.goal = match command {
            Command::PlayAt { at, position } => Goal::Play { at, position },
            Command::PauseAt { position } | Command::SeekTo { position } => {
                Goal::Pause { position }
            }
            Command::Stop => Goal::Stopped,
        };
        self.ready_owed = matches!(command, Command::SeekTo { .. });
        self.resettled = false;
        self.start_check = matches!(command, Command::PlayAt { .. });
        self.fresh = true;
        self.deferring = None;
        self.seek = None;
    }

    /// Where the group's goal puts the player at `now`, once there is one.
    #[must_use]
    pub fn expected(&self, now: GroupTime) -> Option<f64> {
        match self.goal {
            Goal::Play { at, position } => Some(position + (now.0 - at.0).max(0.0)),
            Goal::Pause { position } => Some(position),
            Goal::Idle | Goal::Stopped => None,
        }
    }

    /// When a scheduled start is due, so the caller can tick on time.
    #[must_use]
    pub fn starts_at(&self) -> Option<GroupTime> {
        match self.goal {
            Goal::Play { at, .. } => Some(at),
            _ => None,
        }
    }

    /// One step: what the player and the group need now.
    ///
    /// `reading` must be taken after the previous tick's actions were applied.
    pub fn tick(&mut self, reading: Reading, now: GroupTime) -> Vec<Action> {
        let mut actions = Vec::new();
        if self.goal == Goal::Stopped {
            if std::mem::take(&mut self.fresh) {
                actions.push(Action::Stop);
            }
            return actions;
        }
        if self
            .deferring
            .is_some_and(|since| now.0 - since.0 > REQUEST_PATIENCE)
        {
            // Unanswered: line up with the group again, as after a command.
            self.deferring = None;
            self.fresh = true;
        }
        // At the end mpv holds the last frame paused by itself; that pause,
        // and the position that stops there, are not the user's to report.
        // A group seek that lands there still owes its `Ready`.
        let held_at_end = reading.ended && !self.fresh && !self.ready_owed;
        if reading.loaded && self.goal != Goal::Idle && !held_at_end {
            if !self.fresh && self.seek.is_none() {
                self.notice_user(reading, now, &mut actions);
            }
            self.watch_seek(reading, now);
            self.watch_stall(reading, now, &mut actions);
            if self.deferring.is_none() {
                self.pursue(reading, now, &mut actions);
            }
            self.fresh = false;
        }
        if self.deferring.is_some() || reading.stalled || self.seek.is_some() || held_at_end {
            self.nudge = 0.0;
        }
        // A speed changed in the player itself (the HDR window has its own
        // keys) is put back: the group's clock sets the pace.
        if reading.loaded && (reading.speed - self.speed).abs() > SPEED_CHANGED {
            self.speed = reading.speed;
        }
        self.set_speed(1.0 + self.nudge * NUDGE, &mut actions);
        self.last = reading.loaded.then_some(Last {
            position: reading.position,
            at: now,
            moving: !reading.paused && !reading.stalled,
        });
        actions
    }

    /// Leave the group: what gives the player back to the user.
    #[must_use]
    pub fn leave(mut self) -> Vec<Action> {
        let mut actions = Vec::new();
        let speed = self.user_speed;
        self.set_speed(speed, &mut actions);
        actions
    }

    /// A pause or a jump in the player that the follower did not cause.
    fn notice_user(&mut self, reading: Reading, now: GroupTime, actions: &mut Vec<Action>) {
        if self.paused.is_some_and(|paused| paused != reading.paused) {
            self.paused = Some(reading.paused);
            self.deferring = Some(now);
            actions.push(Action::Request(if reading.paused {
                Request::Pause
            } else {
                Request::Unpause
            }));
        }
        let Some(last) = self.last else {
            return;
        };
        let moved = if last.moving {
            (now.0 - last.at.0) * self.speed
        } else {
            0.0
        };
        if (reading.position - last.position - moved).abs() > USER_JUMP {
            self.deferring = Some(now);
            actions.push(Action::Request(Request::Seek {
                position: reading.position,
            }));
        }
    }

    /// Clear a seek once it has landed, or once it has had long enough.
    fn watch_seek(&mut self, reading: Reading, now: GroupTime) {
        let Some(seek) = self.seek else {
            return;
        };
        let playing = !reading.paused && matches!(self.goal, Goal::Play { .. });
        let moved = if playing { now.0 - seek.sent.0 } else { 0.0 };
        let landed = reading.position >= seek.position - LANDED
            && reading.position <= seek.position + moved + LANDED;
        if (!reading.stalled && landed) || now.0 - seek.sent.0 > SEEK_PATIENCE {
            self.seek = None;
        }
    }

    /// Tell the group about a stall that outlasts the grace, and its end.
    fn watch_stall(&mut self, reading: Reading, now: GroupTime, actions: &mut Vec<Action>) {
        if !reading.stalled {
            self.stalled_since = None;
            // A group seek still owes its own `Ready`, sent once it lands.
            if std::mem::take(&mut self.buffering) && !self.ready_owed {
                actions.push(Action::Ready {
                    position: reading.position,
                });
                // A `Ready` while playing sends this player no command: the
                // server starts the others when this player reaches where
                // they wait, so the group now runs from here.
                if !reading.paused && matches!(self.goal, Goal::Play { .. }) {
                    self.goal = Goal::Play {
                        at: now,
                        position: reading.position,
                    };
                }
            }
            return;
        }
        let since = *self.stalled_since.get_or_insert(now);
        let group_plays = matches!(self.goal, Goal::Play { at, .. } if now >= at);
        if group_plays && !self.buffering && now.0 - since.0 > STALL_GRACE {
            self.buffering = true;
            actions.push(Action::Buffering {
                position: reading.position,
            });
        }
    }

    /// Bring the player to the goal.
    fn pursue(&mut self, reading: Reading, now: GroupTime, actions: &mut Vec<Action>) {
        match self.goal {
            Goal::Play { at, position } => {
                let started = now >= at;
                let target = position + (now.0 - at.0).max(0.0);
                let off = (reading.position - target).abs() > START_SEEK;
                if self.fresh && off {
                    self.seek_to(target, now, actions);
                } else if started && self.start_check && self.moving(reading) {
                    self.start_check = false;
                    if off {
                        self.seek_to(target, now, actions);
                    } else {
                        self.correct_drift(reading, target, now, actions);
                    }
                } else if started {
                    self.correct_drift(reading, target, now, actions);
                }
                self.set_paused(!started, actions);
            }
            Goal::Pause { position } => {
                self.set_paused(true, actions);
                if self.ready_owed {
                    self.settle(reading, position, now, actions);
                } else if self.fresh && (reading.position - position).abs() > START_SEEK {
                    self.seek_to(position, now, actions);
                }
            }
            Goal::Idle | Goal::Stopped => {}
        }
    }

    /// Seek for a group seek, then say `Ready` once it has landed there.
    fn settle(
        &mut self,
        reading: Reading,
        position: f64,
        now: GroupTime,
        actions: &mut Vec<Action>,
    ) {
        if self.fresh {
            self.seek_to(position, now, actions);
        } else if self.seek.is_none() && !reading.stalled {
            // Seeks here are exact, so `Ready` waits for one that landed on
            // the spot: a player that has just opened reads its start
            // position before the seek has run, and may then play on a
            // little. A second seek that lands near enough is taken as it is.
            let error = (reading.position - position).abs();
            if error <= DRIFT_IGNORE || reading.ended || (self.resettled && error <= LANDED) {
                self.ready_owed = false;
                actions.push(Action::Ready {
                    position: reading.position,
                });
            } else {
                self.resettled = true;
                self.seek_to(position, now, actions);
            }
        }
    }

    /// The three drift bands: leave alone, nudge the speed, or seek.
    fn correct_drift(
        &mut self,
        reading: Reading,
        target: f64,
        now: GroupTime,
        actions: &mut Vec<Action>,
    ) {
        if reading.stalled || self.seek.is_some() {
            return;
        }
        let drift = reading.position - target;
        let catch_up = if drift < 0.0 { 1.0 } else { -1.0 };
        if drift.abs() > DRIFT_SEEK {
            self.nudge = 0.0;
            self.seek_to(target, now, actions);
        } else if self.nudge != 0.0 {
            self.nudge = if drift.abs() <= DRIFT_DONE {
                0.0
            } else {
                catch_up
            };
        } else if drift.abs() >= DRIFT_IGNORE {
            self.nudge = catch_up;
        }
    }

    /// Whether the player has moved on since the last reading, by itself.
    fn moving(&self, reading: Reading) -> bool {
        self.seek.is_none()
            && !reading.stalled
            && self
                .last
                .is_some_and(|last| last.moving && reading.position > last.position)
    }

    fn seek_to(&mut self, position: f64, now: GroupTime, actions: &mut Vec<Action>) {
        self.seek = Some(Seek {
            position,
            sent: now,
        });
        actions.push(Action::SeekTo(position));
    }

    fn set_paused(&mut self, paused: bool, actions: &mut Vec<Action>) {
        if self.paused != Some(paused) {
            self.paused = Some(paused);
            actions.push(Action::SetPaused(paused));
        }
    }

    fn set_speed(&mut self, speed: f64, actions: &mut Vec<Action>) {
        if (self.speed - speed).abs() > f64::EPSILON {
            self.speed = speed;
            actions.push(Action::SetSpeed(speed));
        }
    }
}
