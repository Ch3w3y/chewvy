//! The egui overlay and the app-FBO -> egui texture upload path.
//!
//! The app owns the texture. The subtitle-ownership **decision** is recorded in
//! `docs/CAPABILITIES.md`: subtitle drawing stays with mpv/libass and this overlay draws
//! controls only - it never draws subtitle glyphs. What is **[U]** is the
//! mechanism, i.e. whether libmpv's libass layer lands in
//! [`crate::gl::fbo::AppFbo`] at all. The software-GL harness measured:
//! mpv's own OSD text layer **does** reach this
//! app's FBO (`SUBOSD ... verdict=mpv-text-layer-did-reach-app-fbo`, 3 distinct
//! hashes / 40981 lit vs the osd-level-0 control's 2 / 40865 - the readback path
//! is calibrated), while all four subrip configs (`sub-id=no`, `1`, `auto`,
//! mpv's untouched default) show **no** hash or lit-pixel difference from either
//! control (`SUBDIFF hashes_only_in_case=0 max_non_black_delta=0`,
//! `SUBOWNERSHIP verdict=no-visible-difference-from-either-control`). A libass
//! glyph layer in this app's FBO has therefore never been measured.
//!
//! What this module does is deliberately independent of that answer: it turns
//! the app's readback of that FBO ([`crate::gl::fbo::AppFbo::read_rgba`]) into
//! an egui texture and paints the overlay controls *on top* of it. If a
//! subtitle-display discriminator (`sub-reason`/`sub-text`/`sub-start`/
//! `track-list`) ever shows a subtitle being displayed while our FBO shows
//! nothing, that decision is overturned and drawing subtitles here becomes a
//! separate decision.
//!
//! Two paths are described here and they are deliberately different:
//!
//! * the **upload path** (`egui::Context::load_texture` / `TextureHandle::set`
//!   with a [`egui::ColorImage`]) is backend-independent: it works with the glow
//!   painter, and it is the path the harness measures (per-frame upload pacing,
//!   `VideoTexture::report_line`);
//! * the **native-texture path** (`egui_glow::Painter::register_native_texture`
//!   of [`crate::gl::fbo::AppFbo::texture`], `egui_glow-0.36.1/src/painter.rs:649`)
//!   avoids the copy and is what a desktop shell should use once it owns a window
//!   surface. It needs a live EGL surface and is *not* measured here. The
//!   durations these types report exist only once a run happens.
//!
//! ## Row order
//!
//! `glReadPixels` returns scanlines bottom-up (GL convention); `egui::ColorImage`
//! is row-major top-down. [`flip_rows`] is the explicit conversion - egui 0.36.1
//! has no `ColorImage::flip` (its `ColorImage` API is
//! `new/filled/from_rgba_*/from_gray/region/...`), so the flip is ours and is
//! unit-tested instead of assumed.

use std::time::{Duration, Instant};

use egui::{
    Color32, ColorImage, Context, Rect, Response, Sense, TextureHandle, TextureOptions, Ui, Vec2,
};

use crate::gl::fbo::CapturedFrame;

/// Name of the single video texture in the egui texture namespace.
pub const VIDEO_TEXTURE_NAME: &str = "plex-video-frame";

/// Texture filtering for the video frame: video frames are 1:1 texels and are
/// replaced wholesale every frame, so nearest-neighbour avoids resampling cost.
pub const VIDEO_TEXTURE_OPTIONS: TextureOptions = TextureOptions::NEAREST;

/// Why one overlay step failed.
///
/// Plain struct (no `thiserror`: `Cargo.lock` is frozen and the gates run
/// `--locked`), naming the step and the reason, same shape as
/// [`crate::render::SessionError`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OverlayError {
    /// The step that failed, e.g. `read_rgba length`.
    pub step: String,
    /// The reason.
    pub detail: String,
}

impl OverlayError {
    fn new(step: &str, detail: impl std::fmt::Display) -> Self {
        Self {
            step: step.to_string(),
            detail: detail.to_string(),
        }
    }
}

impl std::fmt::Display for OverlayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.step, self.detail)
    }
}

impl std::error::Error for OverlayError {}

