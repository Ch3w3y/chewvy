// Adapted from the `egui_glow` crate example `examples/pure_glow.rs`
// (egui_glow 0.36.1; upstream licence: MIT OR Apache-2.0).
// Upstream: https://github.com/emilk/egui/blob/0.36.1/crates/egui_glow/examples/pure_glow.rs
//! Window host: a real winit window painted by `egui` through
//! `egui_glow`, adapted near-verbatim from the upstream example named above.
//!
//! What is in this file:
//!
//! * `GlutinWindowContext` - a `winit` window plus its glutin GL
//!   display/context/surface, created in
//!   [`winit::application::ApplicationHandler::resumed`] with
//!   `glutin_winit::DisplayBuilder` (and `finalize_window` if the backend
//!   handed back no window while it searched for a GL configuration).
//! * `HostApp` - the `ApplicationHandler`: `resumed` builds the window + GL +
//!   `egui_glow`, `window_event` handles `CloseRequested`/`Destroyed`,
//!   `RedrawRequested` (clear, draw one egui label, paint, swap buffers) and
//!   `Resized`, `exiting` destroys the painter.
//! * [`run_host`] - builds a `winit` event loop and `run_app`s it.
//!
//! Differences from the upstream example, all deliberate:
//!
//! * `fn main()` became [`run_host`], which returns `Result` instead of
//!   `expect`ing, so a window/GL failure is reported to the caller.
//! * The demo side panel, Quit button and colour picker became **one egui
//!   label**. There is no video frame here: the video mode adds the player.
//! * `smoke_frames: Some(n)` counts painted frames and calls
//!   [`winit::event_loop::ActiveEventLoop::exit`] after the n-th present, so a
//!   smoke run is bounded and measurable. `None` keeps the example's
//!   repaint-delay/`WaitUntil` control flow for interactive use.
//! * A smoke run is also bounded by a **deadline**: [`HOST_SMOKE_WATCHDOG_MS`]
//!   (overridable per run by the `PLEX_HOST_WATCHDOG_MS` environment variable).
//!   A watcher thread sends [`UserEvent::Deadline`] once it expires and the
//!   handler exits with an error stating how many frames were painted, so a
//!   compositor that never configures a window produces a *reported* failure
//!   instead of blocking forever with no output. If the loop does not answer
//!   that event, the watcher prints the measured frame count itself and exits 1
//!   ([`HostProgress`]).
//! * `.unwrap()`/`.expect()` in the GL setup became `?`, except the glutin
//!   config picker: `glutin_winit::DisplayBuilder::build` types its picker as
//!   `FnOnce(...) -> Config`, which cannot report an error, so that one
//!   upstream `.expect` is kept and labelled in place.
//! * The window title comes from [`HOST_WINDOW_TITLE`] instead of the example's
//!   literal, and `resumed` requests the first redraw explicitly.
//! * Presentation pacing is a policy ([`swap_interval_policy`]) instead of the
//!   example's hardcoded `SwapInterval::Wait(1)`, and a rejected swap interval
//!   is reported rather than treated as a window-creation failure.
//! * `exiting` releases painter -> glow loader -> unbound context -> EGL
//!   surface -> context -> display -> window in that order (see
//!   [`GlutinWindowContext`]); the example relies on its struct field order.
//!
//! Adds the **video mode** of the same host: [`run_host_video_frames`],
//! [`VideoMode`] and [`VideoRunReport`]. In that mode `RedrawRequested` runs one
//! [`FramePump`](super::window::FramePump) frame per window frame -
//! [`GlFrameSource`](super::window::GlFrameSource) (`mpv_render_context_render`
//! into the app FBO + RGBA readback in the player's surfaceless EGL context) ->
//! egui texture upload -> egui pass (video quad, then transport overlay) ->
//! [`WindowFrameSink`], which paints that egui output twice with
//! `egui_glow::Painter`: once into the window context's own composite FBO (the
//! pixels [`VideoRunReport`] hashes) and once into the window surface's back
//! buffer before `swap_buffers`, so the window really shows the playing item
//! under the overlay. The frame crosses the two EGL contexts as an **RGBA copy**
//! (bytes read out of the player FBO, uploaded as an egui texture in the window
//! context); nothing here claims a shared-context or zero-copy path, and each
//! frame re-binds the window context with [`GlutinWindowContext::bind`] because
//! [`SharedGl::make_current`] took the thread's current context for the render.
//!
//! Not in this file: intent plumbing driven from UI widgets - the pump's `draw`
//! closure returns nothing in video mode, no `egui_winit` input is fed to the
//! pump and no overlay action is applied to the player (UI click plumbing) - and
//! Plex sign-in, which [`crate::watch`] runs before this host is entered.

#![allow(unsafe_code)]

use std::error::Error;
use std::num::NonZeroU32;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
#[cfg(target_os = "linux")]
use std::time::Duration;

#[cfg(target_os = "linux")]
use egui_glow::Painter;
use egui_winit::winit;
#[cfg(target_os = "linux")]
use winit::raw_window_handle::HasDisplayHandle as _;
use winit::raw_window_handle::HasWindowHandle as _;

#[cfg(target_os = "linux")]
use crate::gl::{AppFbo, SharedGl};
#[cfg(target_os = "linux")]
use crate::overlay::TransportState;
#[cfg(target_os = "linux")]
use crate::render::{MpvGlSession, SessionConfig};
#[cfg(target_os = "linux")]
use crate::shell::window::{FramePump, FrameReport, FrameSink, GlFrameSource};

/// Title of the app window.
pub const HOST_WINDOW_TITLE: &str = "Chewvy";

/// The application id the Linux desktop entry is filed under, so the
/// compositor can show the app's icon and group its windows.
#[cfg(target_os = "linux")]
const APP_ID: &str = "io.github.ch3w3y.chewvy";

/// Bounded wait each pump frame gives the player for one rendered frame.
///
/// The window event loop never waits longer than this for a video frame, which
/// is the mechanical reason a buffering or stalled stream cannot freeze the UI.
pub const FRAME_POLL_MS: u64 = 16;

/// Title of the window the video mode creates (it has a player frame in it).
#[cfg(target_os = "linux")]
pub const HOST_VIDEO_WINDOW_TITLE: &str = "Chewvy video smoke (player frame + egui)";

/// The player side of a video run: what to load, and at what size.
///
/// `frame` is the app FBO size the player was opened with
/// ([`SessionConfig::default`]), i.e. the size the window is created at: the
/// video mode has no scaling, so [`WindowFrameSink`] paints at exactly this size
/// and a window surface of another size is *stated* in
/// [`VideoRunReport::surface_matches_frame`] rather than letterboxed or scaled.
#[cfg(target_os = "linux")]
pub struct VideoMode {
    /// The file passed to `loadfile` (an absolute path is what
    /// `crates/plex-app/src/main.rs` hands it today).
    pub fixture: String,
    /// The fixed physical window size, equal to the player frame size.
    pub frame: [u32; 2],
}

#[cfg(target_os = "linux")]
impl VideoMode {
    /// A mode that plays `fixture` at the app's configured player frame size.
    #[must_use]
    pub fn new(fixture: &str) -> Self {
        let cfg = SessionConfig::default();
        let frame = [
            u32::try_from(cfg.width).unwrap_or(1),
            u32::try_from(cfg.height).unwrap_or(1),
        ];
        Self {
            fixture: fixture.to_owned(),
            frame,
        }
    }
}

/// The window, its GL context and its GL surface (same members as the upstream
/// example, which takes them from `eframe`).
///
/// The field order differs from upstream on purpose: Rust drops struct fields
/// in declaration order, and each glutin handle releases its EGL object in its
/// `Drop` (`Surface` -> `eglDestroySurface` plus, for a window surface, the
/// `wl_egl_window` it owns; `ContextInner` -> `eglDestroyContext`; `Display`
/// -> `eglTerminate` when it owns the display reference). With the upstream
/// order the winit window died first, so the surface's `Drop` released a
/// native window that was already gone. That ordering was measured as SIGSEGV
/// (exit 139, empty stdout, 3/3 repeats) after all requested frames were
/// painted; no stack trace was
/// captured, so the causal mechanism above is an inference from glutin's `Drop`
/// implementations, not a measured one. This order releases
/// surface -> context -> display -> window, and [`HostApp::exiting`] also does
/// that teardown explicitly so correctness does not rely on field order alone.
/// The configuration that crashed (app `Wait(1)` plus the Mesa knob
/// `vblank_mode=0`) now exits 0.
struct GlutinWindowContext {
    gl_surface: glutin::surface::Surface<glutin::surface::WindowSurface>,
    gl_context: glutin::context::PossiblyCurrentContext,
    gl_display: glutin::display::Display,
    window: winit::window::Window,
}

