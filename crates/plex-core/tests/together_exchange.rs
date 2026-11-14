//! The state exchange replayed against a recorded web app session: given the
//! same states and the same player readings, [`Exchange`] answers what Plex's
//! own web app answered, and says what to follow.
//!
//! The recording covers a local play, a local pause and play, two local
//! seeks, the other participant's pauses, plays and seek, and the server's
//! `ignoringOnTheFly.server` states after each.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use plex_core::together::exchange::{Answer, Exchange, Reading};
use plex_core::together::message::{Message, State};
use serde_json::Value;

const SESSION: &str = include_str!("fixtures/together/web_session.jsonl");
const WEB_DEVICE: &str = "web-device-0001";
const OTHER_DEVICE: &str = "other-device-0001";

/// One server state and the web app's answer to it.
struct Step {
    state: State,
    reply: Value,
}

fn steps() -> Vec<Step> {
    let mut states = Vec::new();
    let mut replies = Vec::new();
    for line in SESSION.lines() {
        let record: Value = serde_json::from_str(line).unwrap();
        let text = record["text"].as_str().unwrap();
        match record["dir"].as_str().unwrap() {
            "in" => {
                if let Message::State(state) = Message::parse(text) {
                    states.push(state);
                }
            }
            _ => {
                let value: Value = serde_json::from_str(text).unwrap();
                if let Some(reply) = value.get("State") {
                    replies.push(reply.clone());
                }
            }
        }
    }
    assert_eq!(states.len(), replies.len(), "every state is answered once");
    states
        .into_iter()
        .zip(replies)
        .map(|(state, reply)| Step { state, reply })
        .collect()
}

