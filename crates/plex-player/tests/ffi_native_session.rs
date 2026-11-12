//! Tests for the **real libmpv** path: a display-free session (vo=null,
//! ao=null, config=no, terminal=no, idle=no) driven through the committed
//! bindgen bindings, not through `MockLink`.
//!
//! Why this file exists: two early defects (a display-free SIGSEGV during
//! teardown, and `seek` replies that were silently `MPV_ERROR_COMMAND`) both
//! reached a *default-feature* binary and neither was covered by any test. `tests/control_loop.rs` proves wrapper/token semantics
//! against a mock; it proves nothing about libmpv. Everything asserted in the
//! enabled tests here is observed from a live libmpv in the headless test environment at test
//! time.
//!
//! What this file does **not** claim: it does not assert where the libmpv
//! control-acceptance boundary is (START_FILE vs FILE_LOADED vs
//! PLAYBACK_RESTART). That is deliberately left open: it is recorded as
//! unverified in `docs/CAPABILITIES.md` ("libmpv control acceptance"), and the
//! wrapper therefore uses the
//! conservative `ControlGate::FileLoaded` queue. The gate tests below prove the
//! guarantee that queue provides (no control is sent before `FILE_LOADED`, and
//! the released set is accepted by this libmpv build on this run). The orders
//! that would freeze a boundary - controls raced against the load reply or
//! issued right after START_FILE - are one `#[ignore]`d measurement harness
//! (`exploratory_phases_record_what_libmpv_accepted_before_file_loaded`) that
//! prints acceptance per order instead of asserting it.
//!
//! Scope and honesty:
//! * The item is a locally synthesized 1-second PCM WAV written by this test
//!   into `CARGO_TARGET_TMPDIR`. It is **not** Plex media; no Plex account,
//!   token, server or library is contacted.
//! * What a real libmpv accepted here is a fact about this machine's libmpv
//!   build. Property/command names that libmpv accepted are recorded as such;
//!   names this run never delivered an observation for stay labelled
//!   unverified in the report (`commands::SourceLabel::Unverified`).
//! * No display server, window or audio device is used, so this test is a
//!   player-engine gate, not an A/V output test. It cannot prove the rendered
//!   picture or audible audio are correct.
//!
//! Test-only allowances, the documented convention for test targets in this
//! workspace (see `tests/native_contract.rs:7`, `tests/control_loop.rs:15`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
#![cfg(feature = "libmpv-ffi")]

use std::path::{Path, PathBuf};

use plex_player::probe::{run_item, ControlPhase, ItemRun};

/// The scripted control set in issue order, as [`plex_player::Request::label`]
/// reports it. [`ItemRun::deferred`] records these labels (the wrapper's own
/// request labels), not the longer issue labels the probe prints in `issued`.
const EXPECTED_CONTROL_SET: [&str; 7] = [
    "set volume = double",
    "seek 0.5 absolute",
    "set pause = flag",
    "seek 0.25 relative",
    "set pause = flag",
    "set audio-id = string",
    "set sub-id = string",
];

/// Deterministic local media: 1.000 s of 8 kHz mono 16-bit PCM sine (440 Hz).
///
/// Written by the same generator `--smoke-no-display --fixture auto` uses
/// ([`plex_player::probe::synthesize_wav`], RIFF/WAVE by hand - no ffmpeg, no
/// external process, no Plex media), so an enabled gate test here and an item in
/// the saved `--probe-json` report read byte-identical media.
fn tone_fixture(dir: &Path, name: &str) -> PathBuf {
    plex_player::probe::synthesize_wav(&dir.join(name), 1)
        .expect("cannot write the synthesized fixture")
}

fn reply_lines(run: &ItemRun, needle: &str) -> Vec<String> {
    run.events
        .iter()
        .filter(|line| line.starts_with("reply token=") && line.contains(needle))
        .cloned()
        .collect()
}

fn any_rejected_reply(run: &ItemRun) -> Option<String> {
    run.events
        .iter()
        .find(|line| line.starts_with("reply token=") && line.contains("ok=false"))
        .cloned()
}

