//! The display-free **real libmpv** control session.
//!
//! This is the run that turns the wrapper's *claims* into facts. It creates one
//! `mpv_handle` per item with `vo=null` / `ao=null` (no window, no audio device,
//! no display server), drives it only with async commands carrying reply tokens,
//! and records what the installed libmpv actually accepted: which command names
//! replied, which property names delivered a `MPV_EVENT_PROPERTY_CHANGE`, in
//! which format, with which value, and what `END_FILE` reasons it produced.
//!
//! Machine-readable by construction: every fact is a `key=value` string built
//! here, so `plex_cli --smoke-no-display` can embed them in JSON verbatim and
//! the `--probe-json` report is a captured run, not prose.
//!
//! Two independent dimensions, never mixed in one line:
//!
//! * **label** (`header:<cite>` / `unverified:<cite>`, `commands::SourceLabel`)
//!   - whether the installed libmpv *headers* back the name as a symbol. This
//!     is a compile-time fact about the headers, not about the run.
//! * **observed=yes/no** - whether *this run* delivered a
//!   `MPV_EVENT_PROPERTY_CHANGE` for the name. This is a runtime fact about the
//!   run, not proof that the name is a header symbol.
//!
//! So a name can be `unverified` **and** `observed=yes`: the observation is
//! evidence about this run only and the name stays `unverified` as a header
//! claim. A gate test must not turn `observed=yes` into "header-backed".
//!
//! Media fixtures for this probe are synthesized by [`synthesize_wav`] (hand
//! written RIFF/WAVE, no ffmpeg, no downloaded media). Running
//! `plex_cli --smoke-no-display --fixture auto --probe-json <path>` regenerates
//! the fixture and the captured report from this repository alone.
//!
//! The loop here is the same shape the UI loop will use: the wakeup callback
//! sends on a channel, the driver thread blocks on that channel with a timeout
//! (never a busy poll) and calls [`PlayerSession::pump`] (`mpv_wait_event(h, 0)`)
//! after each wakeup, under an overall deadline.

use std::sync::mpsc::{channel, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use crate::commands::{cmds, props, SeekMode};
use crate::engine::{ControlGate, PlayerSession};
use crate::events::{PlayerError, PlayerEvent};
use crate::link::PlayerLink;
use crate::mpv_link::{build_time_version, runtime_client_api_version, FfiPlayer};
use crate::tracks::TrackSelection;

/// Options this probe sets before `mpv_initialize`, in order.
///
/// These are pre-initialization **option** names, not properties: they are the
/// documented libmpv options for a client that owns no window (`client.h`
/// documents the option/property distinction, not these names - they are
/// labelled unverified like every other non-header name).
pub const OPTIONS: &[(&str, &str)] = &[
    ("config", "no"),
    ("vo", "null"),
    ("ao", "null"),
    ("terminal", "no"),
    ("idle", "no"),
];

/// How long one item session may run before the probe gives up on it.
pub const SESSION_TIMEOUT: Duration = Duration::from_secs(20);
/// Raw events one [`PlayerSession::pump`] call may take.
pub const PUMP_BUDGET: usize = 64;
/// How long the driver waits for a wakeup before pumping an empty queue again.
pub const WAKE_TICK: Duration = Duration::from_millis(50);
/// How long [`run_item`] waits for the load reply before issuing the control set.
pub const LOAD_REPLY_TIMEOUT: Duration = Duration::from_secs(5);
/// Pump calls the load-reply wait may make before giving up.
pub const LOAD_REPLY_MAX_PUMPS: usize = 40;
/// How long [`run_item`] waits for a playback-started signal (an
/// `MPV_EVENT_START_FILE` delivery) before issuing the control set anyway.
pub const PLAYBACK_GATE_TIMEOUT: Duration = Duration::from_secs(5);
/// Pump calls the playback-started wait may make before giving up.
pub const PLAYBACK_GATE_MAX_PUMPS: usize = 100;
/// Synthesize deterministic local media: `seconds` of 8 kHz mono 16-bit PCM
/// sine (440 Hz), written as RIFF/WAVE by hand.
///
/// No ffmpeg, no external process, no downloaded media, no Plex media. The same
/// generator is used by `--smoke-no-display --fixture auto` and by
/// `tests/ffi_native_session.rs`, so a captured report item and an enabled gate
/// test read byte-identical media and the report is reproducible from the
/// repository (see `synthesize_fixture_for_cli` in `crates/plex-cli`).
pub fn synthesize_wav(path: &std::path::Path, seconds: u32) -> std::io::Result<std::path::PathBuf> {
    let rate: u32 = 8_000;
    let samples: u32 = rate * seconds;
    let data_len = samples * 2;
    let mut bytes: Vec<u8> = Vec::new();
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes()); // PCM chunk size
    bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
    bytes.extend_from_slice(&1u16.to_le_bytes()); // mono
    bytes.extend_from_slice(&rate.to_le_bytes());
    bytes.extend_from_slice(&(rate * 2).to_le_bytes()); // byte rate
    bytes.extend_from_slice(&2u16.to_le_bytes()); // block align
    bytes.extend_from_slice(&16u16.to_le_bytes()); // bits
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for n in 0..samples {
        // Integer sine approximation good enough for a demux/decode session.
        let phase = (n % 18) as f64 / 18.0;
        let value = (2.0 * std::f64::consts::PI * phase).sin();
        bytes.extend_from_slice(&((value * 8_000.0) as i16).to_le_bytes());
    }
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    std::fs::write(path, &bytes)?;
    Ok(path.to_path_buf())
}

