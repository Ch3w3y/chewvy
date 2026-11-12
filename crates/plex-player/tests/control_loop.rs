//! Acceptance tests for the async control loop (PLAN M4 acceptance_checks[1]).
//!
//! Everything here runs against `plex_player::mock::MockLink`, which **records**
//! the exact libmpv call it would have made. That proves the wrapper's call
//! shape, token correlation and delivery rules. It is *not* libmpv evidence:
//! which command/property names a real libmpv 2.5.0 actually accepted can only
//! come from an observed display-free probe run (`--smoke-no-display` writes
//! the `--probe-json` report when one is actually captured).
//!
//! Mock call strings are wrapper facts, not header facts. The libmpv entry
//! points named here (`mpv_command_async`, `mpv_set_property_async`) *are*
//! header symbols and are read from the committed bindings.

// Test-only allowance, the same documented convention every other test target in
// this workspace uses (see crates/plex-player/tests/native_contract.rs:7,
// crates/plex-core/tests/*.rs). A failing assertion in a test target is allowed
// to panic loudly; product code (`src/`) keeps `unwrap_used`/`expect_used` on.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use plex_player::commands::{lookup_property, props};
use plex_player::engine::ControlGate;
use plex_player::mock::{
    end_file_event, file_loaded_event, lifecycle_event, other_event, playback_restart_event,
    property_event, queue_overflow_event, reply_event, set_property_reply_event, start_file_event,
    MOCK_VERSION,
};
use plex_player::probe::ControlPhase;
use plex_player::{
    error_name, is_success, load_request, mock_error_code, mock_success_code, observed_format,
    pause_request, seek_request, stop_request, volume_request, EndReason, EventKind, ItemSource,
    MockLink, ObservedState, PlayerError, PlayerEvent, PlayerLink, PlayerSession, PumpReport,
    Request, SeekMode, TokenAllocator, TrackSelection, ValueKind, MAX_TRACKED_EVENTS,
    MAX_TRACKED_PUMP_REPORTS, MAX_VOLUME, NO_TOKEN,
};

/// A session already opened on a mock link.
fn session(uri: &str) -> PlayerSession<MockLink> {
    PlayerSession::open(MockLink::new(), uri).expect("mock session opens")
}

/// The reply token for the `loadfile` issued by `open`.
///
/// `open` observes `props::ALL` first (one token per property, none of them
/// tracked as outstanding) and only then issues `loadfile`, which is the only
/// request left outstanding.
fn load_token(session: &PlayerSession<MockLink>) -> u64 {
    *session
        .pending_tokens()
        .last()
        .expect("open issues loadfile")
}

fn code(name: &str) -> i32 {
    mock_error_code(name).unwrap_or_else(|| panic!("headers do not declare {name}"))
}

fn prop(name: &str, value: plex_player::PropertyValue) -> plex_player::RawEvent {
    property_event(name, value)
}

// ---------------------------------------------------------------------------
// 1. command / reply correlation
// ---------------------------------------------------------------------------

#[test]
fn tokens_start_at_one_and_are_never_reused() {
    let mut tokens = TokenAllocator::new();
    assert_eq!(NO_TOKEN, 0, "libmpv reserves 0 as 'no reply_userdata'");
    assert_eq!(tokens.alloc(), 1);
    assert_eq!(tokens.alloc(), 2);
    assert_eq!(tokens.alloc(), 3);
    assert_eq!(tokens.issued(), 3);
    assert_eq!(tokens.next_token(), 4);
}

#[test]
fn open_issues_one_token_per_observation_then_the_loadfile_token() {
    let s = session("/media/fixture.mkv");
    // Only loadfile is outstanding; observation tokens are issued but never
    // carry a command reply.
    let tokens = s.pending_tokens();
    assert_eq!(tokens.len(), 1, "exactly one request awaits a reply");
    assert!(!tokens.contains(&NO_TOKEN));
    assert_eq!(tokens[0], load_token(&s));
    let calls = s.link().calls();
    assert_eq!(
        calls.iter().filter(|c| c.starts_with("observe ")).count(),
        props::ALL.len(),
        "every observed property is registered: {calls:?}"
    );
    assert!(
        calls
            .last()
            .is_some_and(|c| c.starts_with("command token=")),
        "loadfile must be the last thing open does: {calls:?}"
    );
}

#[test]
fn a_reply_is_attributed_to_the_request_that_issued_its_token_not_to_the_newest_command() {
    let mut s = session("/media/fixture.mkv");
    let load = load_token(&s);
    let pause_token = s.set_pause(true).expect("pause issue");
    let seek_token = s.seek(30.0, SeekMode::Relative).expect("seek issue");
    assert_ne!(pause_token, seek_token);
    assert_ne!(load, pause_token);

    // The seek reply arrives first (libmpv replies in completion order) while
    // the loadfile reply has not arrived at all.
    s.link_mut()
        .push_event(reply_event(seek_token, mock_success_code()));
    let event = s
        .pump_until_reply(seek_token, 8)
        .expect("seek reply correlated");
    match &event {
        PlayerEvent::CommandReply {
            token,
            command,
            ok,
            error,
        } => {
            assert_eq!(*token, seek_token);
            assert!(*ok);
            assert!(error.is_none(), "a success reply carries no error");
            assert!(
                command.contains("seek"),
                "label must come from the correlated request: {command}"
            );
        }
        other => panic!("expected a correlated command reply, got {other:?}"),
    }
    // Correlation must not guess anything away.
    assert!(s.pending_tokens().contains(&pause_token));
    assert!(s.pending_tokens().contains(&load));
    assert!(!s.pending_tokens().contains(&seek_token));
    assert!(s.unmatched_replies().is_empty());

    // Now the *older* loadfile reply arrives: it is attributed to the load,
    // not to the most recently issued command.
    s.link_mut()
        .push_event(reply_event(load, mock_success_code()));
    let event = s.pump_until_reply(load, 8).expect("load reply correlated");
    match &event {
        PlayerEvent::CommandReply { token, command, .. } => {
            assert_eq!(*token, load);
            assert!(
                command.starts_with("loadfile"),
                "the load reply keeps the load label: {command}"
            );
        }
        other => panic!("expected the loadfile reply, got {other:?}"),
    }
    assert!(s.pending_tokens().contains(&pause_token));
}

#[test]
fn a_reply_for_a_stale_token_is_reported_uncorrelated_instead_of_being_guessed() {
    let mut s = session("/media/fixture.mkv");
    let token = s.set_volume(50.0).expect("volume issue");
    // A reply for a token this session never issued.
    s.link_mut()
        .push_event(reply_event(9_999, mock_success_code()));
    s.link_mut()
        .push_event(reply_event(token, mock_success_code()));
    let report = s.pump(16);
    assert_eq!(report.drained, 2);
    let events = s.events();
    let uncorrelated: Vec<&PlayerEvent> = events
        .iter()
        .filter(|e| matches!(e, PlayerEvent::Uncorrelated { .. }))
        .collect();
    assert_eq!(
        uncorrelated.len(),
        1,
        "exactly one reply may be unattributed: {events:?}"
    );
    match uncorrelated[0] {
        PlayerEvent::Uncorrelated { token: got, reason } => {
            assert_eq!(*got, 9_999);
            assert!(
                reason.contains("no outstanding request"),
                "the reason must say why it was not attributed: {reason}"
            );
        }
        other => unreachable!("filtered to Uncorrelated, got {other:?}"),
    }
    let correlated: Vec<&PlayerEvent> = events
        .iter()
        .filter(|e| {
            matches!(e, PlayerEvent::CommandReply { token: got, ok, .. }
                if *got == token && *ok)
        })
        .collect();
    assert_eq!(
        correlated.len(),
        1,
        "the real reply must still correlate: {events:?}"
    );
    assert_eq!(
        s.unmatched_replies(),
        &[(9_999, plex_player::UNMATCHED_NO_REQUEST)],
        "the miss is recorded with a reason, not dropped"
    );
    // The correlated request is no longer outstanding; the stale one never was.
    assert!(!s.pending_tokens().contains(&token));
}

