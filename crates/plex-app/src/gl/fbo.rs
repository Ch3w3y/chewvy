//! The app-owned OpenGL texture + FBO that libmpv draws into.
//!
//! Ownership matters: **the app** creates the texture and the framebuffer
//! object, attaches the texture, verifies completeness, and deletes them. mpv
//! only receives the FBO *id* through `MPV_RENDER_PARAM_OPENGL_FBO`
//! (`/usr/include/mpv/render.h:199-203` in mpv 2.5.0) and draws into it. That is
//! the arrangement `render_link.rs` documents for
//! [`plex_player::render_link::GlFbo`], and it is what lets the same texture be
//! handed to egui afterwards.
//!
//! ## The `INVALID_ENUM` / `0x0` failure this module fixes
//!
//! A C scratch probe which hand-cast `eglGetProcAddress` function pointers for its
//! own texture/FBO calls produced `glCheckFramebufferStatus -> 0x0` with
//! `glGetError 0x500` (GL_INVALID_VALUE), after which libmpv logged
//! "after creating texture: OpenGL error INVALID_ENUM". Here the app-side GL
//! calls go through [`glow`]'s real prototypes (`glow`'s own loader resolves
//! each entry point into a typed field), the internal format is passed as the
//! `i32` `tex_image_2d` expects, and framebuffer completeness is asserted
//! (`GL_FRAMEBUFFER_COMPLETE 0x8CD5`) before a single frame is measured.
//!
//! All `unsafe` in this module is `glow` call sites, each in an `unsafe` block
//! with a `SAFETY:` note (the workspace `unsafe_code = "deny"` lint is reconciled
//! by the module-level `#![allow(unsafe_code)]`, the same pattern
//! `crates/plex-player/src/render_link.rs:60` uses).

#![allow(unsafe_code)]

use glow::HasContext;

use plex_player::render_link::GlFbo;

use super::GlError;

/// `GL_FRAMEBUFFER_COMPLETE` (`/usr/include/GL/glext.h`, value 0x8CD5).
pub const GL_FRAMEBUFFER_COMPLETE: u32 = 0x8CD5;
/// `GL_RGBA8` (0x8058) - the internal format the app asks mpv to use.
pub const GL_RGBA8: u32 = 0x8058;

/// What one framebuffer verification found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FboStatus {
    /// `glCheckFramebufferStatus` (`glow::Context::check_framebuffer_status`).
    pub status: u32,
    /// `glGetError` drained after the check; 0 means clean.
    pub gl_error: u32,
    /// FBO width in pixels as reported by the check.
    pub width: i32,
    /// FBO height in pixels as reported by the check.
    pub height: i32,
}

impl FboStatus {
    /// `true` when `glCheckFramebufferStatus` reported
    /// `GL_FRAMEBUFFER_COMPLETE` and no `glGetError` was left pending.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.status == GL_FRAMEBUFFER_COMPLETE && self.gl_error == 0
    }
}

/// The app-owned color texture and the framebuffer it is attached to.
///
/// The ids are what `GlFbo` reports to libmpv. Dropping this deletes both GL
/// objects, so the libmpv render context that draws into them must be dropped
/// first ([`crate::render::MpvGlSession`] borrows [`crate::gl::SharedGl`] to
/// enforce exactly that ordering).
///
/// `glow::Context` is not `Clone` (`glow-0.17.0/src/native.rs:46`), so this type
/// stores only the GL **object ids** and takes `&glow::Context` on every method.
/// Dropping the EGL context releases every GL object it created, which is the
/// teardown order `crate::gl::SharedGl` + `crate::render::MpvGlSession` enforce;
/// [`AppFbo::destroy`] is there for a desktop shell that wants to free the video
/// surface while keeping the window/context alive.
pub struct AppFbo {
    fbo: glow::Framebuffer,
    texture: glow::Texture,
    width: i32,
    height: i32,
}

