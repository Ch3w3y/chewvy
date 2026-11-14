//! The embedded player engine: libmpv renders straight into the app window.
//!
//! One item plays at a time. Its mpv core lives on the UI thread (the thread
//! that owns the window's GL context), in a thread-local slot, so both the UI
//! code that drives the controls and the window host that paints each frame can
//! reach it without locks or channels.
//!
//! Each window frame the host calls [`render_behind`] after clearing and before
//! egui paints: mpv draws the video (with its own letterboxing) into the
//! window's default framebuffer, then egui paints the controls on top. Nothing
//! is read back: the frame stays on the GPU that owns the window, so hardware
//! decoding and GPU scaling/tone-mapping apply.

pub mod hdr;
mod memory;
pub mod prefs;
pub mod profile;
pub mod quality;
pub mod request;
mod state;

pub use state::{
    clock, codec_name, marker_at, ChapterMark, MarkerKind, MarkerSpan, MediaFacts, NextUp,
    Snapshot, SubFile, Track, TrackKind,
};

use std::cell::RefCell;
use std::time::{Duration, Instant};

use plex_core::secret::SecretString;
use plex_player::events::{EndReason, EventKind};
use plex_player::link::PlayerLink as _;
use plex_player::mpv_link::FfiPlayer;
use plex_player::render_link::{GlFbo, RenderApi, RenderContext, RenderOptions};
use winit::raw_window_handle::RawDisplayHandle;

use crate::gl::GlutinResolver;
use crate::together::follow::Request;
use hdr::Output;

/// Everything the UI needs to open one item.
pub struct PlayRequest {
    /// The URL libmpv loads. Carries no token.
    pub source: String,
    /// The header line that authorises this URL — `X-Plex-Token` for a Plex
    /// server, `MediaBrowser` for a Jellyfin one (never logged).
    pub header: SecretString,
    /// The item's server key, for the next-episode hand-off and progress.
    pub rating_key: String,
    /// Main title: the movie, or the show for an episode.
    pub title: String,
    /// Second line: "S4 E2 · Blue Rafter" for an episode, the year for a movie.
    pub subtitle: Option<String>,
    /// Seconds to open at (the item's `viewOffset`).
    pub start: Option<f64>,
    /// Intro/credits ranges the server reported.
    pub markers: Vec<MarkerSpan>,
    /// Chapter starts and thumbnail paths the server reported.
    pub chapters: Vec<ChapterMark>,
    /// The episode that follows this one, when there is one.
    pub next: Option<NextUp>,
    /// What the server says about the file (for the info sheet).
    pub media: MediaFacts,
    /// Index into [`quality::QUALITIES`] this was opened at.
    pub quality: usize,
    /// The server stream session, for Direct Stream and transcodes.
    pub session: Option<String>,
    /// The item's full length in seconds: a stream's playlist grows while
    /// the server works, so mpv's own duration is short until it finishes.
    pub duration: Option<f64>,
    /// Where in the item a stream's own clock starts: the server restarts
    /// timestamps at the offset it was asked for. Zero for Direct Play.
    pub origin: f64,
    /// Whose remembered audio/subtitle languages apply: the show for an
    /// episode, the item itself otherwise.
    pub title_key: String,
    /// Sidecar subtitle files to load with the item.
    pub sidecars: Vec<SubFile>,
    /// The app chose this quality itself, after the one before it failed.
    pub fell_back: bool,
}

/// How a finished item ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ended {
    /// The user stopped it.
    Stopped,
    /// It played to the end.
    Finished,
    /// mpv could not open or decode what the server sent (user-safe text, no
    /// URL): another delivery of the same item may still play.
    Unplayable(String),
    /// The picture could not be drawn here (user-safe text); no other
    /// delivery changes that.
    Failed(String),
}

/// Whether mpv has been given the item's source yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LoadState {
    /// Not yet. `vo=libmpv` refuses a file that opens before a render context
    /// exists ("No render context set" → `MPV_ERROR_NOTHING_TO_PLAY`), which a
    /// fast local source hits every time and a slow network one usually misses.
    Pending,
    /// `loadfile` has been issued for this item.
    Loading,
}