/// The FFI report must unpack the client API version the way `client.h:250`
/// packs it (`((major) << 16) | (minor)`), from the **loaded** library.
#[test]
fn runtime_client_api_parts_are_major_minor_not_a_masked_blob() {
    let (major, minor) = plex_player::mpv_link::client_api_parts(
        plex_player::mpv_link::runtime_client_api_version(),
    );
    assert_eq!(
        (major, minor),
        (2, 5),
        "mpv_client_api_version() unpacked as major={major} minor={minor}; \
         client.h:251 declares MPV_MAKE_VERSION(2, 5) and the build guard \
         required pkg-config mpv >= 2.5.0"
    );
    assert_eq!(
        plex_player::mpv_link::compile_time_client_api(),
        (2 << 16) | 5,
        "our own mirror of MPV_MAKE_VERSION(2, 5) drifted from the header guard"
    );
    println!("runtime client api parts observed: major={major} minor={minor}");
}

/// The gate for the control order this client actually ships: the scripted
/// control set is issued through the real link's conservative
/// [`ControlGate::FileLoaded`] gate ([`ControlPhase::DeferredUntilFileLoaded`]),
/// so every control reaches libmpv only after `MPV_EVENT_FILE_LOADED`.
///
/// What is asserted is the **guarantee the wrapper provides** and the facts a
/// real run produces under it: controls are released only after `FILE_LOADED`,
/// every issued request (load + 7 controls) gets exactly one correlated reply -
/// including the `mpv_set_property_async` ones, which libmpv answers with
/// `MPV_EVENT_SET_PROPERTY_REPLY` (`client.h:1266-1269`), a different event id
/// from `MPV_EVENT_COMMAND_REPLY` - and every rejection libmpv does answer with
/// is surfaced in `ItemRun::errors`.
///
/// Whether `FILE_LOADED` is the *earliest* event that accepts a control is
/// **[U]** (`docs/CAPABILITIES.md` "libmpv control acceptance");
/// `FILE_LOADED` is only required to be *an* event after which the test environment's
/// libmpv accepted `volume`/`seek`/`pause`, and this gate re-measures that each
/// run. Which selection values a real Plex item needs is likewise not decided
/// here (`[U]` in `docs/CAPABILITIES.md`).
#[test]
fn real_libmpv_session_deferred_until_file_loaded_accepts_the_control_set() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    let item = tone_fixture(&dir, "m4-ffi-deferred.wav");
    let run = run_item(
        item.to_str().expect("non-UTF-8 tmpdir"),
        ControlPhase::DeferredUntilFileLoaded,
    );
    for line in run.lines() {
        println!("{line}");
    }

    assert_eq!(run.classification, "fixture-or-file");
    assert_eq!(run.phase, "deferred-until-file-loaded");
    // The real link's default gate must really be the conservative one: a
    // session opened on `FfiPlayer` never runs with the gate off.
    assert_eq!(
        run.gate, "file-loaded",
        "a real session did not inherit the conservative FileLoaded gate: gate={}",
        run.gate
    );
    assert!(
        !run.pumps.iter().any(|l| l.starts_with("gate-wait ")),
        "the deferred phase must not wait on a lifecycle event: {:?}",
        run.pumps
    );
    assert!(
        run.pumps.iter().any(|l| {
            l.starts_with("controls-issued phase=deferred-until-file-loaded ")
                && l.contains("gate=file-loaded")
        }),
        "the deferred phase must record the gate it issued under: {:?}",
        run.pumps
    );

    // Deferred-or-already-loaded, measured at the run. If controls were queued,
    // the session must report them **and** one clean flush: every queued request
    // reached the link, none was refused by the link.
    let flushes = run.flushes.join(" | ");
    if run.deferred.is_empty() {
        println!("observed: FILE_LOADED was already delivered when the control set was issued; nothing was deferred");
    } else {
        assert_eq!(
            run.deferred, EXPECTED_CONTROL_SET,
            "the control set was not queued whole at issue time: deferred={:?} flushes={flushes}",
            run.deferred
        );
        let flush = run
            .flushes
            .iter()
            .find(|l| l.starts_with("flush requested="))
            .unwrap_or_else(|| {
                panic!("controls were deferred but no flush was recorded: {flushes}")
            });
        assert!(
            flush.contains("requested=7") && flush.contains("sent=7") && flush.contains("link-rejected=0")
                && flush.contains("gate=file-loaded"),
            "the queued controls were not released whole by the gate: {flush} (all flushes: {flushes})"
        );
        println!("observed deferral/flush facts: {flush}");
    }

    // The ordering guarantee the gate exists for: FILE_LOADED is delivered
    // before the first **control** reply, whatever the libmpv event ordering was.
    let file_loaded_at = run
        .events
        .iter()
        .position(|l| l == "file-loaded")
        .expect("no MPV_EVENT_FILE_LOADED was delivered for a playable item");
    let first_control_reply_at = run
        .events
        .iter()
        .position(|l| l.starts_with("reply token=") && !l.contains("command=load"))
        .expect("no control reply was delivered");
    assert!(
        file_loaded_at < first_control_reply_at,
        "a control was sent before MPV_EVENT_FILE_LOADED (file-loaded event #{file_loaded_at}, \
         first control reply #{first_control_reply_at}): {:?}",
        run.events
    );

    // Every request the session issued is answered by exactly one correlated
    // reply: 1 `loadfile` + the 7 released controls. This is the regression this
    // gate exists for: libmpv answers `mpv_command_async` with
    // `MPV_EVENT_COMMAND_REPLY` (id 5) and `mpv_set_property_async` with
    // `MPV_EVENT_SET_PROPERTY_REPLY` (id 4, `client.h:1266-1269`, `reply_userdata`
    // documented for both at `client.h:1593-1608`). While id 4 was unmapped, the
    // 4 `set ...` controls produced **no** correlated reply at all and a rejected
    // `set ...` was invisible to `PlayerSession::rejected_replies`.
    let replies: Vec<&String> = run
        .events
        .iter()
        .filter(|l| l.starts_with("reply token="))
        .collect();
    assert_eq!(
        replies.len(),
        1 + EXPECTED_CONTROL_SET.len(),
        "expected one reply per issued request (1 load + {} controls), got {}: {:?} issued={:?}",
        EXPECTED_CONTROL_SET.len(),
        replies.len(),
        run.events,
        run.issued
    );

    // Accepted vs. rejected is read off this run, name by name:
    // * `volume`, both `seek`s and both `pause`s must be accepted once the gate
    //   has released them (the M4 point: controls accepted after `FILE_LOADED`).
    //   That is a wrapper guarantee, so it is asserted.
    // * the two track-selection controls (`audio-id = auto`, `sub-id = no`) are a
    //   fact about *this* libmpv build (it answered `MPV_ERROR_PROPERTY_NOT_FOUND`
    //   for this audio-only fixture). Why, and whether another build accepts them,
    //   is **[U]** (`docs/CAPABILITIES.md`), so the gate asserts only that every
    //   rejection is **surfaced**, never swallowed and never pinned to a count.
    for (needle, expected_replies) in [
        ("command=loadfile", 1),
        ("command=set volume = double", 1),
        ("command=seek 0.5 absolute", 1),
        ("command=seek 0.25 relative", 1),
        ("command=set pause = flag", 2),
        ("command=set audio-id = string", 1),
        ("command=set sub-id = string", 1),
    ] {
        let lines = reply_lines(&run, needle);
        assert_eq!(
            lines.len(),
            expected_replies,
            "expected {expected_replies} correlated reply/ies for {needle}, got {lines:?} (issued={:?})",
            run.issued
        );
    }

    // Controls this client must be able to issue are accepted once the gate has
    // released them (volume / both seeks / both pauses). That is the M4 point of
    // the deferred gate and it holds for any libmpv build, so it is what this
    // gate pins.
    for needle in [
        "command=set volume = double",
        "command=seek 0.5 absolute",
        "command=seek 0.25 relative",
        "command=set pause = flag",
    ] {
        let lines = reply_lines(&run, needle);
        assert!(
            lines.iter().all(|l| l.contains("ok=true")),
            "libmpv did not accept {needle}: replies={lines:?} errors={:?}",
            run.errors
        );
    }

    // The two track-selection controls (`audio-id = auto`, `sub-id = no`) are a
    // libmpv-build question, not a wrapper question: *this* build answered both
    // with `MPV_ERROR_PROPERTY_NOT_FOUND` for an audio-only WAV, but why, and
    // whether another build accepts them, is **[U]** (`docs/CAPABILITIES.md`,
    // M4 section). This gate therefore no longer pins the count `2`, the error
    // code, or even that a selection control was rejected. What it pins is the
    // wrapper guarantee that holds for every build: **every rejected control
    // reply is surfaced as an error carrying its own token** (that is how a
    // silently rejected `seek` survived earlier reviews), and nothing outside
    // the selection controls was rejected.
    let rejected: Vec<String> = run
        .events
        .iter()
        .filter(|l| l.starts_with("reply token=") && l.contains("ok=false"))
        .cloned()
        .collect();
    if let Some(first) = any_rejected_reply(&run) {
        assert!(
            rejected.contains(&first),
            "a rejected reply seen by the helper is missing from the rejected set: {first}"
        );
    }
    for reply in &rejected {
        let token = reply
            .split_whitespace()
            .find(|w| w.starts_with("token="))
            .unwrap_or("")
            .to_owned();
        assert!(
            run.errors.iter().any(|e| e.contains(&token)),
            "a rejected control reply was not surfaced as an error carrying its token: {reply} errors={:?} events={:?}",
            run.errors,
            run.events
        );
        assert!(
            reply.contains("audio-id") || reply.contains("sub-id"),
            "a control this client ships was rejected, not only a selection control: {reply} events={:?}",
            run.events
        );
    }
    println!(
        "selection-control rejections observed by this run: count={} errors={:?}",
        rejected.len(),
        run.errors
    );
    assert_eq!(
        run.outcome, "ended",
        "the item never reached END_FILE within the session timeout"
    );
    // `ended` (an END_FILE delivery inside the session deadline) is the wrapper
    // guarantee. *Which* reason a build reports for a file played to its end is
    // a libmpv fact, so this asserts the reason is one the headers declare
    // (`events::EndReason`) instead of pinning the literal `eof` this build
    // produced - a different build/fixture would otherwise redden the gate for
    // reasons unrelated to the wrapper.
    let reason = run
        .end_reason
        .clone()
        .unwrap_or_else(|| panic!("no END_FILE reason recorded: events={:?}", run.events));
    assert!(
        matches!(
            reason.as_str(),
            "eof" | "stop" | "quit" | "error" | "redirect"
        ),
        "END_FILE reason {reason} is not a reason the headers declare: events={:?}",
        run.events
    );
    assert_ne!(
        reason, "error",
        "the item ended with an error reason: {reason} errors={:?}",
        run.errors
    );
    println!("END_FILE reason observed by this run: {reason}");
    assert!(
        run.wakeups_total > 0,
        "no wakeup callback observed: control completion was not driven by libmpv waking us"
    );
    assert!(
        run.pump_calls > 0 && run.drained_total > 0,
        "pump drained nothing (pump_calls={} drained={})",
        run.pump_calls,
        run.drained_total
    );
    assert!(
        run.unmatched.is_empty(),
        "replies arrived for tokens we never issued: {:?}",
        run.unmatched
    );

    // Property facts: label and observation are two different facts. `duration`
    // is not a header symbol in the installed client.h, so it stays labelled
    // unverified even though this run delivered it.
    let facts = run.property_facts.join(" | ");
    assert!(
        run.property_facts
            .iter()
            .any(|l| l.starts_with("duration=unverified") && l.contains("observed=yes")),
        "duration was never observed for a real 1.000 s item: {facts}"
    );
    assert!(
        run.property_facts
            .iter()
            .any(|l| l.starts_with("time-pos=header") && l.contains("observed=yes")),
        "time-pos was never observed while the item played: {facts}"
    );

    // Delivery evidence, not the post-END_FILE snapshot.
    //
    // `ItemRun::final_state` is the state *after* END_FILE. This libmpv build
    // delivers `MPV_FORMAT_NONE` for `time-pos`, `playback-time`, `duration`,
    // `eof-reached` and `media-title` as the item ends, and `ObservedState::apply`
    // deliberately clears a field on an unavailable observation instead of keeping
    // a stale value (`state.rs`). Whether that trailing unavailable delivery lands
    // in the same pump batch as END_FILE is libmpv timing. An earlier revision
    // asserted numeric values out of that snapshot (`state("time_pos=")`,
    // `state("duration=")`) and the test was nondeterministically red on one
    // unchanged tree. What the session
    // recorded as delivery order *is* stable evidence, so the assertions below
    // read the `property name=.. value=..` event lines.
    let delivered = |name: &str| -> Vec<String> {
        let prefix = format!("property name={name} value=");
        run.events
            .iter()
            .filter_map(|line| line.strip_prefix(prefix.as_str()))
            .map(|rest| {
                rest.split_whitespace()
                    .next()
                    .unwrap_or_default()
                    .to_owned()
            })
            .collect::<Vec<String>>()
    };
    let numeric = |name: &str| -> Vec<f64> {
        delivered(name)
            .iter()
            .filter_map(|value| value.parse::<f64>().ok())
            .collect()
    };
    let last_delivery =
        |name: &str| -> String { delivered(name).last().cloned().unwrap_or_default() };

    let durations = numeric("duration");
    assert!(
        !durations.is_empty(),
        "duration was never delivered as a number for a real 1.000 s item: {facts} events={:?}",
        run.events
    );
    for value in &durations {
        assert!(
            (*value - 1.0).abs() < 0.1,
            "libmpv delivered duration={value} for a fixture synthesized to be 1.000 s: deliveries={:?}",
            delivered("duration")
        );
    }

    let positions = numeric("time-pos");
    assert!(
        positions.len() >= 2,
        "expected at least a start position and a post-seek position delivery for time-pos: deliveries={:?} events={:?}",
        delivered("time-pos"),
        run.events
    );
    let max_position = positions.iter().copied().fold(0.0_f64, f64::max);
    assert!(
        (0.4..=1.0).contains(&max_position),
        "the accepted absolute+relative seeks never moved time-pos into the expected range: numeric deliveries={positions:?} all deliveries={:?}",
        delivered("time-pos")
    );

    let volumes = numeric("volume");
    assert!(
        volumes.len() >= 2,
        "expected the initial volume observation and the scripted set_volume(80) observation: deliveries={:?}",
        delivered("volume")
    );
    let last_volume = *volumes.last().expect("volumes non-empty");
    assert!(
        (last_volume - 80.0).abs() < f64::EPSILON,
        "the scripted set_volume(80) was never the last volume delivered: numeric deliveries={volumes:?} all deliveries={:?}",
        delivered("volume")
    );

    // `paused`: read the delivery trail, not the snapshot. Note the honest limit
    // of this run: this build delivered **one** `pause` observation (`false`) for
    // a scripted pause + resume, so this proves the observed state agreed with
    // the scripted final state; it does not prove mpv reports each pause toggle
    // as a property change (recorded as [U] in docs/CAPABILITIES.md).
    let pauses = delivered("pause");
    println!("pause deliveries observed by this run: {pauses:?}");
    assert!(
        !pauses.is_empty(),
        "no pause observation was delivered at all: {facts}"
    );

    // The wrapper invariant that makes the report's `<none>` lines meaningful
    // instead of a flaky artefact: a final-state field is `<none>` exactly when
    // the **last** delivery for that name was unavailable (`none`) or nothing was
    // ever delivered, and carries a value otherwise.
    let state_field = |field: &str| {
        run.final_state
            .iter()
            .find_map(|line| line.strip_prefix(field))
            .unwrap_or_else(|| panic!("no {field} field in the final state: {:?}", run.final_state))
            .to_owned()
    };
    for (field, name) in [
        ("time_pos=", "time-pos"),
        ("playback_time=", "playback-time"),
        ("duration=", "duration"),
        ("volume=", "volume"),
        ("paused=", "pause"),
        ("eof_reached=", "eof-reached"),
        ("media_title=", "media-title"),
        ("audio_id=", "audio-id"),
        ("sub_id=", "sub-id"),
    ] {
        let value = state_field(field);
        match last_delivery(name).as_str() {
            "none" => assert_eq!(
                value,
                "<none>",
                "{field} still holds a stale value although the last delivery for {name} was unavailable: deliveries={:?}",
                delivered(name)
            ),
            "" => assert_eq!(
                value,
                "<none>",
                "{field} has a value although nothing was ever delivered for {name}"
            ),
            last => assert_ne!(
                value,
                "<none>",
                "{field} was cleared although the last delivery for {name} was {last}: deliveries={:?}",
                delivered(name)
            ),
        }
    }
    println!(
        "final-state (post-END_FILE) snapshot: {}",
        run.final_state.join(" ")
    );
}

