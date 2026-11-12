//! Display-free smoke session assembly (PLAN M4 deliverable 3).
//!
//! A [`SmokeReport`] has three explicitly labelled sections so a reader can never
//! confuse them:
//!
//! * `build` - what `build.rs` proved about the installed libmpv at compile time
//!   (pkg-config version, includes, link paths, binding file, FFI count).
//! * `mock` - a [`MockLink`] driven through exactly the [`PlayerSession`] calls a
//!   UI frame loop makes. Every line is a **wrapper** fact: the exact call this
//!   layer would have sent and the reply it correlated to. libmpv is not
//!   consulted, so nothing here is libmpv evidence.
//! * `probe` - the real-libmpv run from [`crate::probe`]: `vo=null`/`ao=null`,
//!   one `mpv_handle` per item, async commands with reply tokens,
//!   `mpv_wait_event(handle, 0)` pumped from a wakeup-driven loop. Only this
//!   section can support an "observed" claim.
//!
//! No window is created, no display server or audio device is contacted, no Plex
//! server/account/token is touched, and the only media read is the generated
//! fixture paths passed in.

use crate::commands::PropertyValue;
use crate::engine::PlayerSession;
use crate::events::{EndReason, PlayerError, PlayerEvent};
use crate::mock::{end_file_event, property_event, reply_event, MockLink};
use crate::probe::ProbeReport;

/// The source the scripted mock session pretends to play.
///
/// A plausible path, **not** a file this session reads: the mock does no I/O, so
/// this shows the wrapper's `loadfile` argument without implying libmpv accepted
/// it.
pub const MOCK_SOURCE: &str = "/media/plex-m4-smoke-item.mkv";

/// One scripted mock session.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MockRun {
    /// Source handed to [`PlayerSession::open`].
    pub source: String,
    /// `open` / `open-rejected`.
    pub outcome: String,
    /// Exact recorded calls, in call order (`set-property token=N …`, `command …`).
    pub calls: Vec<String>,
    /// Recorded `observe` requests (`observe <name> <format> token=N`).
    pub observations: Vec<String>,
    /// Requests the session issued here (`token=N <label>`).
    pub issued: Vec<String>,
    /// Delivered events, in delivery order ([`PlayerEvent::label`]).
    pub events: Vec<String>,
    /// Errors the layer reported.
    pub errors: Vec<String>,
    /// Replies that could not be attributed to a request.
    pub unmatched: Vec<String>,
    /// Cumulative observation counts (`name:count`).
    pub observed_counts: Vec<String>,
    /// `key=value` / `key=<none>` final state fields.
    pub state: Vec<String>,
    /// Pump reports (`pump#N drained=.. wakeups=.. other=.. exhausted=..`).
    pub pumps: Vec<String>,
}

impl MockRun {
    /// Machine-readable lines, prefixed `mock`.
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        let mut out = vec![
            format!("mock source={}", self.source),
            format!("mock outcome={}", self.outcome),
            "mock note every mock line is this wrapper's own behaviour; libmpv was not consulted"
                .to_owned(),
        ];
        out.extend(
            self.observations
                .iter()
                .map(|l| format!("mock observe {l}")),
        );
        out.extend(self.calls.iter().map(|l| format!("mock call {l}")));
        out.extend(self.issued.iter().map(|l| format!("mock issued {l}")));
        out.extend(self.pumps.iter().map(|l| format!("mock {l}")));
        out.extend(self.events.iter().map(|l| format!("mock event {l}")));
        out.extend(self.errors.iter().map(|l| format!("mock error {l}")));
        out.extend(self.unmatched.iter().map(|l| format!("mock unmatched {l}")));
        out.extend(
            self.observed_counts
                .iter()
                .map(|l| format!("mock observed-count {l}")),
        );
        out.extend(self.state.iter().map(|l| format!("mock state {l}")));
        out
    }
}

