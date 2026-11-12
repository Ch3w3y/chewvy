//! The frame side of the desktop client: the per-frame pump and the player
//! source contract it drives.
//!
//! In this module: [`FramePump`], [`FrameSource`], [`FrameSink`],
//! [`GlFrameSource`] (one frame over [`MpvGlSession`]) and the
//! [`VideoTexture`]/overlay hand-off between them. The window itself (winit
//! window + glutin display/surface/context + `egui_glow` painter) lives in
//! [`crate::shell::host`], which creates it in `ApplicationHandler::resumed` and
//! runs one pump frame per `RedrawRequested`.
//!
//! ## The per-frame order
//!
//! [`FramePump::frame`] is the only place that order is defined, and it is
//! defined *in code*, not in prose:
//!
//! 1. **player render** - exactly one frame through [`FrameSource::render_frame`]:
//!    context current -> `mpv_render_context_render(fbo = app FBO)` -> FBO status
//!    -> RGBA readback + hash ([`MpvGlSession::step_frame`]);
//! 2. **frame -> egui texture** - the RGBA bytes from step 1 go to
//!    [`VideoTexture`] ([`VideoTexture::load`] on the first frame, then
//!    [`VideoTexture::update`]);
//! 3. **egui overlay** - [`egui::Context::run_ui`] draws the video quad
//!    ([`VideoTexture::paint_at`]) plus the transport overlay
//!    ([`draw_transport`]) plus whatever the caller's `draw` closure adds;
//! 4. **present** - a [`FrameSink`] paints that geometry over the frame and hands
//!    the frame to the window system (`egui_glow::EguiGlow` in the window host).
//!
//! [`FramePump`] refuses to run step 2 or 3 before step 1 has produced a frame,
//! and records every step it ran, in order, in [`FramePump::steps`].
//!
//! ## Two GL contexts, and what crosses between them
//!
//! The player renders in the surfaceless EGL context
//! ([`crate::gl::SharedGl`]); egui paints in the context glutin created for the
//! window surface. A glutin window surface and that surfaceless context are not
//! the same EGL object, so the frame crosses from the player context to the egui
//! context as an **RGBA copy** (step 1 readback -> step 2 upload). The zero-copy
//! variant (one EGL context, egui sampling [`crate::gl::AppFbo`]'s own texture)
//! is not implemented here.

use std::time::Duration;

use egui::{Context as EguiContext, RawInput, Rect};

use crate::gl::fbo::CapturedFrame;
use crate::overlay::{draw_transport, TransportState, VideoTexture};
use crate::render::MpvGlSession;

/// One step of the per-frame order, recorded in the order it ran.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameStep {
    /// Step 1: `mpv_render_context_render` into the app FBO + RGBA readback.
    Render,
    /// Step 2: those RGBA bytes became an egui texture.
    Upload,
    /// Step 3: `egui::Context::run_ui` produced the frame's geometry.
    Overlay,
    /// Step 4: the sink painted that geometry over the frame.
    Present,
}

/// Step 1: the player side of one frame.
///
/// Implemented by the GL session holder ([`GlFrameSource`]). The pump calls only
/// this method, so the pump's ordering is testable without EGL, libmpv or a
/// window.
pub trait FrameSource {
    /// Render exactly one player frame and return its summary plus its RGBA bytes.
    ///
    /// # Errors
    ///
    /// A bounded description of the render failure.
    fn render_frame(&mut self) -> Result<(CapturedFrame, Vec<u8>), String>;
}

/// Step 4: the side that paints the overlay and hands the frame over.
pub trait FrameSink {
    /// The size, in pixels, the overlay is painted at.
    #[must_use]
    fn size_px(&self) -> [u32; 2];

    /// Prepare this sink to receive painting (bind the target, make the context
    /// current). Must leave the sink's own target bound.
    ///
    /// # Errors
    ///
    /// A bounded description of the failure.
    fn begin_paint(&mut self) -> Result<(), String> {
        Ok(())
    }