impl GlutinWindowContext {
    fn new(
        event_loop: &winit::event_loop::ActiveEventLoop,
        title: &str,
        swap_interval: glutin::surface::SwapInterval,
        player_frame: Option<[u32; 2]>,
    ) -> Result<Self, Box<dyn Error>> {
        use glutin::context::NotCurrentGlContext as _;
        use glutin::display::GetGlDisplay as _;
        use glutin::display::GlDisplay as _;
        use glutin::prelude::GlSurface as _;

        // The label mode keeps the upstream example's resizable 800x600 logical
        // window. The video mode asks for a fixed physical window of exactly the
        // player frame: `FramePump` sizes the video texture from the sink
        // (`super::window::frame_px`) and `VideoTexture::update` refuses bytes
        // that do not match it, so this mode paints 1:1 or reports a size
        // mismatch - it does not scale a 320x240 frame into a bigger window.
        let winit_window_builder = match player_frame {
            Some([width, height]) => winit::window::WindowAttributes::default()
                .with_resizable(false)
                .with_inner_size(winit::dpi::PhysicalSize::new(width, height)),
            None => winit::window::WindowAttributes::default()
                .with_resizable(true)
                .with_decorations(crate::ui::chrome::DECORATED)
                .with_inner_size(winit::dpi::LogicalSize {
                    width: 1280.0,
                    height: 800.0,
                }),
        }
        .with_title(title)
        // Keep hidden until something has been painted (upstream note:
        // emilk/egui#2279), so a window never flashes empty.
        .with_visible(false);
        #[cfg(target_os = "linux")]
        let winit_window_builder = {
            use winit::platform::wayland::WindowAttributesExtWayland as _;
            winit_window_builder.with_name(APP_ID, "chewvy")
        };

        let config_template_builder = glutin::config::ConfigTemplateBuilder::new()
            .prefer_hardware_accelerated(None)
            .with_depth_size(0)
            .with_stencil_size(0)
            .with_transparency(false);

        log::debug!("trying to get gl_config");
        // mpv's VAAPI/GL interop needs an EGL display (under GLX it reports
        // "VAAPI hwdec only works with OpenGL or Vulkan backends" and falls
        // back to vulkan-copy), so Linux asks for EGL first and keeps GLX as
        // the fallback. Windows keeps glutin's WGL-first default; macOS (CGL)
        // ignores the preference and picks its own API.
        #[cfg(target_os = "linux")]
        let api_preference = glutin_winit::ApiPreference::PreferEgl;
        #[cfg(not(target_os = "linux"))]
        let api_preference = glutin_winit::ApiPreference::FallbackEgl;
        // D13: the config picker cannot report "no config" - glutin types it as
        // `FnOnce(..) -> Config`, so an empty iterator leaves `expect` as the
        // only way out. Catching it here turns "this display has no usable
        // OpenGL configuration" into a reported error instead of an abort.
        let (mut window, gl_config) =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                glutin_winit::DisplayBuilder::new()
                    .with_preference(api_preference)
                    .with_window_attributes(Some(winit_window_builder.clone()))
                    .build(
                        event_loop,
                        config_template_builder,
                        |mut config_iterator| {
                            // See the catch above: the picker has no Result to return.
                            #[allow(clippy::expect_used)]
                        config_iterator.next().expect(
                            "failed to find a matching configuration for creating glutin config",
                        )
                        },
                    )
            }))
            .map_err(|_| {
                "failed to find a matching configuration for creating glutin config".to_owned()
            })?
            .map_err(|e| format!("failed to build the glutin display: {e}"))?;
        let gl_display = gl_config.display();
        log::debug!("found gl_config: {gl_config:?}");

        let raw_window_handle = match window.as_ref() {
            Some(w) => Some(
                w.window_handle()
                    .map_err(|e| format!("failed to get window handle: {e}"))?
                    .as_raw(),
            ),
            None => None,
        };
        log::debug!("raw window handle: {raw_window_handle:?}");
        // Desktop GL. On Windows (WGL) an unversioned request yields a 1.1
        // context, which `glow` cannot load GL3+ entry points through, so GL 3.2
        // core is asked for explicitly. macOS (CGL) picks its own core profile
        // when it chooses the pixel format and rejects a GLES request outright
        // (`glutin/src/api/cgl/context.rs`, `api/cgl/config.rs`), so the same
        // request is accepted there and the GLES fallback below is a Linux/WGL
        // path only. Linux keeps the unversioned request it has always used.
        #[cfg(target_os = "linux")]
        let context_attributes =
            glutin::context::ContextAttributesBuilder::new().build(raw_window_handle);
        #[cfg(not(target_os = "linux"))]
        let context_attributes = glutin::context::ContextAttributesBuilder::new()
            .with_context_api(glutin::context::ContextApi::OpenGl(Some(
                // The same constant the startup message states as Chewvy's
                // requirement, so message and request cannot drift apart.
                glutin::context::Version::new(
                    crate::gl::support::GL_REQUEST.0,
                    crate::gl::support::GL_REQUEST.1,
                ),
            )))
            .with_profile(glutin::context::GlProfile::Core)
            .build(raw_window_handle);
        let fallback_context_attributes = glutin::context::ContextAttributesBuilder::new()
            .with_context_api(glutin::context::ContextApi::Gles(None))
            .build(raw_window_handle);
        let not_current_gl_context = unsafe {
            gl_display.create_context(&gl_config, &context_attributes)
                .or_else(|_| {
                    log::debug!(
                        "failed to create gl_context with attributes, retrying with fallback GLES attributes"
                    );
                    gl_config.display().create_context(
                        &gl_config,
                        &fallback_context_attributes,
                    )
                })
                .map_err(|e| format!("failed to create context even with fallback attributes: {e}"))?
        };

        // This is where the window is created if the backend did not create it
        // while searching for a suitable gl_config.
        let window = match window.take() {
            Some(w) => w,
            None => {
                log::debug!("window doesn't exist yet: creating one with finalize_window");
                glutin_winit::finalize_window(event_loop, winit_window_builder.clone(), &gl_config)?
            }
        };
        let (width, height): (u32, u32) = window.inner_size().into();
        let width = NonZeroU32::new(width).unwrap_or(NonZeroU32::MIN);
        let height = NonZeroU32::new(height).unwrap_or(NonZeroU32::MIN);
        let surface_attributes =
            glutin::surface::SurfaceAttributesBuilder::<glutin::surface::WindowSurface>::new()
                .build(
                    window
                        .window_handle()
                        .map_err(|e| format!("failed to get window handle: {e}"))?
                        .as_raw(),
                    width,
                    height,
                );
        log::debug!("creating surface with attributes: {surface_attributes:?}");
        let gl_surface = unsafe {
            gl_display
                .create_window_surface(&gl_config, &surface_attributes)
                .map_err(|e| format!("failed to create window surface: {e}"))?
        };
        log::debug!("surface created: making context current");
        let gl_context = not_current_gl_context
            .make_current(&gl_surface)
            .map_err(|e| format!("failed to make context current: {e}"))?;

        // Upstream treats a rejected swap interval as fatal. This host reports
        // it and keeps running instead: presentation pacing is a policy
        // ([`swap_interval_policy`]), and a compositor that refuses it must not
        // turn into "could not create a window".
        if let Err(err) = gl_surface.set_swap_interval(&gl_context, swap_interval) {
            log::warn!(
                "window host: swap interval {} rejected by the driver: {err}",
                swap_interval_label(swap_interval),
            );
        }

        Ok(Self {
            gl_surface,
            gl_context,
            gl_display,
            window,
        })
    }

    /// Re-bind this window's context as the thread's current EGL context.
    ///
    /// Needed every video frame: the player half of the pump calls
    /// [`SharedGl::make_current`] for `mpv_render_context_render`, which takes
    /// the current-context slot from this window's context (EGL allows only one
    /// current context per thread, and rebinding between two contexts is legal).
    /// The label-only mode has one context and never re-binds, which is why this
    /// is Linux/video-mode only.
    ///
    /// # Errors
    ///
    /// glutin's own error text if `eglMakeCurrent` fails.
    #[cfg(target_os = "linux")]
    fn bind(&self) -> Result<(), Box<dyn Error>> {
        use glutin::context::PossiblyCurrentGlContext as _;
        self.gl_context
            .make_current(&self.gl_surface)
            .map_err(|e| format!("re-binding the window EGL context: {e}").into())
    }

    fn window(&self) -> &winit::window::Window {
        &self.window
    }

    fn resize(&self, physical_size: winit::dpi::PhysicalSize<u32>) {
        use glutin::surface::GlSurface as _;
        self.gl_surface.resize(
            &self.gl_context,
            NonZeroU32::new(physical_size.width).unwrap_or(NonZeroU32::MIN),
            NonZeroU32::new(physical_size.height).unwrap_or(NonZeroU32::MIN),
        );
    }

    fn swap_buffers(&self) -> glutin::error::Result<()> {
        use glutin::surface::GlSurface as _;
        self.gl_surface.swap_buffers(&self.gl_context)
    }

    fn get_proc_address(&self, addr: &std::ffi::CStr) -> *const std::ffi::c_void {
        use glutin::display::GlDisplay as _;
        self.gl_display.get_proc_address(addr)
    }
}

/// User event carrying an egui repaint delay (as in the upstream example; this
/// is why the event loop is `EventLoop::<UserEvent>::with_user_event()` and not
/// a plain `EventLoop::new()`).
#[derive(Debug)]
pub enum UserEvent {
    /// Repaint after this delay (`egui::Context::set_request_repaint_callback`).
    Redraw(std::time::Duration),
    /// The smoke-run deadline expired (`run_host_report` schedules this when
    /// `smoke_frames` is `Some(n)`); the handler turns it into a reported
    /// failure instead of waiting for a frame that may never come.
    Deadline,
}