#[test]
fn a_reply_that_carries_no_token_is_recorded_as_unmatched() {
    let mut s = session("/media/fixture.mkv");
    // A real libmpv non-reply event carries reply_userdata == 0; if one ever
    // arrives as COMMAND_REPLY it must never be attributed to a request.
    s.link_mut()
        .push_event(reply_event(NO_TOKEN, mock_success_code()));
    let report = s.pump(8);
    assert_eq!(report.drained, 1);
    assert!(s
        .events()
        .iter()
        .any(|e| matches!(e, PlayerEvent::Uncorrelated { token: 0, .. })));
    assert_eq!(
        s.unmatched_replies(),
        &[(NO_TOKEN, plex_player::UNMATCHED_NO_TOKEN)],
        "both report sites use the same reason label"
    );
    assert!(s.events().iter().any(
        |e| matches!(e, PlayerEvent::Uncorrelated { token: 0, reason } if
            *reason == plex_player::UNMATCHED_NO_TOKEN)
    ));
    // Nothing outstanding was consumed.
    assert!(s.pending_tokens().contains(&load_token(&s)));
}

#[test]
fn a_reply_with_a_libmpv_error_becomes_a_command_failure_naming_the_async_call() {
    let mut s = session("/media/fixture.mkv");
    let token = s.seek(1e9, SeekMode::Absolute).expect("seek issue");
    s.link_mut()
        .push_event(reply_event(token, code("MPV_ERROR_COMMAND")));
    let err = s
        .pump_until_reply(token, 8)
        .expect_err("a rejected command is an error");
    match &err {
        PlayerError::Command {
            call,
            command,
            code: got,
            name,
        } => {
            // A command reply reports the async entry point the request was
            // issued through (Request::call_name), recorded at issue time.
            assert_eq!(call, "mpv_command_async");
            assert!(command.contains("seek"));
            assert_eq!(*got, code("MPV_ERROR_COMMAND"));
            assert_eq!(name, "MPV_ERROR_COMMAND");
        }
        other => panic!("expected a command failure, got {other:?}"),
    }
    assert!(!s.pending_tokens().contains(&token));

    // A failed set-property reply reports the other async entry point.
    let volume_token = s.set_volume(20.0).expect("volume issue");
    s.link_mut()
        .push_event(reply_event(volume_token, code("MPV_ERROR_PROPERTY_FORMAT")));
    let err = s
        .pump_until_reply(volume_token, 8)
        .expect_err("set volume failed");
    match &err {
        PlayerError::Command { call, command, .. } => {
            assert_eq!(call, "mpv_set_property_async");
            assert!(command.contains("volume"));
        }
        other => panic!("expected a command failure, got {other:?}"),
    }
}

#[test]
fn a_command_that_is_never_replied_for_is_an_error_not_an_infinite_wait() {
    let mut s = session("/media/fixture.mkv");
    let token = s.set_pause(true).expect("pause issue");
    // Three scripted non-reply events so the drain *budget* is what ends the
    // wait: `drains` counts raw events actually taken from the link.
    s.link_mut().push_event(other_event(50));
    s.link_mut()
        .push_event(prop("time-pos", plex_player::PropertyValue::Double(1.0)));
    s.link_mut().push_event(other_event(51));
    let start = Instant::now();
    let err = s
        .pump_until_reply(token, 3)
        .expect_err("no reply was scripted");
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "pump must not wait"
    );
    match &err {
        PlayerError::NoReply {
            command,
            token: got,
            drains,
        } => {
            assert_eq!(*got, token);
            assert!(command.contains("pause"));
            assert_eq!(*drains, 3, "the bounded drain is what ended the wait");
        }
        other => panic!("expected NoReply, got {other:?}"),
    }
    // An empty queue ends the drain immediately: no spin, no wait.
    let err = s
        .pump_until_reply(token, 8)
        .expect_err("still no reply, and nothing queued");
    assert!(
        matches!(err, PlayerError::NoReply { drains: 0, .. }),
        "an empty queue must not be spun against the budget: {err:?}"
    );
    // The request is still outstanding: an unacked command is not a lost token.
    assert!(s.pending_tokens().contains(&token));

    // A request the link refused never became outstanding.
    let mut s2 = session("/media/fixture.mkv");
    s2.link_mut().reject_next(code("MPV_ERROR_COMMAND"));
    let err = s2.set_volume(10.0).expect_err("link rejected the request");
    match &err {
        PlayerError::Rejected {
            call,
            command,
            code: got,
            ..
        } => {
            assert_eq!(call, "mpv_set_property_async");
            assert!(command.contains("volume"));
            assert_eq!(*got, code("MPV_ERROR_COMMAND"));
        }
        other => panic!("expected a rejection, got {other:?}"),
    }
    let load = load_token(&s2);
    assert_eq!(
        s2.pending_tokens().iter().filter(|t| **t != load).count(),
        0,
        "a rejected issue must not stay pending"
    );
}

// ---------------------------------------------------------------------------
// 2. property-change delivery
// ---------------------------------------------------------------------------

#[test]
fn observed_properties_reach_state_only_through_delivered_events() {
    let mut s = session("/media/fixture.mkv");
    // Nothing observed yet: state is unknown, not guessed.
    let state = s.state();
    assert_eq!(state.time_pos, None);
    assert_eq!(state.paused, None);
    assert!(state.playing());
    assert!(state.observed_counts.is_empty());

    for (name, value) in [
        ("time-pos", plex_player::PropertyValue::Double(12.5)),
        ("duration", plex_player::PropertyValue::Double(250.0)),
        ("pause", plex_player::PropertyValue::Flag(true)),
        // Observed as *idle*: this is what makes `playing()` false. The mock
        // never assumes it - without this observation the state is unknown and
        // `playing()` is true (nothing observed as ended), asserted above.
        ("idle-active", plex_player::PropertyValue::Flag(true)),
        (
            "media-title",
            plex_player::PropertyValue::Str("Episode".to_owned()),
        ),
    ] {
        s.link_mut().push_event(prop(name, value));
    }
    let report = s.pump(32);
    assert_eq!(report.drained, 5);

    let state = s.state();
    assert_eq!(state.time_pos, Some(12.5));
    assert_eq!(state.duration, Some(250.0));
    assert_eq!(state.paused, Some(true));
    assert_eq!(state.idle_active, Some(true));
    assert_eq!(state.media_title.as_deref(), Some("Episode"));
    assert!(
        !state.playing(),
        "idle-active=true is observed, not assumed"
    );
    assert_eq!(state.observed_counts.len(), 5);
    assert!(state.unrecognized.is_empty());
    // The events the UI sees are the same observations.
    let delivered = s
        .events()
        .iter()
        .filter(|e| matches!(e, PlayerEvent::PropertyChange { .. }))
        .count();
    assert_eq!(delivered, 5);
    assert!(s.events().iter().any(
        |e| matches!(e, PlayerEvent::PropertyChange { observed } if observed.name == "time-pos")
    ));

    // The other direction is observed too: a later `idle-active=false`
    // observation replaces the earlier one, and `playing()` follows the newest
    // observation rather than a label or a default.
    s.link_mut()
        .push_event(prop("idle-active", plex_player::PropertyValue::Flag(false)));
    s.pump(4);
    assert_eq!(s.state().idle_active, Some(false));
    assert!(s.state().playing(), "the newest observation decides");
    assert_eq!(
        s.state().observed_counts.len(),
        5,
        "the same five names, one of them re-observed"
    );
}