impl LoadState {
    /// The one legal step: hand over the source once - and only once - the
    /// render context exists. `true` means `loadfile` should be issued now.
    fn load_if_rendered(&mut self, render_ready: bool) -> bool {
        if *self == Self::Pending && render_ready {
            *self = Self::Loading;
            true
        } else {
            false
        }
    }
}

/// One playing item.
pub struct Player {
    mpv: FfiPlayer,
    /// Created on the first window frame, on the GL thread.
    render: Option<RenderContext>,
    /// The source waits in the request until [`LoadState::Loading`].
    load: LoadState,
    repaint: egui::Context,
    request: PlayRequest,
    ended: Option<Ended>,
    /// A key in the HDR window asked for the next episode.
    next_asked: bool,
    snapshot: Snapshot,
    slow_poll: Option<Instant>,
    restart: Option<f64>,
    /// The remembered track languages have been applied to this item.
    tracks_applied: bool,
    /// mpv's `hwdec-current` has been logged once for this item.
    hwdec_logged: bool,
    /// Slow polls in a row that found a stream of the file dropped.
    undecoded_polls: u8,
    /// When the source went to mpv, until it reaches a first position.
    loading_since: Option<Instant>,
    /// Where the picture is presented.
    output: Output,
    /// While watching together: the person's own play, pause and seek, held
    /// as requests for the group instead of acting here.
    group: Option<Vec<Request>>,
}

thread_local! {
    static ACTIVE: RefCell<Option<Player>> = const { RefCell::new(None) };
}

/// `MPV_EVENT_SHUTDOWN` (`client.h`): the core is going away.
const MPV_EVENT_SHUTDOWN: u32 = 1;

/// How often the slower-changing properties (tracks, video info) are re-read.
const SLOW_POLL: Duration = Duration::from_millis(500);

/// Slow polls that must find the same stream dropped before the item is given
/// up on.
const UNDECODED_POLLS: u8 = 2;

/// How long a source may take to reach its first position. mpv waits out a
/// server whose stream never produces a segment for minutes, a network timeout
/// per segment, without ever reporting an error.
const START_TIMEOUT: Duration = Duration::from_secs(30);

/// Start playing `request`, replacing whatever was playing; `hardware` is the
/// Settings choice for hardware decoding (`hwdec=auto-safe`, else `no`), and
/// `output` says whether the picture is drawn here or in the HDR window.
///
/// # Errors
///
/// User-safe text when libmpv cannot be created or refuses an option.
pub fn start(
    request: PlayRequest,
    hardware: bool,
    output: Output,
    repaint: &egui::Context,
) -> Result<(), String> {
    stop();
    let player = Player::open(request, hardware, output, repaint)?;
    ACTIVE.with(|slot| *slot.borrow_mut() = Some(player));
    repaint.request_repaint();
    Ok(())
}

/// Stop and drop the playing item (no-op when nothing plays).
pub fn stop() {
    ACTIVE.with(|slot| {
        if let Some(player) = slot.borrow_mut().take() {
            player.teardown();
        }
    });
}

/// `true` while an item is loaded.
#[must_use]
pub fn is_active() -> bool {
    ACTIVE.with(|slot| slot.borrow().is_some())
}

/// Run `f` against the playing item.
pub fn with<R>(f: impl FnOnce(&mut Player) -> R) -> Option<R> {
    ACTIVE.with(|slot| slot.borrow_mut().as_mut().map(f))
}

/// Where the playing file is, and whether it is paused — the two things a
/// server needs to hear about a session. `None` with nothing playing, or with a
/// file whose clock has not started yet: a position nobody knows is not a
/// position of zero.
#[must_use]
pub fn position() -> Option<(f64, bool)> {
    with(|player| {
        let snapshot = player.snapshot();
        snapshot.position.map(|at| (at, snapshot.paused))
    })
    .flatten()
}

/// Window host hook: draw the current video frame into the default framebuffer.
///
/// Called with the window's GL context current, after the clear and before egui
/// paints. `size` is the framebuffer size in pixels; `display` is the glutin
/// display that context belongs to, which is where mpv's GL entry points come
/// from. `native_display` is the window's raw display handle (its X11/Wayland
/// display is what EGL-based hwdec interop needs; see
/// [`RenderContext::create`]), `None` where the host passes nothing.
pub fn render_behind(
    size: [u32; 2],
    display: &glutin::display::Display,
    native_display: Option<RawDisplayHandle>,
) {
    with(|player| player.render_frame(size, display, native_display));
}