/// Frame progress shared between the event loop and the deadline watcher.
///
/// The counter is atomic and lives outside the event loop on purpose: a
/// compositor that never releases a buffer blocks inside the present call, and
/// then no event - not even [`UserEvent::Deadline`] - can be handled. The
/// watcher can still state how many frames were painted.
#[derive(Debug)]
pub struct HostProgress {
    /// Frames the run was asked to present; `None` = run until the window closes.
    pub target: Option<u32>,
    /// Deadline of this run in milliseconds (0 = no deadline watcher).
    pub deadline_ms: u64,
    frames: AtomicU32,
    /// Claimed by whichever reporting path acts first, so a stuck run is reported once.
    reported: AtomicBool,
}

impl HostProgress {
    fn new(target: Option<u32>, deadline_ms: u64) -> Self {
        Self {
            target,
            deadline_ms,
            frames: AtomicU32::new(0),
            reported: AtomicBool::new(false),
        }
    }

    /// Frames cleared + painted + swapped so far.
    pub fn frames(&self) -> u32 {
        self.frames.load(Ordering::SeqCst)
    }

    /// Count one presented frame and return the new total.
    fn present(&self) -> u32 {
        self.frames.fetch_add(1, Ordering::SeqCst).saturating_add(1)
    }

    /// The one-line failure text a bounded run reports when it stops early.
    fn failure_msg(&self, why: &str) -> String {
        format!(
            "{why} (frames painted {} of {}, window-host deadline {} ms)",
            self.frames(),
            self.target.unwrap_or(0),
            self.deadline_ms
        )
    }

    /// Take the single report slot; `true` for the caller that got it.
    fn claim(&self) -> bool {
        !self.reported.swap(true, Ordering::SeqCst)
    }
}

/// The `ApplicationHandler` behind [`run_host`] and [`run_host_video_frames`]
/// (upstream example: `GlowApp`).
///
/// `'a` is the lifetime of the player [`SharedGl`] the video mode renders in:
/// [`VideoRun`] borrows it, so the label-only mode passes `'static`.
/// The app UI drawn into the window each frame.
pub type AppUi<'a> = Box<dyn FnMut(&mut egui::Ui) + 'a>;

struct HostApp<'a> {
    proxy: winit::event_loop::EventLoopProxy<UserEvent>,
    gl_window: Option<GlutinWindowContext>,
    gl: Option<Arc<glow::Context>>,
    egui_glow: Option<egui_glow::EguiGlow>,
    repaint_delay: std::time::Duration,
    clear_color: [f32; 3],
    /// `Some(n)`: exit after n painted frames. `None`: run until the window
    /// is closed, repainting at egui's requested delay.
    smoke_frames: Option<u32>,
    /// Frames actually cleared + painted + swapped so far (mirrors
    /// [`HostProgress::frames`], which the watcher thread also reads).
    frames_painted: u32,
    /// Shared with the deadline watcher; see [`HostProgress`].
    progress: Arc<HostProgress>,
    /// A window/GL failure, if any: reported by [`run_host`] instead of panicking.
    failure: Option<Box<dyn Error>>,
    /// `Some(..)` selects the **video mode**: every `RedrawRequested`
    /// runs one [`FramePump`] frame into this window. `None` is the M6d1
    /// label-only mode.
    #[cfg(target_os = "linux")]
    video: Option<VideoRun<'a>>,
    /// What the video mode measured, recorded in [`HostApp::exiting`] while the
    /// pump, sink and frames are still alive (dropping the sink releases the
    /// surface and readback data the report is built from). `None` until a video
    /// run tears down; [`run_host_video_frames`] returns exactly this.
    #[cfg(target_os = "linux")]
    final_video_report: Option<VideoRunReport>,
    /// The app UI drawn each frame in label mode; `None` keeps the smoke label.
    ui: Option<AppUi<'a>>,
    /// Audit driver, only when `PLEX_APP_DRIVE` is set.
    driver: Option<super::drive::Driver>,
}

/// What the video mode owns beyond the window: the player context
/// and session, the pump, the overlay state and what the run measured.
///
/// `shared` is **borrowed**, not owned: [`MpvGlSession`] holds the [`SharedGl`]
/// it renders in, so the player context and the session must both be owned one
/// level up ([`run_host_video_frames`]); a struct that owned both would be
/// self-referential.
#[cfg(target_os = "linux")]
struct VideoRun<'a> {
    /// What to play, at what size.
    mode: VideoMode,
    /// The player's surfaceless EGL context (owned by [`run_host_video_frames`]).
    shared: &'a SharedGl,
    /// Opened in [`HostApp::resumed`]; dropped in [`HostApp::exiting`] with this
    /// context current again.
    session: Option<MpvGlSession<'a>>,
    /// The pump: egui context, video texture, the four step order.
    pump: FramePump,
    /// What the overlay shows. This run has no property plumbing, so it
    /// is the fixed state documented on [`VideoMode`] - not an observed player
    /// state.
    transport: TransportState,
    /// Per-frame bounded wait handed to [`GlFrameSource`] (see
    /// [`FRAME_POLL_MS`]), so one stalled player frame cannot outlive the
    /// run's watchdog.
    poll_ms: u64,
    /// The window-side sink, created once the window and its context exist.
    sink: Option<WindowFrameSink>,
    /// One [`FrameReport`] per pump frame that completed.
    frames: Vec<FrameReport>,
    /// Bounded text of pump frames that failed, each ending the run.
    frame_errors: Vec<String>,
    /// When this run was created: the span [`video_run_report`] measures as
    /// `wall_ms` (window creation, player load, every pump frame, teardown).
    started: std::time::Instant,
}

impl<'a> HostApp<'a> {
    fn new(
        proxy: winit::event_loop::EventLoopProxy<UserEvent>,
        smoke_frames: Option<u32>,
        progress: Arc<HostProgress>,
    ) -> Self {
        Self {
            proxy,
            gl_window: None,
            gl: None,
            egui_glow: None,
            repaint_delay: std::time::Duration::MAX,
            clear_color: [0.1, 0.1, 0.1],
            smoke_frames,
            frames_painted: 0,
            progress,
            failure: None,
            #[cfg(target_os = "linux")]
            video: None,
            #[cfg(target_os = "linux")]
            final_video_report: None,
            ui: None,
            driver: None,
        }
    }

    /// The same host in **video mode**: `mode` names the fixture and the frame
    /// size, `shared` is the player context the session renders in.
    #[cfg(target_os = "linux")]
    fn new_video(
        proxy: winit::event_loop::EventLoopProxy<UserEvent>,
        smoke_frames: Option<u32>,
        progress: Arc<HostProgress>,
        mode: VideoMode,
        shared: &'a SharedGl,
    ) -> Self {
        let mut app = Self::new(proxy, smoke_frames, progress);
        app.video = Some(VideoRun {
            mode,
            shared,
            session: None,
            pump: FramePump::new(),
            transport: TransportState::default(),
            poll_ms: FRAME_POLL_MS,
            sink: None,
            frames: Vec::new(),
            frame_errors: Vec::new(),
            started: std::time::Instant::now(),
        });
        app
    }

    /// Turn an expired deadline into the failure [`run_host_report`] reports.
    fn deadline_hit(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        if self.progress.claim() {
            self.failure = Some(Box::from(
                self.progress.failure_msg("window host deadline expired"),
            ));
        }
        event_loop.exit();
    }