/// The fixture the CLI generates for `--fixture auto` (path relative to the
/// workspace, in gitignored `target/`).
pub const GENERATED_FIXTURE: &str = "target/probe-fixtures/m4-tone-1s.wav";

/// Upper bound on recorded pump/event lines per item (a long session must not
/// grow the report without bound).
pub const MAX_RECORD_LINES: usize = 120;

/// When the scripted control requests are issued relative to the item's own
/// load reply / playback-started signal, and whether the wrapper's conservative
/// [`ControlGate`] is left on or switched off for that measurement.
///
/// Why the phases exist at all: an async `loadfile` **reply** is delivered *before*
/// `MPV_EVENT_START_FILE` in the test environment's display-free runs, and controls
/// issued early in an item run were answered `MPV_ERROR_COMMAND`. That is recorded
/// in the `--probe-json` report's item runs, and it is re-measured by the
/// `tests/ffi_native_session.rs` gate. **Which** lifecycle event is the earliest
/// one that makes a control acceptable is *not* established by this repository
/// (`docs/CAPABILITIES.md`), so no phase below may be presented as "the order a
/// UI must use": the phases exist so a run records, per order, whether libmpv
/// accepted the control set or rejected it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlPhase {
    /// Wait for the load reply, then drive the control set with the gate
    /// switched off (requests reach libmpv at that moment). A display-free run
    /// answered `MPV_ERROR_COMMAND` for `seek` in this order; that is a
    /// fact about this build, kept as the documented negative order because it
    /// is how a naive UI issues controls.
    AfterLoadReply,
    /// Wait for the load reply **and** for an `MPV_EVENT_START_FILE` delivery,
    /// then drive the control set with the gate switched off.
    ///
    /// An exploratory order only. Earlier sessions reported that `seek` was
    /// accepted in this order; the frozen gate in this repository does **not**
    /// reproduce that, so this phase's outcome is reported per run and is not a
    /// claim a UI may rely on.
    AfterPlaybackStarted,
    /// Issue the control set with the link's default gate ([`ControlGate::FileLoaded`])
    /// left on: the wrapper defers every control until `MPV_EVENT_FILE_LOADED`
    /// has been delivered, then sends them in issue order. This is the order a
    /// real session in this client uses, and the report records whether libmpv
    /// accepted the deferred set (`run.errors` / `control-flush` lines).
    DeferredUntilFileLoaded,
    /// Drive the control set immediately after issuing the load, before any
    /// reply has been observed, with the gate switched off. The expected outcome
    /// is a rejected control; a run records whether it was.
    BeforeLoadReply,
    /// Issue no control requests (used for sources this layer must reject).
    None,
}

impl ControlPhase {
    /// Machine-readable name recorded in the probe report for each item run.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::AfterLoadReply => "after-load-reply",
            Self::AfterPlaybackStarted => "after-playback-started",
            Self::DeferredUntilFileLoaded => "deferred-until-file-loaded",
            Self::BeforeLoadReply => "before-load-reply",
            Self::None => "none",
        }
    }
}