    /// Paint the tessellated overlay and apply egui's texture deltas.
    ///
    /// # Errors
    ///
    /// A bounded description of the failure.
    fn present(
        &mut self,
        clipped: &[egui::ClippedPrimitive],
        textures_delta: &mut egui::TexturesDelta,
        pixels_per_point: f32,
    ) -> Result<(), String>;

    /// Read back what this sink now holds (video **plus** the painted overlay).
    ///
    /// Only sinks that can read their own target implement this; the returned
    /// pixels are what [`CompositedFrame`] measures.
    ///
    /// # Errors
    ///
    /// A bounded description of why this sink cannot read back.
    fn read_composite(&mut self) -> Result<Vec<u8>, String> {
        Err("sink cannot read back its own target [U]".to_owned())
    }
}

/// A frame that is video *and* overlay, measured in one readback.
///
/// This is how the harness distinguishes "mpv drew a frame" from "the app drew a
/// frame with its UI on it": both hashes come from the same pixels the sink holds
/// after [`FrameSink::present`]. It only exists for a sink that can read back
/// (smoke mode); windowed mode reports `None` and is measured by its own log
/// lines instead.
#[derive(Clone, Debug, PartialEq)]
pub struct CompositedFrame {
    /// The video-only summary from step 1.
    pub video: CapturedFrame,
    /// The summary of the sink's target after the overlay was painted.
    pub composite: CapturedFrame,
    /// `true` when the two differ, i.e. the overlay changed the presented pixels.
    pub overlay_visible: bool,
}

/// What one pump frame did - the numbers the harness log prints.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameReport {
    /// 1-based frame number.
    pub frame: u64,
    /// Wall microseconds of step 1 (player render + readback).
    pub render_us: u64,
    /// Wall microseconds of step 2 (texture upload).
    pub upload_us: u64,
    /// Wall microseconds of step 3 (egui run + tessellate).
    pub overlay_us: u64,
    /// Wall microseconds of step 4 (paint + present).
    pub present_us: u64,
    /// The video-only summary (step 1).
    pub video: CapturedFrame,
    /// How many times the frame's pixels went into the egui video texture
    /// (step 2), as the texture itself reports it ([`VideoTexture::uploads`]).
    pub texture_uploads: u64,
    /// The composited summary, when the sink could read back.
    pub composited: Option<CompositedFrame>,
    /// Control actions the transport overlay reported for this frame.
    pub overlay_actions: Vec<crate::overlay::OverlayAction>,
    /// The steps this frame ran, in the order they ran.
    pub steps: Vec<FrameStep>,
}

impl FrameReport {
    /// One machine-readable line per frame (`FRAME …`), same style as the M5b
    /// harness lines.
    #[must_use]
    pub fn report_line(&self) -> String {
        let composited = self.composited.as_ref().map_or_else(
            || "composited=none".to_owned(),
            |c| {
                format!(
                    "composite_hash={:016x} composite_non_black={} overlay_visible={}",
                    c.composite.hash, c.composite.non_black, c.overlay_visible
                )
            },
        );
        format!(
            "FRAME n={} video_hash={:016x} video_non_black={} render_us={} upload_us={} texture_uploads={} overlay_us={} present_us={} {} overlay_actions={} steps=[{}",
            self.frame,
            self.video.hash,
            self.video.non_black,
            self.render_us,
            self.upload_us,
            self.texture_uploads,
            self.overlay_us,
            self.present_us,
            composited,
            self.overlay_actions.len(),
            self.steps
                .iter()
                .map(|s| format!("{s:?}"))
                .collect::<Vec<_>>()
                .join(","),
        )
    }
}

/// The per-frame pump: it owns the egui context, the video texture and the
/// **order** of the four steps, and refuses to run a later step without the
/// earlier one.
pub struct FramePump {
    egui: EguiContext,
    video: Option<VideoTexture>,
    steps: Vec<FrameStep>,
    frames: u64,
}