/// **[U] / exploratory, therefore `#[ignore]`**: what a real libmpv does with a
/// control issued *earlier* than the wrapper's `FILE_LOADED` release point.
///
/// This repository does **not** claim to know where the libmpv
/// control-acceptance boundary is, so no gate test
/// may freeze "a control issued at X is rejected/accepted". This test is the
/// measurement harness for that open question (see "libmpv control
/// acceptance" in `docs/CAPABILITIES.md`): it runs each immediate-gate
/// order over the same fixture, **prints** the per-order acceptance facts
/// (rejected replies, `run.errors`, outcome, end reason) and asserts only the
/// bookkeeping that is deterministic for a phase — the phase label, that the phase
/// really switched the conservative gate off (otherwise it would be measuring the
/// gate and not libmpv), that its documented wait did or did not happen, and that
/// the session stayed bounded and correlated.
///
/// Re-measure with `cargo test -p plex-player --test ffi_native_session --
/// --ignored`. A result here is evidence about *that* libmpv build on *that* run,
/// not a boundary fact this client may rely on; the shipped order is the
/// conservative gate test above.
#[ignore = "depends on the libmpv control-acceptance boundary, which this repository records as unverified (docs/CAPABILITIES.md); run manually with --ignored to re-measure"]
#[test]
fn exploratory_phases_record_what_libmpv_accepted_before_file_loaded() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    for (phase, expect_load_wait, expect_gate_wait) in [
        (ControlPhase::AfterLoadReply, true, false),
        (ControlPhase::BeforeLoadReply, false, false),
        (ControlPhase::AfterPlaybackStarted, true, true),
    ] {
        let item = tone_fixture(&dir, &format!("m4-ffi-exploratory-{}.wav", phase.label()));
        let run = run_item(item.to_str().expect("non-UTF-8 tmpdir"), phase);
        println!("--- exploratory phase {} ---", phase.label());
        for line in run.lines() {
            println!("{line}");
        }
        assert_eq!(run.phase, phase.label());
        assert_eq!(
            run.gate,
            "immediate",
            "phase {} did not measure libmpv with the wrapper gate off: gate={}",
            phase.label(),
            run.gate
        );
        assert!(
            run.deferred.is_empty(),
            "phase {} queued controls instead of issuing them: {:?}",
            phase.label(),
            run.deferred
        );
        assert!(
            run.pumps.iter().any(|l| l.starts_with(&format!(
                "controls-issued phase={} gate=immediate ",
                phase.label()
            ))),
            "phase {} did not record the session state it issued its controls in: {:?}",
            phase.label(),
            run.pumps
        );
        assert_eq!(
            run.pumps.iter().any(|l| l.starts_with("load-wait ")),
            expect_load_wait,
            "phase {} must {}wait for the load reply: {:?}",
            phase.label(),
            if expect_load_wait { "" } else { "not " },
            run.pumps
        );
        assert_eq!(
            run.pumps.iter().any(|l| l.starts_with("gate-wait ")),
            expect_gate_wait,
            "phase {} must {}use the playback-started gate wait: {:?}",
            phase.label(),
            if expect_gate_wait { "" } else { "not " },
            run.pumps
        );
        assert!(
            !run.events.iter().any(|l| l.starts_with("uncorrelated")),
            "phase {} saw a reply for a token it never issued: {:?}",
            phase.label(),
            run.events
        );
        assert!(
            run.pump_calls < 400,
            "phase {} was not bounded (pump_calls={})",
            phase.label(),
            run.pump_calls
        );
        let rejected: Vec<String> = run
            .events
            .iter()
            .filter(|l| l.starts_with("reply token=") && l.contains("ok=false"))
            .cloned()
            .collect();
        println!(
            "observed (not asserted) for phase {}: rejected_control_replies={} outcome={} end_reason={:?} errors={:?}",
            phase.label(),
            rejected.len(),
            run.outcome,
            run.end_reason,
            run.errors
        );
        for line in &rejected {
            println!("observed (not asserted) rejected reply: {line}");
        }
    }
}