/// Reverse the scanline order of a bottom-up GL readback.
///
/// Returns a top-down buffer of the same length. A short/odd `src` is an error
/// in the caller's capture, not something to guess about, so the function
/// documents the requirement rather than padding.
#[must_use]
pub fn flip_rows(width: usize, height: usize, src: &[u8]) -> Vec<u8> {
    let row = width.saturating_mul(4);
    let mut out = vec![0u8; row.saturating_mul(height)];
    for y in 0..height {
        let src_row = height
            .saturating_sub(1)
            .saturating_sub(y)
            .saturating_mul(row);
        let Some(src_slice) = src.get(src_row..src_row.saturating_add(row)) else {
            // A truncated capture: keep the destination row black so a broken
            // readback shows up as a black band instead of wrong pixels.
            continue;
        };
        let Some(dst_slice) = out.get_mut(y * row..(y + 1) * row) else {
            continue;
        };
        dst_slice.copy_from_slice(src_slice);
    }
    out
}

/// A GL readback (bottom-up) as an egui image (top-down).
pub fn color_image_from_gl_rgba(size: [usize; 2], src: &[u8]) -> Result<ColorImage, OverlayError> {
    let [w, h] = size;
    let need = w.saturating_mul(h).saturating_mul(4);
    if src.len() != need {
        return Err(OverlayError::new(
            "color_image_from_gl_rgba length",
            format!("expected {need} bytes for {w}x{h} RGBA, got {}", src.len()),
        ));
    }
    Ok(ColorImage::from_rgba_unmultiplied(
        size,
        &flip_rows(w, h, src),
    ))
}

/// The app FBO frame uploaded into egui, with its measured upload pacing.
///
/// One texture handle, replaced in place each frame (`TextureHandle::set`) so egui
/// does not re-allocate a texture per video frame. The upload durations are the
/// egui-side cost of the copy path - measured only when an egui run happens,
/// and none is recorded yet: the software-GL harness measures the GL render loop
/// and its readback pacing, not the egui texture upload.
pub struct VideoTexture {
    tex: TextureHandle,
    size: [usize; 2],
    uploads: u64,
    upload_us: Vec<u64>,
    last_summary: Option<CapturedFrame>,
}

impl VideoTexture {
    /// Allocate the texture from the first captured frame.
    pub fn load(ctx: &Context, size: [usize; 2], rgba: &[u8]) -> Result<Self, OverlayError> {
        let t0 = Instant::now();
        let image = color_image_from_gl_rgba(size, rgba)?;
        let tex = ctx.load_texture(VIDEO_TEXTURE_NAME, image, VIDEO_TEXTURE_OPTIONS);
        let us = Self::elapsed_us(t0);
        Ok(Self {
            tex,
            size,
            uploads: 1,
            upload_us: vec![us],
            last_summary: Some(CapturedFrame::summarize(rgba)),
        })
    }

    /// Replace the texture contents with the next captured frame.
    pub fn update(&mut self, rgba: &[u8]) -> Result<(), OverlayError> {
        if rgba.len() != self.expected_bytes() {
            return Err(OverlayError::new(
                "VideoTexture::update length",
                format!(
                    "expected {} bytes for {:?}, got {}",
                    self.expected_bytes(),
                    self.size,
                    rgba.len()
                ),
            ));
        }
        let t0 = Instant::now();
        let image = color_image_from_gl_rgba(self.size, rgba)?;
        self.tex.set(image, VIDEO_TEXTURE_OPTIONS);
        self.upload_us.push(Self::elapsed_us(t0));
        self.uploads += 1;
        self.last_summary = Some(CapturedFrame::summarize(rgba));
        Ok(())
    }

    fn expected_bytes(&self) -> usize {
        self.size[0].saturating_mul(self.size[1]).saturating_mul(4)
    }

    fn elapsed_us(t0: Instant) -> u64 {
        Duration::as_micros(&t0.elapsed()) as u64
    }

    /// The egui texture to paint.
    #[must_use]
    pub fn texture(&self) -> &TextureHandle {
        &self.tex
    }

    /// Texture id, for the native-texture path / assertions.
    #[must_use]
    pub fn texture_id(&self) -> egui::TextureId {
        self.tex.id()
    }
    /// Number of frame uploads (including the initial `load`).
    #[must_use]
    pub fn uploads(&self) -> u64 {
        self.uploads
    }

    /// Mean upload duration in microseconds.
    #[must_use]
    pub fn mean_upload_us(&self) -> u64 {
        if self.upload_us.is_empty() {
            return 0;
        }
        self.upload_us.iter().sum::<u64>() / self.upload_us.len() as u64
    }