impl Player {
    /// Release this item: the render context dies here, the mpv core elsewhere.
    ///
    /// Taking the fields out is what fixes the order: the render context keeps the
    /// core alive, so `FfiPlayer::drop` defers `mpv_destroy` until that context is
    /// released - which, left to field order, would run the destroy inline here.
    fn teardown(self) {
        let Player { mpv, render, .. } = self;
        // `mpv_render_context_free` needs the window's GL context current, which
        // it always is on this thread, and it must happen before the core goes
        // (`render.h:122-123`).
        drop(render);
        // `mpv_destroy` waits for the core to shut down and can block for seconds
        // on a stalled stream (`client.h:505-514`), so it must not run on the
        // thread that paints the window.
        if let Err(err) = std::thread::Builder::new()
            .name("mpv-teardown".to_owned())
            .spawn(move || drop(mpv))
        {
            // `spawn` drops the closure - and the core inside it - itself when no
            // thread could be created, so that destroy already ran here.
            log::debug!("mpv core destroyed on this thread instead: {err}");
        }
    }

    fn open(
        request: PlayRequest,
        hardware: bool,
        output: Output,
        repaint: &egui::Context,
    ) -> Result<Self, String> {
        let mut mpv = FfiPlayer::new(None).map_err(|e| format!("could not start mpv: {e}"))?;
        let start = request.start.map(|secs| format!("{secs:.3}"));
        let mut options: Vec<(&str, &str)> = vec![
            ("profile", "high-quality"),
            ("hwdec", if hardware { "auto-safe" } else { "no" }),
            // mpv's default pool holds 6 spare decoded frames: at 4K 10-bit
            // that is ~290 MiB of VRAM measured on NVDEC for no visible gain.
            ("hwdec-extra-frames", "3"),
            ("keep-open", "yes"),
            ("idle", "yes"),
            ("osc", "no"),
            ("osd-level", "0"),
            ("input-default-bindings", "no"),
            ("input-vo-keyboard", "no"),
            ("ytdl", "no"),
            ("cache", "yes"),
            // Read-ahead held in RAM: ~17 s of an 80 Mbps 4K remux, plenty
            // over a LAN; the seek-back cache is kept small for the same reason.
            ("demuxer-max-bytes", "128MiB"),
            ("demuxer-max-back-bytes", "32MiB"),
            ("demuxer-readahead-secs", "30"),
            ("volume-max", "150"),
            ("tone-mapping", "auto"),
            ("hdr-compute-peak", "auto"),
            ("sub-auto", "fuzzy"),
            ("sub-font-size", "44"),
            ("sub-border-size", "2.5"),
            ("sub-shadow-offset", "1"),
            ("audio-client-name", "Chewvy"),
            // With no sound device mpv would drop the audio track, which is
            // also what it does to audio it cannot decode; this keeps the two
            // apart and lets the picture play either way.
            ("audio-fallback-to-null", "yes"),
            ("title", &request.title),
        ];
        // Last, so the HDR window's choices override the embedded defaults above.
        options.extend(output.options());
        if let Some(secs) = &start {
            options.push(("start", secs));
        }
        options.extend(prefs::get().properties());
        for (name, value) in options {
            mpv.set_option(name, value)
                .map_err(|e| format!("mpv rejected option {name}: {e}"))?;
        }
        // Embedded, no `loadfile` here: the render context is created on the first
        // window frame, and a file that opens before it fails under vo=libmpv.
        mpv.initialize_link()
            .map_err(|e| format!("could not start mpv: {e}"))?;
        // String lists split a plain value on commas, and a `MediaBrowser` header
        // has four. `change-list … append` adds one whole item; the `-append`
        // option suffix is CLI-only and libmpv answers it "option not found".
        // Both lists are set before `loadfile`, so the first read carries them.
        let header = request.header.expose();
        let lists = std::iter::once(("http-header-fields", header)).chain(
            request
                .sidecars
                .iter()
                .map(|sub| ("sub-files", sub.url.as_str())),
        );
        for (list, value) in lists {
            mpv.command_sync(&["change-list", list, "append", value])
                .map_err(|e| format!("mpv rejected {list}: {e}"))?;
        }
        // The HDR window draws for itself, so its file goes over at once.
        let (load, loading_since) = if output.is_hdr_window() {
            mpv.command_sync(&["loadfile", &request.source])
                .map_err(|_| format!("“{}” could not be opened", request.title))?;
            if request.next.is_some() {
                let ask = format!("set {} yes", hdr::NEXT_ASKED);
                for key in hdr::NEXT_KEYS {
                    if let Err(e) = mpv.command_sync(&["keybind", key, &ask]) {
                        log::debug!("mpv would not bind {key}: {e}");
                    }
                }
            }
            (LoadState::Loading, Some(Instant::now()))
        } else {
            (LoadState::Pending, None)
        };
        Ok(Self {
            mpv,
            render: None,
            load,
            output,
            repaint: repaint.clone(),
            snapshot: Snapshot {
                sidecars: request.sidecars.clone(),
                ..Snapshot::default()
            },
            slow_poll: None,
            restart: None,
            tracks_applied: false,
            hwdec_logged: false,
            undecoded_polls: 0,
            loading_since,
            ended: None,
            group: None,
            next_asked: false,
            request,
        })
    }