/// Drive one [`MockLink`] session exactly like the UI frame loop will.
///
/// Replies are scripted for the tokens the session actually allocated (after
/// issue), which is what makes the recorded call list meaningful: it shows the
/// token order `open` produced and that each control correlated to its own call.
#[must_use]
pub fn run_mock_session() -> MockRun {
    let mut run = MockRun {
        source: MOCK_SOURCE.to_owned(),
        ..MockRun::default()
    };
    let mut session = match PlayerSession::open(MockLink::new(), MOCK_SOURCE) {
        Ok(session) => session,
        Err(err) => {
            run.outcome = "open-rejected".to_owned();
            run.errors.push(format!("code={} {err}", err.code()));
            return run;
        }
    };
    run.outcome = "open".to_owned();
    run.observations = session
        .link()
        .observations()
        .iter()
        .map(|(name, format, token)| format!("{name} {format} token={token}"))
        .collect();

    let controls: [Result<(u64, String), PlayerError>; 6] = [
        session
            .set_volume(80.0)
            .map(|t| (t, "set volume = double 80".to_owned())),
        session
            .set_pause(true)
            .map(|t| (t, "set pause = flag".to_owned())),
        session
            .seek(0.5, crate::commands::SeekMode::Absolute)
            .map(|t| (t, "seek 0.5 absolute".to_owned())),
        session
            .seek(0.25, crate::commands::SeekMode::Relative)
            .map(|t| (t, "seek 0.25 relative".to_owned())),
        session
            .select_audio(&crate::tracks::TrackSelection::Auto)
            .map(|t| (t, "set audio-id = string auto".to_owned())),
        session
            .select_subtitle(&crate::tracks::TrackSelection::None)
            .map(|t| (t, "set sub-id = string no".to_owned())),
    ];
    for outcome in &controls {
        match outcome {
            Ok((token, label)) => run.issued.push(format!("token={token} {label}")),
            Err(err) => run.errors.push(format!("code={} {err}", err.code())),
        }
    }

    let outstanding: Vec<u64> = session.pending_tokens();
    for token in outstanding {
        session
            .link_mut()
            .push_event(reply_event(token, crate::mock_success_code()));
    }
    session
        .link_mut()
        .push_event(property_event("volume", PropertyValue::Double(80.0)));
    session
        .link_mut()
        .push_event(property_event("pause", PropertyValue::Flag(true)));
    session.link_mut().push_event(property_event(
        "media-title",
        PropertyValue::Str("generated mock item".to_owned()),
    ));
    session
        .link_mut()
        .push_event(end_file_event(EndReason::Eof, crate::mock_success_code()));

    let report = session.pump(64);
    run.pumps.push(format!(
        "pump#1 drained={} wakeups={} other={} exhausted={}",
        report.drained, report.wakeups, report.other_events, report.budget_exhausted
    ));
    run.events
        .extend(session.drain_events().iter().map(PlayerEvent::label));
    run.unmatched = session
        .unmatched_replies()
        .iter()
        .map(|(token, reason)| format!("token={token} reason={reason}"))
        .collect();
    let state = session.state();
    run.observed_counts = state
        .observed_counts
        .iter()
        .map(|(name, count)| format!("{name}:{count}"))
        .collect();
    run.state = vec![
        format!("volume={}", opt(state.volume.map(|v| v.to_string()))),
        format!("paused={}", opt(state.paused.map(|v| v.to_string()))),
        format!("media_title={}", opt(state.media_title.clone())),
        format!("audio_id={}", opt(state.audio_id.clone())),
        format!("sub_id={}", opt(state.sub_id.clone())),
        format!("time_pos={}", opt(state.time_pos.map(|v| v.to_string()))),
        format!("duration={}", opt(state.duration.map(|v| v.to_string()))),
        format!(
            "eof_reached={}",
            opt(state.eof_reached.map(|v| v.to_string()))
        ),
        format!("unrecognized={}", state.unrecognized.join("|")),
    ];
    if let Some(err) = session.item_error() {
        run.errors.push(format!("code={} {err}", err.code()));
    }
    if let Err(err) = session.close() {
        run.errors.push(format!("code={} {err}", err.code()));
    }
    run.calls = session.link().calls().to_vec();
    run
}

fn opt(value: Option<String>) -> String {
    value.unwrap_or_else(|| "<none>".to_owned())
}

/// The whole display-free smoke report.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SmokeReport {
    /// The scripted mock control session (wrapper facts only).
    pub mock: MockRun,
    /// The real-libmpv probe run (the only section that observed libmpv).
    pub probe: ProbeReport,
}

impl SmokeReport {
    /// Flat machine-readable lines: `build …`, `runtime …`, `name …`,
    /// `item[...] …`, `note …`, `mock …`.
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        let mut out = self.probe.lines();
        out.extend(self.mock.lines());
        out
    }

    /// `true` when the probe section really reached libmpv for at least one item.
    ///
    /// A probe whose items are all `libmpv-unavailable` or `initialize-failed`
    /// observed nothing, and a smoke caller must not treat it as a pass.
    #[must_use]
    pub fn libmpv_observed(&self) -> bool {
        self.probe.items.iter().any(|item| {
            item.classification != "libmpv-unavailable" && item.outcome != "initialize-failed"
        })
    }
}

/// Run the mock session plus the real display-free probe over `fixtures`.
#[must_use]
pub fn run_smoke(fixtures: &[&str]) -> SmokeReport {
    SmokeReport {
        mock: run_mock_session(),
        probe: crate::probe::run_probe(fixtures),
    }
}
