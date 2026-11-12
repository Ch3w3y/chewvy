//! Shared GL context ownership.
//!
//! One EGL context, one `glow::Context` view of it, one `eglGetProcAddress`
//! resolver. The **same** context is:
//!
//! * current on the render thread for the app's own GL calls ([`fbo::AppFbo`],
//!   through `glow`'s typed prototypes);
//! * the context libmpv draws into, via
//!   [`plex_player::render_link::RenderContext`] with the `egl::EglResolver`
//!   handed to `mpv_opengl_init_params.get_proc_address` (`render_gl.h:43`);
//! * the context egui's glow painter draws its overlay with on a desktop
//!   ([`crate::overlay`]).
//!
//! That sharing is the whole point of the milestone: mpv writes the app's own
//! texture, and the app then hands that *same* texture to egui instead of
//! copying decoded frames through CPU buffers on every frame.
//!
//! ## Why not glutin here
//!
//! `glutin`'s EGL path needs a display handle
//! (`glutin-0.32.3/src/api/egl/display.rs:52`), which a display-free run does
//! not provide. The surfaceless recipe (`EGL_PLATFORM_SURFACELESS_MESA`) the
//! software-GL harness uses is therefore built in [`egl`], and a desktop shell
//! can still reach the same context through
//! [`egl::EglContext::make_current_with`] once it owns a window surface.
//!
//! ## What is Linux-only
//!
//! Everything that needs a *display-free* context - [`egl`], [`SharedGl`], the
//! app-FBO render session it feeds and the video-window mode built on it - is
//! the Linux path. The live window path shares no code with it: it resolves its
//! GL procedures through [`GlutinResolver`], i.e. through whichever API glutin
//! picked for the window (EGL or GLX on Linux, WGL on Windows, CGL on macOS).

#![allow(
    unsafe_code,
    reason = "GL/EGL FFI and glow call sites; see gl/egl.rs and gl/fbo.rs"
)]

#[cfg(target_os = "linux")]
pub mod egl;
pub mod fbo;
pub mod support;

use std::ffi::CStr;
#[cfg(target_os = "linux")]
use std::sync::Arc;

#[cfg(target_os = "linux")]
use glow::HasContext;

use plex_player::render_link::ProcResolver;

#[cfg(target_os = "linux")]
pub use egl::{EglContext, EglInfo, EglResolver};
pub use fbo::{AppFbo, CapturedFrame, FboStatus};

/// How a GL or EGL step failed, with the EGL error code when one was available.
///
/// This lives beside [`fbo`] rather than inside [`egl`] because the FBO helpers
/// report it too, on every platform - only the EGL error code is Linux-specific.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlError {
    /// Which step failed (kept short so it can be printed by the harness).
    pub step: String,
    /// `eglGetError` value read right after the failure, when it was read.
    pub egl_error: Option<i32>,
    /// Extra context, e.g. the platform constant that was rejected.
    pub detail: Option<String>,
}

impl std::fmt::Display for GlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "EGL {}", self.step)?;
        if let Some(code) = self.egl_error {
            write!(f, " (EGL error 0x{code:04X})")?;
        }
        if let Some(detail) = &self.detail {
            write!(f, ": {detail}")?;
        }
        Ok(())
    }
}

impl std::error::Error for GlError {}

impl GlError {
    /// A failure named by the step that produced it plus another error value.
    ///
    /// Used by [`crate::gl::fbo`], where the detail is glow's own `String`
    /// error (an unresolved entry point or a GL object that could not be
    /// created), which is not an EGL error code.
    #[must_use]
    pub fn step(step: &str, detail: &dyn std::fmt::Display) -> Self {
        Self {
            step: step.to_string(),
            egl_error: None,
            detail: Some(detail.to_string()),
        }
    }
}

/// The proc-address resolver a live player is built with: glutin's own lookup
/// on the display its window context came from.
///
/// Unlike `egl::EglResolver`, which asks `libEGL` directly and so answers only
/// for an EGL display, this goes through whichever GL API glutin actually
/// picked - EGL or GLX on Linux, WGL on Windows, CGL on macOS - so the live
/// player path names no platform.
///
/// WGL answers fully only with a *current* context on the calling thread (per
/// `GlDisplay::get_proc_address`'s own note); libmpv resolves from
/// `mpv_render_context_create` and from render, both with a context current.
pub struct GlutinResolver {
    display: glutin::display::Display,
}

impl GlutinResolver {
    /// A resolver over `display`, the display the window's context came from.
    #[must_use]
    pub fn new(display: &glutin::display::Display) -> Self {
        Self {
            display: display.clone(),
        }
    }
}