    /// One frame: clear, draw (one egui label via `egui_glow`), swap buffers,
    /// then decide whether the event loop keeps going (upstream `redraw` closure).
    fn redraw(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        let Some(gl_window) = self.gl_window.as_mut() else {
            return;
        };
        let window = gl_window.window();
        if let Some(egui_glow) = self.egui_glow.as_mut() {
            if let Some(driver) = self.driver.as_mut() {
                let ppp = egui_glow.egui_ctx.pixels_per_point();
                driver.before_frame(egui_glow.egui_winit.egui_input_mut(), ppp);
            }
            // The app draws its own title bar, so it needs the window's real
            // maximised / fullscreen state each frame.
            let ctx = egui_glow.egui_ctx.clone();
            let input = egui_glow.egui_winit.egui_input_mut();
            let info = input.viewports.entry(egui::ViewportId::ROOT).or_default();
            egui_winit::update_viewport_info(info, &ctx, window, false);
            let app_ui = &mut self.ui;
            egui_glow.run(window, |ui| {
                if let Some(draw) = app_ui.as_mut() {
                    draw(ui);
                } else {
                    egui::CentralPanel::default().show(ui, |ui| {
                        ui.heading("Chewvy window host");
                        ui.label("egui overlay painted through egui_glow");
                    });
                }
            });
            if crate::ui::chrome::take_quit() {
                event_loop.exit();
                return;
            }
        }

        {
            let Some(gl) = self.gl.as_mut() else { return };
            let [r, g, b] = self.clear_color;
            unsafe {
                use glow::HasContext as _;
                gl.clear_color(r, g, b, 1.0);
                gl.clear(glow::COLOR_BUFFER_BIT);
            }
            // The playing item's video goes behind the egui controls. `gl_display`
            // is what the player's GL resolver resolves through. The window's
            // native display joins it only on Linux: libmpv's EGL hwdec interop
            // wants the Xlib `Display*`/`wl_display*` behind it (`render_gl.h`),
            // while WGL (Windows) and CGL (macOS) get no native-display param.
            let size = window.inner_size();
            #[cfg(target_os = "linux")]
            let native_display = window.display_handle().ok().map(|h| h.as_raw());
            #[cfg(not(target_os = "linux"))]
            let native_display = None;
            crate::player::render_behind(
                [size.width, size.height],
                &gl_window.gl_display,
                native_display,
            );
            if let Some(egui_glow) = self.egui_glow.as_mut() {
                egui_glow.paint(window);
            }
            if let Some(driver) = self.driver.as_mut() {
                let size = window.inner_size();
                driver.after_paint(gl, [size.width, size.height]);
            }
            // draw things on top of egui here
            if let Err(err) = gl_window.swap_buffers() {
                self.failure = Some(Box::from(format!("swap_buffers failed: {err}")));
                event_loop.exit();
                return;
            }
            gl_window.window().set_visible(true);
        }

        self.frames_painted = self.progress.present();

        match self.smoke_frames {
            Some(target) => {
                if self.frames_painted >= target {
                    event_loop.exit();
                } else {
                    window.request_redraw();
                    event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);
                }
            }
            None => {
                if self.repaint_delay.is_zero() || self.driver.is_some() {
                    window.request_redraw();
                    event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);
                } else if let Some(repaint_after_instant) =
                    std::time::Instant::now().checked_add(self.repaint_delay)
                {
                    event_loop.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(
                        repaint_after_instant,
                    ));
                } else {
                    event_loop.set_control_flow(winit::event_loop::ControlFlow::Wait);
                }
            }
        }
    }

    /// End the run with the one message a display that cannot run Chewvy
    /// deserves: log it, raise it where the user will see it, and hand the same
    /// text to whoever called `run_host*`, which turns it into a non-zero exit.
    fn fail_graphics(&mut self, event_loop: &winit::event_loop::ActiveEventLoop, text: &str) {
        crate::gl::support::report_graphics_failure(text);
        self.failure = Some(Box::from(text));
        event_loop.exit();
    }

    /// Record a run failure once, stating what the run actually reached. Only
    /// the video mode reports a partial run this way; a label-only run returns
    /// its error to the caller.
    #[cfg(target_os = "linux")]
    fn record_failure(&mut self, why: &str) {
        if self.failure.is_some() {
            return;
        }
        self.failure = Some(Box::from(format!(
            "{why} (frames presented {} of {})",
            self.progress.frames(),
            self.smoke_frames.unwrap_or(0)
        )));
    }

    /// Run **one** pump frame against the video mode's own state.
    ///
    /// `GlFrameSource` borrows the session and the pump borrows the sink and the
    /// overlay state at the same time, and a struct cannot hold a borrow of its
    /// own field, so the two handles a frame needs are taken out of
    /// [`VideoRun`] for the frame and put back afterwards. A failed frame puts
    /// them back too: a player or sink that fails is dropped by
    /// [`Self::redraw_video`], which ends the run instead of retrying.
    #[cfg(target_os = "linux")]
    fn pump_video_frame(&mut self) -> Result<FrameReport, String> {
        let video = self
            .video
            .as_mut()
            .ok_or_else(|| "video mode: no video run".to_owned())?;
        let Some(mut session) = video.session.take() else {
            return Err("video mode: no player session".to_owned());
        };
        let Some(mut sink) = video.sink.take() else {
            video.session = Some(session);
            return Err("video mode: no window sink (window creation failed)".to_owned());
        };
        let poll = Duration::from_millis(video.poll_ms);
        // `GlFrameSource` holds `&mut MpvGlSession`, so it is dropped before the
        // session is moved back into the run.
        let result = {
            let mut source = GlFrameSource::with_poll(&mut session, poll);
            video
                .pump
                .frame(&mut source, &mut sink, &video.transport, |_ui| {})
        };
        video.session = Some(session);
        video.sink = Some(sink);
        result
    }

    /// One **video** frame: [`FramePump::frame`] against [`WindowFrameSink`], so
    /// the window shows the playing item under the egui overlay, then the same
    /// loop decision [`Self::redraw`] makes.
    ///
    /// A pump frame that fails ends the run: its text is kept in
    /// [`VideoRun::frame_errors`] and reported by [`run_host_video_frames`] as a
    /// failure that states how many frames were presented. A stalled player is
    /// not retried forever, and a partial run is never reported as a pass.
    #[cfg(target_os = "linux")]
    fn redraw_video(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        // `resumed` already recorded its own failure and exited the loop when it
        // could not open the player or the window; the `session: None` case is
        // that state, not a reason to spin on `RedrawRequested`.
        let missing = match self.video.as_ref() {
            None => Some("video mode: no video run".to_owned()),
            Some(video) if video.session.is_none() => None,
            Some(video) if video.sink.is_none() => {
                Some("video mode: no window sink (window creation failed)".to_owned())
            }
            Some(_) => None,
        };
        if let Some(why) = missing {
            self.record_failure(&why);
            event_loop.exit();
            return;
        }

        match self.pump_video_frame() {
            Ok(report) => {
                log::info!("window host video {}", report.report_line());
                if let Some(video) = self.video.as_mut() {
                    video.frames.push(report);
                }
                self.frames_painted = self.progress.present();
            }
            Err(err) => {
                log::error!("window host video frame failed: {err}");
                if let Some(video) = self.video.as_mut() {
                    video.frame_errors.push(err.clone());
                }
                self.record_failure(&format!("window host video frame failed: {err}"));
                event_loop.exit();
                return;
            }
        }

        // Same loop decision as the label mode, except the frame was already
        // handed to the window inside the sink's `present` (`swap_buffers` is
        // where the frame is handed over), so there is no second swap here.
        match self.smoke_frames {
            Some(target) if self.frames_painted >= target => event_loop.exit(),
            _ => {
                if let Some(window) = self
                    .video
                    .as_ref()
                    .and_then(|video| video.sink.as_ref())
                    .map(WindowFrameSink::window)
                {
                    window.request_redraw();
                }
                event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);
            }
        }
    }

    /// `resumed` for the video mode: player context + session first (they need
    /// the **player** EGL context current), then the window at the player frame
    /// size, then the window-side sink (which needs the **window** context
    /// current), then the first explicit redraw.
    ///
    /// The fixture is queued through [`MpvGlSession::load`] - the same
    /// `loadfile` path the player crate and the headless GL harness use.
    #[cfg(target_os = "linux")]
    fn resumed_video(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        let smoke_frames = self.smoke_frames;
        let Some(video) = self.video.as_ref() else {
            self.record_failure("video mode: no video run");
            event_loop.exit();
            return;
        };
        let frame = video.mode.frame;
        let fixture = video.mode.fixture.clone();
        // `MpvGlSession::open` documents that its context is already current
        // (`crate::render`); `SharedGl::surfaceless` made it current when it was
        // built, but `resumed` cannot rely on that ordering, so it is re-bound.
        let shared = video.shared;
        if let Err(err) = shared.make_current() {
            log::error!("video mode: player context: {err}");
            self.record_failure(&format!("video mode: player context: {err}"));
            event_loop.exit();
            return;
        }
        let mut session = match MpvGlSession::open(shared, SessionConfig::default()) {
            Ok(session) => session,
            Err(err) => {
                log::error!("video mode: player session: {err}");
                self.record_failure(&format!("video mode: player session: {err}"));
                event_loop.exit();
                return;
            }
        };
        if let Err(err) = session.load(&fixture) {
            log::error!("video mode: loadfile: {err}");
            self.record_failure(&format!("video mode: loadfile: {err}"));
            event_loop.exit();
            return;
        }
        if let Some(video) = self.video.as_mut() {
            video.session = Some(session);
        }

        // The window half, at the player's frame size: this mode paints 1:1 and
        // reports a size mismatch instead of scaling (see [`WindowFrameSink`]).
        match create_display(
            event_loop,
            HOST_VIDEO_WINDOW_TITLE,
            smoke_frames,
            Some(frame),
        ) {
            Ok((gl_window, gl)) => {
                let gl = Arc::new(gl);
                let sink = match WindowFrameSink::create(gl_window, Arc::clone(&gl), frame) {
                    Ok(sink) => sink,
                    Err(err) => {
                        log::error!("video mode: window sink: {err}");
                        self.record_failure(&format!("video mode: window sink: {err}"));
                        event_loop.exit();
                        return;
                    }
                };
                let surface = sink.surface_px();
                log::info!(
                    "video mode: window created title={} frame={}x{} window_surface={}x{} fixture={}",
                    HOST_VIDEO_WINDOW_TITLE,
                    sink.size_px()[0],
                    sink.size_px()[1],
                    surface[0],
                    surface[1],
                    fixture
                );
                if let Some(video) = self.video.as_mut() {
                    video.sink = Some(sink);
                }
                self.gl = Some(gl);
                if let Some(window) = self
                    .video
                    .as_ref()
                    .and_then(|video| video.sink.as_ref())
                    .map(WindowFrameSink::window)
                {
                    window.request_redraw();
                }
            }
            Err(err) => {
                log::error!("video mode: window creation failed: {err}");
                self.record_failure(&format!("video mode: window creation failed: {err}"));
                event_loop.exit();
            }
        }
    }
}