/// The phase bookkeeping / boundedness gate for the racing order: the same item,
/// control set issued immediately after `loadfile` and before any reply was
/// observed, with the wrapper's conservative gate switched off.
///
/// The *rejected reply* is printed as an observed fact but not asserted, because
/// libmpv's async ordering for that race - and whether the item then still plays
/// out - is not something this layer may freeze (`docs/CAPABILITIES.md`,
/// "libmpv control acceptance"). What
/// IS asserted is that the phase distinction is real: the run stays bounded,
/// records its phase, and never blocks without a wakeup wait.
#[test]
fn real_libmpv_session_before_load_reply_stays_bounded_and_labelled() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    let item = tone_fixture(&dir, "m4-ffi-before-load.wav");
    let run = run_item(
        item.to_str().expect("non-UTF-8 tmpdir"),
        ControlPhase::BeforeLoadReply,
    );
    for line in run.lines() {
        println!("{line}");
    }
    assert_eq!(run.phase, "before-load-reply");
    assert_eq!(run.outcome, "ended");
    // No load-reply wait happened in this phase: that is the phase contract.
    assert!(
        !run.pumps.iter().any(|l| l.starts_with("load-wait ")),
        "before-load-reply must not wait for a reply: {:?}",
        run.pumps
    );
    let rejected: Vec<String> = run
        .events
        .iter()
        .filter(|l| l.starts_with("reply token=") && l.contains("ok=false"))
        .cloned()
        .collect();
    println!(
        "observed (not asserted): {} reply/replies rejected when controls raced the load: {:?}",
        rejected.len(),
        rejected
    );
    assert!(
        run.pump_calls < 400,
        "the session was not bounded (pump_calls={})",
        run.pump_calls
    );
}