impl Default for FramePump {
    fn default() -> Self {
        Self::new()
    }
}

impl FramePump {
    /// An empty pump (no texture yet: the first frame's `load` allocates it).
    #[must_use]
    pub fn new() -> Self {
        Self {
            egui: EguiContext::default(),
            video: None,
            steps: Vec::new(),
            frames: 0,
        }
    }

    /// Frames completed so far.
    #[must_use]
    pub fn frames(&self) -> u64 {
        self.frames
    }

    fn split(&mut self) -> (&EguiContext, &mut Option<VideoTexture>) {
        (&self.egui, &mut self.video)
    }

    /// Run **one** frame in the deliverable's order.
    ///
    /// `draw` is the app's own UI: it is called with the overlay `Ui` after the
    /// video quad has been painted into it, so the app's widgets are on top of the
    /// video (and the transport overlay is drawn by the pump, which is the only
    /// thing that knows the frame's geometry).
    ///
    /// # Errors
    ///
    /// The first step that failed, with a bounded description. A failed frame is
    /// not recorded as a frame ([`Self::frames`] does not grow).
    pub fn frame<S, K, D>(
        &mut self,
        source: &mut S,
        sink: &mut K,
        transport: &TransportState,
        mut draw: D,
    ) -> Result<FrameReport, String>
    where
        S: FrameSource + ?Sized,
        K: FrameSink + ?Sized,
        D: FnMut(&mut egui::Ui),
    {
        self.steps.clear();

        // Step 1: player render -> app FBO -> RGBA readback.
        let t0 = std::time::Instant::now();
        let (video, rgba) = source.render_frame()?;
        let render_us = duration_us(t0.elapsed());
        self.steps.push(FrameStep::Render);

        // Step 2: those bytes become an egui texture (load, then update).
        let t1 = std::time::Instant::now();
        let [sink_w, sink_h] = sink.size_px();
        let size = [
            frame_px(sink_w, "sink width")?,
            frame_px(sink_h, "sink height")?,
        ];
        let uploaded = {
            let (ctx, video) = self.split();
            if let Some(tex) = video.as_mut() {
                tex.update(&rgba).map_err(|e| e.to_string())?;
            } else {
                let mut tex = VideoTexture::load(ctx, size, &rgba).map_err(|e| e.to_string())?;
                // The first frame's pixels are already in the texture; measure the
                // upload the `load` performed.
                tex.update(&rgba).map_err(|e| e.to_string())?;
                *video = Some(tex);
            }
            video.as_ref().map(VideoTexture::uploads).unwrap_or(0)
        };
        let texture_uploads = uploaded;
        let upload_us = duration_us(t1.elapsed());
        self.steps.push(FrameStep::Upload);

        // Step 3: the egui pass - video quad first, then the transport overlay,
        // then the app's own screen widgets.
        let t2 = std::time::Instant::now();
        let (clipped, mut textures_delta, pixels_per_point, overlay_actions) = {
            // No window-system input reaches the pump yet: it derives its own
            // viewport from the sink's size.
            let mut input = RawInput::default();
            let (ctx, video) = self.split();
            let [tex_w, tex_h] = size;
            input.screen_rect = Some(Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(
                    f32::from(u16::try_from(tex_w).unwrap_or(u16::MAX)),
                    f32::from(u16::try_from(tex_h).unwrap_or(u16::MAX)),
                ),
            ));
            // egui 0.36.1 takes the scale factor *in* through
            // `ViewportInfo::native_pixels_per_point` (data/input/viewport_info.rs:43);
            // `RawInput` has no `pixels_per_point` field. A missing or invalid
            // native scale is normalised to 1.0 so the pump never tessellates at 0.
            let native = input.viewport().native_pixels_per_point;
            if native.is_none_or(|scale| !scale.is_finite() || scale <= 0.0) {
                // `RawInput::viewport()` is a read-only accessor
                // (egui-0.36.1/src/data/input/raw_input.rs:116); the writable place
                // is the `viewports` map keyed by `viewport_id` (:23).
                input
                    .viewports
                    .entry(input.viewport_id)
                    .or_default()
                    .native_pixels_per_point = Some(1.0);
            }
            let mut actions = Vec::new();
            let out = ctx.run_ui(input, |ui| {
                if let Some(tex) = video.as_ref() {
                    tex.paint_at(ui, ui.max_rect());
                }
                actions = draw_transport(ui, transport);
                draw(ui);
            });
            let ppp = out.pixels_per_point;
            let clipped = ctx.tessellate(out.shapes, ppp);
            (clipped, out.textures_delta, ppp, actions)
        };
        let overlay_us = duration_us(t2.elapsed());
        self.steps.push(FrameStep::Overlay);