impl winit::application::ApplicationHandler<UserEvent> for HostApp<'_> {
    fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        #[cfg(target_os = "linux")]
        if self.video.is_some() {
            self.resumed_video(event_loop);
            return;
        }
        match create_display(event_loop, HOST_WINDOW_TITLE, self.smoke_frames, None) {
            Ok((gl_window, gl)) => {
                let gl = Arc::new(gl);
                // What the driver actually gave is not what was asked for: WGL
                // answers a 3.2-core request with its built-in GL 1.1 on a VM
                // without a GL driver. `EguiGlow::new` below does not survive
                // that (it unwraps its painter), so the version is checked first.
                if let Some(text) = crate::gl::support::context_failure(&gl) {
                    self.fail_graphics(event_loop, &text);
                    return;
                }
                gl_window.window().set_visible(true);

                // Belt and braces for the residual case: a driver new enough to
                // pass the version gate and still reject egui's shaders. Upstream
                // `EguiGlow::new` unwraps `Painter::new`, and `EguiGlow`'s other
                // fields are private, so the crate's own `Result` cannot be taken
                // apart from outside - this is what keeps the panic out of the
                // run. The crate logs the real error at ERROR level first.
                let egui_glow = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    egui_glow::EguiGlow::new(event_loop, Arc::clone(&gl), None, None, true)
                })) {
                    Ok(egui_glow) => egui_glow,
                    Err(_) => {
                        self.fail_graphics(
                            event_loop,
                            &crate::gl::support::graphics_failure_text(
                                None,
                                "egui_glow could not build its OpenGL painter (see the LOG ERROR line above)",
                            ),
                        );
                        return;
                    }
                };
                crate::ui::theme::apply(&egui_glow.egui_ctx);

                let event_loop_proxy = egui::mutex::Mutex::new(self.proxy.clone());
                egui_glow
                    .egui_ctx
                    .set_request_repaint_callback(move |info| {
                        if let Err(err) = event_loop_proxy
                            .lock()
                            .send_event(UserEvent::Redraw(info.delay))
                        {
                            log::debug!("cannot send repaint user event: {err}");
                        }
                    });
                log::info!(
                    "window host: window created (glutin surface + egui_glow painter ready)"
                );
                self.gl_window = Some(gl_window);
                self.gl = Some(gl);
                self.egui_glow = Some(egui_glow);

                // Ask for the first frame explicitly: a smoke run must make
                // progress without waiting for an external event.
                if let Some(gl_window) = self.gl_window.as_mut() {
                    gl_window.window().request_redraw();
                }
            }
            Err(err) => {
                self.fail_graphics(
                    event_loop,
                    &crate::gl::support::graphics_failure_text(None, &err.to_string()),
                );
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: winit::event::WindowEvent,
    ) {
        use winit::event::WindowEvent;

        if matches!(event, WindowEvent::CloseRequested | WindowEvent::Destroyed) {
            event_loop.exit();
            return;
        }

        if matches!(event, WindowEvent::RedrawRequested) {
            // The two modes of this one host: the label-only frame, or the
            // video frame (player render -> egui -> this window's buffer).
            #[cfg(target_os = "linux")]
            if self.video.is_some() {
                self.redraw_video(event_loop);
                return;
            }
            self.redraw(event_loop);
            return;
        }

        if let WindowEvent::Resized(physical_size) = &event {
            match self.gl_window.as_mut() {
                Some(gl_window) => gl_window.resize(*physical_size),
                // A video run keeps its window context in the sink, so that is
                // where a resize lands instead.
                #[cfg(target_os = "linux")]
                None => {
                    if let Some(sink) = self.video.as_mut().and_then(|v| v.sink.as_mut()) {
                        // No scaling here: the surface size is recorded so the run
                        // can state it (`VideoRunReport::surface_matches_frame`).
                        sink.resize_surface(*physical_size);
                    }
                }
                #[cfg(not(target_os = "linux"))]
                None => {}
            }
        }

        if let (Some(gl_window), Some(egui_glow)) =
            (self.gl_window.as_mut(), self.egui_glow.as_mut())
        {
            let event_response = egui_glow.on_window_event(gl_window.window(), &event);
            if event_response.repaint {
                gl_window.window().request_redraw();
            }
        }
    }

    fn user_event(&mut self, event_loop: &winit::event_loop::ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Redraw(delay) => {
                self.repaint_delay = delay;
                // A request from another thread (a new video frame, a loaded
                // poster) must wake the loop now, not at the next idle deadline.
                if delay.is_zero() {
                    if let Some(gl_window) = self.gl_window.as_mut() {
                        gl_window.window().request_redraw();
                    }
                }
            }
            UserEvent::Deadline => self.deadline_hit(event_loop),
        }
    }

    fn new_events(
        &mut self,
        _event_loop: &winit::event_loop::ActiveEventLoop,
        cause: winit::event::StartCause,
    ) {
        if let winit::event::StartCause::ResumeTimeReached { .. } = &cause {
            if let Some(gl_window) = self.gl_window.as_mut() {
                gl_window.window().request_redraw();
            }
        }
    }

    /// Teardown order that addresses the measured SIGSEGV
    /// (see [`GlutinWindowContext`]): release the egui painter while
    /// the GL context is still current (its `Painter::destroy` deletes GL
    /// objects, and is idempotent), then the `glow` loader, then unbind the
    /// context, then EGL surface -> context -> display -> winit window.
    fn exiting(&mut self, _event_loop: &winit::event_loop::ActiveEventLoop) {
        // Video mode first, in the order the objects need their context current:
        // drop the pump's window sink (painter -> composite FBO -> glutin
        // surface/context, see [`WindowFrameSink::teardown`]) while the **window**
        // context is current, then make the **player** context current and drop
        // the [`MpvGlSession`] so `mpv_render_context_free` runs against the
        // context its GL callbacks resolve through. Struct field drop order would
        // release the session before the sink, so both are taken out here.
        #[cfg(target_os = "linux")]
        if let Some(mut video) = self.video.take() {
            // Measure **before** the teardown: the sink's `Drop` releases the EGL
            // surface whose size it reported, and `VideoRun::frames` (the pump's
            // per-frame readback hashes) is dropped with the session. Doing it
            // here is what keeps the report non-empty for a run that did work.
            self.final_video_report =
                Some(video_run_report(&video, &self.progress, self.smoke_frames));
            // `WindowFrameSink::drop` is the teardown: it destroys the painter and
            // the composite FBO while the window context is current, then releases
            // surface -> context -> display -> window in field order.
            drop(video.sink.take());
            if let Err(err) = video.shared.make_current() {
                log::debug!("video mode: could not rebind the player context: {err}");
            }
            drop(video.session.take());
        }
        if let Some(mut egui_glow) = self.egui_glow.take() {
            egui_glow.destroy();
            drop(egui_glow);
        }
        self.gl.take();
        if let Some(gl_window) = self.gl_window.take() {
            use glutin::context::PossiblyCurrentGlContext as _;
            let GlutinWindowContext {
                gl_surface,
                gl_context,
                gl_display,
                window,
            } = gl_window;
            if let Err(err) = gl_context.make_not_current_in_place() {
                log::debug!("window host: could not unbind the gl context: {err}");
            }
            drop(gl_surface);
            drop(gl_context);
            drop(gl_display);
            drop(window);
        }
    }
}

/// Window + `glow` context for the GL functions, as in the upstream example
/// (`glow::Context::from_loader_function` fed by glutin's `get_proc_address`).
fn create_display(
    event_loop: &winit::event_loop::ActiveEventLoop,
    title: &str,
    smoke_frames: Option<u32>,
    player_frame: Option<[u32; 2]>,
) -> Result<(GlutinWindowContext, glow::Context), Box<dyn Error>> {
    let glutin_window_context = GlutinWindowContext::new(
        event_loop,
        title,
        swap_interval_policy(smoke_frames),
        player_frame,
    )?;
    // `glow` panics here when the driver gives it nothing to work with: it reads
    // `GL_VERSION` from the context made current by `GlutinWindowContext::new`
    // and panics if the answer is null, then parses that answer with `unwrap`.
    // Both are a driver's answer rather than a bug of ours, so they are reported
    // to the user instead of aborting the run.
    let gl = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // SAFETY: the loader resolves against the context `GlutinWindowContext`
        // made current immediately above, and that context outlives this call.
        unsafe {
            glow::Context::from_loader_function(|s| {
                let Ok(s) = std::ffi::CString::new(s) else {
                    // A GL function name never contains a NUL byte; if one did,
                    // glutin would be asked for a name we cannot pass and would
                    // answer with null, which `glow` then reports as unsupported.
                    log::error!("gl function name {s:?} cannot be passed to C");
                    return std::ptr::null();
                };
                glutin_window_context.get_proc_address(&s)
            })
        }
    }))
    .map_err(|_| {
        "the GL driver could not give glow a readable context (see the panic line above)"
    })?;

    Ok((glutin_window_context, gl))
}

