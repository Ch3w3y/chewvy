//! The app-owned GL render session used by the smoke modes and the software-GL
//! harness. (The app itself renders straight into its window: see
//! [`crate::player`].)
//!
//! * the app creates the EGL/GL context ([`crate::gl::SharedGl`]) and the app's
//!   own FBO + texture ([`crate::gl::fbo::AppFbo`]);
//! * the app creates the libmpv render context with
//!   `plex_player::render_link::RenderContext::create(.., RenderApi::OpenGL, ..)`
//!   and hands it `EglResolver` (`eglGetProcAddress`) as
//!   `mpv_opengl_init_params.get_proc_address_ctx` (`render_gl.h:36-44`);
//! * frames are driven by libmpv's **update callback** (`render.h:634-661`):
//!   the callback only sends on a channel, and the loop below blocks on that
//!   channel between `mpv_render_context_update()` calls. There is no busy poll
//!   and no timer-based redraw;
//! * every rendered frame is verified from the app's own side: FBO completeness
//!   is re-checked after `mpv_render_context_render` and the colour attachment
//!   is read back through `glReadPixels` and summarised ([`CapturedFrame`]).
//!   Every number the harness prints from those summaries is a **software-GL
//!   (llvmpipe) number**: hardware decode and desktop performance are not
//!   measured here;
//! * with `MPV_RENDER_PARAM_ADVANCED_CONTROL` enabled the loop calls
//!   `mpv_render_context_report_swap` after each render, as `render.h:711-722`
//!   requires when advanced control is used.
//!
//! ## Ownership and teardown
//!
//! The session **owns** the `FfiPlayer`, the render context and the FBO, and the
//! field order encodes the teardown order: `ctx` (which frees the
//! `mpv_render_context`) is dropped first, then the FBO/texture it draws into,
//! then the mpv core. That is the ordering `render.h:122-123` states (a render
//! context must be freed before its core is destroyed). Owning the player here,
//! rather than borrowing it, is also what keeps the app free to call
//! `PlayerLink::request`/`next_event` (`&mut self`) while the render context
//! that was created from it is alive - a `&'a FfiPlayer` borrow would forbid
//! exactly those calls.
//!
//! ## Subtitle ownership
//!
//! Subtitle drawing stays with mpv/libass, this app owns selection and its own
//! egui transport/OSD controls, and neither this module nor [`crate::overlay`]
//! draws subtitle glyphs (`docs/CAPABILITIES.md`).
//!
//! What the software-GL harness measured: mpv's OSD text layer **does** reach
//! `AppFbo` (`SUBOSD verdict=mpv-text-layer-did-reach-app-fbo extra_hashes=3
//! max_non_black_delta=116`), while `sub-id=no`, `1`, `auto` and mpv's untouched
//! default all produced `SUBDIFF hashes_only_in_case=0 max_non_black_delta=0`
//! against both controls. **No run has measured a libass glyph layer in this
//! FBO**, and a negative `SUBDIFF` cannot distinguish "mpv drew it somewhere
//! other than our FBO" from "mpv never displayed the fixture event".
//!
//! What the wiring here does is *support either outcome*: it uploads the app
//! FBO to egui and draws transport/OSD controls on top of it
//! ([`crate::overlay`]) without drawing subtitle glyphs itself.

use std::collections::BTreeSet;
use std::fs;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use plex_core::secret::SecretString;
use plex_player::controls::load_request;
use plex_player::events::{EndReason, EventKind};
use plex_player::link::PlayerLink;
use plex_player::mpv_link::FfiPlayer;
use plex_player::render_link::{RenderApi, RenderContext, RenderOptions, MPV_RENDER_UPDATE_FRAME};
use plex_player::tracks::{select_subtitle, TrackSelection};

use crate::gl::fbo::CapturedFrame;
use crate::gl::{AppFbo, SharedGl};

/// Why one step of a render session failed.
///
/// Kept as a plain struct (no `thiserror` in `plex-app`: `Cargo.lock` is frozen
/// and `--locked` gates may not add dependency edges) with the failing step and
/// the underlying message, so a GL or libmpv failure is always attributable to
/// the call that produced it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionError {
    /// The step that failed, e.g. `mpv_render_context_create(OpenGL)`.
    pub step: String,
    /// The underlying `PlayerError`/`GlError` message.
    pub detail: String,
}

impl SessionError {
    fn new(step: impl Into<String>, detail: impl std::fmt::Display) -> Self {
        Self {
            step: step.into(),
            detail: detail.to_string(),
        }
    }
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.step, self.detail)
    }
}

impl std::error::Error for SessionError {}

/// Peak RSS of this process from `/proc/self/status` (`VmHWM`), in KiB.
///
/// The same measurement the software-render test uses, so the two sets of
/// numbers are comparable. Returns 0 on a host
/// without `/proc/self/status` (and says so in the report field).
#[must_use]
pub fn peak_rss_kib() -> u64 {
    let status = fs::read_to_string("/proc/self/status").unwrap_or_default();
    status
        .lines()
        .filter(|l| l.starts_with("VmHWM:"))
        .filter_map(|l| {
            l.split_whitespace()
                .nth(1)
                .and_then(|v| v.parse::<u64>().ok())
        })
        .next()
        .unwrap_or(0)
}