impl AppFbo {
    /// Create the texture + FBO on the current context.
    ///
    /// The context must already be current (see [`crate::gl::SharedGl`]).
    pub fn create(gl: &glow::Context, width: i32, height: i32) -> Result<Self, GlError> {
        if width <= 0 || height <= 0 {
            return Err(GlError {
                step: "AppFbo::create".to_string(),
                egl_error: None,
                detail: Some(format!("bad size {width}x{height}")),
            });
        }
        let texture =
            unsafe { gl.create_texture() }.map_err(|e| GlError::step("glCreateTexture", &e))?;
        let fbo = unsafe { gl.create_framebuffer() }
            .map_err(|e| GlError::step("glCreateFramebuffer", &e))?;

        // SAFETY: both ids were just returned by GL on the current context, and
        // a texture object is a legal `TEXTURE_2D` binding.
        unsafe {
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            // glow's typed prototype, real RGBA8
            // internal format, no data (mpv fills it), and NEAREST filtering so
            // the captured pixels are the texture's own, not a resample.
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                GL_RGBA8 as i32,
                width,
                height,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MIN_FILTER,
                glow::NEAREST as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MAG_FILTER,
                glow::NEAREST as i32,
            );
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(texture),
                0,
            );
            gl.viewport(0, 0, width, height);
        }

        let me = Self {
            fbo,
            texture,
            width,
            height,
        };
        let status = me.status(gl);
        // Unbind so an incomplete FBO cannot be handed to libmpv by accident.
        // SAFETY: plain GL state restore.
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        }
        if !status.is_complete() {
            return Err(GlError {
                step: "FBO completeness".to_string(),
                egl_error: None,
                detail: Some(format!(
                    "glCheckFramebufferStatus 0x{:04X} glGetError 0x{:04X} (expected 0x{:04X})",
                    status.status, status.gl_error, GL_FRAMEBUFFER_COMPLETE
                )),
            });
        }
        Ok(me)
    }

    /// The value to put in `RenderOptions::fbo` for every render call.
    #[must_use]
    pub fn target(&self) -> GlFbo {
        GlFbo {
            fbo: self.fbo.0.get() as i32,
            width: self.width,
            height: self.height,
            // RGBA8 is what we allocated, so mpv's default must match it
            // (`render_gl.h:141-144`: `internal_format` - "GL_SRGBA8"-style
            // internal format, "or 0 if unknown"; 0 makes mpv pick RGBA8, so
            // passing it explicitly just states the same contract).
            internal_format: GL_RGBA8 as i32,
        }
    }

    /// `(width, height)` of the app-owned FBO, in pixels.
    #[must_use]
    pub fn size(&self) -> (i32, i32) {
        (self.width, self.height)
    }

    /// `glCheckFramebufferStatus` + `glGetError` on this FBO.
    ///
    /// Called both right after creation and after each libmpv render: the
    /// a probe showed a broken app-side FBO is not reported by libmpv
    /// as a render error, it silently produces 0 frames.
    #[must_use]
    pub fn status(&self, gl: &glow::Context) -> FboStatus {
        // SAFETY: `self.fbo` is a live framebuffer object we own; binding it
        // for the check and then restoring the previous binding is the
        // documented way to query completeness. `gl` is a view of the same
        // context the ids were created on.
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.fbo));
            let status = gl.check_framebuffer_status(glow::FRAMEBUFFER);
            // Drain any pending error so `gl_error == 0` really means clean.
            let gl_error = gl.get_error();
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            FboStatus {
                status,
                gl_error,
                width: self.width,
                height: self.height,
            }
        }
    }

    /// Read the whole color attachment back as RGBA bytes (bottom-up, GL order).
    ///
    /// This is the *capture* used for the frame hashes and for the egui
    /// texture upload: it is the app's own readback of the app's own texture,
    /// not something mpv offered us.
    pub fn read_rgba(&self, gl: &glow::Context) -> Result<Vec<u8>, GlError> {
        let len = (self.width as usize)
            .saturating_mul(self.height as usize)
            .saturating_mul(4);
        let mut pixels = vec![0u8; len];
        // SAFETY: `self.fbo` is live and sized `width x height`; the RGBA8
        // allocation guarantees 4 bytes per pixel, which `pixels` provides.
        let err = unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.fbo));
            gl.read_pixels(
                0,
                0,
                self.width,
                self.height,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(Some(&mut pixels)),
            );
            let read_error = gl.get_error();
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            read_error
        };
        if err != 0 {
            return Err(GlError {
                step: "glReadPixels".to_string(),
                egl_error: None,
                detail: Some(format!("glGetError 0x{err:04X}")),
            });
        }
        Ok(pixels)
    }

    /// The `glow` texture handle, for the desktop path where egui draws it
    /// directly (`egui_glow::Painter::register_native_texture`, `painter.rs:649`).
    #[must_use]
    pub fn texture(&self) -> glow::Texture {
        self.texture
    }

    /// The `glow` framebuffer handle of this FBO.
    ///
    /// [`Self::target`] is what libmpv wants (a plain `i32` in
    /// `RenderOptions::fbo`); a shell that binds the FBO itself through `glow`
    /// (the composite target does) needs the typed handle, and `glow` does
    /// not let a caller build one from a raw id. The handle stays owned here: the
    /// shell only ever passes it to `glow` calls on the same context.
    #[must_use]
    pub fn fbo_handle(&self) -> glow::Framebuffer {
        self.fbo
    }
}