/// The **window-side** [`FrameSink`]: the frame the pump composites is
/// painted in the window's own EGL context and handed to the window by
/// [`GlutinWindowContext::swap_buffers`].
///
/// The frame is a **copy**, not a shared texture: step 1 of the pump read the
/// player's RGBA bytes out of the app FBO in
/// the player's surfaceless context ([`SharedGl::make_current`]) and those bytes
/// are what the pump uploads as an egui texture; this sink's painter uploads them
/// into a GL texture of its **own** (window) context. Nothing here claims an
/// EGL-shared or zero-copy path; the alternative (one EGL context shared with
/// the mpv render context) is unmeasured.
///
/// One [`FrameSink::present`] call, in this order:
///
/// 1. re-bind the window EGL context ([`GlutinWindowContext::bind`]): step 1 made
///    the player context current and EGL allows one current context per thread;
/// 2. bind this sink's own composite [`AppFbo`] (allocated on the window context)
///    and paint the tessellated egui frame into it with [`egui_glow::Painter`],
///    applying this frame's texture deltas;
/// 3. `bind_framebuffer(_, None)` (the window surface's default framebuffer) and
///    paint the **same** tessellated primitives again with no deltas left;
/// 4. [`GlutinWindowContext::swap_buffers`] - the only step that puts pixels on
///    the window - then count the frame and record the surface size;
/// 5. [`FrameSink::read_composite`] (called by the pump after `present`) re-binds
///    the composite target and reads it back: those are the only pixels a
///    measured run hashes.
///
/// Steps 2 and 3 are two paints of the same primitives on purpose: a swapped front
/// buffer cannot be read back reliably, so the measured pixels come from a target
/// this sink owns. That costs a second paint pass, which is why a run reports its
/// own per-frame `present_us` and wall time instead of a frame rate.
///
/// No scaling: both paints use [`VideoMode::frame`] as the screen size. A window
/// surface of another size is recorded ([`Self::surface_px`,
/// `VideoRunReport::surface_matches_frame`]) instead of letterboxed or scaled.
#[cfg(target_os = "linux")]
pub(crate) struct WindowFrameSink {
    gl_window: GlutinWindowContext,
    gl: Arc<glow::Context>,
    composite: AppFbo,
    painter: Painter,
    /// Set once by [`Drop`]: `AppFbo::destroy` is not idempotent, so this guard
    /// is the only thing keeping a second teardown from double-deleting.
    composite_destroyed: bool,
    frame: [i32; 2],
    surface: [u32; 2],
    resizes: u64,
    painted: u64,
}

#[cfg(target_os = "linux")]
impl WindowFrameSink {
    /// Create the sink for a window that already exists: its composite target,
    /// then the painter. Both need the **window** context current, which
    /// [`GlutinWindowContext::new`] left it as.
    ///
    /// # Errors
    ///
    /// A bounded description of the failed allocation, or the frame size being
    /// unpaintable.
    fn create(
        gl_window: GlutinWindowContext,
        gl: Arc<glow::Context>,
        frame: [u32; 2],
    ) -> Result<Self, String> {
        let [width, height] = frame;
        let (Ok(width), Ok(height)) = (i32::try_from(width), i32::try_from(height)) else {
            return Err(format!(
                "player frame {width}x{height} is not a paintable GL size [U]"
            ));
        };
        let inner = gl_window.window().inner_size();
        gl_window.resize(inner);
        let composite = AppFbo::create(&gl, width, height)
            .map_err(|e| format!("window composite AppFbo {width}x{height}: {e}"))?;
        let painter = egui_glow::Painter::new(Arc::clone(&gl), "", None, false)
            .map_err(|e| format!("window egui_glow::Painter::new: {e}"))?;
        Ok(Self {
            gl_window,
            gl,
            composite,
            painter,
            composite_destroyed: false,
            frame: [width, height],
            surface: [inner.width, inner.height],
            resizes: 0,
            painted: 0,
        })
    }

    /// The winit window this sink presents to.
    #[must_use]
    pub fn window(&self) -> &winit::window::Window {
        self.gl_window.window()
    }

    /// The window surface this sink last painted/swapped at, in physical pixels.
    #[must_use]
    pub fn surface_px(&self) -> [u32; 2] {
        self.surface
    }

    /// `Resized` handling: resize the GL surface and record the new size. The
    /// frame geometry does **not** follow it (see [`VideoMode`]).
    pub fn resize_surface(&mut self, physical_size: winit::dpi::PhysicalSize<u32>) {
        self.gl_window.resize(physical_size);
        self.surface = [physical_size.width, physical_size.height];
        self.resizes = self.resizes.saturating_add(1);
        log::info!(
            "video mode: window resized to {}x{}, player frame stays {}x{} (no scaling applied)",
            physical_size.width,
            physical_size.height,
            self.frame[0],
            self.frame[1]
        );
    }

    /// Frames this sink painted (`present` calls that reached `swap_buffers`).
    #[must_use]
    pub fn painted(&self) -> u64 {
        self.painted
    }

    /// `Resized` events this sink recorded.
    #[must_use]
    pub fn resizes(&self) -> u64 {
        self.resizes
    }
}

#[cfg(target_os = "linux")]
impl FrameSink for WindowFrameSink {
    fn size_px(&self) -> [u32; 2] {
        [
            u32::try_from(self.frame[0]).unwrap_or(1),
            u32::try_from(self.frame[1]).unwrap_or(1),
        ]
    }

    fn begin_paint(&mut self) -> Result<(), String> {
        self.gl_window
            .bind()
            .map_err(|e| format!("window context re-bind: {e}"))?;
        // SAFETY: `self.composite` is an FBO this sink created on `self.gl` and
        // still owns; binding it and setting the viewport to its own size is the
        // same pattern `gl/fbo.rs` uses when it creates an FBO.
        unsafe {
            use glow::HasContext as _;
            self.gl
                .bind_framebuffer(glow::FRAMEBUFFER, Some(self.composite.fbo_handle()));
            self.gl.viewport(0, 0, self.frame[0], self.frame[1]);
            let error = self.gl.get_error();
            if error != 0 {
                return Err(format!("window composite bind glGetError 0x{error:04X}"));
            }
        }
        Ok(())
    }

    fn present(
        &mut self,
        clipped: &[egui::ClippedPrimitive],
        textures_delta: &mut egui::TexturesDelta,
        pixels_per_point: f32,
    ) -> Result<(), String> {
        let size = self.size_px();
        self.painter.clear(size, [0.0, 0.0, 0.0, 1.0]);
        self.painter
            .paint_and_update_textures(size, pixels_per_point, clipped, textures_delta);
        // SAFETY: plain error query and default-framebuffer bind on this sink's own
        // context; FBO 0 of a glutin surface is its back buffer.
        unsafe {
            use glow::HasContext as _;
            let error = self.gl.get_error();
            if error != 0 {
                return Err(format!("window composite paint glGetError 0x{error:04X}"));
            }
            self.gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        }
        self.painter.clear(size, [0.0, 0.0, 0.0, 1.0]);
        self.painter.paint_and_update_textures(
            size,
            pixels_per_point,
            clipped,
            &mut Default::default(),
        );
        unsafe {
            use glow::HasContext as _;
            let error = self.gl.get_error();
            if error != 0 {
                return Err(format!("window back-buffer paint glGetError 0x{error:04X}"));
            }
        }
        self.gl_window.window().set_visible(true);
        self.gl_window
            .swap_buffers()
            .map_err(|e| format!("window swap_buffers: {e}"))?;
        self.painted = self.painted.saturating_add(1);
        Ok(())
    }

    fn read_composite(&mut self) -> Result<Vec<u8>, String> {
        self.gl_window
            .bind()
            .map_err(|e| format!("window context re-bind (readback): {e}"))?;
        // SAFETY: binding this sink's own FBO again after the swap, reading it, and
        // restoring FBO 0.
        unsafe {
            use glow::HasContext as _;
            self.gl
                .bind_framebuffer(glow::FRAMEBUFFER, Some(self.composite.fbo_handle()));
        }
        let pixels = self
            .composite
            .read_rgba(&self.gl)
            .map_err(|e| format!("window composite readback: {e}"))?;
        unsafe {
            use glow::HasContext as _;
            self.gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        }
        Ok(pixels)
    }
}

/// The sink's only teardown path. There is deliberately **one** of it: a second
/// `teardown(self)` alongside this `Drop` would run `AppFbo::destroy`
/// (`crates/plex-app/src/gl/fbo.rs`, which deletes the FBO and its texture with
/// no idempotence guard) twice.
///
/// Order: destroy the painter and the composite target while the **window**
/// context is still current (`Painter::destroy` is idempotent, `AppFbo::destroy`
/// is not), unbind that context, then let the fields release in declaration
/// order - `GlutinWindowContext`'s own order is
/// surface -> context -> display -> window (see its doc comment), and `gl` is
/// dropped after the EGL objects it loads through.
#[cfg(target_os = "linux")]
impl Drop for WindowFrameSink {
    fn drop(&mut self) {
        use glutin::context::PossiblyCurrentGlContext as _;
        // `present`/readback leave the composite FBO unbound (FBO 0), but the
        // painter's GL objects must go while the window context is current.
        if let Err(err) = self.gl_window.bind() {
            log::debug!("video mode: sink drop without window context: {err}");
        }
        self.painter.destroy();
        if !self.composite_destroyed {
            self.composite_destroyed = true;
            self.composite.destroy(&self.gl);
        }
        if let Err(err) = self.gl_window.gl_context.make_not_current_in_place() {
            log::debug!("video mode: could not unbind the window GL context: {err}");
        }
    }
}