    /// Worst upload duration in microseconds.
    #[must_use]
    pub fn max_upload_us(&self) -> u64 {
        self.upload_us.iter().copied().max().unwrap_or(0)
    }

    /// Summary of the last uploaded frame (hash / non-black count), if any.
    #[must_use]
    pub fn last_summary(&self) -> Option<&CapturedFrame> {
        self.last_summary.as_ref()
    }

    /// Paint the video frame into `rect` of a `Ui`.
    ///
    /// Uses `Painter::image` (`egui-0.36.1/src/painter.rs:447`) with the standard
    /// uv rectangle; [`flip_rows`] already put the pixels in egui's order.
    pub fn paint_at(&self, ui: &Ui, rect: Rect) {
        ui.painter().image(
            self.tex.id(),
            rect,
            Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    }

    /// Machine-readable line, same style as the render/GL reports so a harness
    /// log and any later evidence agree.
    #[must_use]
    pub fn report_line(&self) -> String {
        let last = self
            .last_summary
            .as_ref()
            .map_or_else(|| "none".to_string(), CapturedFrame::report_line);
        format!(
            "overlay uploads={} mean_upload_ms={:.3} max_upload_ms={:.3} texture_id={:?} size={}x{} last={}",
            self.uploads,
            self.mean_upload_us() as f64 / 1e3,
            self.max_upload_us() as f64 / 1e3,
            self.tex.id(),
            self.size[0],
            self.size[1],
            last
        )
    }
}

/// A control event the overlay produced, ready to become a `plex_player` request.
///
/// The overlay does not talk to libmpv: it returns these and the shell turns them
/// into `plex_player::controls` requests (the player's vocabulary), keeping UI and
/// player on different threads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OverlayAction {
    /// Pause if playing, play if paused.
    TogglePlay,
    /// Stop/quit the current item.
    Stop,
    /// Relative seek in seconds (the +/-10 buttons).
    SeekRelative(f64),
    /// Ask the shell to cycle the subtitle track (including "none").
    CycleSubtitle,
}

/// What the overlay shows about playback.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TransportState {
    /// Whether the player is currently playing (drives the button label).
    pub playing: bool,
    /// Current position in seconds.
    pub position_secs: f64,
    /// Duration in seconds (0 when unknown).
    pub duration_secs: f64,
    /// Item title; never a token (`lib.rs` shell contract).
    pub title: String,
    /// Selected subtitle track label, or "none".
    pub subtitle: String,
    /// Error/unsupported-codec surface text, if any.
    pub error: Option<String>,
}

/// Draw the transport/OSD overlay and return the actions the user took.
///
/// Subtitles are *not* drawn here (mpv owns them inside the app FBO); this is
/// transport controls, progress, the subtitle label and the error surface.
pub fn draw_transport(ui: &mut Ui, state: &TransportState) -> Vec<OverlayAction> {
    let mut actions = Vec::new();
    ui.horizontal(|ui| {
        let play_label = if state.playing { "pause" } else { "play" };
        let play = ui.add(
            egui::Button::new(egui::RichText::new(play_label)).min_size(Vec2::new(64.0, 24.0)),
        );
        if clicked(&play) {
            actions.push(OverlayAction::TogglePlay);
        }
        let stop =
            ui.add(egui::Button::new(egui::RichText::new("stop")).min_size(Vec2::new(64.0, 24.0)));
        if clicked(&stop) {
            actions.push(OverlayAction::Stop);
        }
        let back = ui.add(egui::Button::new(egui::RichText::new("-10s")));
        if clicked(&back) {
            actions.push(OverlayAction::SeekRelative(-10.0));
        }
        let fwd = ui.add(egui::Button::new(egui::RichText::new("+10s")));
        if clicked(&fwd) {
            actions.push(OverlayAction::SeekRelative(10.0));
        }
        let sub_label = format!(
            "subtitles: {}",
            if state.subtitle.is_empty() {
                "none"
            } else {
                &state.subtitle
            }
        );
        let sub = ui.button(sub_label);
        if clicked(&sub) {
            actions.push(OverlayAction::CycleSubtitle);
        }
    });
    ui.horizontal(|ui| {
        let title = if state.title.is_empty() {
            "no item".to_string()
        } else {
            state.title.clone()
        };
        ui.label(title);
        ui.label(format!(
            "{:.1}s / {:.1}s",
            state.position_secs, state.duration_secs
        ));
    });
    if state.duration_secs > 0.0 {
        // Progress is a read-only bar: seeking goes through SeekRelative so the
        // player stays the only one that changes position.
        let t = (state.position_secs / state.duration_secs).clamp(0.0, 1.0) as f32;
        let (bar_area, _resp) = ui.allocate_exact_size(Vec2::new(220.0, 8.0), Sense::HOVER);
        let bar = Rect::from_min_size(bar_area.min, Vec2::new(220.0 * t, 8.0));
        ui.painter().rect_filled(bar, 2.0, Color32::from_gray(160));
    }
    if let Some(err) = &state.error {
        ui.colored_label(Color32::LIGHT_RED, err);
    }
    actions
}