    fn render_frame(
        &mut self,
        size: [u32; 2],
        display: &glutin::display::Display,
        native_display: Option<RawDisplayHandle>,
    ) {
        if self.output.is_hdr_window() {
            return;
        }
        if self.render.is_none() {
            let repaint = self.repaint.clone();
            let wake: Box<dyn Fn() + Send + Sync> = Box::new(move || repaint.request_repaint());
            match RenderContext::create(
                &self.mpv,
                RenderApi::OpenGL,
                Box::new(GlutinResolver::new(display)),
                Some(wake),
                false,
                native_display,
            ) {
                Ok(ctx) => self.render = Some(ctx),
                Err(e) => {
                    self.ended = Some(Ended::Failed(format!("video output failed: {e}")));
                    return;
                }
            }
        }
        // mpv idles until here; the source goes over only once the render
        // context exists, so vo=libmpv never opens a file with nowhere to draw.
        if self.load.load_if_rendered(self.render.is_some()) {
            self.loading_since = Some(Instant::now());
            if self
                .mpv
                .command_sync(&["loadfile", &self.request.source])
                .is_err()
            {
                self.ended = Some(Ended::Unplayable(format!(
                    "“{}” could not be opened",
                    self.request.title
                )));
                return;
            }
        }
        let Some(ctx) = self.render.as_mut() else {
            return;
        };
        let (Ok(width), Ok(height)) = (i32::try_from(size[0]), i32::try_from(size[1])) else {
            return;
        };
        let _ = ctx.take_update();
        let options = RenderOptions {
            fbo: Some(GlFbo {
                fbo: 0,
                width,
                height,
                internal_format: 0,
            }),
            // The default framebuffer is bottom-up; never block the UI thread.
            flip_y: true,
            block_for_target_time: false,
            skip_rendering: false,
        };
        if let Err(e) = ctx.render(&options) {
            self.ended = Some(Ended::Failed(format!("video output failed: {e}")));
        }
    }