/// How a session is opened.
///
/// `block_for_target_time` is part of the measurement contract: pacing numbers
/// are only meaningful
/// when this is named.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionConfig {
    /// App FBO size in pixels (the video surface).
    pub width: i32,
    /// App FBO height in pixels.
    pub height: i32,
    /// `MPV_RENDER_PARAM_BLOCK_FOR_TARGET_TIME` for every render call.
    pub block_for_target_time: bool,
    /// Stop after this many rendered frames.
    pub max_frames: usize,
    /// Upper bound on the whole frame loop.
    pub deadline: Duration,
    /// Ask libmpv for advanced control (`report_swap` then required).
    pub advanced_control: bool,
    /// `hwdec` value to request before init (`no` in the software-GL harness,
    /// where hardware decode is neither measured nor claimed).
    pub hwdec: &'static str,
    /// `audio` value to request before init.
    pub audio: &'static str,
    /// `osd-level` value to request before init.
    ///
    /// Not cosmetic. On this host's mpv (v0.41.0-dirty) `mpv --list-options`
    /// reports `--osd-level Choices: 0 1 2 3 (default: 1)`, and the installed
    /// man page documents level 0 as "OSD completely disabled (subtitles
    /// only)". This app owns its own
    /// transport/OSD layer in egui ([`crate::overlay`]), so it asks for level
    /// 0: that level is the documented way to drop mpv's OSD text layer while
    /// mpv's subtitle layer stays active, which is what the subtitle-ownership
    /// comparison in `tests/gl_harness.rs` would have to rely on to attribute a
    /// difference to a subtitle layer rather than to OSD text.
    ///
    /// **Measured in the direction that matters**: mpv's OSD
    /// text layer really is composited into *this* app's FBO - the harness's
    /// `control-osd-level-2` positive control measured 3 distinct hashes /
    /// 40981 lit pixels against the osd-level-0 control's 2 / 40865
    /// (`SUBOSD verdict=mpv-text-layer-did-reach-app-fbo`), so the OSD confound this option
    /// removes is measured, not assumed. Still **[U]**: whether mpv's *subtitle*
    /// layer lands in this FBO as well (see the module doc and
    /// `docs/CAPABILITIES.md`).
    pub osd_level: &'static str,
    /// Pre-init `http-header-fields` for a source that needs the account token
    /// (live Plex playback). A [`SecretString`]: read only through `expose` when
    /// it is handed to libmpv, so it never reaches `Debug`.
    pub http_header: Option<SecretString>,
    /// Pre-init `start` (mpv accepts a seconds text). It must be set before
    /// init: an option set after `mpv_initialize` is rejected.
    pub start: Option<String>,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            width: 320,
            height: 240,
            block_for_target_time: true,
            max_frames: 60,
            deadline: Duration::from_secs(10),
            advanced_control: true,
            hwdec: "no",
            audio: "no",
            osd_level: "0",
            http_header: None,
            start: None,
        }
    }
}

/// What one [`MpvGlSession::drive`] run measured.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameLoopReport {
    /// Fixture label (also the report line key).
    pub fixture: String,
    /// Render calls that produced a frame.
    pub frames: usize,
    /// Loop-iteration intervals: microseconds between the *return* of the
    /// previous render call and the start of this one. Excludes time spent
    /// inside `mpv_render_context_render` (measured separately in [`Self::
    /// render_us`]), so these are **not** display pacing - see
    /// [`Self::frames_per_second`].
    pub intervals_us: Vec<u64>,
    /// Microseconds spent inside `mpv_render_context_render`, per frame.
    pub render_us: Vec<u64>,
    /// Start-to-start intervals, microseconds: from the instant frame N's render
    /// call began to the instant frame N+1's render call began.
    ///
    /// Unlike [`Self::intervals_us`] this **includes** the time inside
    /// `mpv_render_context_render` plus the readback/`report_swap` work that
    /// follows it, so it is the pacing an actual viewer of this loop would see
    /// and it is the only figure that may be quoted as frame pacing (see
    /// [`Self::paced_frames_per_second`]).
    pub frame_gap_us: Vec<u64>,
    /// `FrameInfo::target_time_us` of each rendered frame, in the
    /// `mpv_get_time_us()` clock, keeping only the frames for which libmpv
    /// reported a non-zero target time (`render.h:539-552`: 0 means "no valid
    /// target time").
    ///
    /// This is what lets a run state *which part of the media timeline it
    /// actually displayed* - e.g. whether a harness run covered a subtitle
    /// window at all, instead of just counting frames.
    pub target_time_us: Vec<i64>,
    /// Distinct [`CapturedFrame`] hashes seen (per-frame readback of our FBO).
    pub distinct_hashes: BTreeSet<u64>,
    /// Hash of the first and last captured frame.
    pub first_hash: u64,
    /// Last captured frame (full summary: hash, non-black count, mean red).
    pub last_frame: Option<CapturedFrame>,
    /// Highest `non_black` pixel count of any captured frame in this run (0 if
    /// no frame was captured).
    ///
    /// The subtitle-ownership comparison needs a *pixel* measure, not a frame
    /// count: on a constant-colour fixture every undecorated frame has the same
    /// pixels, so what distinguishes a libass-drawn frame is how many pixels
    /// are lit, not how many frames there were.
    pub max_non_black: u64,
    /// `mpv_render_context_update()` wake callbacks observed.
    pub update_callbacks: u64,
    /// `MPV_RENDER_PARAM_ADVANCED_CONTROL` actually recorded at creation.
    pub advanced_control: bool,
    /// Peak RSS at the end of the run, KiB.
    pub peak_rss_kib: u64,
    /// Frames skipped because `FrameInfo` was empty (`is_empty()`).
    pub empty_frame_infos: usize,
    /// Idle waits (no `MPV_RENDER_UPDATE_FRAME`) before the loop ended.
    pub idle_waits: u32,
}