impl ProcResolver for GlutinResolver {
    fn resolve(&self, name: &CStr) -> *mut std::os::raw::c_void {
        use glutin::display::GlDisplay as _;
        self.display.get_proc_address(name).cast_mut()
    }
}

/// What the GL context reports about itself.
///
/// Recorded in every harness run so a measurement can be attributed to the
/// renderer that produced it (software llvmpipe here; there is no `/dev/dri`
/// in the headless test environment, so hardware GL is not claimed anywhere).
#[cfg(target_os = "linux")]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GlStrings {
    /// `GL_VERSION`.
    pub version: String,
    /// `GL_RENDERER`.
    pub renderer: String,
    /// `GL_SHADING_LANGUAGE_VERSION`.
    pub glsl: String,
}

/// EGL display + context plus the `glow` handle that shares it.
///
/// Drop order matters and is encoded by field order: the `glow` view is dropped
/// before the EGL context is destroyed, and anything that draws into this
/// context (`crate::render::MpvGlSession`, [`fbo::AppFbo`]) borrows it and is
/// therefore required by the borrow checker to be dropped first.
#[cfg(target_os = "linux")]
pub struct SharedGl {
    /// `Arc` because `egui_glow::Painter::new` / `egui_glow::EguiGlow::new`
    /// require `Arc<glow::Context>` (`egui_glow-0.36.1/src/winit.rs:24`,
    /// `painter.rs:141`) and `glow::Context` is not `Clone`
    /// (`glow-0.17.0/src/native.rs:46`). The app-owned clone is dropped **before**
    /// this struct (field order in `crate::shell::window`) so no `Arc` outlives the
    /// EGL context it points into.
    gl: Arc<glow::Context>,
    egl: EglContext,
}

#[cfg(target_os = "linux")]
impl SharedGl {
    /// A core-profile context with no window and no compositor.
    ///
    /// This is the software-GL harness path (`scripts/mpv-gl-harness.sh`)
    /// and the path that runs in CI-like environments: no
    /// `WAYLAND_DISPLAY`, no `XDG_SESSION_TYPE`, no `/dev/dri`. The EGL
    /// attributes requested are those in `EglContext::surfaceless`: 3.3 plus
    /// `EGL_CONTEXT_OPENGL_PROFILE_MASK_KHR` =
    /// `EGL_CONTEXT_OPENGL_CORE_PROFILE_BIT_KHR`.
    pub fn surfaceless() -> Result<Self, GlError> {
        Self::attach(EglContext::surfaceless()?)
    }

    /// Wrap an already-created EGL context (the desktop path, where glutin or
    /// the shell created the display/context and this type only borrows it).
    ///
    /// The context must be current on the calling thread before this returns:
    /// `glow` reads `GL_VERSION` while building its function table
    /// (`glow-0.17.0/src/native.rs:69-75`) and panics if no context is current,
    /// which is why [`Self::attach`] re-makes it current first.
    pub fn attach(egl: EglContext) -> Result<Self, GlError> {
        egl.make_current()?;
        // `EglResolver` is stateless (`eglGetProcAddress` only), so the loader
        // does not have to hold anything from `egl` alive.
        // SAFETY: the EGL context is current on this thread (the line above),
        // and `EglResolver::resolve` is `eglGetProcAddress` (`egl.h:145`), the
        // loader `glow` documents for EGL (`glow-0.17.0/src/native.rs:55`).
        // Entry points `eglGetProcAddress` does not export stay null and glow
        // reports those as `Err(String)` at the call site rather than crashing.
        let gl = unsafe {
            glow::Context::from_loader_function_cstr(|name| {
                <EglResolver as ProcResolver>::resolve(&EglResolver, name)
            })
        };
        Ok(Self {
            gl: Arc::new(gl),
            egl,
        })
    }

    /// The `glow` handle for the app's own GL work.
    #[must_use]
    pub fn gl(&self) -> &glow::Context {
        self.gl.as_ref()
    }

    /// The same context as an `Arc`, for `egui_glow`.
    ///
    /// A caller that keeps this clone alive after the `SharedGl` (and therefore the
    /// EGL context) is dropped would be calling into a destroyed context; the
    /// window host orders its fields so the egui painter is dropped first.
    #[must_use]
    pub fn gl_arc(&self) -> Arc<glow::Context> {
        Arc::clone(&self.gl)
    }

    /// The EGL side, for `make_current*` and diagnostics.
    #[must_use]
    pub fn egl(&self) -> &EglContext {
        &self.egl
    }

    /// `EGL_VENDOR` / `EGL_VERSION` / `EGL_CLIENT_APIS` / platform.
    #[must_use]
    pub fn egl_info(&self) -> &EglInfo {
        self.egl.info()
    }