/// What one bounded video run measured, taken from the pump's own
/// [`FrameReport`]s and the window sink - nothing in it is inferred from the
/// requested frame target.
#[cfg(target_os = "linux")]
pub struct VideoRunReport {
    /// The fixture handed to `loadfile`.
    pub fixture: String,
    /// The player frame size the window was created at.
    pub frame: [u32; 2],
    /// The window surface size the last frame was swapped at.
    pub surface: [u32; 2],
    /// `false` when the surface is not exactly `frame` (stated, never scaled).
    pub surface_matches_frame: bool,
    /// Pump frames completed ([`FramePump::frames`]).
    pub pump_frames: u64,
    /// Frames this host counted as presented ([`HostProgress::frames`]).
    pub frames_presented: u32,
    /// `WindowFrameSink::present` calls that reached `swap_buffers`.
    pub sink_paints: u64,
    /// `Resized` events the sink recorded.
    pub sink_resizes: u64,
    /// Frames whose readback had any non-zero channel, i.e. the window really
    /// received something other than the clear colour.
    pub composite_non_black_frames: u64,
    /// Composite hashes, one per pump frame that reported one.
    pub composite_hashes: Vec<u64>,
    /// Player-frame hashes, one per pump frame.
    pub video_hashes: Vec<u64>,
    /// Pump frames that failed, each ending the run.
    pub frame_errors: Vec<String>,
    /// The pacing this run asked the driver for ([`swap_interval_label`]).
    pub swap_interval: &'static str,
    /// Wall milliseconds of the whole run.
    pub wall_ms: u64,
    /// Peak resident size of this process in KiB ([`crate::render::peak_rss_kib`]).
    pub peak_rss_kib: u64,
}

#[cfg(target_os = "linux")]
impl VideoRunReport {
    /// One machine-readable line (`RUN_RESULT ...`), same style as the harness
    /// harness lines. The hashes are printed as 16 hex digits like
    /// [`FrameReport::report_line`] does.
    #[must_use]
    pub fn report_line(&self) -> String {
        let last_composite = self
            .composite_hashes
            .last()
            .map_or_else(|| "none".to_owned(), |h| format!("{h:016x}"));
        let last_video = self
            .video_hashes
            .last()
            .map_or_else(|| "none".to_owned(), |h| format!("{h:016x}"));
        format!(
            "RUN_RESULT mode=window-video fixture={} frame={}x{} surface={}x{} surface_matches_frame={} pump_frames={} frames_presented={} sink_paints={} sink_resizes={} composite_non_black_frames={} last_video_hash={last_video} last_composite_hash={last_composite} composite_hashes=[{}] swap_interval={} wall_ms={} peak_rss_kib={} frame_errors={}",
            self.fixture,
            self.frame[0],
            self.frame[1],
            self.surface[0],
            self.surface[1],
            self.surface_matches_frame,
            self.pump_frames,
            self.frames_presented,
            self.sink_paints,
            self.sink_resizes,
            self.composite_non_black_frames,
            self
                .composite_hashes
                .iter()
                .map(|h| format!("{h:016x}"))
                .collect::<Vec<_>>()
                .join(","),
            self.swap_interval,
            self.wall_ms,
            self.peak_rss_kib,
            self.frame_errors.len(),
        )
    }
}

/// What a video run measured, read out of its [`VideoRun`] by
/// [`HostApp::exiting`] via this helper.
///
/// Every field here is read while the run still owns it: `pump` (completed frame
/// count), `sink` (paint/resize counts and the surface size last swapped at),
/// `frames` (the per-frame readbacks that give the composite/video hashes and
/// the non-black count), `frame_errors`, `mode` (fixture and frame size) and
/// `started`. A run that recorded nothing reports zeros and
/// `surface_matches_frame: false`: the two are derived from the same recorded
/// frames, so an empty run can never look like a matching one.
#[cfg(target_os = "linux")]
fn video_run_report(
    video: &VideoRun<'_>,
    progress: &HostProgress,
    smoke_frames: Option<u32>,
) -> VideoRunReport {
    let frames_presented = progress.frames();
    let surface = video
        .sink
        .as_ref()
        .map_or([0, 0], WindowFrameSink::surface_px);
    let recorded = frames_presented > 0 || !video.frames.is_empty();
    VideoRunReport {
        fixture: video.mode.fixture.clone(),
        frame: video.mode.frame,
        surface,
        surface_matches_frame: recorded && video.mode.frame == surface,
        pump_frames: video.pump.frames(),
        frames_presented,
        sink_paints: video
            .sink
            .as_ref()
            .map(WindowFrameSink::painted)
            .unwrap_or(0),
        sink_resizes: video
            .sink
            .as_ref()
            .map(WindowFrameSink::resizes)
            .unwrap_or(0),
        composite_non_black_frames: video
            .frames
            .iter()
            .filter(|f| {
                f.composited
                    .as_ref()
                    .is_some_and(|c| c.composite.non_black > 0)
            })
            .count()
            .try_into()
            .unwrap_or(u64::MAX),
        composite_hashes: video
            .frames
            .iter()
            .filter_map(|f| f.composited.as_ref().map(|c| c.composite.hash))
            .collect(),
        video_hashes: video.frames.iter().map(|f| f.video.hash).collect(),
        frame_errors: video.frame_errors.clone(),
        swap_interval: swap_interval_label(swap_interval_policy(smoke_frames)),
        wall_ms: u64::try_from(video.started.elapsed().as_millis()).unwrap_or(u64::MAX),
        peak_rss_kib: crate::render::peak_rss_kib(),
    }
}

/// [`run_host_report`] in **video mode**: play `fixture` in the window this host
/// creates and pump `smoke_frames` frames through [`FramePump`] into
/// [`WindowFrameSink`], reporting what each frame measured.
///
/// The player half is the [`SharedGl`] + [`MpvGlSession`]; the window
/// half is this file's [`GlutinWindowContext`]. The run is bounded by the same
/// [`watchdog_ms`] watcher as [`run_host_report`]: a run that cannot present
/// frames ends with its measured counts, not with a hang or a panic.
///
/// The returned report is the one [`HostApp::exiting`] recorded
/// ([`HostApp::final_video_report`]): the run's own pump/sink/frame data is
/// only readable before that teardown drops them, so it is measured there and
/// handed back here unchanged.
///
/// # Errors
///
/// The first window/GL/player/pump failure, with what the run reached. No Plex
/// sign-in is involved: `fixture` is a local file.
#[cfg(target_os = "linux")]
pub fn run_host_video_frames(
    fixture: &str,
    smoke_frames: u32,
) -> Result<VideoRunReport, Box<dyn Error>> {
    let shared = SharedGl::surfaceless().map_err(|e| format!("SharedGl::surfaceless: {e}"))?;
    let event_loop = winit::event_loop::EventLoop::<UserEvent>::with_user_event()
        .build()
        .map_err(|err| format!("winit EventLoop::build: {err}"))?;
    let proxy = event_loop.create_proxy();
    let deadline = watchdog_ms();
    let progress = Arc::new(HostProgress::new(Some(smoke_frames), deadline));
    let mut app = HostApp::new_video(
        proxy.clone(),
        Some(smoke_frames),
        Arc::clone(&progress),
        VideoMode::new(fixture),
        &shared,
    );
    let watch = Arc::clone(&progress);
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(watch.deadline_ms));
        if watch.frames() >= smoke_frames {
            return;
        }
        if let Err(err) = proxy.send_event(UserEvent::Deadline) {
            log::warn!("cannot send window-host deadline event: {err}");
        }
        std::thread::sleep(std::time::Duration::from_millis(
            watch.deadline_ms.min(5_000),
        ));
        if watch.frames() >= smoke_frames {
            return;
        }
        if watch.claim() {
            eprintln!(
                "RUN_FAILURE {}",
                watch.failure_msg(
                    "video window host deadline expired: the event loop handled no deadline event",
                )
            );
            std::process::exit(1);
        }
    });
    log::info!(
        "video window host: event loop built target {} frame(s) deadline {} ms swap_interval {} fixture={}",
        smoke_frames,
        deadline,
        swap_interval_label(swap_interval_policy(Some(smoke_frames))),
        fixture,
    );
    event_loop
        .run_app(&mut app)
        .map_err(|err| format!("winit run_app: {err}"))?;

    let Some(report) = app.final_video_report.take() else {
        // Only reachable if the event loop exited without running
        // `HostApp::exiting`; the counts the watcher saw are reported instead.
        return Err(format!(
            "video window host: HostApp::exiting recorded no report (progress frames={}, pump target={smoke_frames})",
            progress.frames(),
        )
        .into());
    };
    log::info!("{}", report.report_line());
    match app.failure {
        Some(err) => Err(format!("{err}; {}", report.report_line()).into()),
        None => Ok(report),
    }
}

/// Default smoke-run deadline for [`run_host_report`], in milliseconds. A
/// compositor that accepts the client but never configures a window would
/// otherwise leave `run_app` blocked with no output at all.
pub const HOST_SMOKE_WATCHDOG_MS: u64 = 20_000;

/// Upper bound accepted for `PLEX_HOST_WATCHDOG_MS` by [`watchdog_ms`], and the
/// reason it is not larger: a run whose watchdog exceeds what its harness lets it
/// wait for can never self-report (the harness kills it first), which is what a
/// 120 s clamp allowed against a 60 s `timeout`.
pub const HOST_WATCHDOG_MAX_MS: u64 = 60_000;