    /// Drain mpv's events and refresh the [`Snapshot`]; call once per UI frame.
    pub fn tick(&mut self) {
        while let Some(event) = self.mpv.next_event() {
            // The HDR window's own close button and `q` shut the core down;
            // that is the user leaving the item, as Back would be.
            if event.kind
                == (EventKind::Other {
                    raw: MPV_EVENT_SHUTDOWN,
                })
            {
                self.ended.get_or_insert(Ended::Stopped);
                return;
            }
            if event.kind == EventKind::EndFile {
                if let Some((EndReason::Error, code)) = event.end_file {
                    self.ended = Some(Ended::Unplayable(format!(
                        "“{}” could not be played (mpv error {code})",
                        self.request.title
                    )));
                }
            }
        }
        let now = Instant::now();
        let slow = self
            .slow_poll
            .is_none_or(|last| now.duration_since(last) >= SLOW_POLL);
        if slow {
            self.slow_poll = Some(now);
        }
        self.snapshot.refresh(&self.mpv, slow);
        if self.output.is_hdr_window() && hdr::asked(self.mpv.get_str(hdr::NEXT_ASKED).as_deref()) {
            self.set(hdr::NEXT_ASKED, "no");
            self.next_asked = true;
        }
        if self.request.session.is_some() {
            self.snapshot.duration = self.request.duration.or(self.snapshot.duration);
            let origin = self.request.origin;
            self.snapshot.position = self.snapshot.position.map(|p| p + origin);
        }
        if !self.tracks_applied && !self.snapshot.tracks.is_empty() {
            self.tracks_applied = true;
            self.apply_remembered_tracks();
        }
        if self.snapshot.eof && self.ended.is_none() {
            self.ended = Some(Ended::Finished);
        }
        if slow {
            self.watch_for_undecoded();
            self.watch_for_a_start();
        }
        // One line of proof of what the real render path picked (vaapi,
        // vulkan-copy, "no", …); the value carries no token or path.
        if !self.hwdec_logged && self.snapshot.position.is_some() {
            if let Some(hwdec) = self.snapshot.video.hwdec.as_deref() {
                self.hwdec_logged = true;
                log::info!("mpv hwdec-current: {hwdec}");
            }
        }
    }

    /// A source that has not reached a first position in [`START_TIMEOUT`] is
    /// given up on, so another delivery can be tried while someone is waiting.
    fn watch_for_a_start(&mut self) {
        let Some(since) = self.loading_since else {
            return;
        };
        if self.snapshot.position.is_some() {
            self.loading_since = None;
        } else if self.ended.is_none() && since.elapsed() >= START_TIMEOUT {
            self.ended = Some(Ended::Unplayable(format!(
                "“{}” did not start playing within {} seconds",
                self.request.title,
                START_TIMEOUT.as_secs()
            )));
        }
    }

    /// A stream mpv dropped stays dropped, so a short run of polls that agree
    /// ends the item as unplayable instead of leaving it half-playing.
    fn watch_for_undecoded(&mut self) {
        // The HDR window can also lose its picture to its own output failing,
        // which no other delivery of the item fixes.
        let stream = self
            .snapshot
            .undecoded
            .filter(|kind| *kind != "video" || !self.output.is_hdr_window())
            .filter(|_| self.snapshot.position.is_some() && self.ended.is_none());
        let Some(stream) = stream else {
            self.undecoded_polls = 0;
            return;
        };
        self.undecoded_polls += 1;
        if self.undecoded_polls >= UNDECODED_POLLS {
            self.ended = Some(Ended::Unplayable(format!(
                "the {stream} of “{}” could not be decoded",
                self.request.title
            )));
        }
    }

    /// Where a replacement for this item should open: the last position played,
    /// else the one it was opened at.
    #[must_use]
    pub fn resume_point(&self) -> Option<f64> {
        let asked = Some(self.request.origin).filter(|at| *at > 0.0);
        self.snapshot.position.or(self.request.start).or(asked)
    }