/// One item's display-free session facts.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ItemRun {
    /// Source handed to `PlayerSession::open`.
    pub source: String,
    /// `fixture-or-file` / `unsupported-source` / `libmpv-unavailable`.
    pub classification: String,
    /// [`ControlPhase::label`] - when the control set was issued.
    pub phase: String,
    /// [`crate::engine::ControlGate::label`] this item session ran with.
    ///
    /// `immediate` means the requests reached libmpv at the moment the phase
    /// issued them; `file-loaded` means the wrapper queued them until
    /// `MPV_EVENT_FILE_LOADED` (the shipped real-session gate).
    pub gate: String,
    /// Control labels still queued when the control set finished issuing.
    pub deferred: Vec<String>,
    /// Gate/flush facts from [`PlayerSession::control_flushes`] (including
    /// `flush-error` lines), in the order the session recorded them.
    pub flushes: Vec<String>,
    /// `ended` / `item-failed` / `timeout` / `open-rejected` / `initialize-failed` /
    /// `no-fixture-given` / `unavailable`.
    ///
    /// `item-failed` means libmpv delivered `END_FILE` with reason `error`: an
    /// item that reached an end event but did not play (e.g. a missing file).
    /// Ending is not success, so the reason is recorded separately too.
    pub outcome: String,
    /// `END_FILE` reason label (typed, from [`crate::events::EndReason`]) when
    /// an end event was delivered, otherwise `None`.
    pub end_reason: Option<String>,
    /// Requests issued, machine-readable (`token=N <label>`).
    pub issued: Vec<String>,
    /// Delivered events in delivery order ([`PlayerEvent::label`]).
    pub events: Vec<String>,
    /// One line per pump call: `pump#N drained=.. wakeups=.. other=.. exhausted=..`.
    pub pumps: Vec<String>,
    /// Errors the layer reported (code + message).
    pub errors: Vec<String>,
    /// Replies we could not attribute to a live request.
    pub unmatched: Vec<String>,
    /// `state::ObservedState::provenance()` - name, label, observed=yes/no.
    pub property_facts: Vec<String>,
    /// Final observed state fields (`field=value` / `field=<none>`).
    pub final_state: Vec<String>,
    /// Total raw events drained across all pumps.
    pub drained_total: usize,
    /// Total wakeup flags observed.
    pub wakeups_total: usize,
    /// Pump calls performed.
    pub pump_calls: usize,
}

impl ItemRun {
    fn new(source: &str, classification: &str, phase: ControlPhase) -> Self {
        Self {
            source: source.to_owned(),
            classification: classification.to_owned(),
            phase: phase.label().to_owned(),
            // Expected gate for this phase before a session exists; [`run_item_on`]
            // replaces it with the gate the session actually ran with.
            gate: match phase {
                ControlPhase::DeferredUntilFileLoaded => ControlGate::FileLoaded.label(),
                ControlPhase::None => "none",
                _ => ControlGate::Immediate.label(),
            }
            .to_owned(),
            outcome: String::new(),
            ..Self::default()
        }
    }

    /// Machine-readable lines, prefixed `item.<classification>.`.
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        let tag = format!(
            "item[{}]",
            self.classification.replace(['[', ']', '.'], "-")
        );
        let mut out = vec![
            format!("{tag} source={}", self.source),
            format!("{tag} control-phase={}", self.phase),
            format!("{tag} control-gate={}", self.gate),
            format!("{tag} outcome={}", self.outcome),
            format!(
                "{tag} end-reason={}",
                self.end_reason
                    .clone()
                    .unwrap_or_else(|| "<none>".to_owned())
            ),
            format!(
                "{tag} pumps={} drained={} wakeups={}",
                self.pump_calls, self.drained_total, self.wakeups_total
            ),
        ];
        out.extend(self.issued.iter().map(|l| format!("{tag} issued {l}")));
        if !self.deferred.is_empty() {
            out.push(format!(
                "{tag} deferred-at-issue count={} labels={}",
                self.deferred.len(),
                self.deferred.join("|")
            ));
        }
        out.extend(
            self.flushes
                .iter()
                .map(|l| format!("{tag} control-flush {l}")),
        );
        out.extend(self.events.iter().map(|l| format!("{tag} {l}")));
        out.extend(self.pumps.iter().map(|l| format!("{tag} {l}")));
        out.extend(self.errors.iter().map(|l| format!("{tag} error {l}")));
        out.extend(
            self.unmatched
                .iter()
                .map(|l| format!("{tag} unmatched {l}")),
        );
        out.extend(
            self.property_facts
                .iter()
                .map(|l| format!("{tag} property-fact {l}")),
        );
        out.extend(
            self.final_state
                .iter()
                .map(|l| format!("{tag} final-state {l}")),
        );
        out
    }
}

/// The whole display-free probe run.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProbeReport {
    /// Build-time pkg-config / bindings stamps (from `build.rs`).
    pub build: Vec<String>,
    /// Facts that only a loaded libmpv can answer.
    pub runtime: Vec<String>,
    /// Provenance of every name this wrapper uses.
    pub names: Vec<String>,
    /// One entry per played/rejected item.
    pub items: Vec<ItemRun>,
    /// Honest caveats about what this run does **not** prove.
    pub notes: Vec<String>,
}

impl ProbeReport {
    /// Machine-readable flat form (`section key=value` per line).
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        let mut out = self
            .build
            .iter()
            .map(|l| format!("build {l}"))
            .collect::<Vec<String>>();
        out.extend(self.runtime.iter().map(|l| format!("runtime {l}")));
        out.extend(self.names.iter().map(|l| format!("name {l}")));
        for item in &self.items {
            out.extend(item.lines());
        }
        out.extend(self.notes.iter().map(|l| format!("note {l}")));
        out
    }
}