fn clicked(resp: &Response) -> bool {
    resp.clicked()
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

    fn rgba(w: usize, h: usize, fill: impl Fn(usize, usize) -> [u8; 4]) -> Vec<u8> {
        let mut v = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            for x in 0..w {
                v.extend_from_slice(&fill(x, y));
            }
        }
        v
    }

    /// One egui frame that paints the video texture (`egui 0.36.1
    /// `Context::run_ui`, `egui-0.36.1/src/context.rs:793`).
    fn paint_video(ctx: &Context, vt: &VideoTexture) -> egui::FullOutput {
        ctx.run_ui(egui::RawInput::default(), |ui| {
            vt.paint_at(
                ui,
                Rect::from_min_size(egui::pos2(0.0, 0.0), Vec2::new(4.0, 2.0)),
            );
        })
    }

    fn painted_meshes(out: &egui::FullOutput, id: egui::TextureId) -> usize {
        out.shapes
            .iter()
            .filter(|cs| matches!(&cs.shape, egui::Shape::Mesh(m) if m.texture_id == id))
            .count()
    }

    /// What one frame asked the backend to upload.
    #[derive(Debug)]
    struct Uploads {
        /// `ImageDelta`s queued for our video texture.
        video: usize,
        /// How many of those cover the whole texture (`ImageDelta::pos == None`).
        whole: usize,
        /// Size of the first video delta, when there is one.
        first_size: Option<[usize; 2]>,
        /// `ImageDelta`s queued for other texture ids (egui's own font atlas).
        other: usize,
    }

    /// Inspect and then drain one frame's `TexturesDelta`.
    ///
    /// This test has no render backend, so the deltas cannot be applied. epaint
    /// 0.36.1 `debug_assert`s at drop time that unapplied deltas were handled
    /// (`Dropped TexturesDelta with N unapplied deltas`,
    /// `epaint-0.36.1/src/textures.rs:335-341`), which aborts the test process,
    /// so they are cleared deliberately after being read.
    fn take_uploads(out: &mut egui::FullOutput, id: egui::TextureId) -> Uploads {
        let mut u = Uploads {
            video: 0,
            whole: 0,
            first_size: None,
            other: 0,
        };
        for (tex_id, deltas) in &out.textures_delta.set {
            for delta in deltas {
                if *tex_id == id {
                    u.video += 1;
                    if delta.pos.is_none() {
                        u.whole += 1;
                    }
                    if u.first_size.is_none() {
                        u.first_size = Some(delta.image.size());
                    }
                } else {
                    u.other += 1;
                }
            }
        }
        out.textures_delta.clear();
        u
    }

    /// Drain every delta of a frame that has no textures of its own; returns the
    /// number read (same epaint drop-assertion reason as [`take_uploads`]).
    fn drain_uploads(out: &mut egui::FullOutput) -> usize {
        let n = out.textures_delta.set.len() + out.textures_delta.free.len();
        out.textures_delta.clear();
        n
    }

    #[test]
    fn flip_rows_reverses_scanline_order() {
        // Bottom-up GL readback: row 0 is the bottom of the picture.
        let bottom_up = rgba(2, 2, |x, y| [x as u8, y as u8, 0, 255]);
        let top_down = flip_rows(2, 2, &bottom_up);
        // After the flip the first row must be the GL top row (y == 1).
        assert_eq!(&top_down[0..8], &[0, 1, 0, 255, 1, 1, 0, 255]);
        assert_eq!(top_down.len(), bottom_up.len());
    }

    #[test]
    fn flip_rows_leaves_a_truncated_capture_black_instead_of_wrong() {
        let short = [1u8; 4]; // only one scanline for a 1x2 image
        let out = flip_rows(1, 2, &short);
        assert_eq!(out, vec![0u8, 0, 0, 0, 1, 1, 1, 1]);
    }

    #[test]
    fn color_image_rejects_a_capture_of_the_wrong_size() {
        let e = color_image_from_gl_rgba([2, 2], &[0u8; 4]).expect_err("must reject");
        assert_eq!(e.step, "color_image_from_gl_rgba length");
        assert!(e.detail.contains("expected 16 bytes"), "{}", e.detail);
    }

    #[test]
    fn upload_path_reaches_egui_textures_and_shapes() {
        // Real egui context (no window, no GL): the upload must show up as an
        // egui texture delta and as a mesh shape referencing that texture.
        let ctx = Context::default();
        let frame = rgba(4, 2, |x, y| [(x * 40) as u8, (y * 60) as u8, 20, 255]);
        let mut vt = VideoTexture::load(&ctx, [4, 2], &frame).expect("load must succeed");
        assert_eq!(vt.texture().size(), [4, 2]);
        assert_eq!(vt.texture().byte_size(), 32);

        let next = rgba(4, 2, |x, y| [(x * 40 + 7) as u8, (y * 60) as u8, 20, 255]);
        vt.update(&next).expect("in-place update");
        assert_eq!(vt.uploads(), 2);

        // Measured in this session against the pinned egui/epaint 0.36.1
        // (`Cargo.lock`): one `Context::run_ui` frame emits at most ONE whole
        // `ImageDelta` per texture id - `TexturesDelta::push` discards previous
        // deltas for an id when the new one covers the whole texture
        // (`epaint-0.36.1/src/textures.rs:299-307`) - so `load` + `update` before
        // the first frame produce a single 4x2 delta, not two. A frame that does
        // not call `TextureHandle::set` produces no video delta at all.
        let mut frame1 = paint_video(&ctx, &vt);
        let up1 = take_uploads(&mut frame1, vt.texture_id());
        assert_eq!(up1.video, 1, "one upload delta expected: {up1:?}");
        assert_eq!(up1.whole, 1, "the upload must cover the texture: {up1:?}");
        assert_eq!(up1.first_size, Some([4, 2]));
        // egui's own font atlas rides along in the first frame (measured here:
        // one whole 2048x32 delta on `Managed(0)`); asserting it is present keeps
        // the video count above unambiguous.
        assert!(up1.other >= 1, "expected egui's own texture delta: {up1:?}");
        assert_eq!(
            painted_meshes(&frame1, vt.texture_id()),
            1,
            "exactly one video mesh expected"
        );

        vt.update(&next).expect("in-place update #2");
        assert_eq!(vt.uploads(), 3);
        let mut frame2 = paint_video(&ctx, &vt);
        let up2 = take_uploads(&mut frame2, vt.texture_id());
        assert_eq!(up2.video, 1, "an update must re-upload: {up2:?}");
        assert_eq!(up2.first_size, Some([4, 2]));

        // A frame with no new frame data re-uses the uploaded texture.
        let mut frame3 = paint_video(&ctx, &vt);
        let up3 = take_uploads(&mut frame3, vt.texture_id());
        assert_eq!(
            up3.video, 0,
            "a paint-only frame must not re-upload: {up3:?}"
        );
        assert_eq!(
            painted_meshes(&frame3, vt.texture_id()),
            1,
            "the texture is still painted without a re-upload"
        );

        let line = vt.report_line();
        println!("OVERLAY {line}");
        assert!(line.contains("uploads=3"), "{line}");
        assert!(line.contains("hash="), "{line}");
    }

    #[test]
    fn transport_overlay_builds_and_reports_no_action_without_clicks() {
        let ctx = Context::default();
        let state = TransportState {
            playing: true,
            position_secs: 12.5,
            duration_secs: 100.0,
            title: "Test Item".to_string(),
            subtitle: "eng".to_string(),
            error: Some("unsupported codec: dts".to_string()),
        };
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let actions = draw_transport(ui, &state);
            // Nothing was clicked in a fresh frame with no pointer events.
            assert!(actions.is_empty(), "unexpected actions {actions:?}");
        });
        // No backend here either: drain the frame's texture deltas (see
        // [`take_uploads`] on epaint 0.36.1's drop assertion).
        let _ = drain_uploads(&mut out);
        // The overlay produced real widgets (buttons/labels), not prose.
        assert!(out.shapes.len() > 5, "shapes={}", out.shapes.len());
    }
}
