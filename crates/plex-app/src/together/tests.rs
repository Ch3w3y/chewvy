use std::collections::HashMap;

use super::clock::{GroupClock, GroupTime, Sample};
use super::follow::{Action, Command, Follower, Reading, Request};
use crate::player::Snapshot;

/// How often the fake player is ticked, in seconds: a power of two, so the
/// fake clock lands exactly on the times the tests schedule.
const TICK: f64 = 0.0625;

/// A fake player on a fake group clock: it applies the follower's actions and
/// plays on between ticks.
struct Sim {
    follower: Follower,
    now: f64,
    position: f64,
    paused: bool,
    speed: f64,
    /// A seek shows its target at once, then stalls this long.
    seek_time: f64,
    stalled_for: f64,
    /// Stalled whatever the follower does (a slow network).
    starved: bool,
    /// Held paused on the last frame, as mpv does at the end of an item.
    ended: bool,
    /// The share of playing time lost to a decoder that cannot keep up.
    lag: f64,
    /// Seconds of playing time a newly opened player stands still for
    /// without saying it is stalled.
    frozen: f64,
    log: Vec<(f64, Action)>,
}

impl Sim {
    fn new(user_speed: f64) -> Self {
        Self {
            follower: Follower::new(user_speed),
            now: 0.0,
            position: 0.0,
            paused: true,
            speed: user_speed,
            seek_time: 0.0,
            stalled_for: 0.0,
            starved: false,
            ended: false,
            lag: 0.0,
            frozen: 0.0,
            log: Vec::new(),
        }
    }

    fn stalled(&self) -> bool {
        self.starved || self.stalled_for > 0.0
    }

    fn step(&mut self) {
        let reading = Reading {
            position: self.position,
            paused: self.paused,
            stalled: self.stalled(),
            loaded: true,
            speed: self.speed,
            ended: self.ended,
        };
        for action in self.follower.tick(reading, GroupTime(self.now)) {
            match action {
                Action::SetPaused(paused) => self.paused = paused,
                Action::SeekTo(at) => {
                    self.position = at;
                    self.stalled_for = self.seek_time;
                }
                Action::SetSpeed(speed) => self.speed = speed,
                _ => {}
            }
            self.log.push((self.now, action));
        }
        if !self.paused && !self.stalled() {
            if self.frozen > 0.0 {
                self.frozen -= TICK;
            } else {
                self.position += TICK * self.speed * (1.0 - self.lag);
            }
        }
        self.stalled_for = (self.stalled_for - TICK).max(0.0);
        self.now += TICK;
    }

    /// Run for `secs` and return what the follower did in that time.
    fn run(&mut self, secs: f64) -> Vec<Action> {
        let from = self.log.len();
        let until = self.now + secs - TICK / 2.0;
        while self.now < until {
            self.step();
        }
        self.log[from..].iter().map(|(_, a)| *a).collect()
    }

    /// Start playing `position` now and run until the follower is in step.
    fn playing(position: f64) -> Self {
        let mut sim = Self::new(1.0);
        sim.position = position;
        sim.follower.command(Command::PlayAt {
            at: GroupTime(0.0),
            position,
        });
        sim.run(1.0);
        sim
    }

    /// Where the group is now while it plays from `position` at time 0.
    fn drift(&self, position: f64) -> f64 {
        self.position - (position + self.now)
    }
}