fn build_stamps() -> Vec<String> {
    vec![
        format!("libmpv_version={}", build_time_version()),
        format!(
            "pkgconfig={}",
            option_env!("PLEX_MPV_PKGCONFIG").unwrap_or("<unset>")
        ),
        format!(
            "pkgconfig_includes={}",
            option_env!("PLEX_MPV_INCLUDES").unwrap_or("<empty>")
        ),
        format!(
            "pkgconfig_link_paths={}",
            option_env!("PLEX_MPV_LINK_PATHS").unwrap_or("<empty>")
        ),
        format!(
            "bindings={}",
            option_env!("PLEX_MPV_BINDINGS").unwrap_or("<unset>")
        ),
        format!(
            "bindgen_version={}",
            option_env!("PLEX_MPV_BINDGEN_VERSION").unwrap_or("<unset>")
        ),
        format!(
            "ffi_functions={}",
            option_env!("PLEX_MPV_FFI_FNS").unwrap_or("<unset>")
        ),
        format!(
            "ffi_constants={}",
            option_env!("PLEX_MPV_FFI_CONSTS").unwrap_or("<unset>")
        ),
        format!(
            "regeneration_feature={}",
            option_env!("PLEX_MPV_REGEN_FEATURE").unwrap_or("<unset>")
        ),
        format!(
            "libmpv_ffi_feature={}",
            option_env!("PLEX_MPV_FFI_FEATURE").unwrap_or("<unset>")
        ),
    ]
}

fn name_facts() -> Vec<String> {
    props::ALL
        .iter()
        .map(|named| format!("kind=property {}", named.fact()))
        .chain(
            cmds::ALL
                .iter()
                .map(|named| format!("kind=command {}", named.fact())),
        )
        .collect()
}

fn err_line(err: &PlayerError) -> String {
    format!("code={} message={err}", err.code())
}

/// Classify a source the way `ItemSource::parse` will, without a libmpv handle.
fn classify(source: &str) -> String {
    if crate::commands::ItemSource::parse(source).is_ok() {
        "fixture-or-file".to_owned()
    } else {
        "unsupported-source".to_owned()
    }
}

/// Run one item session end to end and collect its facts.
///
/// `phase` decides whether the scripted volume/seek/pause/track control set is
/// issued at all and where it sits relative to the load reply. A source that
/// still gets the control set while nothing was ever loaded is what records how
/// libmpv answers control requests for an item it could not play.
#[must_use]
pub fn run_item(source: &str, phase: ControlPhase) -> ItemRun {
    let mut run = ItemRun::new(source, &classify(source), phase);
    let (tx, rx) = channel::<()>();
    let link = match FfiPlayer::new(Some(tx)) {
        Ok(link) => link,
        Err(err) => {
            run.classification = "libmpv-unavailable".to_owned();
            run.outcome = "unavailable".to_owned();
            run.errors.push(err_line(&err));
            return run;
        }
    };
    run_item_on(link, &rx, source, phase, &mut run);
    run
}

/// Issue the scripted control set.
///
/// The set ends by resuming playback (`pause = false`): a session left paused
/// never reaches `END_FILE`, which is how an earlier run recorded
/// `outcome=timeout` for an item libmpv had actually finished playing.
fn issue_controls(session: &mut PlayerSession<FfiPlayer>, run: &mut ItemRun) {
    let requests: Vec<(String, Result<u64, PlayerError>)> = vec![
        (
            "set volume = double value=80".to_owned(),
            session.set_volume(80.0),
        ),
        (
            "seek 0.5 absolute".to_owned(),
            session.seek(0.5, SeekMode::Absolute),
        ),
        (
            "set pause = flag value=true".to_owned(),
            session.set_pause(true),
        ),
        (
            "seek 0.25 relative".to_owned(),
            session.seek(0.25, SeekMode::Relative),
        ),
        (
            "set pause = flag value=false".to_owned(),
            session.set_pause(false),
        ),
        (
            "set audio-id = string value=auto".to_owned(),
            session.select_audio(&TrackSelection::Auto),
        ),
        (
            "set sub-id = string value=no".to_owned(),
            session.select_subtitle(&TrackSelection::None),
        ),
    ];
    for (label, outcome) in requests {
        match outcome {
            Ok(token) => run.issued.push(format!("token={token} {label}")),
            Err(err) => run.errors.push(err_line(&err)),
        }
    }
}