impl FrameLoopReport {
    fn percentile(v: &[u64], p: u64) -> u64 {
        if v.is_empty() {
            return 0;
        }
        let mut v = v.to_vec();
        v.sort_unstable();
        let idx = ((v.len() as u64).saturating_sub(1)) * p / 100;
        v.get(usize::try_from(idx).unwrap_or(0))
            .copied()
            .unwrap_or(0)
    }

    fn mean_us(&self, v: &[u64]) -> u64 {
        if v.is_empty() {
            return 0;
        }
        v.iter().sum::<u64>() / v.len() as u64
    }

    /// Frames per second derived from [`Self::intervals_us`] - i.e. loop
    /// iteration overhead, with the render call's own time subtracted out.
    /// This is **not** measured display pacing and must not be quoted as frame
    /// pacing; the render-call duration is
    /// `render_mean_ms` / `render_max_ms` in [`Self::report_line`].
    #[must_use]
    pub fn frames_per_second(&self) -> f64 {
        if self.frames < 2 || self.intervals_us.is_empty() {
            return 0.0;
        }
        (self.frames - 1) as f64 * 1e6 / self.intervals_us.iter().sum::<u64>().max(1) as f64
    }

    /// Frames per second from [`Self::frame_gap_us`] (render-call start to
    /// render-call start). This **is** measured pacing of this loop, unlike
    /// [`Self::frames_per_second`].
    #[must_use]
    pub fn paced_frames_per_second(&self) -> f64 {
        if self.frames < 2 || self.frame_gap_us.is_empty() {
            return 0.0;
        }
        self.frame_gap_us.len() as f64 * 1e6 / self.frame_gap_us.iter().sum::<u64>().max(1) as f64
    }

    /// `max - min` over [`Self::target_time_us`] in microseconds; 0 when fewer
    /// than two target times were reported.
    #[must_use]
    pub fn target_time_span_us(&self) -> i64 {
        let (Some(min), Some(max)) = (
            self.target_time_us.iter().min().copied(),
            self.target_time_us.iter().max().copied(),
        ) else {
            return 0;
        };
        max.saturating_sub(min)
    }

    /// How many rendered frames have a display time inside
    /// `[first + start_us, first + end_us]`, where `first` is this run's first
    /// sampled target time.
    ///
    /// Media-relative by construction (the target-time clock is absolute, but
    /// `mpv_render_context_render` blocks until it, so the *differences* are
    /// media-relative display offsets). Returns 0 when no target time was
    /// reported, i.e. when coverage is unknown rather than absent.
    #[must_use]
    pub fn frames_in_display_window(&self, start_us: i64, end_us: i64) -> usize {
        let Some(&first) = self.target_time_us.first() else {
            return 0;
        };
        self.target_time_us
            .iter()
            .filter(|t| {
                let off = t.saturating_sub(first);
                off >= start_us && off <= end_us
            })
            .count()
    }

    /// One machine-readable line per run, so a harness log
    /// (`scripts/mpv-gl-harness.sh`) carries the numbers without reinterpretation.
    #[must_use]
    pub fn report_line(&self) -> String {
        let last = self
            .last_frame
            .as_ref()
            .map_or_else(|| "none".to_string(), CapturedFrame::report_line);
        format!(
            "GL fixture={} frames={} frames_per_s={:.2} mean_ms={:.2} p50_ms={:.2} p95_ms={:.2} max_ms={:.2} render_mean_ms={:.2} render_max_ms={:.2} pacing_frames_per_s={:.2} pacing_mean_ms={:.2} pacing_p50_ms={:.2} pacing_p95_ms={:.2} pacing_max_ms={:.2} target_time_frames={} target_time_span_s={:.3} distinct_frame_hashes={} first_hash={:016x} max_non_black={} last={} update_callbacks={} advanced_control={} empty_frame_infos={} idle_waits={} peak_rss_kib={}",
            self.fixture,
            self.frames,
            self.frames_per_second(),
            self.mean_us(&self.intervals_us) as f64 / 1e3,
            Self::percentile(&self.intervals_us, 50) as f64 / 1e3,
            Self::percentile(&self.intervals_us, 95) as f64 / 1e3,
            self.intervals_us.iter().max().copied().unwrap_or(0) as f64 / 1e3,
            self.mean_us(&self.render_us) as f64 / 1e3,
            self.render_us.iter().max().copied().unwrap_or(0) as f64 / 1e3,
            self.paced_frames_per_second(),
            self.mean_us(&self.frame_gap_us) as f64 / 1e3,
            Self::percentile(&self.frame_gap_us, 50) as f64 / 1e3,
            Self::percentile(&self.frame_gap_us, 95) as f64 / 1e3,
            self.frame_gap_us.iter().max().copied().unwrap_or(0) as f64 / 1e3,
            self.target_time_us.len(),
            (self.target_time_span_us() as f64) / 1e6,
            self.distinct_hashes.len(),
            self.first_hash,
            self.max_non_black,
            last,
            self.update_callbacks,
            self.advanced_control,
            self.empty_frame_infos,
            self.idle_waits,
            self.peak_rss_kib,
        )
    }
}