    /// The latest polled state.
    #[must_use]
    pub fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }

    /// Where the picture is presented.
    #[must_use]
    pub fn output(&self) -> Output {
        self.output
    }

    /// What the item was opened with.
    #[must_use]
    pub fn request(&self) -> &PlayRequest {
        &self.request
    }

    /// Set once the item has ended or failed.
    #[must_use]
    pub fn ended(&self) -> Option<&Ended> {
        self.ended.as_ref()
    }

    /// Whether a key in the HDR window asked for the next episode since the
    /// last call.
    pub fn take_next_asked(&mut self) -> bool {
        std::mem::take(&mut self.next_asked)
    }

    /// Show `text` over the picture for `time`. Only the HDR window has no
    /// other place for a message: this window draws its own.
    pub fn show_text(&self, text: &str, time: Duration) {
        self.cmd(&["show-text", text, &time.as_millis().to_string()]);
    }

    /// Clear a `Finished` state after the user seeks back into the item.
    pub fn resume_after_end(&mut self) {
        if self.ended == Some(Ended::Finished) {
            self.ended = None;
        }
    }

    fn cmd(&self, args: &[&str]) {
        if let Err(e) = self.mpv.command_sync(args) {
            log::debug!("mpv command {:?} failed: {e}", args.first());
        }
    }

    fn set(&self, name: &str, value: &str) {
        if let Err(e) = self.mpv.set_str(name, value) {
            log::debug!("mpv set {name} failed: {e}");
        }
    }

    /// Flip pause, or ask the group to.
    pub fn toggle_pause(&mut self) {
        let paused = self.snapshot.paused;
        if let Some(asks) = &mut self.group {
            asks.push(if paused {
                Request::Unpause
            } else {
                Request::Pause
            });
            return;
        }
        self.cmd(&["cycle", "pause"]);
    }

    /// Pause or play, whatever the player was doing.
    pub fn set_paused(&self, paused: bool) {
        self.set("pause", if paused { "yes" } else { "no" });
    }

    /// Seek by `secs` (negative goes back), to the nearest keyframe.
    pub fn seek_by(&mut self, secs: f64) {
        // Before the first position report there is nothing to step from.
        let Some(position) = self.snapshot.position else {
            return;
        };
        let target = position + secs;
        if let Some(asks) = &mut self.group {
            asks.push(Request::Seek {
                position: target.max(0.0),
            });
            return;
        }
        if target < self.request.origin {
            return self.seek_to(target, false);
        }
        self.resume_after_end();
        self.cmd(&["seek", &format!("{secs:.3}"), "relative"]);
    }

    /// Seek to `secs` into the item; `exact` decodes to the precise frame.
    ///
    /// A stream cannot go back before where the server started it, so such a
    /// seek is left in [`Player::take_restart`] for the screen to reopen at.
    pub fn seek_to(&mut self, secs: f64, exact: bool) {
        let secs = secs.max(0.0);
        if let Some(asks) = &mut self.group {
            asks.push(Request::Seek { position: secs });
            return;
        }
        self.place(secs, exact);
    }

    /// Seek exactly where the group is: the group's own moves never go back
    /// to it as requests.
    pub fn align(&mut self, secs: f64) {
        self.place(secs.max(0.0), true);
    }

    fn place(&mut self, secs: f64, exact: bool) {
        if self.request.session.is_some() && secs < self.request.origin {
            self.restart = Some(secs);
            return;
        }
        self.resume_after_end();
        let flags = if exact {
            "absolute+exact"
        } else {
            "absolute+keyframes"
        };
        let local = secs - self.request.origin;
        self.cmd(&["seek", &format!("{local:.3}"), flags]);
    }

    /// A seek the stream could not make, to reopen the item at.
    pub fn take_restart(&mut self) -> Option<f64> {
        self.restart.take()
    }

    /// Volume in percent (0-150).
    pub fn set_volume(&self, percent: f64) {
        self.set("volume", &format!("{:.0}", percent.clamp(0.0, 150.0)));
    }

    /// Mute on/off.
    pub fn set_mute(&self, mute: bool) {
        self.set("mute", if mute { "yes" } else { "no" });
    }

    /// Playback speed (0.25-8).
    pub fn set_speed(&self, speed: f64) {
        self.set("speed", &format!("{:.2}", speed.clamp(0.25, 8.0)));
    }

    /// Select an audio track by mpv id, and remember its language for the title.
    pub fn set_audio(&self, id: i64) {
        self.set("aid", &id.to_string());
        let lang = self.track_lang(TrackKind::Audio, id);
        memory::remember(&self.request.title_key, |c| c.audio = lang);
    }

    /// Select a subtitle track by mpv id, or `None` for off, and remember it.
    pub fn set_subtitle(&self, id: Option<i64>) {
        self.set(
            "sid",
            &id.map_or_else(|| "no".to_owned(), |i| i.to_string()),
        );
        let choice = match id {
            None => Some(memory::OFF.to_owned()),
            Some(id) => self.track_lang(TrackKind::Subtitle, id),
        };
        memory::remember(&self.request.title_key, |c| c.subtitle = choice);
    }

    /// mpv numbers each track type separately, so the kind is part of the key.
    fn track_lang(&self, kind: TrackKind, id: i64) -> Option<String> {
        self.snapshot
            .tracks
            .iter()
            .find(|t| t.kind == kind && t.id == id)
            .and_then(|t| t.lang.clone())
    }

    /// Select the languages remembered for this title, where the file has them.
    fn apply_remembered_tracks(&self) {
        let Some(choice) = memory::recall(&self.request.title_key) else {
            return;
        };
        let tracks = &self.snapshot.tracks;
        if let Some(t) = choice
            .audio
            .as_deref()
            .and_then(|lang| memory::pick(tracks, TrackKind::Audio, lang))
        {
            self.set("aid", &t.id.to_string());
        }
        match choice.subtitle.as_deref() {
            Some(memory::OFF) => self.set("sid", "no"),
            Some(lang) => {
                if let Some(t) = memory::pick(tracks, TrackKind::Subtitle, lang) {
                    self.set("sid", &t.id.to_string());
                }
            }
            None => {}
        }
    }

    /// Subtitle delay in seconds.
    pub fn set_sub_delay(&self, secs: f64) {
        self.set("sub-delay", &format!("{secs:.2}"));
    }

    /// Audio delay in seconds.
    pub fn set_audio_delay(&self, secs: f64) {
        self.set("audio-delay", &format!("{secs:.2}"));
    }

    /// Subtitle scale factor (1.0 = default size).
    pub fn set_sub_scale(&self, scale: f64) {
        self.set("sub-scale", &format!("{:.2}", scale.clamp(0.3, 3.0)));
    }

    /// Subtitle vertical position in percent of the screen (100 = bottom).
    pub fn set_sub_pos(&self, pos: i64) {
        self.set("sub-pos", &pos.clamp(0, 150).to_string());
    }

    /// Video zoom as a log2 factor (0 = fit).
    pub fn set_zoom(&self, zoom: f64) {
        self.set("video-zoom", &format!("{zoom:.3}"));
    }

    /// Aspect override ("no" restores the file's own).
    pub fn set_aspect(&self, aspect: &str) {
        self.set("video-aspect-override", aspect);
    }

    /// Apply the carried-over choices ([`prefs::Sticky`]) to this item.
    pub fn apply_sticky(&self, sticky: prefs::Sticky) {
        for (name, value) in sticky.properties() {
            self.set(name, value);
        }
    }

    /// Follow a group: the person's play, pause and seek become requests
    /// ([`Player::take_requests`]) until `false` gives the controls back.
    pub fn follow_group(&mut self, on: bool) {
        self.group = on.then(Vec::new);
        // A group needs every seek on the spot. In a file with no index an
        // exact seek past the cache can land on the next keyframe instead
        // (half a second late in an MPEG stream); starting the demuxer a
        // second early leaves the decoder room to reach the frame asked for.
        self.set("hr-seek-demuxer-offset", if on { "1" } else { "0" });
    }

    /// Whether the controls belong to a group.
    #[must_use]
    pub fn in_group(&self) -> bool {
        self.group.is_some()
    }

    /// The requests the person made since the last call.
    pub fn take_requests(&mut self) -> Vec<Request> {
        self.group.as_mut().map(std::mem::take).unwrap_or_default()
    }

    /// Jump to a chapter by index.
    pub fn set_chapter(&mut self, index: usize) {
        if let Some(asks) = &mut self.group {
            if let Some((start, _)) = self.snapshot.chapters.get(index) {
                asks.push(Request::Seek { position: *start });
            }
            return;
        }
        self.resume_after_end();
        self.set("chapter", &index.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::LoadState;

    /// The ordering that needs no display to prove: the source must not reach
    /// mpv before the render context does, and `loadfile` must fire once.
    #[test]
    fn loadfile_waits_for_the_render_context_and_fires_once() {
        let mut load = LoadState::Pending;
        assert!(!load.load_if_rendered(false), "no render context yet");
        assert_eq!(load, LoadState::Pending);
        assert!(load.load_if_rendered(true), "the render context now exists");
        assert_eq!(load, LoadState::Loading);
        assert!(!load.load_if_rendered(true), "never a second loadfile");
        assert!(!load.load_if_rendered(false), "never a second loadfile");
    }
}