/// Watchdog for this run: [`HOST_SMOKE_WATCHDOG_MS`] unless `PLEX_HOST_WATCHDOG_MS`
/// asks for something else, clamped to `500..=HOST_WATCHDOG_MAX_MS` (an
/// unparsable value is ignored and the default is used).
pub fn watchdog_ms() -> u64 {
    let configured = std::env::var("PLEX_HOST_WATCHDOG_MS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(|ms| ms.clamp(500, HOST_WATCHDOG_MAX_MS));
    configured.unwrap_or(HOST_SMOKE_WATCHDOG_MS)
}

/// Presentation pacing of this host, as requested from the GL driver.
///
/// Measured, not assumed: with `SwapInterval::Wait(1)` (the upstream default)
/// against Mesa software GL and `kwin_wayland --virtual`, this host created a
/// window, presented exactly one frame and then never made progress again - no
/// further `RedrawRequested`, and the [`UserEvent::Deadline`] event was never
/// answered either (reproduced with `PLEX_HOST_WATCHDOG_MS=60000` over a 65 s
/// wall window). With a watcher that can answer, the same configuration exits
/// 1 and self-reports "frames painted 1 of 8". A bounded smoke run therefore
/// asks for [`glutin::surface::SwapInterval::DontWait`] so the run stays
/// measurable; an interactive run (`smoke_frames: None`) keeps the upstream
/// vsync behaviour (that path is NOT measured in the headless test environment). `PLEX_HOST_SWAP_INTERVAL=0|1` overrides both cases.
///
/// What this does NOT claim: that waiting swaps are broken in general. It is
/// what this host measured in the headless test environment with this compositor, and the mode
/// actually used is stated in the log line and in the binary's JSON report.
pub fn swap_interval_policy(smoke_frames: Option<u32>) -> glutin::surface::SwapInterval {
    let forced = std::env::var("PLEX_HOST_SWAP_INTERVAL")
        .ok()
        .and_then(|v| v.trim().parse::<u32>().ok());
    let wait = match forced {
        Some(0) => false,
        Some(1) => true,
        // Unset or unparsable: pace an interactive run, do not pace a bounded one.
        _ => smoke_frames.is_none(),
    };
    if wait {
        glutin::surface::SwapInterval::Wait(NonZeroU32::MIN)
    } else {
        glutin::surface::SwapInterval::DontWait
    }
}

/// `"wait1"` / `"dontwait"` - the pacing actually requested, for logs and the
/// smoke report, so a run can be attributed to the policy it used.
pub fn swap_interval_label(interval: glutin::surface::SwapInterval) -> &'static str {
    match interval {
        glutin::surface::SwapInterval::DontWait => "dontwait",
        glutin::surface::SwapInterval::Wait(_) => "wait1",
    }
}

/// Run the window until it is closed, drawing `ui` every frame.
///
/// # Errors
///
/// When the event loop or the window/GL context cannot be created.
pub fn run_host_ui(ui: AppUi<'_>) -> Result<(), Box<dyn Error>> {
    let event_loop = winit::event_loop::EventLoop::<UserEvent>::with_user_event()
        .build()
        .map_err(|err| format!("winit EventLoop::build: {err}"))?;
    let progress = Arc::new(HostProgress::new(None, 0));
    let mut app = HostApp::new(event_loop.create_proxy(), None, progress);
    app.ui = Some(ui);
    app.driver = super::drive::Driver::from_env();
    event_loop
        .run_app(&mut app)
        .map_err(|err| format!("winit run_app: {err}"))?;
    match app.failure.take() {
        Some(err) => Err(err),
        None => Ok(()),
    }
}

/// Run the window host until the window closes, or - when `smoke_frames` is
/// `Some(n)` - until n frames have been presented.
///
/// Returns `Err` with the first window/GL failure instead of panicking, so a
/// machine without a usable compositor reports what actually failed.
pub fn run_host(smoke_frames: Option<u32>) -> Result<(), Box<dyn Error>> {
    run_host_report(smoke_frames).map(|_frames| ())
}

/// Same run as [`run_host`], but reporting how many frames were *actually*
/// painted and presented (a window closed early is not counted as n frames).
///
/// When `smoke_frames` is `Some(n)` a watcher thread waits [`watchdog_ms`]
/// milliseconds and then asks the event loop to end the run with a stated frame
/// count ([`UserEvent::Deadline`]). If the loop is blocked inside a present call
/// it cannot answer that event, so the watcher reports the measured progress on
/// stderr and ends the process itself (exit 1): a bounded smoke run must either
/// print its JSON report or print a failure that says what it reached.
pub fn run_host_report(smoke_frames: Option<u32>) -> Result<u32, Box<dyn Error>> {
    let event_loop = winit::event_loop::EventLoop::<UserEvent>::with_user_event()
        .build()
        .map_err(|err| format!("winit EventLoop::build: {err}"))?;
    let proxy = event_loop.create_proxy();
    let deadline = watchdog_ms();
    let progress = Arc::new(HostProgress::new(
        smoke_frames,
        if smoke_frames.is_some() { deadline } else { 0 },
    ));
    let mut app = HostApp::new(proxy.clone(), smoke_frames, Arc::clone(&progress));
    if let Some(target) = smoke_frames {
        let watch = Arc::clone(&progress);
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(watch.deadline_ms));
            if watch.frames() >= target {
                return;
            }
            if let Err(err) = proxy.send_event(UserEvent::Deadline) {
                log::warn!("cannot send window-host deadline event: {err}");
            }
            // Give the event loop a chance to end the run itself before this
            // thread ends it - bounded so a stuck run cannot outlive the harness.
            std::thread::sleep(std::time::Duration::from_millis(
                watch.deadline_ms.min(5_000),
            ));
            if watch.frames() >= target {
                return;
            }
            if watch.claim() {
                // States only what was measured: the counter value, and that no
                // Deadline event was handled. Whether the loop was blocked
                // inside a present call or idle is not knowable from here.
                eprintln!(
                    "RUN_FAILURE {}",
                    watch.failure_msg(
                        "window host deadline expired: the event loop handled no deadline event",
                    )
                );
                std::process::exit(1);
            }
        });
        log::info!(
            "window host: event loop built, target {} frame(s), deadline {} ms, swap_interval {}",
            target,
            deadline,
            swap_interval_label(swap_interval_policy(Some(target))),
        );
    }
    event_loop
        .run_app(&mut app)
        .map_err(|err| format!("winit run_app: {err}"))?;
    log::debug!("window host painted {} frame(s)", progress.frames());
    match app.failure {
        Some(err) => Err(err),
        None => Ok(progress.frames()),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        swap_interval_label, swap_interval_policy, watchdog_ms, HostProgress,
        HOST_SMOKE_WATCHDOG_MS, HOST_WATCHDOG_MAX_MS,
    };

    #[test]
    fn progress_reports_only_presented_frames_and_one_claim() {
        let progress = HostProgress::new(Some(3), 1_000);
        assert_eq!(progress.frames(), 0);
        assert_eq!(progress.present(), 1);
        assert_eq!(progress.present(), 2);
        assert_eq!(progress.frames(), 2);
        // The failure text states the measured count, never the requested one.
        let msg = progress.failure_msg("window host deadline expired");
        assert!(msg.contains("frames painted 2 of 3"), "{msg}");
        assert!(msg.contains("deadline 1000 ms"), "{msg}");
        // Only one reporting path may report a stuck run.
        assert!(progress.claim());
        assert!(!progress.claim());
    }

    #[test]
    fn default_watchdog_is_the_documented_bound() {
        // No PLEX_HOST_WATCHDOG_MS in the test environment (cargo test does not set it);
        // if a harness ever does, this test only asserts the clamp bounds.
        let ms = watchdog_ms();
        assert!((500..=HOST_WATCHDOG_MAX_MS).contains(&ms), "{ms}");
        if std::env::var("PLEX_HOST_WATCHDOG_MS").is_err() {
            assert_eq!(ms, HOST_SMOKE_WATCHDOG_MS);
        }
    }

    /// The pacing policy is what a run reports, so its two cases and the
    /// override are pinned here (this test does not read the environment of a
    /// real run; it sets one for itself).
    #[test]
    fn swap_interval_policy_paces_interactive_runs_only() {
        use std::env::{remove_var, set_var};
        remove_var("PLEX_HOST_SWAP_INTERVAL");
        assert_eq!(
            swap_interval_label(swap_interval_policy(Some(8))),
            "dontwait"
        );
        assert_eq!(swap_interval_label(swap_interval_policy(None)), "wait1");
        set_var("PLEX_HOST_SWAP_INTERVAL", "1");
        assert_eq!(swap_interval_label(swap_interval_policy(Some(8))), "wait1");
        set_var("PLEX_HOST_SWAP_INTERVAL", "0");
        assert_eq!(swap_interval_label(swap_interval_policy(None)), "dontwait");
        set_var("PLEX_HOST_SWAP_INTERVAL", "nonsense");
        assert_eq!(
            swap_interval_label(swap_interval_policy(Some(8))),
            "dontwait"
        );
        remove_var("PLEX_HOST_SWAP_INTERVAL");
    }
}