/// What one [`MpvGlSession::step_frame`] measured: the render gate **and** the
/// frame it produced.
///
/// `had_frame_update` is the honest split between "the player gave us a frame"
/// and "we rendered whatever the player currently has". A step with
/// `had_frame_update=false` is still a measured render, but its pixels are not
/// a player frame and no log line may call them one.
#[derive(Clone, Debug)]
pub struct FrameStepReport {
    /// The player-frame summary (hash, lit pixels) from the app FBO readback.
    pub summary: CapturedFrame,
    /// The exact RGBA bytes that summary was computed from - the overlay's
    /// texture source ([`crate::overlay::VideoTexture`]).
    pub pixels: Vec<u8>,
    /// `true` when `mpv_render_context_update` reported a frame (and its
    /// `NEXT_FRAME_INFO` was non-empty) before this render.
    pub had_frame_update: bool,
    /// `true` when a frame update arrived but its `NEXT_FRAME_INFO` was empty,
    /// so the step waited out that update and rendered after the poll budget.
    pub empty_frame_info: bool,
    /// How many bounded waits the gate used.
    pub waits: u32,
    /// Wall time of the `mpv_render_context_render` + readback itself.
    pub render_time: Duration,
    /// Wall time the gate spent waiting before rendering.
    pub waited: Duration,
}

impl FrameStepReport {
    /// Machine-readable gate line, same style as the software-GL harness lines.
    #[must_use]
    pub fn gate_line(&self) -> String {
        format!(
            "FRAMEGATE had_frame_update={} empty_frame_info={} waits={} waited_us={} render_us={} video_hash={:016x} video_non_black={}",
            self.had_frame_update,
            self.empty_frame_info,
            self.waits,
            self.waited.as_micros(),
            self.render_time.as_micros(),
            self.summary.hash,
            self.summary.non_black,
        )
    }
}

/// The app-side OpenGL render session: context + FBO + libmpv render context.
///
/// Borrows [`SharedGl`] (so the context must outlive it) and owns everything
/// mpv-side. Everything here must run on the thread that has the GL context
/// current (`render.h:68-70`); [`SharedGl::make_current`] is called before each
/// render for exactly that reason.
pub struct MpvGlSession<'a> {
    /// The render context is dropped first (frees the `mpv_render_context`),
    /// then the FBO it draws into, then the mpv core - `render.h:122-123`.
    ctx: RenderContext,
    fbo: AppFbo,
    player: FfiPlayer,
    gl: &'a SharedGl,
    rx: Receiver<()>,
    cfg: SessionConfig,
    last_frame: Option<CapturedFrame>,
    end_file: Option<(EndReason, i32)>,
}