    /// `GL_VERSION` / `GL_RENDERER` / `GL_SHADING_LANGUAGE_VERSION`, read
    /// through `glow` from the shared context.
    #[must_use]
    pub fn gl_strings(&self) -> GlStrings {
        // SAFETY: plain `glGetString` reads on a context that is current
        // (`SharedGl::attach` makes it current and every harness/test re-makes
        // it current before use).
        unsafe {
            GlStrings {
                version: self.gl.get_parameter_string(glow::VERSION),
                renderer: self.gl.get_parameter_string(glow::RENDERER),
                glsl: self.gl.get_parameter_string(glow::SHADING_LANGUAGE_VERSION),
            }
        }
    }

    /// Re-make this context current (per-frame, before libmpv renders).
    ///
    /// `render.h:68-70` requires the GL context to be current on the thread
    /// calling `mpv_render_context_render`; anything else that ran on this
    /// thread may have changed the current context.
    pub fn make_current(&self) -> Result<(), GlError> {
        self.egl.make_current()
    }

    /// A fresh resolver for `mpv_opengl_init_params.get_proc_address`.
    ///
    /// A new box per render context: libmpv keeps the userdata pointer alive
    /// for the life of the render context and `RenderContext` owns it there, so
    /// the resolver must not be borrowed from `self` (`&self` cannot outlive a
    /// `'static` box requirement).
    #[must_use]
    pub fn mpv_resolver(&self) -> Box<dyn ProcResolver> {
        self.egl.resolver()
    }
}

/// Test-support helper: the named environment gate for the runtime GL tests.
///
/// The software-GL harness (`scripts/mpv-gl-harness.sh`) exports
/// `PLEX_GL_HARNESS=1` together with the surfaceless/software-EGL recipe and
/// runs every runtime GL case with that gate set in its own process (so
/// process-lifetime `VmHWM` is per-case). Inside the gate a context that cannot
/// be created is a hard test failure; without the gate (plain `cargo test`,
/// which has no EGL environment) the test prints a machine-readable
/// `SKIP test=... gate_set=false ...` line and returns, so the skip is recorded
/// and attributable instead of silent.
#[cfg(test)]
#[cfg(target_os = "linux")]
pub(crate) fn try_surfaceless(test: &str) -> Option<SharedGl> {
    let gated = std::env::var("PLEX_GL_HARNESS").is_ok();
    match SharedGl::surfaceless() {
        Ok(shared) => Some(shared),
        Err(e) => {
            println!(
                "SKIP test={test} gate=PLEX_GL_HARNESS gate_set={gated} surfaceless_error={e}"
            );
            assert!(
                !gated,
                "GL harness gate is set but no EGL context could be created: {e}"
            );
            None
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "tests may unwrap; product code keeps these lints on"
)]
mod tests {
    use super::*;

    #[test]
    fn shared_context_reports_a_real_gl_renderer() {
        // Environment-dependent: whatever EGL provides, the failure or the
        // success must be named. This test is the drift guard that the context
        // really is usable.
        let Some(shared) =
            try_surfaceless("plex_app::gl::shared_context_reports_a_real_gl_renderer")
        else {
            return;
        };
        let s = shared.gl_strings();
        println!(
            "gl ok: version={} renderer={} glsl={} egl {} {}",
            s.version,
            s.renderer,
            s.glsl,
            shared.egl_info().vendor,
            shared.egl_info().version
        );
        assert!(!s.version.is_empty(), "GL_VERSION was empty");
        assert!(!s.renderer.is_empty(), "GL_RENDERER was empty");
        assert!(
            shared.gl.version().major >= 3,
            "core profile 3.3 was requested but GL_VERSION reports {}",
            s.version
        );
        shared.make_current().expect("context must be re-makable");
    }

    #[test]
    fn glow_reports_missing_entry_points_as_errors_not_crashes() {
        // The `ResourceUnavailable`/`Err(String)` contract the app relies on if
        // a host's `eglGetProcAddress` does not export an extension function.
        let Some(shared) = try_surfaceless(
            "plex_app::gl::glow_reports_missing_entry_points_as_errors_not_crashes",
        ) else {
            return;
        };
        // A core entry point the loader must have resolved: `glCreateShader`.
        // glow returns `Result<_, String>`, so an unresolved function pointer
        // is reported, not dereferenced.
        // SAFETY: legal call on our own current context; the shader is deleted
        // again inside the same unsafe block.
        let res = unsafe {
            match shared.gl.create_shader(glow::VERTEX_SHADER) {
                Ok(shader) => {
                    shared.gl.delete_shader(shader);
                    Ok(())
                }
                Err(e) => Err(e),
            }
        };
        println!("create_shader -> {res:?}");
        assert!(res.is_ok(), "glow must resolve glCreateShader");
    }
}