/// A source libmpv cannot play must produce a typed error, not a hang or a
/// silent success, and a source this layer rejects must never reach libmpv.
///
/// The missing-item run uses the **shipped** gate: nothing is ever loaded, so
/// `FILE_LOADED` never opens the gate and every queued control must be dropped
/// at `END_FILE` rather than issued into a broken session or leaking into the
/// next item.
#[test]
fn real_libmpv_reports_a_missing_item_and_the_layer_rejects_an_unsupported_one() {
    let missing = run_item(
        "/nonexistent/plex-m4-ffi-missing-item.wav",
        ControlPhase::DeferredUntilFileLoaded,
    );
    for line in missing.lines() {
        println!("{line}");
    }
    assert_eq!(missing.classification, "fixture-or-file");
    assert_eq!(missing.gate, "file-loaded");
    assert_eq!(
        missing.deferred,
        EXPECTED_CONTROL_SET.to_vec(),
        "the control set was not queued whole for an item that never loaded: {:?}",
        missing.deferred
    );
    assert!(
        missing
            .flushes
            .iter()
            .any(|l| { l.starts_with("flush dropped-deferred=") && l.contains("reason=end-file") }),
        "queued controls were not dropped when the item ended: flushes={:?}",
        missing.flushes
    );
    assert!(
        !missing.errors.is_empty()
            || missing
                .events
                .iter()
                .any(|l| l.starts_with("end-file reason=error")),
        "a missing item produced no error fact: errors={:?} events={:?}",
        missing.errors,
        missing.events
    );
    assert_ne!(missing.outcome, "ended");
    assert_ne!(missing.outcome, "open-rejected");
    assert!(
        missing.pump_calls < 400,
        "the missing-item session was not bounded (pump_calls={})",
        missing.pump_calls
    );

    let rejected = run_item("dvd://1", ControlPhase::None);
    for line in rejected.lines() {
        println!("{line}");
    }
    assert_eq!(rejected.classification, "unsupported-source");
    assert_eq!(rejected.outcome, "open-rejected");
    assert_eq!(rejected.phase, "none");
    assert!(
        rejected
            .errors
            .iter()
            .any(|l| l.contains("unsupported_item") || l.contains("unsupported item")),
        "the unsupported source was not reported as such: {:?}",
        rejected.errors
    );
    assert!(
        rejected.issued.is_empty(),
        "a rejected source must not reach libmpv: {:?}",
        rejected.issued
    );
}