impl MpvGlSession<'_> {
    /// Create the core, the OpenGL render context and the app FBO.
    ///
    /// The context must already be current on this thread ([`SharedGl::attach`]
    /// / [`SharedGl::surfaceless`] guarantee that).
    pub fn open(gl: &SharedGl, cfg: SessionConfig) -> Result<MpvGlSession<'_>, SessionError> {
        let (tx, rx) = mpsc::channel::<()>();
        let mut player =
            FfiPlayer::new(Some(tx.clone())).map_err(|e| SessionError::new("mpv_create", e))?;
        // Options must be set before `mpv_initialize` (`client.h` `mpv_set_option`);
        // `vo=libmpv` is what routes video to this render context.
        player
            .set_option("audio", cfg.audio)
            .map_err(|e| SessionError::new("set_option audio", e))?;
        player
            .set_option("hwdec", cfg.hwdec)
            .map_err(|e| SessionError::new("set_option hwdec", e))?;
        player
            .set_option("vo", "libmpv")
            .map_err(|e| SessionError::new("set_option vo", e))?;
        if !cfg.osd_level.is_empty() {
            player
                .set_option("osd-level", cfg.osd_level)
                .map_err(|e| SessionError::new("set_option osd-level", e))?;
        }
        // A token header is a pre-init option like the rest: `expose` is the only
        // read and the value goes straight to `mpv_set_option_string`.
        if let Some(header) = &cfg.http_header {
            player
                .set_option("http-header-fields", header.expose())
                .map_err(|e| SessionError::new("set_option http-header-fields", e))?;
        }
        if let Some(start) = &cfg.start {
            player
                .set_option("start", start.as_str())
                .map_err(|e| SessionError::new("set_option start", e))?;
        }
        player
            .initialize_link()
            .map_err(|e| SessionError::new("mpv_initialize", e))?;

        gl.make_current()
            .map_err(|e| SessionError::new("eglMakeCurrent (before create)", e))?;
        // `tx` is moved here: the only remaining sender is the wake closure, so
        // no further clone is needed (the session reads `rx` only).
        let wake_tx = tx;
        let ctx = RenderContext::create(
            &player,
            RenderApi::OpenGL,
            gl.mpv_resolver(),
            // The wake closure only schedules: it sends on a channel and never
            // calls libmpv or GL (`render.h:634-661`).
            Some(Box::new(move || {
                let _ = wake_tx.send(());
            })),
            cfg.advanced_control,
            // The harness has no window-server display to hand libmpv.
            None,
        )
        .map_err(|e| SessionError::new("mpv_render_context_create(OpenGL)", e))?;
        if cfg.advanced_control && !ctx.advanced_control() {
            return Err(SessionError::new(
                "MPV_RENDER_PARAM_ADVANCED_CONTROL",
                "requested advanced control but libmpv did not record it",
            ));
        }

        let fbo = AppFbo::create(gl.gl(), cfg.width, cfg.height)
            .map_err(|e| SessionError::new("AppFbo::create", e))?;

        Ok(MpvGlSession {
            ctx,
            fbo,
            player,
            gl,
            rx,
            cfg,
            last_frame: None,
            end_file: None,
        })
    }

    /// The app-owned FBO target handed to libmpv.
    #[must_use]
    pub fn fbo(&self) -> &AppFbo {
        &self.fbo
    }

    /// The libmpv render context, for diagnostics (`callback_count`, `api`).
    #[must_use]
    pub fn render_context(&self) -> &RenderContext {
        &self.ctx
    }

    /// The last frame captured from the app FBO, if any render has happened.
    #[must_use]
    pub fn last_frame(&self) -> Option<&CapturedFrame> {
        self.last_frame.as_ref()
    }

    /// The `end-file` a [`Self::step_frame`] drained, when the player finished
    /// (or could not play) the loaded file.
    ///
    /// [`Self::step_frame`] drains libmpv's queue on every step, so without
    /// recording it here a caller could not tell "no frame yet" from "the file
    /// ended".
    #[must_use]
    pub fn end_file(&self) -> Option<(EndReason, i32)> {
        self.end_file
    }

    /// Wait (bounded) for libmpv to present a renderable frame, then render
    /// exactly **one** frame, returning its measured summary **and the RGBA
    /// bytes it was measured from**.
    ///
    /// The gate is not decoration. `mpv_render_context_render` into an app FBO
    /// is not an error when the player has no decoded frame yet - it draws the
    /// player's empty current image, i.e. **black pixels** that hash like a
    /// frame. The earlier single-call entry point did exactly that: a
    /// smoke run measured `video_non_black=0` on every frame while the
    /// composite looked alive only because egui had drawn its own overlay on
    /// top. [`Self::drive`] never
    /// had that defect because it renders **only** after
    /// `mpv_render_context_update` reports `MPV_RENDER_UPDATE_FRAME` and a
    /// non-empty next-frame-info - the loop that produced
    /// `non_black=76800` in the harness. This method is
    /// that same gate, expressed as one bounded step for an event-loop caller
    /// that cannot block for a whole fixture.
    ///
    /// `poll` is the ceiling on waiting, not a promise: with no decoded frame
    /// inside `poll` the step still renders (so a stalled player still shows
    /// the app's UI) but reports `had_frame_update=false` and the caller must
    /// call that frame what it is - a black frame, not a player frame.
    pub fn step_frame(&mut self, poll: Duration) -> Result<FrameStepReport, SessionError> {
        let started = Instant::now();
        let deadline = started + poll;
        let mut waits = 0u32;
        let mut had_frame_update = false;
        let mut empty_frame_info = false;
        loop {
            // Drain libmpv's event queue first: the core waker and the render
            // wake share one channel (`open`), and `end-file`/reply events must
            // not sit in the queue while this step waits.
            while let Some(event) = self.player.next_event() {
                if event.kind == EventKind::EndFile {
                    self.end_file = event.end_file;
                }
            }
            let update = self
                .ctx
                .take_update()
                .map_err(|e| SessionError::new("mpv_render_context_update (step)", e))?;
            if update.has_frame() {
                let info = self.ctx.next_frame_info().map_err(|e| {
                    SessionError::new("mpv_render_context_get_info(NEXT_FRAME_INFO)", e)
                })?;
                if info.as_ref().is_some_and(|f| f.is_empty()) {
                    // Same rule `drive` applies: an empty NEXT_FRAME_INFO is not
                    // a frame, and rendering it would be measured as one.
                    empty_frame_info = true;
                } else {
                    had_frame_update = true;
                    break;
                }
            }
            if Instant::now() >= deadline {
                break;
            }
            waits = waits.saturating_add(1);
            match self.rx.recv_timeout(Duration::from_millis(5)) {
                Ok(()) | Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        let (captured, pixels, render_time) = self.render_one()?;
        Ok(FrameStepReport {
            summary: captured,
            pixels,
            had_frame_update,
            empty_frame_info,
            waits,
            render_time,
            waited: started.elapsed(),
        })
    }

    /// Issue one already-mapped libmpv request with the caller's command token.
    ///
    /// A control request reaches this method unchanged: it does not reinterpret,
    /// clamp or re-map it, so the request the caller asked for is the request
    /// `libmpv` receives (`plex_player::controls` is the only place a control
    /// becomes a command). Tokens label the async reply only, so a command token
    /// can be told apart from the [`Self::load`] token.
    ///
    /// # Errors
    ///
    /// A [`SessionError`] naming the submit step when `libmpv` refuses the
    /// submission (no error means *accepted for submission*: `mpv_command_node_async`
    /// is asynchronous, so the observed effect arrives later as a reply/event).
    pub fn apply(
        &mut self,
        request: &plex_player::Request,
        token: u64,
    ) -> Result<(), SessionError> {
        self.player
            .request(token, request)
            .map_err(|e| SessionError::new(format!("async {}", request.label()), e))
    }

    /// `loadfile <path> replace` through the player's control vocabulary
    /// ([`plex_player::controls::load_request`]) so the app issues the same
    /// request shape a real libmpv run confirmed, not a private one.
    pub fn load(&mut self, raw_source: &str) -> Result<(), SessionError> {
        let request = load_request(raw_source).map_err(|e| SessionError::new("load_request", e))?;
        self.player
            .request(1, &request)
            .map_err(|e| SessionError::new(format!("loadfile {raw_source}"), e))
    }

    /// Select/disable subtitles with `sub-id` (`props::SUB_ID`, the player's vocabulary
    /// entry for it) through `mpv_set_property_async`.
    ///
    /// The subtitle decision is applied at run time here instead of through the
    /// pre-init option list. What is measured and what is inferred:
    ///
    /// * measured (by the recipe in
    ///   `crates/plex-player/tests/render_sw_session.rs`): pre-init
    ///   `mpv_set_option_string("vid", "1")` -> 0 (accepted) while
    ///   `"vid", "yes"` -> -7 (`MPV_ERROR_OPTION_ERROR`);
    /// * inferred, not measured: that `sid`/`sub-id` would be rejected from the
    ///   pre-init option list too. No `mpv_set_option_string("sid", ...)` run is
    ///   recorded anywhere in the tree, so this stays an assumption from the same
    ///   class of per-playback track-selection option, and it is only an
    ///   assumption about *how* the option is set - applying `sub-id` as a run-time
    ///   property is the behaviour the player's property vocabulary actually exercises.
    pub fn apply_subtitle(
        &mut self,
        selection: &TrackSelection,
        token: u64,
    ) -> Result<(), SessionError> {
        let request =
            select_subtitle(selection).map_err(|e| SessionError::new("select_subtitle", e))?;
        self.player
            .request(token, &request)
            .map_err(|e| SessionError::new(format!("set sub-id={}", selection.value()), e))
    }

    /// One render: context current, `mpv_render_context_render` into our FBO,
    /// FBO completeness re-check, `glReadPixels` capture, `report_swap`.
    fn render_one(&mut self) -> Result<(CapturedFrame, Vec<u8>, Duration), SessionError> {
        self.gl
            .make_current()
            .map_err(|e| SessionError::new("eglMakeCurrent (before render)", e))?;
        let t0 = Instant::now();
        self.ctx
            .render(&RenderOptions {
                fbo: Some(self.fbo.target()),
                block_for_target_time: self.cfg.block_for_target_time,
                flip_y: false,
                skip_rendering: false,
            })
            .map_err(|e| SessionError::new("mpv_render_context_render", e))?;
        let render_time = t0.elapsed();

        // A broken app-side FBO is *not* reported by libmpv as a render error:
        // a probe showed it silently producing 0 frames, so the app checks it.
        let status = self.fbo.status(self.gl.gl());
        if !status.is_complete() {
            return Err(SessionError::new(
                "FBO completeness after render",
                format!(
                    "glCheckFramebufferStatus 0x{:04X} glGetError 0x{:04X}",
                    status.status, status.gl_error
                ),
            ));
        }

        let pixels = self
            .fbo
            .read_rgba(self.gl.gl())
            .map_err(|e| SessionError::new("glReadPixels (app FBO)", e))?;
        let captured = CapturedFrame::summarize(&pixels);
        self.last_frame = Some(captured.clone());

        if self.cfg.advanced_control {
            self.ctx
                .report_swap()
                .map_err(|e| SessionError::new("mpv_render_context_report_swap", e))?;
        }
        Ok((captured, pixels, render_time))
    }

    /// Drive the frame loop until `max_frames` frames, the deadline, or the
    /// stream ends (`idle_waits` limit reached after the final `end-file`).
    ///
    /// Wake-up is callback-driven: `rx.recv_timeout` is a blocking receive, only
    /// used when `mpv_render_context_update()` returned no `MPV_RENDER_UPDATE_FRAME`.
    pub fn drive(&mut self, fixture: &str) -> Result<FrameLoopReport, SessionError> {
        let mut report = FrameLoopReport {
            fixture: fixture.to_string(),
            frames: 0,
            intervals_us: Vec::new(),
            render_us: Vec::new(),
            frame_gap_us: Vec::new(),
            target_time_us: Vec::new(),
            distinct_hashes: BTreeSet::new(),
            first_hash: 0,
            last_frame: None,
            max_non_black: 0,
            update_callbacks: 0,
            advanced_control: self.ctx.advanced_control(),
            peak_rss_kib: 0,
            empty_frame_infos: 0,
            idle_waits: 0,
        };
        let deadline = Instant::now() + self.cfg.deadline;
        let mut last_render: Option<Instant> = None;
        let mut last_render_start: Option<Instant> = None;
        let mut idle_waits = 0u32;

        while Instant::now() < deadline && report.frames < self.cfg.max_frames {
            // Drain libmpv's event queue so replies/end-file are consumed
            // (the core waker feeds the same channel as the render callback).
            while self.player.next_event().is_some() {}

            let update = self
                .ctx
                .take_update()
                .map_err(|e| SessionError::new("mpv_render_context_update", e))?;
            report.update_callbacks = self.ctx.callback_count();
            if !update.has_frame() {
                if update.flags & !MPV_RENDER_UPDATE_FRAME != 0 {
                    return Err(SessionError::new(
                        "mpv_render_context_update",
                        format!("unexpected update flags {:#x}", update.flags),
                    ));
                }
                idle_waits = idle_waits.saturating_add(1);
                report.idle_waits = idle_waits;
                if idle_waits > 40 {
                    break;
                }
                match self.rx.recv_timeout(Duration::from_millis(50)) {
                    Ok(()) | Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break,
                }
                continue;
            }

            let info = self.ctx.next_frame_info().map_err(|e| {
                SessionError::new("mpv_render_context_get_info(NEXT_FRAME_INFO)", e)
            })?;
            if info.as_ref().is_some_and(|f| f.is_empty()) {
                report.empty_frame_infos += 1;
                continue;
            }

            // `frame_start` is taken *before* the render call so the gap is
            // start-to-start (pacing), while `last_render` is taken after it so
            // `intervals_us` stays what it always was: inter-iteration overhead.
            let frame_start = Instant::now();
            if let Some(prev) = last_render {
                report
                    .intervals_us
                    .push(prev.elapsed().as_micros().min(u128::from(u64::MAX)) as u64);
            }
            if let Some(prev_start) = last_render_start {
                report
                    .frame_gap_us
                    .push(prev_start.elapsed().as_micros().min(u128::from(u64::MAX)) as u64);
            }
            if let Some(f) = info.as_ref().filter(|f| f.has_target_time()) {
                report.target_time_us.push(f.target_time_us);
            }
            // The fixture loop only needs the summary; `drive` never shows a frame,
            // so the pixels are dropped here (and `step_frame` is what the window
            // host uses when it *does* show a frame).
            let (captured, _pixels, render_time) = self.render_one()?;
            last_render = Some(Instant::now());
            last_render_start = Some(frame_start);

            report.frames += 1;
            report
                .render_us
                .push(render_time.as_micros().min(u128::from(u64::MAX)) as u64);
            if report.frames == 1 {
                report.first_hash = captured.hash;
            }
            report.distinct_hashes.insert(captured.hash);
            report.max_non_black = report.max_non_black.max(captured.non_black);
            report.last_frame = Some(captured);
        }
        report.peak_rss_kib = peak_rss_kib();
        Ok(report)
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "tests may unwrap; product code keeps these lints on"
)]
mod tests {
    use super::*;
    use crate::gl::GlStrings;
    use plex_player::commands::{PropertyValue, Request};

    #[test]
    fn session_error_names_the_step_and_detail() {
        let e = SessionError::new("mpv_render_context_render", "target time blocked");
        assert_eq!(
            e.to_string(),
            "mpv_render_context_render: target time blocked"
        );
        assert_eq!(e.step, "mpv_render_context_render");
    }

    #[test]
    fn config_defaults_are_the_measured_settings() {
        let c = SessionConfig::default();
        // Pacing numbers must name these: block-for-target-time on, advanced
        // control on, no hardware decode (no /dev/dri in the headless test environment).
        assert!(c.block_for_target_time);
        assert!(c.advanced_control);
        assert_eq!(c.hwdec, "no");
        assert_eq!(c.audio, "no");
        // mpv's own OSD overlay is off: the egui overlay owns that layer and an
        // OSD toast landing in the app FBO would contaminate pixel measurement.
        assert_eq!(c.osd_level, "0");
        assert_eq!((c.width, c.height), (320, 240));
    }

    #[test]
    fn report_line_is_machine_readable_and_labels_a_black_frame() {
        let mut report = FrameLoopReport {
            fixture: "empty".to_string(),
            frames: 0,
            intervals_us: Vec::new(),
            render_us: Vec::new(),
            frame_gap_us: Vec::new(),
            target_time_us: Vec::new(),
            distinct_hashes: BTreeSet::new(),
            first_hash: 0,
            last_frame: None,
            max_non_black: 0,
            update_callbacks: 0,
            advanced_control: true,
            peak_rss_kib: 0,
            empty_frame_infos: 0,
            idle_waits: 0,
        };
        let line = report.report_line();
        assert!(line.starts_with("GL fixture=empty frames=0 "));
        assert!(line.contains("last=none"));
        assert!(line.contains("max_non_black=0 "));
        assert!(line.contains("frames_per_s=0.00"));
        report.max_non_black = 2;
        report.last_frame = Some(CapturedFrame::summarize(&[0u8; 8]));
        assert!(report.report_line().contains("non_black=0/2"));
        assert!(report.report_line().contains("max_non_black=2 "));
        assert_eq!(report.frames_per_second(), 0.0);
        assert_eq!(report.paced_frames_per_second(), 0.0);
        assert_eq!(report.target_time_span_us(), 0);
        assert_eq!(report.frames_in_display_window(200_000, 1_800_000), 0);
    }

    #[test]
    fn pacing_and_target_time_window_are_measured_separately() {
        // Two clocks, two answers: 1000 us of loop overhead between renders and
        // 40 000 us between render-call starts. Quoting the first as pacing
        // would claim 1000 fps on a loop that displays one frame every 40 ms.
        let mut report = FrameLoopReport {
            fixture: "math".to_string(),
            frames: 3,
            intervals_us: vec![1_000, 1_000],
            render_us: vec![38_000, 38_000],
            frame_gap_us: vec![40_000, 40_000],
            target_time_us: vec![10_000_000, 10_200_000, 12_000_000],
            distinct_hashes: BTreeSet::new(),
            first_hash: 0,
            last_frame: None,
            max_non_black: 0,
            update_callbacks: 0,
            advanced_control: true,
            peak_rss_kib: 0,
            empty_frame_infos: 0,
            idle_waits: 0,
        };
        assert_eq!(report.frames_per_second(), 1_000.0);
        assert_eq!(report.paced_frames_per_second(), 25.0);
        assert_eq!(report.target_time_span_us(), 2_000_000);
        assert_eq!(report.frames_in_display_window(200_000, 1_800_000), 1);
        let line = report.report_line();
        assert!(line.contains("pacing_frames_per_s=25.00 "), "{line}");
        assert!(line.contains("pacing_mean_ms=40.00 "), "{line}");
        assert!(line.contains("target_time_frames=3 "), "{line}");
        assert!(line.contains("target_time_span_s=2.000"), "{line}");
        report.frame_gap_us = Vec::new();
        assert_eq!(report.paced_frames_per_second(), 0.0);
        assert!(report.report_line().contains("pacing_frames_per_s=0.00"));
    }

    #[test]
    fn peak_rss_reads_vmhwm() {
        // On Linux this is a real number; elsewhere 0, which the report labels
        // rather than hiding.
        let v = peak_rss_kib();
        if cfg!(target_os = "linux") {
            assert!(v > 0, "VmHWM should be readable on Linux");
        } else {
            assert_eq!(v, 0);
        }
    }

    #[test]
    fn load_uses_the_m4_control_vocabulary() {
        // Request-shape check without libmpv: the app must reuse the accepted M4
        // request builders rather than invent command names.
        let r = load_request("/tmp/fixture.mkv").expect("load_request");
        match r {
            Request::Command { name, args } => {
                assert_eq!(name.name, "loadfile");
                assert_eq!(
                    args,
                    vec!["/tmp/fixture.mkv".to_string(), "replace".to_string()]
                );
            }
            Request::SetProperty { .. } => panic!("load must be a command request"),
        }
        let sel = select_subtitle(&TrackSelection::None).expect("sub-id request");
        match sel {
            Request::SetProperty { name, value } => {
                assert_eq!(name.name, "sub-id");
                assert_eq!(value, PropertyValue::Str("no".to_string()));
            }
            Request::Command { .. } => panic!("sub-id selection must be a property request"),
        }
    }

    /// The GL session itself, gated by `PLEX_GL_HARNESS` (see
    /// `crate::gl::try_surfaceless`; `scripts/mpv-gl-harness.sh` exports it).
    /// This is a
    /// compile-and-wire check outside the harness: `open()` must type-check and
    /// the fields must be reachable.
    #[test]
    fn mpv_gl_session_opens_and_renders_under_the_harness_gate() {
        let Some(gl) = crate::gl::try_surfaceless(
            "plex_app::render::mpv_gl_session_opens_and_renders_under_the_harness_gate",
        ) else {
            return;
        };
        let strings: GlStrings = gl.gl_strings();
        assert!(!strings.renderer.is_empty());
        let session = MpvGlSession::open(&gl, SessionConfig::default());
        match session {
            Ok(s) => {
                assert_eq!(s.render_context().api(), RenderApi::OpenGL);
                assert_eq!(s.fbo().size(), (320, 240));
                assert!(s.last_frame().is_none());
            }
            Err(e) => panic!("open failed under harness gate: {e}"),
        }
    }
}