fn counter(value: &Value, side: &str) -> Option<u32> {
    value["ignoringOnTheFly"][side]
        .as_u64()
        .filter(|n| *n > 0)
        .map(|n| u32::try_from(n).unwrap())
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

/// Replay the session as the web app, returning each step's answer.
fn replay() -> Vec<(Step, Answer)> {
    let mut exchange = Exchange::new(WEB_DEVICE);
    let mut reading = Reading {
        position: 0.0,
        paused: true,
        play_pause_pending: false,
        seek_pending: false,
    };
    let mut out = Vec::new();
    for step in steps() {
        let echo = step
            .state
            .ignoring_on_the_fly
            .and_then(|c| c.server)
            .unwrap_or(0)
            > 0;
        let played = &step.reply["playstate"];
        let change = counter(&step.reply, "client").is_some();
        let seek = played["doSeek"].as_bool().unwrap();
        // The web app's own answer says what its player read, except on an
        // echo, where it repeats the server.
        reading = if echo {
            Reading {
                play_pause_pending: false,
                seek_pending: false,
                ..reading
            }
        } else {
            Reading {
                position: played["position"].as_f64().unwrap(),
                paused: played["paused"].as_bool().unwrap(),
                play_pause_pending: change && !seek,
                seek_pending: change && seek,
            }
        };
        // The web app measures the round trip when a state arrives and
        // stamps its answer a moment later; replay at the arrival.
        let now = step.state.ping.client_latency_calculation.map_or_else(
            || {
                step.reply["ping"]["clientLatencyCalculation"]
                    .as_f64()
                    .unwrap()
            },
            |sent| sent + step.reply["ping"]["clientRtt"].as_f64().unwrap(),
        );
        let answer = exchange.on_state(&step.state, reading, now);
        out.push((step, answer));
    }
    out
}

fn set_by(step: &Step) -> String {
    let by = step
        .state
        .playstate
        .as_ref()
        .unwrap()
        .set_by
        .as_deref()
        .unwrap();
    plex_core::together::message::UserName::parse(by)
        .unwrap()
        .device_identifier
}

#[test]
fn every_answer_matches_the_web_apps() {
    let replayed = replay();
    assert_eq!(replayed.len(), 137);
    for (index, (step, answer)) in replayed.iter().enumerate() {
        let want = &step.reply;
        let got = serde_json::to_value(&answer.reply).unwrap();
        let context = format!("step {index}: got {got}, want {want}");
        assert!(
            close(
                got["playstate"]["position"].as_f64().unwrap(),
                want["playstate"]["position"].as_f64().unwrap()
            ),
            "{context}"
        );
        for field in ["paused", "doSeek", "setBy"] {
            assert_eq!(
                got["playstate"][field], want["playstate"][field],
                "{context}"
            );
        }
        for side in ["client", "server"] {
            assert_eq!(counter(&got, side), counter(want, side), "{context}");
        }
        let stamped = got["ping"]["clientLatencyCalculation"].as_f64().unwrap();
        let want_stamp = want["ping"]["clientLatencyCalculation"].as_f64().unwrap();
        assert!((stamped - want_stamp).abs() < 0.005, "{context}");
        for field in ["clientRtt", "serverRtt", "latencyCalculation"] {
            assert!(
                close(
                    got["ping"][field].as_f64().unwrap(),
                    want["ping"][field].as_f64().unwrap()
                ),
                "{field} {context}"
            );
        }
    }
}

#[test]
fn a_local_play_is_sent_once_and_its_return_is_not_followed() {
    let replayed = replay();
    let at = replayed
        .iter()
        .position(|(step, _)| {
            counter(&step.reply, "client").is_some()
                && step.reply["playstate"]["paused"] == false
                && step.reply["playstate"]["doSeek"] == false
        })
        .unwrap();
    let (_, sent) = &replayed[at];
    assert!(sent.sent_change);
    assert!(sent.follow.is_none(), "a pending change is not overridden");

    // The next state is the server imposing that very change: echoed, and
    // not followed because it is this device's own.
    let (step, back) = &replayed[at + 1];
    assert_eq!(
        counter(&serde_json::to_value(&back.reply).unwrap(), "server"),
        Some(1)
    );
    assert_eq!(set_by(step), WEB_DEVICE);
    assert!(!back.sent_change);
    assert!(back.follow.is_none());
}

#[test]
fn a_local_seek_is_sent_as_a_jump() {
    let replayed = replay();
    let seeks: Vec<&Answer> = replayed
        .iter()
        .filter(|(step, _)| {
            counter(&step.reply, "client").is_some() && step.reply["playstate"]["doSeek"] == true
        })
        .map(|(_, answer)| answer)
        .collect();
    assert_eq!(seeks.len(), 2);
    for answer in seeks {
        assert!(answer.sent_change);
        assert!(answer.reply.playstate.do_seek);
        assert_eq!(answer.reply.ignoring_on_the_fly.unwrap().client, Some(1));
    }
}

#[test]
fn a_remote_change_is_followed_and_echoed() {
    let replayed = replay();
    let remote: Vec<&(Step, Answer)> = replayed
        .iter()
        .filter(|(step, _)| {
            step.state
                .ignoring_on_the_fly
                .and_then(|c| c.server)
                .is_some()
                && set_by(step) == OTHER_DEVICE
        })
        .collect();
    assert!(remote.len() >= 3, "{} remote changes", remote.len());
    for (step, answer) in remote {
        let group = step.state.playstate.as_ref().unwrap();
        let target = answer.follow.as_ref().expect("a remote change is followed");
        assert_eq!(target.paused, group.paused);
        assert_eq!(target.do_seek, group.do_seek);
        assert_eq!(
            target.set_by.as_ref().unwrap().device_identifier,
            OTHER_DEVICE
        );
        let ahead = if group.paused {
            0.0
        } else {
            step.reply["ping"]["clientRtt"].as_f64().unwrap() / 2.0
        };
        assert!(
            close(target.position, group.position + ahead),
            "{} {} {}",
            target.position,
            group.position,
            ahead
        );
        // The echo repeats the server's state, not the player's.
        assert!(close(answer.reply.playstate.position, group.position));
        assert_eq!(answer.reply.playstate.paused, group.paused);
        assert!(!answer.reply.playstate.do_seek);
    }
    // The remote seek is among them, and is a jump.
    assert!(replayed
        .iter()
        .any(|(_, a)| a.follow.as_ref().is_some_and(|t| t.do_seek)));
}

#[test]
fn a_change_pending_during_an_echo_waits_for_the_next_state() {
    let steps = steps();
    let echo = steps
        .iter()
        .find(|s| s.state.ignoring_on_the_fly.and_then(|c| c.server).is_some())
        .unwrap();
    let mut exchange = Exchange::new(WEB_DEVICE);
    let reading = Reading {
        position: 5.0,
        paused: true,
        play_pause_pending: true,
        seek_pending: false,
    };
    let answer = exchange.on_state(&echo.state, reading, 100.0);
    assert!(!answer.sent_change, "the echo carries the server's state");
    assert!(answer.follow.is_none(), "nor is the group followed over it");

    let quiet = steps
        .iter()
        .find(|s| s.state.ignoring_on_the_fly.is_none())
        .unwrap();
    let answer = exchange.on_state(&quiet.state, reading, 101.0);
    assert!(answer.sent_change);
    assert_eq!(answer.reply.ignoring_on_the_fly.unwrap().client, Some(1));
    assert!(answer.reply.playstate.paused);
    assert!(close(answer.reply.playstate.position, 5.0));
}

#[test]
fn a_state_with_no_ping_or_playstate_still_gets_an_answer() {
    let mut exchange = Exchange::new(WEB_DEVICE);
    let reading = Reading {
        position: 3.0,
        paused: false,
        play_pause_pending: false,
        seek_pending: false,
    };
    let answer = exchange.on_state(&State::default(), reading, 7.0);
    assert!(answer.follow.is_none());
    assert!(close(answer.reply.playstate.position, 3.0));
    assert!(close(answer.reply.ping.client_latency_calculation, 7.0));
    assert!(answer.reply.ignoring_on_the_fly.is_none());
    assert!(close(exchange.round_trip(), 0.0));
}