#[test]
fn a_property_that_is_unavailable_clears_the_field_instead_of_leaving_a_stale_value() {
    let mut s = session("/media/fixture.mkv");
    s.link_mut()
        .push_event(prop("time-pos", plex_player::PropertyValue::Double(4.0)));
    s.pump(4);
    assert_eq!(s.state().time_pos, Some(4.0));
    s.link_mut()
        .push_event(prop("time-pos", plex_player::PropertyValue::None));
    s.pump(4);
    assert_eq!(s.state().time_pos, None, "unavailable must not stay stale");
    // A cumulative observation count survives a cleared field: the name *was*
    // delivered to us twice.
    assert_eq!(s.state().observed_counts, vec![("time-pos".to_owned(), 2)]);
}

#[test]
fn a_property_name_that_is_not_in_the_vocabulary_is_reported_unrecognized() {
    let mut s = session("/media/fixture.mkv");
    s.link_mut().push_event(prop(
        "definitely-not-a-real-mpv-property",
        plex_player::PropertyValue::Str("x".to_owned()),
    ));
    s.pump(4);
    assert_eq!(
        s.state().unrecognized,
        vec!["definitely-not-a-real-mpv-property".to_owned()]
    );
    // It is still visible to the UI, as an observation with no backing: it is
    // never merged into a known field. `events::Observed::header_backed` is
    // `None` for a name that did not come from our own vocabulary at all (we
    // cannot claim libmpv does/does not have it); `Some(false)` is reserved for
    // a name **we** list as unverified. The honest label here is `None`.
    assert!(s
        .events()
        .iter()
        .any(|e| matches!(e, PlayerEvent::PropertyChange { observed } if
            observed.name == "definitely-not-a-real-mpv-property"
                && observed.header_backed.is_none())));
    assert_eq!(s.state().media_title, None);
}

#[test]
fn state_apply_is_the_only_way_to_fill_fields_and_it_refuses_a_format_mismatch() {
    let mut state = ObservedState::new();
    let ok = state.apply(&plex_player::Observed {
        name: "volume".to_owned(),
        value: plex_player::PropertyValue::Double(75.0),
        header_backed: None,
    });
    assert!(ok);
    assert_eq!(state.volume, Some(75.0));
    // Wrong format for a known name: field untouched, apply reports false.
    let ok = state.apply(&plex_player::Observed {
        name: "volume".to_owned(),
        value: plex_player::PropertyValue::Str("75".to_owned()),
        header_backed: None,
    });
    assert!(!ok);
    assert_eq!(state.volume, Some(75.0), "a mismatch may not coerce");
    assert_eq!(state.provenance().len(), props::ALL.len());
    assert!(state
        .provenance()
        .iter()
        .any(|p| p.starts_with("time-pos=header") && p.ends_with("observed=no")));
    assert!(state
        .provenance()
        .iter()
        .any(|p| p.starts_with("pause=unverified")));
    assert!(state
        .provenance()
        .iter()
        .any(|p| p.starts_with("volume=unverified") && p.ends_with("observed=yes")));
}