/// Pump (with wakeup waits, never a busy poll) until the reply for `token`
/// arrives or the bounded wait runs out.
///
/// Records one `load-wait pump#N ...` line per pump and a final
/// `load-wait outcome=... pumps=N` line, so both orders are facts rather than
/// an assumption about when a load reply is observable.
///
/// The whole drained batch is recorded before the wait ends: an
/// `MPV_EVENT_START_FILE` that arrives in the same pump as the load reply must
/// not be thrown away, because it is the signal the playback gate looks for.
fn wait_for_load_reply(
    session: &mut PlayerSession<FfiPlayer>,
    rx: &Receiver<()>,
    token: u64,
    run: &mut ItemRun,
) {
    let deadline = Instant::now() + LOAD_REPLY_TIMEOUT;
    let mut outcome = "not-observed";
    let mut pumps = 0usize;
    while pumps < LOAD_REPLY_MAX_PUMPS && Instant::now() < deadline {
        let report = session.pump(PUMP_BUDGET);
        pumps += 1;
        run.drained_total += report.drained;
        run.wakeups_total += report.wakeups;
        if run.pumps.len() < MAX_RECORD_LINES {
            run.pumps.push(format!(
                "load-wait pump#{pumps} drained={} wakeups={} other={} exhausted={}",
                report.drained, report.wakeups, report.other_events, report.budget_exhausted
            ));
        }
        for event in session.drain_events() {
            let reply = match &event {
                PlayerEvent::CommandReply {
                    token: got,
                    ok,
                    error,
                    command,
                } if *got == token => {
                    Some((*ok, error.as_ref().map(|e| e.name.clone()), command.clone()))
                }
                _ => None,
            };
            note_end(run, &event);
            if run.events.len() < MAX_RECORD_LINES {
                run.events.push(event.label());
            }
            if let Some((ok, error, command)) = reply {
                outcome = if ok { "confirmed" } else { "rejected" };
                if !ok {
                    run.errors.push(format!(
                        "load reply command={command} error={}",
                        error.unwrap_or_else(|| "<unnamed>".to_owned())
                    ));
                }
            }
        }
        if outcome != "not-observed" {
            break;
        }
        match rx.recv_timeout(WAKE_TICK) {
            Ok(()) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    run.pumps.push(format!(
        "load-wait token={token} outcome={outcome} pumps={pumps} max-pumps={LOAD_REPLY_MAX_PUMPS} timeout-ms={}",
        LOAD_REPLY_TIMEOUT.as_millis()
    ));
}

/// Record a **rejected control request** as an error fact.
///
/// A `MPV_ERROR_COMMAND` reply for a token this run issued is not a call
/// failure (the request reached libmpv and was answered), but it is a failure
/// of the control order and must not sit in `events` while `errors` stays
/// empty - that is how a silently rejected `seek` survived earlier reviews.
fn note_rejected_control(run: &mut ItemRun, event: &PlayerEvent) {
    if let PlayerEvent::CommandReply {
        token,
        ok: false,
        command,
        error,
    } = event
    {
        if run
            .issued
            .iter()
            .any(|line| line.starts_with(&format!("token={token} ")))
        {
            run.errors.push(format!(
                "control reply token={token} command={command} error={}",
                error
                    .as_ref()
                    .map(|e| e.name.clone())
                    .unwrap_or_else(|| "<unnamed>".to_owned())
            ));
        }
    }
}

/// Record the end reason so `outcome` can tell "the item ended" apart from
/// "the item ended **with an error**" (an unplayable file also ends).
fn note_end(run: &mut ItemRun, event: &PlayerEvent) {
    if let PlayerEvent::EndFile { reason, .. } = event {
        run.end_reason = Some(reason.label().to_owned());
    }
}

/// Pump until libmpv has delivered `MPV_EVENT_START_FILE` for this item, or the
/// bounded wait runs out.
///
/// An exploratory wait, not a proven boundary. the test environment's display-free runs
/// show that the async `loadfile` **reply** arrives before `MPV_EVENT_START_FILE`
/// and that controls issued before START_FILE were rejected; neither that
/// evidence nor the frozen gate establishes START_FILE as the earliest event that
/// makes a control acceptable (`docs/CAPABILITIES.md`). What this helper
/// records is a fact either way: the observed signal, the pump count and whether
/// `FILE_LOADED` had also arrived when the control set was issued.
///
/// The shipped session does **not** gate on START_FILE: it gates on
/// `MPV_EVENT_FILE_LOADED` ([`ControlGate::FileLoaded`]).
fn wait_for_playback_started(
    session: &mut PlayerSession<FfiPlayer>,
    rx: &Receiver<()>,
    run: &mut ItemRun,
) {
    let mut outcome = if session.playback_started() {
        "start-file-already-delivered"
    } else {
        "not-observed"
    };
    let deadline = Instant::now() + PLAYBACK_GATE_TIMEOUT;
    let mut pumps = 0usize;
    while outcome == "not-observed" && pumps < PLAYBACK_GATE_MAX_PUMPS && Instant::now() < deadline
    {
        let report = session.pump(PUMP_BUDGET);
        pumps += 1;
        run.drained_total += report.drained;
        run.wakeups_total += report.wakeups;
        if run.pumps.len() < MAX_RECORD_LINES {
            run.pumps.push(format!(
                "gate-wait pump#{pumps} drained={} wakeups={} other={} exhausted={}",
                report.drained, report.wakeups, report.other_events, report.budget_exhausted
            ));
        }
        for event in session.drain_events() {
            note_end(run, &event);
            if run.events.len() < MAX_RECORD_LINES {
                run.events.push(event.label());
            }
        }
        if session.playback_started() {
            outcome = "start-file";
            break;
        }
        match rx.recv_timeout(WAKE_TICK) {
            Ok(()) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    run.pumps.push(format!(
        "gate-wait outcome={outcome} pumps={pumps} max-pumps={PLAYBACK_GATE_MAX_PUMPS} timeout-ms={} file_loaded={}",
        PLAYBACK_GATE_TIMEOUT.as_millis(),
        session.file_loaded()
    ));
}

fn run_item_on(
    mut link: FfiPlayer,
    rx: &Receiver<()>,
    source: &str,
    phase: ControlPhase,
    run: &mut ItemRun,
) {
    for (name, value) in OPTIONS {
        if let Err(err) = link.set_option(name, value) {
            run.errors
                .push(format!("option {name}={value} {}", err_line(&err)));
        }
    }
    if let Err(err) = link.initialize() {
        run.outcome = "initialize-failed".to_owned();
        run.errors.push(err_line(&err));
        return;
    }
    if let Err(err) = link.request_log("error") {
        run.errors.push(err_line(&err));
    }

    let mut session = match PlayerSession::open(link, source) {
        Ok(session) => session,
        Err(err) => {
            run.outcome = "open-rejected".to_owned();
            run.errors.push(err_line(&err));
            return;
        }
    };
    // The load request is already outstanding (PlayerSession::open issued it);
    // record it so the report shows the token the session started with.
    for token in session.pending_tokens() {
        run.issued.push(format!(
            "token={token} load (issued by PlayerSession::open)"
        ));
    }

    // The exploratory phases must show what libmpv does with a control issued at
    // that moment, so they switch the wrapper's conservative gate off: with the
    // gate left on the request would simply be queued and the phase would say
    // nothing about libmpv. `DeferredUntilFileLoaded` keeps the link default
    // gate, i.e. the order a real session in this client actually uses.
    if matches!(
        phase,
        ControlPhase::AfterLoadReply
            | ControlPhase::AfterPlaybackStarted
            | ControlPhase::BeforeLoadReply
    ) {
        session.set_control_gate(ControlGate::Immediate);
    }
    run.gate = session.control_gate().label().to_owned();

    // `BeforeLoadReply` deliberately skips the wait: that is the order a real
    // run in the headless test environment answered with `MPV_ERROR_COMMAND` replies for `seek`,
    // and the probe keeps recording it as a negative case instead of hiding it.
    if phase == ControlPhase::AfterLoadReply
        || phase == ControlPhase::AfterPlaybackStarted
        || phase == ControlPhase::DeferredUntilFileLoaded
    {
        if let Some(token) = session.pending_tokens().first().copied() {
            wait_for_load_reply(&mut session, rx, token, run);
        } else {
            run.pumps
                .push("load-wait outcome=no-outstanding-load-token".to_owned());
        }
    }
    // `AfterPlaybackStarted` additionally waits for a START_FILE delivery. It is
    // an exploratory order: whether controls are accepted at that point is not
    // established by this repository (see `docs/CAPABILITIES.md`), which is why
    // the run records the outcome instead of assuming one.
    if phase == ControlPhase::AfterPlaybackStarted {
        wait_for_playback_started(&mut session, rx, run);
    }
    if phase != ControlPhase::None {
        // Recorded BEFORE the control set is issued: whether libmpv had already
        // delivered START_FILE / FILE_LOADED when the requests were sent, and
        // whether this phase sends immediately or queues. Without this line the
        // report shows *that* a control was rejected but not which session state
        // the phase actually produced.
        run.pumps.push(format!(
            "controls-issued phase={} gate={} playback_started={} file_loaded={}",
            phase.label(),
            session.control_gate().label(),
            session.playback_started(),
            session.file_loaded()
        ));
        issue_controls(&mut session, run);
        run.deferred = session.deferred_controls();
    }

    let deadline = Instant::now() + SESSION_TIMEOUT;
    let mut ended = false;
    while !ended && Instant::now() < deadline && run.pump_calls < MAX_RECORD_LINES * 4 {
        let report = session.pump(PUMP_BUDGET);
        run.pump_calls += 1;
        run.drained_total += report.drained;
        run.wakeups_total += report.wakeups;
        if run.pumps.len() < MAX_RECORD_LINES {
            run.pumps.push(format!(
                "pump#{} drained={} wakeups={} other={} exhausted={}",
                run.pump_calls,
                report.drained,
                report.wakeups,
                report.other_events,
                report.budget_exhausted
            ));
        }
        // Read-and-clear: the session keeps only a bounded event window, so
        // the probe takes what it has not seen yet instead of re-reading a log.
        let fresh = session.drain_events();
        for event in &fresh {
            if matches!(event, PlayerEvent::EndFile { .. }) {
                ended = true;
            }
            note_end(run, event);
            note_rejected_control(run, event);
            if run.events.len() < MAX_RECORD_LINES {
                run.events.push(event.label());
            }
        }
        if ended {
            break;
        }
        match rx.recv_timeout(WAKE_TICK) {
            Ok(()) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }

    if phase != ControlPhase::None && !ended {
        // The session never reached END_FILE: ask it to stop, then drain the
        // queue once more (bounded, non-blocking) so the report records whether
        // the stop reply and a late END_FILE arrive.
        match session.stop() {
            Ok(token) => run.issued.push(format!("token={token} stop")),
            Err(err) => run.errors.push(err_line(&err)),
        }
        let before = session.delivered_total();
        let report = session.pump(PUMP_BUDGET);
        run.pump_calls += 1;
        run.drained_total += report.drained;
        run.wakeups_total += report.wakeups;
        if run.pumps.len() < MAX_RECORD_LINES {
            run.pumps.push(format!(
                "pump#{} drained={} wakeups={} other={} exhausted={} (post-stop drain)",
                run.pump_calls,
                report.drained,
                report.wakeups,
                report.other_events,
                report.budget_exhausted
            ));
        }
        let fresh = session.drain_events();
        for event in &fresh {
            note_end(run, event);
            if run.events.len() < MAX_RECORD_LINES {
                run.events.push(event.label());
            }
        }
        if fresh.is_empty() && session.delivered_total() == before {
            run.pumps.push(format!(
                "post-stop drain took no further events (queue empty within the {PUMP_BUDGET}-event budget)"
            ));
        }
    }

    if let Some(err) = session.item_error() {
        run.errors.push(err_line(err));
    }
    run.unmatched = session
        .unmatched_replies()
        .iter()
        .map(|(token, reason)| format!("token={token} reason={reason}"))
        .collect();
    run.property_facts = session.state().provenance();
    run.flushes = session.control_flushes().to_vec();
    run.final_state = final_state_lines(session.state());
    if run.outcome.is_empty() {
        run.outcome = match (ended, run.end_reason.as_deref()) {
            // An item that ended because libmpv could not play it is not a
            // successful item.
            (true, Some("error")) => "item-failed".to_owned(),
            (true, _) => "ended".to_owned(),
            (false, _) => "timeout".to_owned(),
        };
    }
    if let Err(err) = session.close() {
        run.errors.push(err_line(&err));
    }
}

fn final_state_lines(state: &crate::state::ObservedState) -> Vec<String> {
    vec![
        format!("time_pos={}", opt_f(state.time_pos)),
        format!("playback_time={}", opt_f(state.playback_time)),
        format!("duration={}", opt_f(state.duration)),
        format!("volume={}", opt_f(state.volume)),
        format!("paused={}", opt_b(state.paused)),
        format!("eof_reached={}", opt_b(state.eof_reached)),
        format!("eof_reason={}", opt_s(&state.eof_reason)),
        format!("idle_active={}", opt_b(state.idle_active)),
        format!("paused_for_cache={}", opt_b(state.paused_for_cache)),
        format!("media_title={}", opt_s(&state.media_title)),
        format!("audio_id={}", opt_s(&state.audio_id)),
        format!("sub_id={}", opt_s(&state.sub_id)),
        format!("unrecognized={}", state.unrecognized.join("|")),
        format!(
            "observed_counts={}",
            state
                .observed_counts
                .iter()
                .map(|(n, c)| format!("{n}:{c}"))
                .collect::<Vec<String>>()
                .join(" ")
        ),
    ]
}

fn opt_f(value: Option<f64>) -> String {
    value.map_or("<none>".to_owned(), |v| v.to_string())
}

fn opt_b(value: Option<bool>) -> String {
    value.map_or("<none>".to_owned(), |v| v.to_string())
}

fn opt_s(value: &Option<String>) -> String {
    value.clone().unwrap_or_else(|| "<none>".to_owned())
}

/// Run the whole display-free probe: every fixture item (if any), a deliberately
/// missing file, and a source this layer must reject before libmpv is called.
#[must_use]
pub fn run_probe(fixtures: &[&str]) -> ProbeReport {
    let api = runtime_client_api_version();
    let (major, minor) = crate::mpv_link::client_api_parts(api);
    let expected = crate::mpv_link::compile_time_client_api();
    let mut report = ProbeReport {
        build: build_stamps(),
        runtime: vec![
            format!("client_api_version_packed=0x{api:08x}"),
            format!("client_api_version_major={major}"),
            format!("client_api_version_minor={minor}"),
            format!(
                "compile_time_client_api=0x{expected:08x} (our own unpacking of client.h:251 MPV_MAKE_VERSION(2,5); bindgen did not emit that macro)"
            ),
            format!("runtime_client_api_matches_compile_time={}", api == expected),
            format!("build_time_pkgconfig_libmpv={}", build_time_version()),
        ],
        names: name_facts(),
        items: Vec::new(),
        notes: vec![
            "vo=null ao=null terminal=no: no window, no audio device, no display server was contacted".to_owned(),
            "only async libmpv calls were used; the pump is mpv_wait_event(handle, 0) plus a wakeup-channel wait, never a blocking wait or a busy poll".to_owned(),
            "the fixture item is a locally generated file (probe::synthesize_wav: 1.000 s 8 kHz mono 16-bit PCM sine), not Plex media; no Plex account, token or server was contacted".to_owned(),
            "label and observation are different facts: 'header' means the installed client.h backs the name as a symbol, 'unverified' means it does not (only an upstream citation); observed=yes/no says whether THIS run delivered a property change for that name. A name labelled unverified here that shows observed=yes is evidence about this run only and stays unverified as a header claim".to_owned(),
            "final-state is the end-of-session snapshot: this libmpv build delivered MPV_FORMAT_NONE for time-pos/playback-time/duration/eof-reached/media-title when the item ended, which legitimately clears those fields to <none>; the per-delivery evidence is each item's `property name=.. value=..` event lines, which also show the numeric values delivered while the item played".to_owned(),
            format!("regenerate this report with: cargo run -p plex-cli -- --smoke-no-display --fixture auto --probe-json probe.json; --fixture auto synthesizes the fixture under gitignored <workspace>/{GENERATED_FIXTURE} from the current working directory, so each item's `source=` path is run-local (an absolute path in that workspace's target/) while the media bytes and the report structure are reproducible from this repository alone; no ffmpeg, no downloaded media") ,
            "unmeasured: which item-lifecycle event is the earliest one libmpv accepts a control at (see docs/CAPABILITIES.md); the wrapper queues until FILE_LOADED conservatively and reports every rejection it still received".to_owned(),
        ],
    };
    if fixtures.is_empty() {
        let mut skipped = ItemRun::new("<none>", "no-fixture", ControlPhase::None);
        skipped.outcome = "no-fixture-given".to_owned();
        skipped.errors.push("run_probe was called without a generated media fixture; the real libmpv command/property facts for a playable item are unproven in this run".to_owned());
        report.items.push(skipped);
    } else {
        for (index, path) in fixtures.iter().enumerate() {
            // Fixture provenance: the report records what the media *was*, so a
            // reader can regenerate the same bytes with `--fixture auto` instead
            // of trusting a machine-local path that only exists in `target/`.
            match std::fs::metadata(path) {
                Ok(meta) => report.notes.push(format!(
                    "fixture path={path} bytes={} synthesized_by=probe::synthesize_wav(1 s, 8 kHz mono 16-bit PCM sine)",
                    meta.len()
                )),
                Err(err) => report
                    .notes
                    .push(format!("fixture path={path} unreadable error={err}")),
            }
            // The order a real session in this client uses: the wrapper defers
            // the control set until libmpv delivers FILE_LOADED, then sends it in
            // issue order. Whether the test environment's libmpv accepted the deferred set
            // is a fact this run records (errors / control-flush lines), not an
            // assumption.
            report
                .items
                .push(run_item(path, ControlPhase::DeferredUntilFileLoaded));
            if index == 0 {
                // The exploratory / negative orders, kept in the report instead of
                // hidden: controls raced the load reply, controls issued before
                // any reply, and controls issued after a START_FILE delivery.
                // None of these phases is established as the acceptance boundary.
                report
                    .items
                    .push(run_item(path, ControlPhase::AfterLoadReply));
                report
                    .items
                    .push(run_item(path, ControlPhase::BeforeLoadReply));
                report
                    .items
                    .push(run_item(path, ControlPhase::AfterPlaybackStarted));
            }
        }
    }
    report.items.push(run_item(
        "/nonexistent/plex-m4-missing-item.mkv",
        ControlPhase::AfterLoadReply,
    ));
    report.items.push(run_item("dvd://1", ControlPhase::None));
    report
}