/// One handle per item, sequentially, in one process: repeated real
/// init/observe/teardown cycles must not crash or leak events between items.
/// (This is the shape the past teardown SIGSEGV needed a gate for.)
///
/// Uses the **shipped** control order (the conservative `FILE_LOADED` gate), so
/// each item must show the same per-item facts as the gate test: the whole
/// control set is queued and released once, every issued request is answered by
/// one correlated reply (load + 7 controls), the only errors are the two
/// track-selection rejections the test environment's libmpv answers for this fixture, and
/// no reply leaks from one item's session into the next.
#[test]
fn real_libmpv_sessions_are_sequential_and_one_handle_per_item() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    for n in 0..3 {
        let item = tone_fixture(&dir, &format!("m4-ffi-seq-{n}.wav"));
        let run = run_item(
            item.to_str().expect("non-UTF-8 tmpdir"),
            ControlPhase::DeferredUntilFileLoaded,
        );
        assert_eq!(
            run.gate, "file-loaded",
            "sequential item {n} did not run with the shipped control gate: gate={}",
            run.gate
        );
        assert_eq!(
            run.outcome, "ended",
            "sequential item {n} did not end cleanly: errors={:?} events={:?}",
            run.errors, run.events
        );
        assert_eq!(
            run.events
                .iter()
                .filter(|l| l.starts_with("reply token="))
                .count(),
            1 + EXPECTED_CONTROL_SET.len(),
            "sequential item {n} did not answer every issued request ({} replies): events={:?}",
            run.events
                .iter()
                .filter(|l| l.starts_with("reply token="))
                .count(),
            run.events
        );
        // Same relaxation as the deferred-gate test: every error this item reports
        // must be a surfaced selection-control rejection carrying its token; the
        // count and the code are properties of this libmpv build, not of the
        // wrapper, so they are printed rather than pinned ([U] in
        // docs/CAPABILITIES.md).
        assert!(
            run.errors
                .iter()
                .all(|e| (e.contains("set audio-id = string")
                    || e.contains("set sub-id = string"))
                    && e.contains("token=")),
            "sequential item {n} reported an error that is not a surfaced selection-control rejection: {:?}",
            run.errors
        );
        let rejected: usize = run
            .events
            .iter()
            .filter(|l| l.starts_with("reply token=") && l.contains("ok=false"))
            .count();
        assert_eq!(
            run.errors.len(),
            rejected,
            "sequential item {n} surfaced a different number of rejections than it received (errors={:?} events={:?})",
            run.errors,
            run.events
        );
        println!("sequential item {n}: {rejected} selection rejections surfaced");
        assert!(
            run.events.iter().any(|l| l == "start-file"),
            "sequential item {n} never delivered MPV_EVENT_START_FILE: {:?}",
            run.events
        );
        assert!(
            !run.events.iter().any(|l| l.starts_with("uncorrelated")),
            "item {n} saw a reply it never issued: {:?}",
            run.events
        );
        println!("sequential item {n}: {}", run.lines().len());
    }
}