#[test]
fn header_backed_and_unverified_names_are_kept_apart_in_every_observation() {
    // A header-mentioned name and a documentation-only name are labelled from
    // their own provenance, independently of each other.
    let mut s = session("/media/fixture.mkv");
    s.link_mut()
        .push_event(prop("time-pos", plex_player::PropertyValue::Double(2.0)));
    s.link_mut()
        .push_event(prop("pause", plex_player::PropertyValue::Flag(false)));
    s.pump(4);
    let backed: Vec<(String, Option<bool>)> = s
        .events()
        .iter()
        .filter_map(|e| match e {
            PlayerEvent::PropertyChange { observed } => {
                Some((observed.name.clone(), observed.header_backed))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        backed,
        vec![
            ("time-pos".to_owned(), Some(true)),
            ("pause".to_owned(), Some(false)),
        ],
        "provenance travels with the observation"
    );
    // The vocabulary itself keeps the two classes apart: only names that appear
    // as literals in client.h are header-backed.
    let header: Vec<&str> = props::ALL
        .iter()
        .filter(|n| n.is_header_backed())
        .map(|n| n.name)
        .collect();
    assert_eq!(header, vec!["time-pos", "playback-time", "idle-active"]);
    assert!(!props::VOLUME.is_header_backed());
    assert!(!props::EOF_REASON.is_header_backed());
}

/// **The observation vocabulary is pinned** (M6b: player UI state may only come
/// from names the player actually reports, so the exact set a session subscribes
/// to is part of the contract, not an implementation detail).
///
/// `props::ALL` is what `PlayerSession::open` issues one `mpv_observe_property`
/// per entry for, so a name added or removed here changes every real session.
/// Asserts, row by row in declaration order:
///
/// * the exact name list, requested format and header-backed flag of `props::ALL`;
/// * every name has an explicit row here **and** `observed_format` agrees with it,
///   so no observed name can fall through `observed_format`'s catch-all arm;
/// * every name is resolvable through `lookup_property`, which is how delivered
///   values get their provenance label;
/// * `aid`/`sid` exist as `[U]` candidates and are **not** observed names.
#[test]
fn the_observed_vocabulary_is_pinned_and_every_name_has_a_declared_format() {
    // (name, format requested at registration, header-mentioned in client.h?)
    const PINNED: [(&str, ValueKind, bool); 13] = [
        ("time-pos", ValueKind::Double, true),
        ("playback-time", ValueKind::Double, true),
        ("idle-active", ValueKind::Flag, true),
        ("pause", ValueKind::Flag, false),
        ("eof-reached", ValueKind::Flag, false),
        ("eof-reason", ValueKind::String, false),
        ("duration", ValueKind::Double, false),
        ("volume", ValueKind::Double, false),
        ("audio-id", ValueKind::String, false),
        ("sub-id", ValueKind::String, false),
        ("track-list", ValueKind::None, false),
        ("paused-for-cache", ValueKind::Flag, false),
        ("media-title", ValueKind::String, false),
    ];
    let observed: Vec<(&str, ValueKind, bool)> = props::ALL
        .iter()
        .map(|n| (n.name, observed_format(*n), n.is_header_backed()))
        .collect();
    assert_eq!(
        observed,
        PINNED.to_vec(),
        "props::ALL is the set of names a real session subscribes to; each row is (name, requested format, header-backed)",
    );
    for (name, want_format, header_backed) in PINNED {
        let named = props::ALL
            .iter()
            .find(|n| n.name == name)
            .unwrap_or_else(|| panic!("{name} is pinned but is not in props::ALL"));
        assert_eq!(
            named.is_header_backed(),
            header_backed,
            "{name} provenance changed: {}",
            named.fact(),
        );
        assert_eq!(
            observed_format(*named),
            want_format,
            "{name} has no explicit observed_format row (it fell through the catch-all arm)",
        );
        assert!(
            lookup_property(name).is_some(),
            "{name} is observed but lookup_property cannot label values delivered for it",
        );
    }
    // The two `[U]` track-selection candidates stay candidates: present in the
    // vocabulary module, absent from the observation list, and never header-backed.
    for candidate in [props::AID, props::SID] {
        assert!(
            !candidate.is_header_backed(),
            "{} is not a header symbol",
            candidate.name
        );
        assert!(
            !props::ALL.iter().any(|n| n.name == candidate.name),
            "{} must stay out of props::ALL until a measured run issues it (see docs/CAPABILITIES.md)",
            candidate.name,
        );
    }
    assert!(
        !lookup_property("aid").is_some(),
        "aid is not an observed name"
    );
    assert_eq!(
        props::ALL.iter().filter(|n| n.is_header_backed()).count(),
        3,
        "exactly three names appear as literals in client.h (time-pos, playback-time, idle-active)",
    );
}

// ---------------------------------------------------------------------------
// 3. exact libmpv calls the wrapper makes (recorded by the mock)
// ---------------------------------------------------------------------------

#[test]
fn open_and_load_send_loadfile_with_the_validated_source_and_replace() {
    let s = PlayerSession::open(MockLink::new(), "file:///media/fixture.mkv").expect("opens");
    let calls = s.link().calls();
    // Observation registration happens before the load, in props::ALL order.
    assert_eq!(
        &calls[..props::ALL.len()],
        props::ALL
            .iter()
            .enumerate()
            .map(|(i, n)| format!(
                "observe {} {} token={}",
                n.name,
                plex_player::observed_format(*n),
                i + 1
            ))
            .collect::<Vec<_>>()
            .as_slice(),
        "observations are registered first, one token each"
    );
    let load = calls.last().expect("a call was made");
    // `replace` is an upstream-documented argument word, labelled unverified
    // (SourceLabel::Unverified) until a real probe run reports it; this test
    // pins the shape we *send*, it does not prove libmpv accepted it.
    assert_eq!(
        load,
        &format!(
            "command token={} loadfile /media/fixture.mkv replace",
            load_token(&s)
        )
    );
    assert!(matches!(
        s.item(),
        Some(ItemSource::File { path }) if path == "/media/fixture.mkv"
    ));
}

#[test]
fn pause_seek_and_volume_produce_the_documented_set_property_and_command_calls() {
    let mut s = session("https://invalid.example/stream.mkv");
    let base = s.link().calls().len();

    let pause_on = s.set_pause(true).expect("pause issue");
    let pause_off = s.set_pause(false).expect("resume issue");
    let seek = s.seek(30.0, SeekMode::Relative).expect("seek issue");
    let volume = s.set_volume(55.0).expect("volume issue");
    let stop = s.stop().expect("stop issue");

    assert_eq!(
        &s.link().calls()[base..],
        &[
            format!("set-property token={pause_on} pause flag true"),
            format!("set-property token={pause_off} pause flag false"),
            format!("command token={seek} seek 30 relative"),
            format!("set-property token={volume} volume double 55"),
            format!("command token={stop} stop"),
        ]
    );
    // Every control request is outstanding until its reply, so a UI can issue
    // several controls without waiting for libmpv. Observations are not awaited.
    assert_eq!(
        s.pending_tokens().len(),
        1 + 5,
        "loadfile plus the five control requests are all outstanding"
    );
}

// ---------------------------------------------------------------------------
// 4. EOF / unsupported-item / queue-overflow handling
// ---------------------------------------------------------------------------

#[test]
fn end_file_with_error_reason_becomes_an_item_failure_the_ui_can_read() {
    let mut s = session("/media/fixture.mkv");
    let err_code = code("MPV_ERROR_LOADING_FAILED");
    s.link_mut()
        .push_event(end_file_event(EndReason::Error, err_code));
    s.pump(8);
    match s.item_error() {
        Some(PlayerError::ItemFailed {
            item,
            reason,
            code,
            name,
        }) => {
            assert_eq!(item, "/media/fixture.mkv");
            assert_eq!(reason, "error");
            assert_eq!(*code, err_code);
            assert_eq!(name, "MPV_ERROR_LOADING_FAILED");
        }
        other => panic!("expected ItemFailed, got {other:?}"),
    }
    assert!(s.events().iter().any(|e| matches!(
        e,
        PlayerEvent::EndFile {
            reason: EndReason::Error,
            error: Some(_)
        }
    )));
}

#[test]
fn a_plain_eof_is_an_end_file_event_and_not_an_error() {
    let mut s = session("/media/fixture.mkv");
    s.link_mut()
        .push_event(end_file_event(EndReason::Eof, mock_success_code()));
    s.pump(8);
    assert!(s.events().iter().any(|e| matches!(
        e,
        PlayerEvent::EndFile {
            reason: EndReason::Eof,
            error: None
        }
    )));
    assert!(s.item_error().is_none(), "EOF is not a failure");
}

#[test]
fn an_end_file_reason_the_headers_do_not_declare_stays_unknown() {
    let raw = end_file_event(EndReason::Unknown { raw: 77 }, code("MPV_ERROR_GENERIC"));
    let mut s =
        PlayerSession::open(MockLink::with_script(vec![raw]), "/media/f.mkv").expect("opens");
    s.pump(4);
    assert!(s.events().iter().any(|e| matches!(
        e,
        PlayerEvent::EndFile {
            reason: EndReason::Unknown { raw: 77 },
            ..
        }
    )));
    assert!(
        s.item_error().is_none(),
        "only reason=error sets an item failure: {:?}",
        s.item_error()
    );
}

#[test]
fn an_unsupported_item_is_refused_before_anything_is_sent_to_the_link() {
    for raw in [
        "dvd://1",
        "cdda://",
        "",
        "   ",
        "plex-media://1234",
        "relative.mkv",
    ] {
        let err = match PlayerSession::open(MockLink::new(), raw) {
            Err(err) => err,
            Ok(_) => panic!("{raw:?} must be refused by this layer"),
        };
        match &err {
            PlayerError::UnsupportedItem { item, reason } => {
                let trimmed = raw.trim();
                assert_eq!(
                    item,
                    if trimmed.is_empty() {
                        "<empty>"
                    } else {
                        trimmed
                    }
                );
                assert!(!reason.is_empty(), "{raw:?} must explain why");
            }
            other => panic!("expected UnsupportedItem for {raw:?}, got {other:?}"),
        }
        // The refusal is our own validation, not a libmpv reply: nothing was
        // queued and no request was ever issued.
        assert!(err.code() == "unsupported_item");
        assert!(
            load_request(raw).is_err(),
            "the same rule guards load_request"
        );
    }
    assert!(matches!(
        PlayerSession::open(MockLink::new(), "dvd://1"),
        Err(PlayerError::UnsupportedItem { .. })
    ));
}

#[test]
fn seek_volume_and_track_inputs_are_validated_before_they_reach_the_link() {
    let mut s = session("/media/fixture.mkv");
    let base = s.link().calls().len();
    assert!(s.seek(f64::NAN, SeekMode::Absolute).is_err());
    assert!(s.seek(f64::INFINITY, SeekMode::Absolute).is_err());
    assert!(s.seek(-5.0, SeekMode::Absolute).is_err());
    assert!(s.seek(-5.0, SeekMode::AbsoluteExact).is_err());
    assert!(s.seek(-1.0, SeekMode::Absolute).is_err());
    assert!(s.set_volume(-1.0).is_err());
    assert!(s.set_volume(101.0).is_err());
    assert!(s.set_volume(f64::NAN).is_err());
    assert!(s.select_audio(&TrackSelection::Index(-1)).is_err());
    assert!(s
        .select_subtitle(&TrackSelection::Id("  ".to_owned()))
        .is_err());
    assert!(s
        .select_audio(&TrackSelection::Id("a\0b".to_owned()))
        .is_err());
    let rejected = s.link().calls().len();
    assert_eq!(rejected, base, "a rejected control must not reach the link");
    assert_eq!(MAX_VOLUME, 100.0);
    // An accepted control *does* reach the link: one recorded call each.
    assert!(s.seek(-5.0, SeekMode::Relative).is_ok());
    assert!(s.set_volume(MAX_VOLUME).is_ok());
    assert_eq!(
        s.link().calls().len(),
        base + 2,
        "the accepted relative seek and the legal volume did reach the link"
    );
}

#[test]
fn track_and_subtitle_selection_set_the_id_properties_as_strings() {
    let mut s = session("/media/fixture.mkv");
    let base = s.link().calls().len();
    let audio = s.select_audio(&TrackSelection::Index(2)).expect("audio");
    let sub = s
        .select_subtitle(&TrackSelection::Id("eng".to_owned()))
        .expect("subtitle");
    let off = s.select_audio(&TrackSelection::None).expect("mute");
    let auto = s
        .select_subtitle(&TrackSelection::Auto)
        .expect("auto subtitle");
    assert_eq!(
        &s.link().calls()[base..],
        &[
            format!("set-property token={audio} audio-id string 2"),
            format!("set-property token={sub} sub-id string eng"),
            format!("set-property token={off} audio-id string no"),
            format!("set-property token={auto} sub-id string auto"),
        ],
        "audio-id/sub-id are set as strings so no/auto are expressible"
    );
}

#[test]
fn queue_overflow_is_surfaced_as_dropped_events_not_silently_ignored() {
    let mut s = session("/media/fixture.mkv");
    s.link_mut().push_event(queue_overflow_event());
    s.link_mut().push_event(other_event(0));
    let report = s.pump(8);
    assert!(s.events().contains(&PlayerEvent::QueueOverflow));
    assert_eq!(report.other_events, 1, "an undistinguished id is counted");
    // An id this layer does not distinguish stays raw and never becomes an event.
    assert_eq!(other_event(50).kind, EventKind::Other { raw: 50 });
    assert!(!s.events().iter().any(|e| matches!(
        e,
        PlayerEvent::CommandReply { .. } | PlayerEvent::EndFile { .. }
    )));
}

// ---------------------------------------------------------------------------
// 5. the pump contract: bounded, non-blocking, bounded memory, UI thread stays free
// ---------------------------------------------------------------------------

#[test]
fn pump_is_bounded_by_its_budget_and_reports_that_more_draining_is_needed() {
    let mut s = PlayerSession::open(
        MockLink::with_script(vec![queue_overflow_event(); 8]),
        "/media/f.mkv",
    )
    .expect("opens");
    let report = s.pump(3);
    assert_eq!(
        report,
        PumpReport {
            drained: 3,
            wakeups: 0,
            other_events: 0,
            budget_exhausted: true,
        }
    );
    assert_eq!(s.link().queued(), 5, "the rest stays queued");
    let report = s.pump(16);
    assert_eq!(report.drained, 5);
    assert!(!report.budget_exhausted, "the queue is drained");
    let report = s.pump(16);
    assert_eq!(report.drained, 0);
    assert!(
        !report.budget_exhausted,
        "a zero-drain pump is not 'exhausted'"
    );
    // `drains()` counts `next_event` calls, not events taken: each drain ends
    // with one extra call that returns "queue empty" (3 + (5 + 1) + 1).
    assert_eq!(
        s.link().drains(),
        3 + 6 + 1,
        "a drain stops at the empty queue"
    );
    let totals = s.pump_totals();
    assert_eq!(totals.pump_calls, 3);
    assert_eq!(totals.drained, 8);
    assert_eq!(totals.budget_exhaustions, 1);
}

#[test]
fn pump_reads_the_wakeup_flag_once_per_wakeup_instead_of_polling() {
    let mut s = session("/media/f.mkv");
    s.link_mut().simulate_wakeup();
    s.link_mut().simulate_wakeup(); // coalesced: still one wakeup
    let report = s.pump(4);
    assert_eq!(report.wakeups, 1);
    assert!(s.events().contains(&PlayerEvent::Woken));
    let report = s.pump(4);
    assert_eq!(report.wakeups, 0, "edge-triggered, not level-triggered");
    s.link_mut().simulate_wakeup();
    let report = s.pump(0);
    assert_eq!(report.wakeups, 1, "budget 0 still reads the wakeup flag");
    assert_eq!(report.drained, 0);
}

#[test]
fn the_session_window_is_bounded_and_a_ui_loop_can_read_and_clear_it() {
    let mut s = PlayerSession::open(
        MockLink::with_script(vec![queue_overflow_event(); MAX_TRACKED_EVENTS + 44]),
        "/media/f.mkv",
    )
    .expect("opens");
    let report = s.pump(MAX_TRACKED_EVENTS + 44);
    assert_eq!(report.drained, MAX_TRACKED_EVENTS + 44);
    assert_eq!(s.delivered_total(), MAX_TRACKED_EVENTS + 44);
    assert_eq!(s.events().len(), MAX_TRACKED_EVENTS, "the window is capped");
    assert_eq!(s.evicted_events(), 44);
    let taken = s.drain_events();
    assert_eq!(taken.len(), MAX_TRACKED_EVENTS);
    assert!(s.events().is_empty(), "drain is read-and-clear");
    assert_eq!(
        s.delivered_total(),
        MAX_TRACKED_EVENTS + 44,
        "totals survive"
    );

    // Pump reports are bounded the same way, with cumulative totals kept.
    for _ in 0..MAX_TRACKED_PUMP_REPORTS + 6 {
        s.pump(1);
    }
    assert_eq!(s.pump_reports().len(), MAX_TRACKED_PUMP_REPORTS);
    assert_eq!(
        s.pump_totals().pump_calls,
        // The one big pump + the loop below. `drain_events` is not a pump.
        1 + MAX_TRACKED_PUMP_REPORTS + 6,
        "totals count every pump, not just the window"
    );
}

/// The negative test required by nothing in the
/// wrapper may turn a per-frame pump into a wait on the UI thread.
///
/// `MockLink::arm_blocking` deliberately violates the `PlayerLink` non-blocking
/// contract (`mpv_wait_event(h, 0)` must never block) so this test can show the
/// engine's own drain ends the wait rather than extending it.
#[test]
fn the_wrapper_never_blocks_the_calling_thread_even_against_a_link_that_blocks() {
    let mut s = session("/media/f.mkv");
    let token = s.set_pause(true).expect("pause issue");
    let start = Instant::now();
    for _ in 0..500 {
        s.pump(64);
    }
    let bounded = start.elapsed();
    assert!(
        bounded < Duration::from_secs(2),
        "500 bounded pumps took {bounded:?}; a UI frame budget cannot tolerate that"
    );

    // A link that *would* block is driven from a separate thread; the caller's
    // thread never waits for it. Releasing the channel (by dropping the sender:
    // every later recv fails immediately) lets that thread finish, and the
    // engine ends its own drain instead of waiting.
    let release = s.link_mut().arm_blocking();
    let parked = thread::spawn(move || {
        let mut s = s;
        s.pump(8);
        s.pump_until_reply(token, 8)
    });
    let caller_free = Instant::now();
    thread::sleep(Duration::from_millis(150));
    assert!(
        !parked.is_finished(),
        "the pump is parked in the link, off the caller's thread"
    );
    drop(release);
    let joined = parked
        .join()
        .expect("the pumped thread completes once the link stops blocking");
    assert!(caller_free.elapsed() < Duration::from_secs(10));
    match &joined {
        Err(PlayerError::NoReply {
            command,
            token: got,
            drains,
        }) => {
            assert_eq!(*got, token);
            assert!(command.contains("pause"));
            assert_eq!(*drains, 0, "the engine did not spin against its budget");
        }
        other => panic!("expected NoReply, got {other:?}"),
    }
}

#[test]
fn a_simulated_ui_frame_loop_drains_every_event_without_stalling() {
    let mut s = session("/media/f.mkv");
    let token = s.seek(60.0, SeekMode::Absolute).expect("seek issue");
    s.link_mut()
        .push_event(prop("time-pos", plex_player::PropertyValue::Double(60.0)));
    s.link_mut()
        .push_event(prop("pause", plex_player::PropertyValue::Flag(false)));
    s.link_mut()
        .push_event(reply_event(token, mock_success_code()));

    let start = Instant::now();
    let mut frames = 0usize;
    while s.link().queued() > 0 && frames < 10 {
        frames += 1;
        s.link_mut().simulate_wakeup();
        let report = s.pump(4);
        assert!(
            !report.budget_exhausted,
            "one bounded frame drains the queue"
        );
    }
    assert_eq!(frames, 1, "the queue drained in a single bounded frame");
    assert!(start.elapsed() < Duration::from_secs(1));
    assert_eq!(s.state().time_pos, Some(60.0));
    assert_eq!(s.state().paused, Some(false));
    assert!(s.events().iter().any(
        |e| matches!(e, PlayerEvent::CommandReply { token: got, ok: true, .. } if *got == token)
    ));
}

// ---------------------------------------------------------------------------
// 6. teardown, thread boundary, and the shared header error-code space
// ---------------------------------------------------------------------------

#[test]
fn close_stops_the_item_makes_nothing_else_outstanding_and_refuses_further_use() {
    let mut s = session("/media/f.mkv");
    let load = load_token(&s);
    s.link_mut()
        .push_event(reply_event(load, mock_success_code()));
    s.pump(8);
    s.close().expect("close");
    assert!(s.is_closed());
    assert!(s.pending_tokens().is_empty(), "teardown clears correlation");
    assert!(
        s.item().is_none(),
        "one session is one item: none after close"
    );
    assert!(s
        .link()
        .calls()
        .iter()
        .any(|c| c.starts_with("command token=") && c.ends_with(" stop")));
    assert!(s
        .events()
        .iter()
        .any(|e| *e == PlayerEvent::Closed || *e == PlayerEvent::Woken));
    assert!(matches!(s.set_pause(true), Err(PlayerError::Closed)));
    assert!(matches!(s.stop(), Err(PlayerError::Closed)));
    assert!(matches!(
        s.observe(props::TIME_POS),
        Err(PlayerError::Closed)
    ));
    assert!(matches!(s.close(), Err(PlayerError::Closed)));
    let report = s.pump(8);
    assert_eq!(report, PumpReport::default(), "a pump after close is inert");
}

#[test]
fn a_session_can_be_driven_from_another_thread_and_its_state_arrives() {
    let (tx, rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        let mut s = session("/media/f.mkv");
        let token = s.set_volume(30.0).expect("volume issue");
        s.link_mut()
            .push_event(reply_event(token, mock_success_code()));
        s.link_mut()
            .push_event(prop("volume", plex_player::PropertyValue::Double(30.0)));
        let reply = s.pump_until_reply(token, 8).expect("reply");
        // `pump_until_reply` is bounded at the matched reply: the property
        // observation queued behind it needs its own pump. State only ever
        // arrives through delivered events, so a second pump is required.
        s.pump(8);
        tx.send((s.state().volume, reply.label())).expect("send");
        s.close().expect("close");
    });
    let (volume, label) = rx.recv().expect("state crossed the thread boundary");
    assert_eq!(volume, Some(30.0));
    assert!(label.contains("token"));
    handle.join().expect("session thread completed");
}

#[test]
fn the_request_labels_are_the_wrapper_shape_a_report_shows() {
    assert_eq!(pause_request(true).label(), "set pause = flag");
    assert_eq!(
        volume_request(20.0).expect("volume").label(),
        "set volume = double"
    );
    assert_eq!(
        seek_request(12.5, SeekMode::Absolute)
            .expect("seek")
            .label(),
        "seek 12.5 absolute"
    );
    assert_eq!(
        seek_request(-3.0, SeekMode::Relative)
            .expect("relative seek")
            .label(),
        "seek -3 relative"
    );
    assert_eq!(
        seek_request(1.0, SeekMode::AbsoluteExact)
            .expect("exact seek")
            .label(),
        "seek 1 absolute+exact"
    );
    assert_eq!(stop_request().label(), "stop");
    assert_eq!(
        load_request("/media/fixture.mkv").expect("load").label(),
        "loadfile /media/fixture.mkv replace"
    );
    // The labels name the async call shape, not a claim about libmpv accepting it.
    assert!(matches!(pause_request(false), Request::SetProperty { .. }));
    assert!(matches!(stop_request(), Request::Command { .. }));
}

#[test]
fn the_mock_and_real_replies_share_the_header_error_code_space() {
    assert!(is_success(mock_success_code()));
    assert_eq!(
        mock_success_code(),
        plex_player::ffi::mpv_error_MPV_ERROR_SUCCESS,
        "the mock's success code **is** the header constant"
    );
    assert!(!is_success(code("MPV_ERROR_COMMAND")));
    assert_eq!(
        error_name(code("MPV_ERROR_UNSUPPORTED")),
        "MPV_ERROR_UNSUPPORTED"
    );
    assert_eq!(
        error_name(123456),
        "MPV_ERROR_NOT_IN_HEADERS",
        "an unknown code is never invented"
    );
    assert_eq!(
        mock_error_code("MPV_ERROR_PENDING"),
        None,
        "libmpv has no MPV_ERROR_PENDING; the wrapper must not assume one"
    );
    // The mock never claims to be libmpv; only the FFI link reports a version.
    let s = session("/media/f.mkv");
    assert_eq!(s.link().libmpv_version(), MOCK_VERSION);
    assert_eq!(
        plex_player::build_time_version(),
        env!("PLEX_MPV_VERSION"),
        "the FFI link's version is the build-time pkg-config fact"
    );
}

#[test]
fn a_link_that_refuses_a_request_is_reported_as_rejected_not_as_a_command_failure() {
    let mut s = session("/media/f.mkv");
    s.link_mut().reject_next(code("MPV_ERROR_UNSUPPORTED"));
    let err = s.stop().expect_err("the link refused the request");
    match &err {
        PlayerError::Rejected {
            call,
            command,
            code: got,
            name,
        } => {
            assert_eq!(
                call, "mpv_command_async",
                "the mock mirrors the FFI call name"
            );
            assert_eq!(command, "stop");
            assert_eq!(*got, code("MPV_ERROR_UNSUPPORTED"));
            assert_eq!(name, "MPV_ERROR_UNSUPPORTED");
        }
        other => panic!("expected Rejected, got {other:?}"),
    }
    assert!(s.pending_tokens().contains(&load_token(&s)));
    assert_eq!(
        s.pending_tokens()
            .iter()
            .filter(|t| **t != load_token(&s))
            .count(),
        0,
        "a refused request never became outstanding"
    );
}

// ---------------------------------------------------------------------------
// 7. the control gate: controls are queued until FILE_LOADED, flushed in order
//
// These tests exercise `ControlGate::FileLoaded`, the deferred queue, the flush
// points and the drop records at mock level. The gate is what a *real* libmpv
// link uses (`MpvLink::default_control_gate`); the mock deliberately defaults to
// `ControlGate::Immediate` so sections 1-6 can isolate correlation, so these
// tests switch the gate explicitly. Which lifecycle event is really the first
// one libmpv accepts a control at is recorded as unverified
// (`docs/CAPABILITIES.md`); the mock proves the wrapper's queue/flush/drop
// rules, not a libmpv acceptance boundary.
// ---------------------------------------------------------------------------

/// Open a session that uses the conservative gate (what a real link defaults to).
fn gated_session(uri: &str) -> PlayerSession<MockLink> {
    let mut s = session(uri);
    s.set_control_gate(ControlGate::FileLoaded);
    assert_eq!(
        s.control_gate(),
        ControlGate::FileLoaded,
        "the gate the tests assume must be the gate the session uses"
    );
    s
}

#[test]
fn the_file_loaded_gate_defers_every_control_but_still_allocates_reply_tokens() {
    let mut s = gated_session("/media/fixture.mkv");
    let base = s.link().calls().len();

    let pause = s.set_pause(true).expect("queued pause still gets a token");
    let seek = s
        .seek(30.0, SeekMode::Relative)
        .expect("queued seek still gets a token");
    let volume = s
        .set_volume(55.0)
        .expect("queued volume still gets a token");

    // Nothing but the loadfile issued by `open` has reached the link.
    assert!(
        s.link().calls()[base..].is_empty(),
        "a gated control must not reach the link before FILE_LOADED: {:?}",
        &s.link().calls()[base..]
    );
    // The queue keeps issue order, which is what the flush preserves.
    assert_eq!(
        s.deferred_controls(),
        vec![
            "set pause = flag".to_owned(),
            "seek 30 relative".to_owned(),
            "set volume = double".to_owned(),
        ]
    );
    // A queued request is not outstanding: nothing can reply to a request the
    // link never saw, so `PlayerEvent::Uncorrelated` stays meaningful.
    assert_eq!(
        s.pending_tokens(),
        vec![load_token(&s)],
        "only loadfile is outstanding while the controls are queued"
    );
    for token in [pause, seek, volume] {
        assert!(
            !s.pending_tokens().contains(&token),
            "queued token {token} must not be registered as outstanding"
        );
    }
    // The loadfile is never gated.
    assert!(s.link().called(&format!(
        "command token={} loadfile /media/fixture.mkv replace",
        load_token(&s)
    )));
}

#[test]
fn file_loaded_flushes_the_queued_controls_in_issue_order_and_records_the_flush() {
    let mut s = gated_session("/media/fixture.mkv");
    let base = s.link().calls().len();
    let pause = s.set_pause(true).expect("pause queued");
    let seek = s.seek(30.0, SeekMode::Relative).expect("seek queued");
    let volume = s.set_volume(55.0).expect("volume queued");

    s.link_mut().push_event(start_file_event());
    s.link_mut().push_event(file_loaded_event());
    let report = s.pump(16);
    assert_eq!(report.drained, 2);

    assert_eq!(
        &s.link().calls()[base..],
        &[
            format!("set-property token={pause} pause flag true"),
            format!("command token={seek} seek 30 relative"),
            format!("set-property token={volume} volume double 55"),
        ],
        "the flush must preserve issue order"
    );
    assert!(s.deferred_controls().is_empty(), "the queue drained");
    assert!(s.file_loaded(), "FILE_LOADED is what opened the gate");
    assert_eq!(
        s.control_flushes(),
        &[format!(
            "flush requested=3 sent=3 link-rejected=0 gate={} file_loaded=true playback_started=true",
            ControlGate::FileLoaded.label()
        )],
        "the flush is a machine-readable fact, not an internal detail"
    );

    // The lifecycle events themselves are surfaced to the UI.
    let kinds: Vec<&PlayerEvent> = s
        .events()
        .iter()
        .filter(|e| matches!(e, PlayerEvent::ItemStarted | PlayerEvent::FileLoaded))
        .collect();
    assert_eq!(
        kinds.len(),
        2,
        "START_FILE and FILE_LOADED both reach the UI"
    );

    // Flushing again is a no-op that records nothing (no phantom flush facts).
    assert_eq!(s.flush_controls(), 0);
    assert_eq!(s.control_flushes().len(), 1);
}

#[test]
fn playback_restart_is_the_fallback_flush_point_when_file_loaded_never_arrives() {
    let mut s = gated_session("/media/fixture.mkv");
    let base = s.link().calls().len();
    let seek = s.seek(12.0, SeekMode::Absolute).expect("seek queued");
    s.link_mut().push_event(playback_restart_event());
    let report = s.pump(16);
    assert_eq!(report.drained, 1);
    assert_eq!(
        &s.link().calls()[base..],
        &[format!("command token={seek} seek 12 absolute")],
        "controls must not stay queued forever when only PLAYBACK_RESTART arrives"
    );
    assert!(
        !s.file_loaded(),
        "PLAYBACK_RESTART does not claim the file loaded"
    );
    assert!(
        s.control_flushes().iter().any(|line| line.starts_with(
            "flush requested=1 sent=1 link-rejected=0 gate=file-loaded file_loaded=false"
        )),
        "the fallback flush is recorded with file_loaded=false: {}\n",
        s.control_flushes().join("\n")
    );
}

#[test]
fn a_control_queued_for_an_item_that_ends_is_dropped_and_recorded() {
    let mut s = gated_session("/media/fixture.mkv");
    s.seek(5.0, SeekMode::Absolute).expect("seek queued");
    s.set_volume(20.0).expect("volume queued");
    let calls_before = s.link().calls().len();

    s.link_mut()
        .push_event(end_file_event(EndReason::Eof, mock_success_code()));
    s.pump(16);

    assert!(
        s.deferred_controls().is_empty(),
        "nothing may leak past END_FILE"
    );
    assert_eq!(
        s.control_flushes(),
        &["flush dropped-deferred=2 reason=end-file".to_owned()],
        "a dropped queue must be visible in the report, not silently cleared"
    );
    assert_eq!(
        s.link().calls().len(),
        calls_before,
        "a control dropped by END_FILE is never sent to the link"
    );
}

#[test]
fn close_drops_the_queued_controls_for_the_closed_item_and_refuses_further_use() {
    let mut s = gated_session("/media/fixture.mkv");
    s.set_pause(true).expect("pause queued");
    let calls_before = s.link().calls().len();

    s.close().expect("close");

    assert!(s.is_closed());
    assert!(s.deferred_controls().is_empty());
    assert_eq!(
        s.control_flushes(),
        &["flush dropped-deferred=1 reason=close".to_owned()],
        "the drop is recorded so a report can explain why a control never arrived"
    );
    // `close` issues `stop` (best effort) and nothing else: the queued control
    // is dropped rather than flushed into a torn-down handle.
    let after = &s.link().calls()[calls_before..];
    assert_eq!(after.len(), 1, "close issues only stop: {after:?}");
    assert!(after[0].ends_with(" stop"), "expected stop, got {after:?}");
    let err = s.seek(9.0, SeekMode::Absolute).expect_err("closed session");
    assert!(
        matches!(err, PlayerError::Closed),
        "a closed session must refuse rather than queue for a next item: {err:?}"
    );
    assert!(s.deferred_controls().is_empty());
}

#[test]
fn a_reply_for_a_still_queued_control_token_is_reported_uncorrelated() {
    let mut s = gated_session("/media/fixture.mkv");
    let queued = s.set_volume(40.0).expect("volume queued");
    // libmpv cannot answer a request it never received; if a reply with that
    // token shows up anyway the wrapper must not attribute it to the queued
    // request.
    s.link_mut()
        .push_event(set_property_reply_event(queued, mock_success_code()));
    let report = s.pump(16);
    assert_eq!(report.drained, 1);
    match &s.events()[0] {
        PlayerEvent::Uncorrelated { token, reason } => {
            assert_eq!(*token, queued);
            assert!(
                reason.contains("no outstanding request"),
                "the reason must name the real cause: {reason}"
            );
        }
        other => panic!("expected Uncorrelated, got {other:?}"),
    }
    assert_eq!(
        s.unmatched_replies(),
        &[(queued, plex_player::UNMATCHED_NO_REQUEST)]
    );
    assert!(s.rejected_replies().is_empty());
    assert_eq!(
        s.deferred_controls(),
        vec!["set volume = double".to_owned()],
        "the request is still queued even though something answered its token"
    );
}

#[test]
fn id_4_set_property_replies_are_correlated_with_the_set_request_that_asked_for_a_reply() {
    // `mpv_set_property_async` answers with MPV_EVENT_SET_PROPERTY_REPLY, not
    // MPV_EVENT_COMMAND_REPLY (client.h:1262-1272). Without the mapping the two
    // `set ...` controls below would vanish into `other_events` and a rejected
    // pause/volume would be invisible to the UI.
    let mut s = session("/media/fixture.mkv");
    assert_eq!(
        s.control_gate(),
        ControlGate::Immediate,
        "the mock defaults to the immediate gate; the gate tests above switch it"
    );
    let volume = s.set_volume(60.0).expect("volume issue");
    let pause = s.set_pause(true).expect("pause issue");

    let cmd_code = code("MPV_ERROR_COMMAND");
    s.link_mut()
        .push_event(set_property_reply_event(volume, mock_success_code()));
    s.link_mut()
        .push_event(set_property_reply_event(pause, cmd_code));
    let report = s.pump(16);
    assert_eq!(report.drained, 2);
    assert_eq!(
        report.other_events, 0,
        "neither reply may be treated as unknown"
    );

    let replies: Vec<(u64, String, bool)> = s
        .events()
        .iter()
        .filter_map(|e| match e {
            PlayerEvent::CommandReply {
                token, command, ok, ..
            } => Some((*token, command.clone(), *ok)),
            _ => None,
        })
        .collect();
    assert_eq!(
        replies,
        vec![
            (volume, "set volume = double".to_owned(), true),
            (pause, "set pause = flag".to_owned(), false),
        ],
        "each id-4 reply is attributed to the request that issued its token"
    );

    // The rejected one is surfaced as a failure naming the async call, and kept
    // in the session's rejection list for the report.
    let rejected = s
        .events()
        .iter()
        .find(
            |e| matches!(e, PlayerEvent::CommandReply { token, ok, .. } if *token == pause && !*ok),
        )
        .expect("the rejected reply is delivered");
    match rejected {
        PlayerEvent::CommandReply {
            command,
            error: Some(err),
            ..
        } => {
            assert_eq!(command, "set pause = flag");
            assert_eq!(err.name, error_name(cmd_code));
            assert_eq!(err.code, cmd_code);
        }
        other => panic!("expected a failed CommandReply, got {other:?}"),
    }
    assert_eq!(
        s.rejected_replies()
            .iter()
            .map(|r| r.fact())
            .collect::<Vec<String>>(),
        vec![format!(
            "token={pause} command=set pause = flag error={}",
            error_name(cmd_code)
        )],
        "the rejection is a session fact the UI/probe report can read"
    );
    assert!(!s.pending_tokens().contains(&volume));
    assert!(!s.pending_tokens().contains(&pause));
}

#[test]
fn the_gate_and_phase_labels_are_the_machine_readable_names_a_report_carries() {
    // ControlGate::label is what `control_gate=` and `gate=` report fields show.
    assert_eq!(ControlGate::Immediate.label(), "immediate");
    assert_eq!(ControlGate::FileLoaded.label(), "file-loaded");
    // ControlPhase::label is what `control_phase=` shows; every variant the
    // probe can run must have a stable name (including the deferred variant).
    assert_eq!(ControlPhase::AfterLoadReply.label(), "after-load-reply");
    assert_eq!(
        ControlPhase::AfterPlaybackStarted.label(),
        "after-playback-started"
    );
    assert_eq!(
        ControlPhase::DeferredUntilFileLoaded.label(),
        "deferred-until-file-loaded"
    );
    assert_eq!(ControlPhase::BeforeLoadReply.label(), "before-load-reply");
    assert_eq!(ControlPhase::None.label(), "none");
    // The scripted lifecycle events carry the header ids when libmpv is linked,
    // so a mock run and a real run disagree on nothing but behaviour.
    for kind in [
        EventKind::StartFile,
        EventKind::FileLoaded,
        EventKind::PlaybackRestart,
    ] {
        let raw = lifecycle_event(kind);
        assert_eq!(raw.kind, kind);
        assert_eq!(
            raw.reply_userdata, NO_TOKEN,
            "lifecycle events reply to nothing"
        );
        assert!(raw.end_file.is_none());
    }
}