        // Step 4: paint it over the frame and hand the frame to the window system.
        let t3 = std::time::Instant::now();
        sink.begin_paint()?;
        sink.present(&clipped, &mut textures_delta, pixels_per_point)?;
        let present_us = duration_us(t3.elapsed());
        self.steps.push(FrameStep::Present);

        let composited = sink.read_composite().ok().map(|pixels| {
            let composite = CapturedFrame::summarize(&pixels);
            CompositedFrame {
                video: video.clone(),
                overlay_visible: composite.hash != video.hash,
                composite,
            }
        });

        self.frames = self.frames.saturating_add(1);
        Ok(FrameReport {
            frame: self.frames,
            render_us,
            upload_us,
            overlay_us,
            present_us,
            video,
            texture_uploads,
            composited,
            overlay_actions,
            steps: self.steps.clone(),
        })
    }
}

fn duration_us(d: std::time::Duration) -> u64 {
    u64::try_from(d.as_micros()).unwrap_or(u64::MAX)
}

/// A sink size becomes a texture size only if it is a real size.
///
/// This refuses instead of coercing: a 0-wide sink used to be silently turned
/// into a 1-pixel texture, which would have hidden a broken sink behind a
/// `Upload` step that looked successful.
fn frame_px(value: u32, what: &str) -> Result<usize, String> {
    if value == 0 {
        return Err(format!(
            "{what} is 0: cannot upload a frame texture of that size"
        ));
    }
    usize::try_from(value).map_err(|e| format!("{what} {value}: {e}"))
}

/// The player half of the pump: the [`MpvGlSession`], stepped once
/// per frame.
///
/// The session's own `loadfile` (token 1, issued by [`MpvGlSession::load`] when
/// the host opens the item) is the only command in this path; a pump frame only
/// renders what the player has produced.
pub struct GlFrameSource<'a, 'b> {
    session: &'b mut MpvGlSession<'a>,
    /// Bounded wait for a player frame before each render (see
    /// [`MpvGlSession::step_frame`]).
    poll: Duration,
}

impl<'a, 'b> GlFrameSource<'a, 'b> {
    /// A source over an already-open session, with an explicit per-frame poll
    /// budget (the window host passes [`FRAME_POLL_MS`](crate::shell::host::FRAME_POLL_MS)).
    pub fn with_poll(session: &'b mut MpvGlSession<'a>, poll: Duration) -> Self {
        Self { session, poll }
    }
}

impl FrameSource for GlFrameSource<'_, '_> {
    fn render_frame(&mut self) -> Result<(CapturedFrame, Vec<u8>), String> {
        let step = self
            .session
            .step_frame(self.poll)
            .map_err(|e| e.to_string())?;
        if !step.had_frame_update {
            return Err(format!(
                "no player frame within {:?}: rendered frame is the player's empty current image (video_non_black={}) [U]",
                self.poll, step.summary.non_black
            ));
        }
        Ok((step.summary, step.pixels))
    }
}