fn count(actions: &[Action], wanted: impl Fn(&Action) -> bool) -> usize {
    actions.iter().filter(|a| wanted(a)).count()
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

#[test]
fn a_start_in_the_future_seeks_holds_paused_and_plays_on_time() {
    let mut sim = Sim::new(1.0);
    sim.follower.command(Command::PlayAt {
        at: GroupTime(2.0),
        position: 30.0,
    });
    let held = sim.run(2.0);
    assert_eq!(held, vec![Action::SeekTo(30.0), Action::SetPaused(true)]);
    assert!(sim.paused);
    let started = sim.run(TICK);
    assert_eq!(started, vec![Action::SetPaused(false)]);
    sim.run(3.0);
    assert!(sim.drift(30.0 - 2.0).abs() < 0.1, "plays in step");
}

#[test]
fn a_start_in_the_past_seeks_to_where_the_group_is_now() {
    let mut sim = Sim::new(1.0);
    sim.now = 10.0;
    sim.follower.command(Command::PlayAt {
        at: GroupTime(8.0),
        position: 30.0,
    });
    let first = sim.run(TICK);
    assert_eq!(first, vec![Action::SeekTo(32.0), Action::SetPaused(false)]);
    sim.run(2.0);
    assert!(sim.drift(22.0).abs() < 0.1);
}

#[test]
fn a_player_that_stands_still_after_opening_seeks_once_it_moves() {
    // Reopened at 9.9 while the group is at 10, it then holds that position
    // for a second without saying it is stalled.
    let mut sim = Sim::new(1.0);
    sim.now = 10.0;
    sim.position = 9.9;
    sim.frozen = 1.0;
    sim.follower.command(Command::PlayAt {
        at: GroupTime(0.0),
        position: 0.0,
    });
    let actions = sim.run(3.0);
    assert_eq!(
        count(&actions, |a| matches!(a, Action::SeekTo(_))),
        1,
        "{actions:?}"
    );
    sim.run(1.0);
    assert!(sim.drift(0.0).abs() < 0.1, "{}", sim.drift(0.0));
}

#[test]
fn small_drift_is_left_alone() {
    let mut sim = Sim::playing(100.0);
    sim.position -= 0.08;
    let actions = sim.run(5.0);
    assert!(actions.is_empty(), "{actions:?}");
}

#[test]
fn a_player_behind_catches_up_faster_then_returns_to_normal_speed() {
    let mut sim = Sim::playing(100.0);
    sim.position -= 0.5;
    let actions = sim.run(12.0);
    assert_eq!(
        actions,
        vec![Action::SetSpeed(1.05), Action::SetSpeed(1.0)],
        "one nudge and back"
    );
    assert!(sim.drift(100.0).abs() <= 0.06, "{}", sim.drift(100.0));
}

#[test]
fn a_player_ahead_slows_down_then_returns_to_normal_speed() {
    let mut sim = Sim::playing(100.0);
    sim.position += 0.5;
    let actions = sim.run(12.0);
    assert_eq!(actions, vec![Action::SetSpeed(0.95), Action::SetSpeed(1.0)]);
    assert!(sim.drift(100.0).abs() <= 0.06);
}

#[test]
fn large_drift_is_one_exact_seek() {
    let mut sim = Sim::playing(100.0);
    sim.seek_time = 0.3;
    // A player that cannot keep up falls behind faster than a nudge catches up.
    sim.lag = 0.6;
    let mut actions = sim.run(5.0);
    sim.lag = 0.0;
    actions.extend(sim.run(3.0));
    assert_eq!(
        count(&actions, |a| matches!(a, Action::SeekTo(_))),
        1,
        "{actions:?}"
    );
    let Some(&(when, Action::SeekTo(to))) =
        sim.log.iter().find(|(_, a)| matches!(a, Action::SeekTo(_)))
    else {
        unreachable!()
    };
    assert!(
        (to - (100.0 + when)).abs() < 0.1,
        "seeks to where the group is: {to} at {when}"
    );
    assert_eq!(
        count(&actions, |a| matches!(a, Action::Request(_))),
        0,
        "no echo"
    );
    // What it lost after the seek, the nudge makes up at 5%.
    sim.run(20.0);
    assert!(sim.drift(100.0).abs() <= 0.06);
    assert!(near(sim.speed, 1.0));
}

#[test]
fn a_stall_is_reported_once_and_its_recovery_once() {
    let mut sim = Sim::playing(100.0);
    sim.starved = true;
    let short = sim.run(0.4);
    assert!(short.is_empty(), "a hiccup is not reported: {short:?}");
    let long = sim.run(3.0);
    assert_eq!(count(&long, |a| matches!(a, Action::Buffering { .. })), 1);
    sim.starved = false;
    let at = sim.position;
    let back = sim.run(1.0);
    // The group waits for this player and starts the others when it reaches
    // them: it plays on from where it is, with no seek to catch up.
    assert_eq!(back, vec![Action::Ready { position: at }]);
    let on = sim.run(3.0);
    assert!(on.is_empty(), "{on:?}");
}

#[test]
fn a_stall_the_group_pauses_for_still_reports_its_recovery() {
    let mut sim = Sim::playing(100.0);
    sim.starved = true;
    sim.run(1.0);
    let at = sim.position;
    sim.follower.command(Command::PauseAt { position: at });
    let waiting = sim.run(2.0);
    assert_eq!(waiting, vec![Action::SetPaused(true)]);
    sim.starved = false;
    let back = sim.run(1.0);
    assert_eq!(back, vec![Action::Ready { position: at }]);
}

#[test]
fn a_group_seek_pauses_seeks_and_says_ready_once_it_lands() {
    let mut sim = Sim::playing(100.0);
    sim.seek_time = 0.3;
    sim.follower.command(Command::SeekTo { position: 500.0 });
    let first = sim.run(TICK);
    assert_eq!(first, vec![Action::SetPaused(true), Action::SeekTo(500.0)]);
    let landing = sim.run(0.2);
    assert!(landing.is_empty(), "not ready mid-seek: {landing:?}");
    let ready = sim.run(1.0);
    assert_eq!(ready, vec![Action::Ready { position: 500.0 }]);
    // The group plays from there.
    sim.follower.command(Command::PlayAt {
        at: GroupTime(sim.now + 0.5),
        position: 500.0,
    });
    let play = sim.run(2.0);
    assert_eq!(play, vec![Action::SetPaused(false)]);
}

#[test]
fn a_group_seek_that_lands_off_the_spot_is_sent_again_before_ready() {
    // A player that has just opened reads its start position at once, then
    // turns out to be elsewhere once the file is really playing.
    let mut sim = Sim::new(1.0);
    sim.position = 13.5;
    sim.follower.command(Command::SeekTo { position: 13.5 });
    sim.run(TICK);
    sim.position = 13.93;
    let settle = sim.run(1.0);
    assert_eq!(
        settle,
        vec![Action::SeekTo(13.5), Action::Ready { position: 13.5 }]
    );
}

#[test]
fn a_second_seek_that_lands_near_enough_says_ready() {
    let mut sim = Sim::new(1.0);
    sim.follower.command(Command::SeekTo { position: 13.5 });
    sim.run(TICK);
    sim.position = 13.8;
    sim.run(TICK);
    sim.position = 13.8;
    let ready = sim.run(1.0);
    assert_eq!(ready, vec![Action::Ready { position: 13.8 }]);
}

#[test]
fn the_followers_own_changes_are_never_requests() {
    let mut sim = Sim::playing(100.0);
    sim.seek_time = 0.2;
    sim.follower.command(Command::PauseAt { position: 140.0 });
    sim.run(1.0);
    sim.follower.command(Command::SeekTo { position: 20.0 });
    sim.run(1.0);
    sim.follower.command(Command::PlayAt {
        at: GroupTime(sim.now),
        position: 20.0,
    });
    sim.run(2.0);
    let all: Vec<Action> = sim.log.iter().map(|(_, a)| *a).collect();
    assert_eq!(
        count(&all, |a| matches!(a, Action::Request(_))),
        0,
        "{all:?}"
    );
}

#[test]
fn a_pause_in_the_player_itself_is_asked_of_the_group_and_left_alone() {
    let mut sim = Sim::playing(100.0);
    sim.paused = true;
    let actions = sim.run(2.0);
    assert_eq!(actions, vec![Action::Request(Request::Pause)]);
    assert!(sim.paused, "not played again while the group answers");
    sim.follower.command(Command::PauseAt {
        position: sim.position,
    });
    assert!(sim.run(1.0).is_empty(), "already where the group is");
}

#[test]
fn a_seek_in_the_player_itself_is_asked_of_the_group() {
    let mut sim = Sim::playing(100.0);
    sim.position = 300.0;
    let actions = sim.run(2.0);
    assert_eq!(actions.len(), 1, "{actions:?}");
    assert!(matches!(
        actions[0],
        Action::Request(Request::Seek { position }) if near(position, 300.0)
    ));
}

#[test]
fn the_users_speed_waits_while_in_a_group_and_comes_back_on_leaving() {
    let mut sim = Sim::new(1.5);
    sim.position = 10.0;
    let joined = sim.run(TICK);
    assert_eq!(joined, vec![Action::SetSpeed(1.0)]);
    sim.follower.command(Command::PlayAt {
        at: GroupTime(sim.now),
        position: 10.0,
    });
    sim.run(1.0);
    sim.position -= 0.5;
    sim.run(1.0);
    assert!(near(sim.speed, 1.05), "nudging when the group is left");
    assert_eq!(sim.follower.leave(), vec![Action::SetSpeed(1.5)]);
}

#[test]
fn an_unanswered_request_gives_the_player_back_to_the_group() {
    let mut sim = Sim::playing(100.0);
    sim.paused = true;
    assert_eq!(sim.run(1.0), vec![Action::Request(Request::Pause)]);
    let later = sim.run(4.0);
    assert!(later.contains(&Action::SetPaused(false)), "{later:?}");
    sim.run(2.0);
    assert!(!sim.paused);
    assert!(sim.drift(100.0).abs() < 0.1, "{}", sim.drift(100.0));
}

#[test]
fn the_pause_at_the_end_of_an_item_is_not_asked_of_the_group() {
    let mut sim = Sim::playing(100.0);
    sim.paused = true;
    sim.ended = true;
    let held = sim.run(10.0);
    assert!(held.is_empty(), "{held:?}");
    // A seek back by the group plays on from there.
    sim.follower.command(Command::SeekTo { position: 20.0 });
    sim.ended = false;
    let back = sim.run(1.0);
    assert!(back.contains(&Action::SeekTo(20.0)), "{back:?}");
    assert!(back.contains(&Action::Ready { position: 20.0 }), "{back:?}");
}

#[test]
fn a_group_seek_to_the_end_still_says_ready() {
    let mut sim = Sim::playing(10.0);
    sim.follower.command(Command::SeekTo { position: 65.0 });
    sim.step();
    sim.ended = true;
    let actions = sim.run(1.0);
    assert!(
        actions.contains(&Action::Ready { position: 65.0 }),
        "{actions:?}"
    );
}

#[test]
fn a_speed_changed_in_the_player_itself_is_put_back() {
    let mut sim = Sim::playing(10.0);
    sim.speed = 2.0;
    assert_eq!(sim.run(TICK), vec![Action::SetSpeed(1.0)]);
    assert!(near(sim.speed, 1.0));
}

#[test]
fn nothing_is_done_to_the_speed_of_a_user_at_normal_speed() {
    let sim = Sim::playing(10.0);
    assert!(sim.follower.leave().is_empty());
}

#[test]
fn stop_is_passed_on_once() {
    let mut sim = Sim::playing(10.0);
    sim.follower.command(Command::Stop);
    assert_eq!(sim.run(1.0), vec![Action::Stop]);
}

#[test]
fn nothing_happens_until_the_item_has_loaded() {
    let mut follower = Follower::new(1.0);
    follower.command(Command::SeekTo { position: 50.0 });
    let loading = Reading {
        position: 0.0,
        paused: false,
        stalled: true,
        loaded: false,
        speed: 1.0,
        ended: false,
    };
    assert!(follower.tick(loading, GroupTime(0.0)).is_empty());
    let loaded = Reading {
        loaded: true,
        stalled: false,
        ..loading
    };
    assert_eq!(
        follower.tick(loaded, GroupTime(1.0)),
        vec![Action::SetPaused(true), Action::SeekTo(50.0)]
    );
}

fn getter(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let map: HashMap<String, String> = pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    move |name: &str| map.get(name).cloned()
}

#[test]
fn the_player_reads_stalled_from_the_cache_and_from_seeking() {
    let read = |pairs: &[(&str, &str)]| {
        let mut snapshot = Snapshot::default();
        snapshot.refresh_from(&getter(pairs), false);
        Reading::from(&snapshot)
    };
    let cache = read(&[("time-pos", "12.5"), ("paused-for-cache", "yes")]);
    assert!(cache.stalled && cache.loaded && near(cache.position, 12.5));
    assert!(read(&[("time-pos", "3"), ("pause", "yes"), ("seeking", "yes")]).stalled);
    // Paused and idle is held, not stalled.
    let held = read(&[("time-pos", "3"), ("pause", "yes"), ("core-idle", "yes")]);
    assert!(held.paused && !held.stalled);
    assert!(!read(&[]).loaded);
}

#[test]
fn the_clock_prefers_the_sample_with_the_shortest_round_trip() {
    let mut clock = GroupClock::default();
    assert_eq!(clock.group_time(5.0), None);
    for (offset, round_trip) in [(10.4, 0.2), (10.1, 0.02), (9.7, 0.3)] {
        clock.add(Sample { offset, round_trip });
    }
    assert_eq!(clock.offset(), Some(10.1));
    assert_eq!(clock.round_trip(), Some(0.02));
    assert!(clock.group_time(5.0).is_some_and(|t| near(t.0, 15.1)));
}

#[test]
fn old_clock_samples_age_out() {
    let mut clock = GroupClock::default();
    clock.add(Sample {
        offset: 1.0,
        round_trip: 0.001,
    });
    for _ in 0..8 {
        clock.add(Sample {
            offset: 2.0,
            round_trip: 0.05,
        });
    }
    assert_eq!(clock.offset(), Some(2.0), "the oldest best sample has gone");
    clock.add(Sample {
        offset: f64::NAN,
        round_trip: 0.0,
    });
    clock.add(Sample {
        offset: 3.0,
        round_trip: -0.001,
    });
    assert_eq!(clock.offset(), Some(3.0), "a hair below zero still counts");
    assert_eq!(clock.round_trip(), Some(0.0));
}