impl AppFbo {
    /// Free the FBO + texture without tearing down the context.
    ///
    /// A desktop shell that resizes the video surface calls this before making
    /// a new [`AppFbo`]. The ids are ours and live, so the deletes are ours to
    /// make; after this call the object must not be used again.
    pub fn destroy(&self, gl: &glow::Context) {
        // SAFETY: both objects are ours and were never deleted elsewhere.
        unsafe {
            gl.delete_framebuffer(self.fbo);
            gl.delete_texture(self.texture);
        }
    }
}

/// A frame captured from the app FBO: the bytes plus a cheap identity hash.
///
/// The hash is FNV-1a 64-bit over the RGBA bytes, plus the count of pixels that
/// are not fully black and the sum of their red channel. That is enough to show
/// different fixtures produce different frames and that a frame is not empty,
/// without shipping pixel data into Git.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapturedFrame {
    /// FNV-1a 64-bit over every RGBA byte.
    pub hash: u64,
    /// Pixels with any non-zero channel.
    pub non_black: u64,
    /// Total pixels (width*height).
    pub total: u64,
    /// Mean red channel over the non-black pixels (`non_black == 0` => 0).
    pub mean_red: u64,
    /// Bytes hashed.
    pub bytes: usize,
}

impl CapturedFrame {
    /// Hash and count a RGBA8 readback buffer (4 bytes per pixel).
    #[must_use]
    pub fn summarize(pixels: &[u8]) -> Self {
        const FNV_OFFSET: u64 = 0x6495_c6a2_4199_3c93;
        const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
        let mut hash = FNV_OFFSET;
        for b in pixels {
            hash ^= u64::from(*b);
            hash = hash.wrapping_mul(FNV_PRIME);
        }
        let mut non_black = 0u64;
        let mut red_sum = 0u64;
        for px in pixels.as_chunks::<4>().0 {
            let (r, g, b) = (px[0], px[1], px[2]);
            if r != 0 || g != 0 || b != 0 {
                non_black += 1;
                red_sum += u64::from(r);
            }
        }
        Self {
            hash,
            non_black,
            total: (pixels.len() / 4) as u64,
            mean_red: red_sum.checked_div(non_black).unwrap_or(0),
            bytes: pixels.len(),
        }
    }

    /// Machine-readable single line (used by the harness and the tests).
    pub fn report_line(&self) -> String {
        format!(
            "hash={:016x} non_black={}/{} mean_red={} bytes={}",
            self.hash, self.non_black, self.total, self.mean_red, self.bytes
        )
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

    #[test]
    fn frame_summary_distinguishes_content_and_flags_black_frames() {
        let black = vec![0u8; 4 * 4];
        let mut content = vec![0u8; 4 * 4];
        content[0] = 12;
        let black_frame = CapturedFrame::summarize(&black);
        let content_frame = CapturedFrame::summarize(&content);
        assert_eq!(black_frame.non_black, 0);
        assert_eq!(content_frame.non_black, 1);
        assert_eq!(content_frame.mean_red, 12);
        assert_eq!(black_frame.total, 4);
        assert_ne!(black_frame.hash, content_frame.hash);
        assert!(content_frame.report_line().starts_with("hash="));
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn fbo_status_reports_completeness_of_the_real_fbo() {
        // Environment-dependent: the same assertion the software-GL harness
        // runs, but reachable from `cargo test` so it cannot rot silently.
        let Some(shared) = crate::gl::try_surfaceless(
            "plex_app::gl::fbo::fbo_status_reports_completeness_of_the_real_fbo",
        ) else {
            return;
        };
        let fbo =
            crate::gl::fbo::AppFbo::create(shared.gl(), 64, 48).expect("app FBO must be complete");
        let status = fbo.status(shared.gl());
        assert!(
            status.is_complete(),
            "FBO incomplete: 0x{:04X} (gl error 0x{:04X})",
            status.status,
            status.gl_error
        );
        assert_eq!(status.width, 64);
        assert_eq!(status.height, 48);
        let target = fbo.target();
        assert_eq!(target.width, 64);
        assert_eq!(target.height, 48);
        assert_ne!(target.fbo, 0, "FBO id must be a real GL object");
        let cap = CapturedFrame::summarize(&fbo.read_rgba(shared.gl()).expect("readback"));
        assert_eq!(cap.total, 64 * 48);
        // Explicit teardown must leave the context usable (window-resize path).
        fbo.destroy(shared.gl());
        let after = shared.gl();
        let err = unsafe_after(after);
        assert_eq!(err, 0, "destroying the FBO must not leave a GL error");
    }

    #[cfg(target_os = "linux")]
    fn unsafe_after(gl: &glow::Context) -> u32 {
        // SAFETY: plain glGetError read on the current context.
        #![allow(unsafe_code)]
        unsafe { glow::HasContext::get_error(gl) }
    }
}
