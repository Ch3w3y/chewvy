//! The mpv **render-context** link.
//!
//! [`crate::mpv_link`] owns the `mpv_handle` and the control/event loop. This
//! module owns the *video output* side of the same core: a
//! `mpv_render_context` that draws the current frame into an **app-owned**
//! OpenGL FBO (the app owns the GL context, the FBO and the texture; mpv only
//! draws into them). That is what lets egui keep drawing its own overlay on top
//! of the same window.
//!
//! Facts taken from the installed headers (`/usr/include/mpv/render.h`,
//! `/usr/include/mpv/render_gl.h`, `/usr/include/mpv/client.h`, mpv 2.5.0 - the
//! same header set `build.rs` found via pkg-config):
//!
//! * `render.h:68-70` - with the OpenGL backend the GL context must be **current
//!   in the calling thread** for every `mpv_render_*` call, and it must be the
//!   same context the render context was created with. This module does not own
//!   a GL context: the caller must guarantee it (see [`RenderContext::create`]).
//! * `render.h:258-265` + `render.h:634-673` - the update callback only *wakes*
//!   us; it carries no flags. `mpv_render_context_update()` is what returns the
//!   flag bitset, and it must run on the render thread and **not** from inside
//!   the callback. That pair is the "redraw when told, never busy-poll" contract
//!   this crate enforces ([`RenderContext::take_update`]).
//! * `render.h:300-317` - `mpv_render_context_render()` blocks until the frame's
//!   target display time unless `MPV_RENDER_PARAM_BLOCK_FOR_TARGET_TIME = 0`;
//!   measurements must say which mode they used ([`RenderOptions::block_for_target_time`]).
//! * `render.h:156` - `MPV_RENDER_PARAM_FLIP_Y` is documented as *ignored* in
//!   this build, so texture orientation has to be proven by pixel readback, not
//!   assumed.
//! * `render.h:235-247` - the X11/Wayland display parameters are creation-time
//!   only ("Valid for `mpv_render_context_create()`") and their
//!   `Display*`/`wl_display*` "must stay valid for the lifetime of the
//!   `mpv_render_context`" - it is owned by the app's window, which outlives
//!   every context created from it.
//! * `render.h:125-141` - `MPV_RENDER_API_TYPE_SW` is the documented software
//!   last-resort backend (`MPV_RENDER_PARAM_SW_*`, "extremely simple (but
//!   slow)"), kept reachable here rather than invented as a custom fallback.
//!
//! # Verified on this machine, and what is *not* verified
//!
//! Every header citation below was checked against libmpv 2.5.0
//! (`pkg-config --modversion mpv`): `render.h` 760 lines, `render_gl.h` 211
//! lines, `client.h` 2032 lines. The symbols used here all come from the
//! bindgen-generated [`crate::ffi`] bindings, not from hand-written declarations.
//!
//! What this crate's own tests exercise:
//!
//! * the pure logic in this file (unit tests below); and
//! * a real `mpv_render_context` against the installed libmpv in
//!   `crates/plex-player/tests/render_sw_session.rs`, which creates a core, a
//!   generated video fixture and a `MPV_RENDER_API_TYPE_SW` context, then calls
//!   `update`/`get_info`/`render`/`report_swap`/`free` for real and hashes the
//!   pixels libmpv wrote. That is a display-free runtime test, not a GL one.
//!
//! The real EGL/GL context and the window it draws into belong to
//! `crates/plex-app`; nothing in this crate needs a display, and no hardware
//! decode or GL number is claimed here.
//!
//! All `unsafe` in this module is FFI glue over the generated [`crate::ffi`]
//! bindings and carries a `SAFETY:` note, like [`crate::mpv_link`].

#![allow(unsafe_code)]

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};
use std::ptr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use raw_window_handle::RawDisplayHandle;

use crate::events::PlayerError;
use crate::ffi as mpv;
use crate::link::PlayerLink;
use crate::mpv_link::{CorePin, FfiPlayer};

/// `MPV_RENDER_UPDATE_FRAME` (`render.h:672`: `1 << 0`).
///
/// Taken from the bindgen-emitted `mpv_render_update_flag` enum value, not
/// hand-written, so it cannot drift from the installed header (the unit test
/// `update_frame_flag_comes_from_header` pins it).
pub const MPV_RENDER_UPDATE_FRAME: u64 = mpv::mpv_render_update_flag_MPV_RENDER_UPDATE_FRAME as u64;

/// Which mpv render backend this context uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderApi {
    /// `MPV_RENDER_API_TYPE_OPENGL` (`render_gl.h`): draw into an app-owned FBO.
    OpenGL,
    /// `MPV_RENDER_API_TYPE_SW` (`render.h:125-141`): CPU blit to a caller-owned
    /// RGBA buffer. Documented as "extremely simple (but slow)" - the last resort
    /// when no GL context can be created at all.
    Software,
}

impl RenderApi {
    /// The `MPV_RENDER_PARAM_API_TYPE` string this build passes to libmpv.
    #[must_use]
    pub fn api_type(self) -> &'static [u8] {
        match self {
            Self::OpenGL => mpv::MPV_RENDER_API_TYPE_OPENGL,
            Self::Software => mpv::MPV_RENDER_API_TYPE_SW,
        }
    }
}

/// Looks up an OpenGL function pointer by name for libmpv.
///
/// The product implementation is the GL context the app already has
/// (`glutin`'s `Context::get_proc_address`, i.e. `eglGetProcAddress`/
/// `glXGetProcAddress`), which is what `crates/plex-app` will pass. There is no
/// GL harness in this repository yet: the SW backend needs no resolver at all,
/// which is why `crates/plex-player/tests/render_sw_session.rs` can exercise
/// libmpv rendering without a display. Returning `null` for a name is allowed
/// and is how a resolver is expected to say "not here" (`render_gl.h:43`); it is
/// not guaranteed - measured 2026-11-05, the test environment's libglvnd EGL hands out a
/// non-null stub for unimplemented names (see `crates/plex-app/src/gl/egl.rs`
/// `EglResolver`), so a resolver must not be relied on to reject names.
pub trait ProcResolver: Send + Sync {
    /// Resolve one GL function pointer. `name` is a NUL-terminated C string.
    fn resolve(&self, name: &CStr) -> *mut c_void;
}

/// The app-owned render target for the OpenGL backend.
///
/// Mirrors `mpv_opengl_fbo` (`render_gl.h`): the FBO id, its size in pixels and
/// the internal format mpv is allowed to use (`0` = let mpv pick RGBA8).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GlFbo {
    /// `GL_FRAMEBUFFER` object id owned by the **app**, not by mpv.
    pub fbo: i32,
    /// FBO width in pixels.
    pub width: i32,
    /// FBO height in pixels.
    pub height: i32,
    /// `internal_format` for the color attachment (0 = mpv default).
    pub internal_format: i32,
}

// `mpv_render_context` (opaque, `client.h`) and every `mpv_render_context_*`
// entry point (`create`, `set_update_callback`, `update`, `render`,
// `report_swap`, `get_info`, `free`) come from the bindgen output in
// `crate::ffi` - nothing here is hand-declared. `mpv_render_context_get_info`
// takes **one** `mpv_render_param` in this build (`mpv_bindings.rs:473-476`),
// not an array, and the frame-info param id in this build is
// `MPV_RENDER_PARAM_NEXT_FRAME_INFO` (11); there is no
// `MPV_RENDER_PARAM_FRAME_INFO` constant in the installed headers.

/// What one `mpv_render_context_render()` call should do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderOptions {
    /// Target surface (OpenGL backend). Ignored by the software backend.
    pub fbo: Option<GlFbo>,
    /// `MPV_RENDER_PARAM_BLOCK_FOR_TARGET_TIME`: 1 = honour mpv's video timing
    /// (blocks until the frame's target display time), 0 = return immediately.
    /// Pacing numbers must name this setting.
    pub block_for_target_time: bool,
    /// `MPV_RENDER_PARAM_FLIP_Y` - sent verbatim; `render.h:156` says this build
    /// ignores it, which is recorded rather than papered over.
    pub flip_y: bool,
    /// `MPV_RENDER_PARAM_SKIP_RENDERING`: consume the frame without drawing
    /// (used for a window that is hidden/minimised).
    pub skip_rendering: bool,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            fbo: None,
            block_for_target_time: true,
            flip_y: false,
            skip_rendering: false,
        }
    }
}

/// Decoded result of one `mpv_render_context_update()` call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderUpdate {
    /// The raw `u64` bitset `mpv_render_context_update()` returned.
    pub flags: u64,
}

impl RenderUpdate {
    /// `MPV_RENDER_UPDATE_FRAME` set: `mpv_render_context_render()` must be
    /// called. This is the *only* redraw trigger the app is allowed to have.
    #[must_use]
    pub fn has_frame(&self) -> bool {
        self.flags & MPV_RENDER_UPDATE_FRAME != 0
    }

    /// Flags this module does not define (reported, not silently dropped).
    #[must_use]
    pub fn unknown_flags(&self) -> u64 {
        self.flags & !MPV_RENDER_UPDATE_FRAME
    }
}

/// `mpv_render_frame_info.flags` bits (`render.h`, mpv 2.5.0).
///
/// These are the bindgen-emitted `mpv_render_update`-style const enum values
/// (`mpv_render_frame_info_flag_MPV_RENDER_FRAME_INFO_*` in `crate::ffi`), so
/// they cannot drift from the installed header.
pub mod frame_info_flag {
    use crate::ffi as mpv;

    /// `MPV_RENDER_FRAME_INFO_PRESENT`.
    pub const PRESENT: u64 = mpv::mpv_render_frame_info_flag_MPV_RENDER_FRAME_INFO_PRESENT as u64;
    /// `MPV_RENDER_FRAME_INFO_REDRAW` - redraw request, not a new frame.
    pub const REDRAW: u64 = mpv::mpv_render_frame_info_flag_MPV_RENDER_FRAME_INFO_REDRAW as u64;
    /// `MPV_RENDER_FRAME_INFO_REPEAT` - same content as the previous frame.
    pub const REPEAT: u64 = mpv::mpv_render_frame_info_flag_MPV_RENDER_FRAME_INFO_REPEAT as u64;
    /// `MPV_RENDER_FRAME_INFO_BLOCK_VSYNC` - the app should block on vsync.
    pub const BLOCK_VSYNC: u64 =
        mpv::mpv_render_frame_info_flag_MPV_RENDER_FRAME_INFO_BLOCK_VSYNC as u64;
}

/// Decoded `mpv_render_frame_info` (`render.h:539-553`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameInfo {
    /// `MPV_RENDER_FRAME_INFO_PRESENT` (`render.h:491`).
    pub present: bool,
    /// `MPV_RENDER_FRAME_INFO_REDRAW`: a redraw request, not a new frame.
    pub redraw: bool,
    /// `MPV_RENDER_FRAME_INFO_REPEAT`: the frame reproduces the previous one.
    pub repeat: bool,
    /// `MPV_RENDER_FRAME_INFO_BLOCK_VSYNC`: the app is expected to block on vsync.
    pub block_vsync: bool,
    /// Raw flag bitset, retained so nothing is hidden.
    pub flags: u64,
    /// Target display time in the `mpv_get_time_us()` clock. libmpv writes 0
    /// when there is no valid target time (`render.h:550-552`); see
    /// [`FrameInfo::has_target_time`].
    pub target_time_us: i64,
}

impl FrameInfo {
    /// Decode the raw `mpv_render_frame_info` returned by
    /// `mpv_render_context_get_info(MPV_RENDER_FRAME_INFO)`.
    #[must_use]
    pub fn from_raw(info: mpv::mpv_render_frame_info) -> Self {
        let flags = info.flags;
        Self {
            present: flags & frame_info_flag::PRESENT != 0,
            redraw: flags & frame_info_flag::REDRAW != 0,
            repeat: flags & frame_info_flag::REPEAT != 0,
            block_vsync: flags & frame_info_flag::BLOCK_VSYNC != 0,
            flags,
            target_time_us: info.target_time,
        }
    }

    /// `MPV_RENDER_FRAME_INFO_PRESENT`: there is a frame worth displaying at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.flags & frame_info_flag::PRESENT == 0
    }

    /// False when libmpv reported 0 (no valid target time).
    #[must_use]
    pub fn has_target_time(&self) -> bool {
        self.target_time_us != 0
    }
}

/// Software-render target for the `MPV_RENDER_API_TYPE_SW` last resort.
///
/// Deliberately *not* `Copy`/`Clone`: it borrows the caller's pixel buffer
/// (`&'a mut [u8]`), and libmpv writes into that buffer through the pointer we
/// pass, so exactly one owner must exist per render call.
///
/// This is a Rust-side description only; it is deliberately **not** `#[repr(C)]`
/// and libmpv is never given a pointer into it. `render.h` types are explicit:
/// `MPV_RENDER_PARAM_SW_SIZE` is `int[2]` (`render.h:353-360`, `int s[2] =
/// {w, h}; param.data = &s[0];`) and `MPV_RENDER_PARAM_SW_STRIDE` is `size_t*`
/// (`render.h:386-390`), so [`RenderContext::render_inner`] builds a `[c_int; 2]`
/// array and a `usize` stride instead of borrowing these fields' addresses.
#[derive(Debug, PartialEq, Eq)]
pub struct SwTarget<'a> {
    /// Frame width in pixels (first element of `MPV_RENDER_PARAM_SW_SIZE`).
    pub width: i32,
    /// Frame height in pixels (second element of `MPV_RENDER_PARAM_SW_SIZE`).
    pub height: i32,
    /// `MPV_RENDER_PARAM_SW_FORMAT` - one of [`SwTarget::DOCUMENTED_FORMATS`].
    ///
    /// `render.h:361-385` lists "rgb0", "bgr0", "0bgr", "0rgb", "rgb24" and says
    /// any other (mpv-internal) format name is not a good idea to rely on, so
    /// "bgra" - which is *not* in the documented list - is not offered here.
    pub format: &'static str,
    /// Row stride in bytes (`MPV_RENDER_PARAM_SW_STRIDE`, `size_t*` => `usize`).
    pub stride: usize,
    /// Caller-owned buffer, at least `height * stride` bytes
    /// (`MPV_RENDER_PARAM_SW_POINTER`: `render.h:412-420` - everything between
    /// `pointer` and `pointer + stride * h` is overwritten).
    pub pixels: &'a mut [u8],
}

impl SwTarget<'_> {
    /// The pixel formats `render.h:366-380` documents for SW rendering, in the
    /// order the header lists them.
    pub const DOCUMENTED_FORMATS: [&'static str; 5] = ["rgb0", "bgr0", "0bgr", "0rgb", "rgb24"];

    /// Bytes per pixel for [`Self::format`] (`render.h:366-380`: 4-byte
    /// components for the `*bgr0`/`0*` formats, 1 byte for `rgb24`).
    /// `None` for anything the header does not document.
    #[must_use]
    pub fn pixel_size(&self) -> Option<usize> {
        match self.format {
            "rgb0" | "bgr0" | "0bgr" | "0rgb" => Some(4),
            "rgb24" => Some(1),
            _ => None,
        }
    }

    /// `true` when `pixels` and `stride` are 64-byte aligned, which
    /// `render.h:392-399` asks for ("should be a multiple of 64 to facilitate
    /// fast SIMD operation"). Only a performance hint - never a correctness
    /// requirement - so it is reported, not enforced.
    #[must_use]
    pub fn is_fast_aligned(&self) -> bool {
        self.stride.is_multiple_of(64) && (self.pixels.as_ptr() as usize).is_multiple_of(64)
    }

    /// The bytes libmpv is allowed to write: `stride * height`, matching the
    /// documented overwrite range (`render.h:412-420`).
    #[must_use]
    pub fn required_bytes(&self) -> Option<usize> {
        usize::try_from(self.height)
            .ok()
            .and_then(|h| h.checked_mul(self.stride))
    }

    /// Validate the SW contract *before* handing libmpv the pointers, so a
    /// bogus size/stride cannot make libmpv write out of bounds.
    ///
    /// # Errors
    ///
    /// `PlayerError::UnsupportedItem` (`code() == "unsupported_item"`) with the
    /// header line that the violated rule comes from.
    pub fn check(&self) -> Result<(), PlayerError> {
        if self.width <= 0 || self.height <= 0 {
            return Err(bad_param(
                "SwTarget width/height must be > 0: MPV_RENDER_PARAM_SW_SIZE is int[2] {w, h} (render.h:353-360)",
            ));
        }
        let Some(pixel) = self.pixel_size() else {
            return Err(bad_param(&format!(
                "SwTarget format {:?} is not one of the documented MPV_RENDER_PARAM_SW_FORMAT values {:?} (render.h:361-385)",
                self.format,
                Self::DOCUMENTED_FORMATS,
            )));
        };
        if !self.stride.is_multiple_of(pixel) {
            return Err(bad_param(&format!(
                "SwTarget stride {} is not a multiple of the {}-byte pixel alignment (render.h:386-390)",
                self.stride, pixel,
            )));
        }
        if usize::try_from(self.width)
            .ok()
            .and_then(|w| w.checked_mul(pixel))
            .is_none_or(|row| self.stride < row)
        {
            return Err(bad_param(&format!(
                "SwTarget stride {} is smaller than one row ({} px * {} bytes) required by MPV_RENDER_PARAM_SW_SIZE (render.h:386-390)",
                self.stride, self.width, pixel
            )));
        }
        match self.required_bytes() {
            Some(need) if self.pixels.len() >= need => Ok(()),
            Some(need) => Err(bad_param(&format!(
                "SwTarget buffer is {} bytes but libmpv writes pointer..pointer+stride*h = {need} bytes (render.h:412-420)",
                self.pixels.len()
            ))),
            None => Err(bad_param(
                "SwTarget stride * height overflows usize (render.h:386-420)",
            )),
        }
    }
}

/// A slot whose address we can hand to libmpv as callback/userdata context.
/// Sized, so the raw pointer we pass is thin.
///
/// `count` is bumped inside the update callback so [`RenderContext::callback_count`]
/// can report how often libmpv woke us; an atomic is required because the
/// callback runs on libmpv's thread, not the render thread.
struct SignalSlot {
    wake: Option<Box<dyn Fn() + Send + Sync>>,
    count: AtomicU64,
}

/// One `mpv_render_context`, created against an app-owned GL context.
///
/// # Threading and GL-context contract (from `render.h`, see module docs)
///
/// * `create`, `update`, `render`, `report_swap` and `free` all run on the
///   **render thread**, with the app's GL context current, for the whole life of
///   this value.
/// * The update callback may fire with no new frame; only
///   [`RenderContext::take_update`] decides whether a redraw is due.
/// * `take_update` must not be called from inside the update callback
///   (`render.h:634-661`).
pub struct RenderContext {
    ctx: *mut mpv::mpv_render_context,
    /// Update-callback userdata: the slot libmpv's callback receives, so it must
    /// outlive `ctx`.
    signal: Arc<SignalSlot>,
    /// `get_proc_address_ctx` userdata: the app's GL resolver.
    ///
    /// Rust never reads this field after `create`: libmpv reads the resolver
    /// through the raw pointer it was handed. `Drop` runs before the field drops
    /// (and clears the callback first), which is what makes the trampoline's
    /// `&*(ctx as *const ProcSlot)` dereference sound for the whole life of the
    /// libmpv context.
    #[allow(dead_code, reason = "read by libmpv through get_proc_address_ctx")]
    proc_slot: Arc<ProcSlot>,
    api: RenderApi,
    /// Whether `MPV_RENDER_PARAM_ADVANCED_CONTROL` was enabled at creation;
    /// exposed by [`RenderContext::advanced_control`] because it decides whether
    /// the app is obliged to call [`RenderContext::report_swap`].
    advanced_control: bool,
    /// A clone of the player core's teardown owner (`crate::mpv_link::CorePin`).
    ///
    /// This is the runtime half of the `render.h:122-123` rule ("free the
    /// context with `mpv_render_context_free()` before the mpv core is
    /// destroyed"): while this value is alive the refcount held by
    /// `FfiPlayer` is >1, so `FfiPlayer::drop` defers `mpv_destroy`, and because
    /// `Drop::drop` runs before *any* field of this struct is dropped (an
    /// ordering guarantee of `Drop`, independent of declaration order - `closed`
    /// is declared after this field), `Drop` has already run
    /// `mpv_render_context_free` before this clone releases the core.
    ///
    /// Rust never reads it: its only role is to be released after the free.
    /// The refcount rule itself is asserted in
    /// `crate::mpv_link::tests::a_live_render_clone_defers_mpv_destroy_to_corepin`;
    /// the free-before-destroy order for a real core is exercised by
    /// `crates/plex-player/tests/render_sw_session.rs`.
    #[expect(
        dead_code,
        reason = "RAII hold on the mpv core, released only after mpv_render_context_free"
    )]
    core: Arc<CorePin>,
    closed: bool,
}

/// Holds the app's GL resolver so libmpv's `get_proc_address_ctx` stays valid.
struct ProcSlot {
    resolver: Option<Box<dyn ProcResolver>>,
}

impl RenderContext {
    /// Create the render context for `player`'s `mpv_handle`.
    ///
    /// # Safety contract (checked at run time, not compile time)
    ///
    /// The caller must have a GL context current on **this thread** that will
    /// stay current and alive until this value is dropped, and must keep calling
    /// every other method on this same thread (`render.h:68-70`). Nothing here
    /// can detect a violation, which is why the app creates the render context
    /// right after it creates its window/GL context on the event-loop thread.
    ///
    /// `wake` is called from libmpv's update-callback thread; it must only
    /// schedule work (e.g. send a winit user event) - it must not call back into
    /// libmpv or touch GL.
    ///
    /// `native_display` is the window's raw display handle. For an OpenGL
    /// context its Xlib/Wayland variant becomes
    /// `MPV_RENDER_PARAM_X11_DISPLAY`/`MPV_RENDER_PARAM_WL_DISPLAY`
    /// (`render_gl.h`: EGL-based hwdec interop needs the native display); every
    /// other handle variant and `None` pass no native-display parameter, and it
    /// is ignored by the SW backend. The `Display*`/`wl_display*` behind the
    /// handle must stay valid until the context is dropped (`render.h:235-247`).
    pub fn create(
        player: &FfiPlayer,
        api: RenderApi,
        resolver: Box<dyn ProcResolver>,
        wake: Option<Box<dyn Fn() + Send + Sync>>,
        advanced_control: bool,
        native_display: Option<RawDisplayHandle>,
    ) -> Result<Self, PlayerError> {
        if player.is_closed() {
            return Err(bad_param(
                "render context needs a live mpv_handle: mpv_render_context_create runs before the core is destroyed (render.h:122-123)",
            ));
        }
        let proc_slot = Arc::new(ProcSlot {
            resolver: Some(resolver),
        });
        let signal = Arc::new(SignalSlot {
            wake,
            count: AtomicU64::new(0),
        });

        let init = mpv::mpv_opengl_init_params {
            get_proc_address: Some(Self::get_proc_address),
            get_proc_address_ctx: Arc::as_ptr(&proc_slot).cast_mut().cast(),
        };
        // `init` is copied into the param array below, so keep it in a named
        // local that outlives the call; libmpv copies the struct itself.
        let mut params: Vec<mpv::mpv_render_param> = vec![param(
            mpv::mpv_render_param_type_MPV_RENDER_PARAM_API_TYPE,
            api.api_type().as_ptr().cast_mut().cast(),
        )];
        if api == RenderApi::OpenGL {
            params.push(param(
                mpv::mpv_render_param_type_MPV_RENDER_PARAM_OPENGL_INIT_PARAMS,
                &init as *const mpv::mpv_opengl_init_params as *mut mpv::mpv_opengl_init_params
                    as *mut c_void,
            ));
            // `render_gl.h`: EGL-based hwdec interop (e.g. Intel/AMD/Linux
            // vaapi) needs the native display; a raw handle from another
            // windowing system contributes nothing.
            params.extend(native_display.and_then(native_display_param));
        }
        // `MPV_RENDER_PARAM_ADVANCED_CONTROL` is `int*` (`render.h:285`). It is
        // held in a named local declared *before* the param array so the pointer
        // stays valid until `mpv_render_context_create` returns, and so
        // `Self::advanced_control` describes the value libmpv was actually given.
        let mut adv = adv_control_flag(advanced_control);
        params.push(param(
            mpv::mpv_render_param_type_MPV_RENDER_PARAM_ADVANCED_CONTROL,
            ptr::from_mut(&mut adv).cast(),
        ));
        params.push(mpv::mpv_render_param {
            type_: mpv::mpv_render_param_type_MPV_RENDER_PARAM_INVALID,
            data: ptr::null_mut(),
        });

        let mut ctx: *mut mpv::mpv_render_context = ptr::null_mut();
        // SAFETY: `params` is a NUL-terminated (INVALID-terminated) array that
        // outlives the call, every `data` pointer points at a live object for
        // the duration of the call (`init` and `adv` are locals of this
        // function, `api.api_type()` is 'static), the mpv handle is live
        // (checked above), and the userdata pointers are kept alive by
        // `proc_slot`/`signal`, which this function moves into `Self`. mpv may
        // call `get_proc_address` during this call, which is legal
        // (render_gl.h: "the OpenGL context is indirectly accessed through the
        // get_proc_address callback"). The native-display `Display*`/`wl_display*`
        // is not a local but the app window's own window-server display, which
        // outlives the render context as `render.h:235-247` requires (the
        // context is created per playing item, the window outlives every item).
        let code = unsafe {
            mpv::mpv_render_context_create(&mut ctx, player.mpv_handle_ptr(), params.as_mut_ptr())
        };
        if code != mpv::mpv_error_MPV_ERROR_SUCCESS || ctx.is_null() {
            // Nothing to free: the failure path holds no libmpv context, and the
            // two userdata `Arc`s are dropped normally by the caller.
            return Err(rejected("mpv_render_context_create", code));
        }

        let me = Self {
            ctx,
            signal,
            proc_slot,
            api,
            // Read back `adv`: it is the int libmpv actually read (and possibly
            // amended) through `MPV_RENDER_PARAM_ADVANCED_CONTROL`.
            advanced_control: adv != 0,
            // Declared last so it is released last: `Drop::drop` frees the
            // libmpv render context, then this clone releases the core.
            core: player.core_pin(),
            closed: false,
        };
        // SAFETY: `ctx` is non-null and live; the callback is a plain C function
        // pointer and the userdata (`Arc::as_ptr(&me.signal)`) outlives the
        // context because `signal` is a field of `Self`, which outlives every
        // other use of `ctx`. Dropping `Self` clears the callback first.
        unsafe {
            mpv::mpv_render_context_set_update_callback(
                ctx,
                Some(Self::update_callback),
                Arc::as_ptr(&me.signal).cast_mut().cast(),
            );
        }
        Ok(me)
    }

    /// `mpv_opengl_init_params.get_proc_address` trampoline.
    ///
    /// # Safety
    ///
    /// `ctx` must be the `Arc::as_ptr` of a live `ProcSlot` field (guaranteed by
    /// [`RenderContext::create`] and cleared on drop), and `name` a live C
    /// string owned by libmpv for the duration of the call.
    unsafe extern "C" fn get_proc_address(ctx: *mut c_void, name: *const c_char) -> *mut c_void {
        if ctx.is_null() || name.is_null() {
            return ptr::null_mut();
        }
        // SAFETY: `ctx` is the `Arc::as_ptr(&proc_slot)` handed to
        // `mpv_render_context_create` in [`RenderContext::create`]. `Self` owns
        // that `Arc` and `Drop` frees the libmpv context first, so the slot is
        // live whenever libmpv can still reach this callback. `name` is a
        // NUL-terminated string owned by libmpv for the duration of the call
        // (`render_gl.h:43` - `get_proc_address(get_proc_address_ctx, name)`).
        let slot = unsafe { &*(ctx as *const ProcSlot) };
        match slot.resolver.as_ref() {
            Some(resolver) => resolver.resolve(unsafe { CStr::from_ptr(name) }),
            None => ptr::null_mut(),
        }
    }

    /// The libmpv update callback (`mpv_render_update_fn`).
    ///
    /// # Safety
    ///
    /// `ctx` is the userdata passed in [`RenderContext::create`]: the address of
    /// a live `SignalSlot` owned by an `Arc` field of `Self`. The callback only
    /// calls the app's `wake` closure - never libmpv, never GL - because
    /// `render.h:634-661` forbids calling `mpv_render_context_update()` (or
    /// anything that re-enters the player) from here.
    unsafe extern "C" fn update_callback(ctx: *mut c_void) {
        if ctx.is_null() {
            return;
        }
        let slot = unsafe { &*(ctx as *const SignalSlot) };
        slot.count.fetch_add(1, Ordering::Relaxed);
        if let Some(wake) = slot.wake.as_ref() {
            wake();
        }
    }

    /// `true` while no libmpv context is held (`Drop` sets this, so a freed
    /// context can never be used again).
    #[must_use]
    pub fn is_null(&self) -> bool {
        self.ctx.is_null()
    }

    /// How many libmpv update callbacks have fired since `create`.
    ///
    /// A callback does *not* mean there is a new frame; use [`Self::take_update`].
    /// Read from the signal slot (the callback increments it on libmpv's thread).
    #[must_use]
    pub fn callback_count(&self) -> u64 {
        self.signal.count.load(Ordering::Relaxed)
    }

    /// `mpv_render_context_update()` - the only sanctioned redraw trigger.
    ///
    /// Must run on the render thread, with the app's GL context current, and
    /// never from inside the update callback (`render.h:634-661`). It must be
    /// called exactly once per wake-up: libmpv only raises the flag once per
    /// frame, so skipping it loses the redraw, and calling it twice for one
    /// frame is what the `update_drains_once_per_frame` test guards against.
    ///
    /// # Errors
    ///
    /// `PlayerError::Closed` if the context has been freed.
    pub fn take_update(&mut self) -> Result<RenderUpdate, PlayerError> {
        self.ensure_live("take_update")?;
        // SAFETY: `ctx` is non-null and live (`ensure_live`), this is the render
        // thread with the app's GL context current (typestate contract, module
        // docs), and we are not inside the update callback: the callback only
        // calls the app's `wake` closure, which schedules this call.
        let flags = unsafe { mpv::mpv_render_context_update(self.ctx) };
        Ok(RenderUpdate { flags })
    }

    /// `mpv_render_context_render()` with `MPV_RENDER_PARAM_BLOCK_FOR_TARGET_TIME`
    /// set from [`RenderOptions::block_for_target_time`] ("block" mode honours
    /// mpv's video timing and may block until the frame's target display time,
    /// `render.h:300-317`; "no-block" mode returns immediately and the caller
    /// paces itself).
    ///
    /// All objects libmpv may read are owned by this wrapper for the duration of
    /// the call (see [`Self::render_inner`]): the caller only has to keep
    /// `opts`/[`SwTarget`] alive and their sizes in sync with the buffer mpv may
    /// write. A GL-backend render without `opts.fbo` is rejected rather than
    /// passed to a backend whose missing-target behaviour is unspecified.
    ///
    /// # Errors
    ///
    /// `PlayerError::Closed` if the context has been freed,
    /// `PlayerError::UnsupportedItem` when the OpenGL backend is given no
    /// `opts.fbo` or when this context was created with [`RenderApi::Software`]
    /// (SW contexts are rendered with [`Self::render_sw`]), or
    /// `PlayerError::Rejected` when libmpv rejects the parameters (a mismatched
    /// FBO size is the usual cause; the OpenGL backend reports it as
    /// `MPV_ERROR_INVALID_PARAMETER`).
    pub fn render(&mut self, opts: &RenderOptions) -> Result<(), PlayerError> {
        self.render_inner(opts, None, false)
    }

    /// Software-backend render (`MPV_RENDER_API_TYPE_SW`): libmpv copies the
    /// frame into the caller-owned [`SwTarget`] buffer. This is the documented
    /// last resort when no GL context can be created at all, and it is the path
    /// `crates/plex-player/tests/render_sw_session.rs` measures.
    ///
    /// # Errors
    ///
    /// `PlayerError::UnsupportedItem` if this context was not created with
    /// [`RenderApi::Software`] or if [`SwTarget::check`] rejects a target that
    /// would let libmpv write out of bounds, then as for [`Self::render`].
    pub fn render_sw(&mut self, target: &mut SwTarget<'_>) -> Result<(), PlayerError> {
        if self.api != RenderApi::Software {
            return Err(bad_param(
                "render_sw needs a context created with RenderApi::Software: MPV_RENDER_PARAM_SW_* is only valid for MPV_RENDER_API_TYPE_SW",
            ));
        }
        self.render_inner(
            &RenderOptions {
                fbo: None,
                block_for_target_time: true,
                flip_y: false,
                skip_rendering: false,
            },
            Some(target),
            true,
        )
    }

    fn render_inner(
        &mut self,
        opts: &RenderOptions,
        mut sw: Option<&mut SwTarget<'_>>,
        sw_api: bool,
    ) -> Result<(), PlayerError> {
        self.ensure_live("render")?;
        // The parameter set must match the backend the context was created with.
        // `render.h:704-706` ("It's left unspecified what happens with unknown
        // parameters"), so an SW context asked for `MPV_RENDER_PARAM_OPENGL_FBO`
        // - or a GL context asked for `MPV_RENDER_PARAM_SW_*` - is refused here
        // rather than passed to libmpv and guessed at.
        match (self.api, sw_api) {
            (RenderApi::Software, false) => {
                return Err(bad_param(
                    "render() passes MPV_RENDER_PARAM_OPENGL_FBO, which needs a context created with MPV_RENDER_API_TYPE_OPENGL; this context is MPV_RENDER_API_TYPE_SW - use render_sw() (render.h:125-141, 704-706)",
                ));
            }
            (RenderApi::OpenGL, true) => {
                return Err(bad_param(
                    "render_sw() passes MPV_RENDER_PARAM_SW_*, which needs a context created with MPV_RENDER_API_TYPE_SW; this context is MPV_RENDER_API_TYPE_OPENGL (render.h:125-141, render_gl.h)",
                ));
            }
            (RenderApi::OpenGL, false) | (RenderApi::Software, true) => {}
        }
        // Every object libmpv may dereference through `params.data` has to be
        // alive for the *whole* `mpv_render_context_render` call: the array only
        // stores raw pointers and libmpv reads them inside the call
        // (`render.h:706-710`). All of them are therefore declared at **this**
        // function's scope - before `params` - so none of them can be dropped at
        // the end of a conditional block while the array still points at them.
        let block = adv_control_flag(opts.block_for_target_time);
        let flip = adv_control_flag(opts.flip_y);
        let skip = adv_control_flag(opts.skip_rendering);
        // `MPV_RENDER_PARAM_SW_SIZE` is `int[2]` (`render.h:353-360`,
        // `int s[2] = {w, h}; param.data = &s[0];`), so the two dimensions are
        // passed as one array instead of borrowing two Rust fields whose
        // adjacency no layout guarantee would give us.
        let mut sw_size: [c_int; 2] = [0, 0];
        // `MPV_RENDER_PARAM_SW_STRIDE` is `size_t*` (`render.h:386-390`), so the
        // value libmpv reads is a `usize`, not a 4-byte int.
        let mut sw_stride: usize = 0;
        // `MPV_RENDER_PARAM_SW_POINTER` is `void*` passed **by value**
        // (`render.h:405-420`, `param.data` *is* the pixel pointer).
        let mut sw_pointer: *mut u8 = ptr::null_mut();
        // `MPV_RENDER_PARAM_SW_FORMAT` is `char*`; the owned `CString` lives here
        // rather than in `SwTarget`, whose `format` is only a `&'static str`.
        let mut sw_format: Option<CString> = None;
        // `MPV_RENDER_PARAM_OPENGL_FBO` is `mpv_opengl_fbo*` (`render.h:199-203`).
        let mut gl_fbo = mpv::mpv_opengl_fbo {
            fbo: 0,
            w: 0,
            h: 0,
            internal_format: 0,
        };

        if sw_api {
            // SW_SIZE/SW_FORMAT/SW_STRIDE/SW_POINTER are mandatory
            // (`render.h:125-141`, `render.h:341-420`).
            let Some(t) = sw.as_mut() else {
                return Err(bad_param(
                    "the SW backend needs MPV_RENDER_PARAM_SW_SIZE, SW_FORMAT, SW_STRIDE and SW_POINTER (render.h:125-141)",
                ));
            };
            t.check()?;
            sw_size = [t.width, t.height];
            sw_stride = t.stride;
            sw_pointer = t.pixels.as_mut_ptr();
            sw_format = Some(
                CString::new(t.format)
                    .map_err(|e| bad_param(&format!("SwTarget::format is not a C string: {e}")))?,
            );
        } else if let Some(fbo) = opts.fbo {
            gl_fbo = mpv::mpv_opengl_fbo {
                fbo: fbo.fbo,
                w: fbo.width,
                h: fbo.height,
                internal_format: fbo.internal_format,
            };
        } else {
            // `render.h:704-706`: "Which parameters are required depends on the
            // backend. It's left unspecified what happens with unknown
            // parameters." A GL render with no target is therefore refused here
            // instead of being handed to libmpv and hoped for.
            return Err(bad_param(
                "the OpenGL backend needs RenderOptions::fbo (MPV_RENDER_PARAM_OPENGL_FBO, render.h:199-203); render.h:704-706 leaves a missing target unspecified",
            ));
        }

        let mut params: Vec<mpv::mpv_render_param> = Vec::new();
        if sw_api {
            params.push(param(
                mpv::mpv_render_param_type_MPV_RENDER_PARAM_SW_SIZE,
                ptr::from_mut(&mut sw_size).cast(),
            ));
            params.push(param(
                mpv::mpv_render_param_type_MPV_RENDER_PARAM_SW_STRIDE,
                ptr::from_mut(&mut sw_stride).cast(),
            ));
            params.push(param(
                mpv::mpv_render_param_type_MPV_RENDER_PARAM_SW_POINTER,
                sw_pointer.cast(),
            ));
            if let Some(fmt) = &sw_format {
                params.push(param(
                    mpv::mpv_render_param_type_MPV_RENDER_PARAM_SW_FORMAT,
                    fmt.as_ptr().cast_mut().cast(),
                ));
            }
        } else {
            params.push(param(
                mpv::mpv_render_param_type_MPV_RENDER_PARAM_OPENGL_FBO,
                ptr::from_mut(&mut gl_fbo).cast(),
            ));
        }
        params.push(param(
            mpv::mpv_render_param_type_MPV_RENDER_PARAM_BLOCK_FOR_TARGET_TIME,
            ptr::from_ref(&block).cast_mut().cast(),
        ));
        // Sent verbatim even though `render.h:156` says this build ignores it:
        // the value must reach libmpv unchanged so the behaviour is measurable.
        params.push(param(
            mpv::mpv_render_param_type_MPV_RENDER_PARAM_FLIP_Y,
            ptr::from_ref(&flip).cast_mut().cast(),
        ));
        if opts.skip_rendering {
            params.push(param(
                mpv::mpv_render_param_type_MPV_RENDER_PARAM_SKIP_RENDERING,
                ptr::from_ref(&skip).cast_mut().cast(),
            ));
        }
        params.push(mpv::mpv_render_param {
            type_: mpv::mpv_render_param_type_MPV_RENDER_PARAM_INVALID,
            data: ptr::null_mut(),
        });
        // SAFETY: `params` is INVALID-terminated and lives for the whole call;
        // every `data` points at something that also outlives the call - the
        // locals `block`, `flip`, `skip`, `sw_size`, `sw_stride`, `sw_format`,
        // `gl_fbo` (all declared above `params`, dropped after it), or the
        // caller-owned buffer behind `sw_pointer`, which [`SwTarget::check`]
        // verified covers `stride * height` (`render.h:412-420`) and which the
        // borrow of `target` keeps alive for the whole call; `ctx` is live; the
        // app's GL context is current on this thread (module docs).
        let code = unsafe { mpv::mpv_render_context_render(self.ctx, params.as_mut_ptr()) };
        if code != mpv::mpv_error_MPV_ERROR_SUCCESS {
            return Err(rejected("mpv_render_context_render", code));
        }
        Ok(())
    }

    /// `mpv_render_context_get_info(MPV_RENDER_PARAM_NEXT_FRAME_INFO)`.
    ///
    /// This asks libmpv what the *next* frame looks like before rendering it; it
    /// is how the app decides whether to block on vsync and when a `render()` may
    /// legitimately return without drawing. In this build the call takes a single
    /// `mpv_render_param` (not a param array) and the only frame-info param id is
    /// `MPV_RENDER_PARAM_NEXT_FRAME_INFO` - the newer `MPV_RENDER_PARAM_FRAME_INFO`
    /// is **not** in the installed headers/bindings, so this wrapper does not use it.
    ///
    /// # Errors
    ///
    /// `PlayerError::Closed` if freed, or `PlayerError::Rejected` when libmpv has
    /// no frame info yet (it answers `MPV_ERROR_INVALID_PARAMETER` in that case).
    pub fn next_frame_info(&mut self) -> Result<Option<FrameInfo>, PlayerError> {
        self.ensure_live("next_frame_info")?;
        let mut info = mpv::mpv_render_frame_info {
            flags: 0,
            target_time: 0,
        };
        let param = param(
            mpv::mpv_render_param_type_MPV_RENDER_PARAM_NEXT_FRAME_INFO,
            ptr::from_mut(&mut info).cast(),
        );
        // SAFETY: `ctx` is live (`ensure_live`), `info` is a live local whose
        // pointer `param` refers to for the whole call, and the app's GL context
        // is current on this thread (module docs).
        let code = unsafe { mpv::mpv_render_context_get_info(self.ctx, param) };
        if code != mpv::mpv_error_MPV_ERROR_SUCCESS {
            // "no frame info available yet" is a normal answer, not a failure.
            if code == mpv::mpv_error_MPV_ERROR_INVALID_PARAMETER {
                return Ok(None);
            }
            return Err(rejected("mpv_render_context_get_info", code));
        }
        Ok(Some(FrameInfo::from_raw(info)))
    }

    /// `mpv_render_context_report_swap()` - tell libmpv the app actually put the
    /// frame on screen ("displayed"). Without it libmpv's A/V sync keeps waiting
    /// for a swap that already happened.
    ///
    /// # Errors
    ///
    /// `PlayerError::Closed` if the context has been freed.
    pub fn report_swap(&mut self) -> Result<(), PlayerError> {
        self.ensure_live("report_swap")?;
        // SAFETY: `ctx` is live; render thread with the app's GL context
        // current (`render.h:68-70`).
        unsafe { mpv::mpv_render_context_report_swap(self.ctx) };
        Ok(())
    }

    /// `true` when the render context was created with
    /// `MPV_RENDER_PARAM_ADVANCED_CONTROL = 1`: libmpv then leaves presentation
    /// pacing to the app, which must call [`RenderContext::report_swap`] after
    /// it has actually shown a frame.
    #[must_use]
    pub fn advanced_control(&self) -> bool {
        self.advanced_control
    }

    /// The API backend this context was created with.
    #[must_use]
    pub fn api(&self) -> RenderApi {
        self.api
    }

    fn ensure_live(&self, _what: &str) -> Result<(), PlayerError> {
        if self.closed || self.ctx.is_null() {
            return Err(PlayerError::Closed);
        }
        Ok(())
    }
}

impl Drop for RenderContext {
    fn drop(&mut self) {
        if !self.ctx.is_null() {
            // Clear the callback before freeing: libmpv must not be able to call
            // `Self::update_callback` with our `Arc`s while they are being freed
            // (the `Arc`s themselves are only released after this runs, since
            // field drops happen after `Drop::drop`).
            unsafe {
                mpv::mpv_render_context_set_update_callback(self.ctx, None, ptr::null_mut());
                mpv::mpv_render_context_free(self.ctx);
            }
            self.ctx = ptr::null_mut();
        }
        self.closed = true;
    }
}

/// `MPV_RENDER_PARAM_ADVANCED_CONTROL` value helper (1 = enable, 0 = disable).
fn adv_control_flag(on: bool) -> c_int {
    i32::from(on)
}

/// Build one `mpv_render_param` entry.
fn param(kind: mpv::mpv_render_param_type, data: *mut c_void) -> mpv::mpv_render_param {
    mpv::mpv_render_param { type_: kind, data }
}

/// Pick the native-display creation parameter for a raw-window-handle display
/// handle: `MPV_RENDER_PARAM_X11_DISPLAY` for Xlib and
/// `MPV_RENDER_PARAM_WL_DISPLAY` for Wayland, `None` for every other variant
/// (Windows/macOS windowing systems, and an Xlib handle without a `Display*`).
///
/// The `data` field carries the `Display*`/`struct wl_display*` value itself
/// (`render.h:235-247`), not a pointer to storage, so this copies the pointer.
#[must_use]
fn native_display_param(handle: RawDisplayHandle) -> Option<mpv::mpv_render_param> {
    let (kind, display) = match handle {
        RawDisplayHandle::Xlib(h) => (
            mpv::mpv_render_param_type_MPV_RENDER_PARAM_X11_DISPLAY,
            h.display?,
        ),
        RawDisplayHandle::Wayland(h) => (
            mpv::mpv_render_param_type_MPV_RENDER_PARAM_WL_DISPLAY,
            h.display,
        ),
        _ => return None,
    };
    Some(param(kind, display.as_ptr()))
}

/// libmpv rejected one of the render-context calls' parameters/state.
///
/// [`PlayerError::Rejected`] is the existing variant for "the queueing call itself
/// rejected the request", with `call` naming the libmpv entry point, so render
/// failures stay machine-readable (`code() == "rejected"`) like control failures.
fn rejected(call: &str, code: i32) -> PlayerError {
    PlayerError::Rejected {
        call: call.to_owned(),
        command: "render_context".to_owned(),
        code,
        name: crate::events::error_name(code).to_owned(),
    }
}

/// A contract violation detected in this wrapper before calling libmpv.
fn bad_param(reason: &str) -> PlayerError {
    PlayerError::UnsupportedItem {
        item: "render_context".to_owned(),
        reason: reason.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_frame_flag_comes_from_header() {
        // Not hand-declared: the value is the bindgen-emitted enum constant.
        assert_eq!(MPV_RENDER_UPDATE_FRAME, 1);
        assert_eq!(
            MPV_RENDER_UPDATE_FRAME,
            mpv::mpv_render_update_flag_MPV_RENDER_UPDATE_FRAME as u64
        );
        assert_eq!(
            RenderUpdate {
                flags: MPV_RENDER_UPDATE_FRAME
            }
            .unknown_flags(),
            0
        );
        assert!(!RenderUpdate { flags: 0 }.has_frame());
        assert!(RenderUpdate {
            flags: MPV_RENDER_UPDATE_FRAME
        }
        .has_frame());
        // An unknown bit is reported rather than silently treated as a frame.
        assert_eq!(RenderUpdate { flags: 1 << 7 }.unknown_flags(), 1 << 7);
        assert!(!RenderUpdate { flags: 1 << 7 }.has_frame());
    }

    #[test]
    fn frame_info_decodes_the_header_flags() {
        let raw = mpv::mpv_render_frame_info {
            flags: mpv::mpv_render_frame_info_flag_MPV_RENDER_FRAME_INFO_PRESENT as u64
                | mpv::mpv_render_frame_info_flag_MPV_RENDER_FRAME_INFO_BLOCK_VSYNC as u64,
            target_time: 1_234_567,
        };
        let info = FrameInfo::from_raw(raw);
        assert!(info.present && !info.redraw && !info.repeat && info.block_vsync);
        assert!(!info.is_empty());
        assert!(info.has_target_time());
        assert_eq!(info.flags, 0b1001);
        let empty = FrameInfo::from_raw(mpv::mpv_render_frame_info {
            flags: 0,
            target_time: 0,
        });
        assert!(empty.is_empty());
        assert!(!empty.has_target_time());
    }

    #[test]
    fn api_type_strings_come_from_the_header() {
        assert_eq!(
            RenderApi::OpenGL.api_type(),
            mpv::MPV_RENDER_API_TYPE_OPENGL
        );
        assert_eq!(RenderApi::Software.api_type(), mpv::MPV_RENDER_API_TYPE_SW);
    }

    #[test]
    fn advanced_control_flag_is_the_c_int_libmpv_expects() {
        assert_eq!(adv_control_flag(true), 1);
        assert_eq!(adv_control_flag(false), 0);
    }

    #[test]
    fn gl_fbo_mirrors_mpv_opengl_fbo_field_order() {
        let app = GlFbo {
            fbo: 7,
            width: 640,
            height: 360,
            internal_format: 0,
        };
        let raw = mpv::mpv_opengl_fbo {
            fbo: app.fbo,
            w: app.width,
            h: app.height,
            internal_format: app.internal_format,
        };
        assert_eq!(
            (raw.fbo, raw.w, raw.h, raw.internal_format),
            (7, 640, 360, 0)
        );
    }

    #[test]
    fn sw_target_check_enforces_the_documented_sw_contract() {
        fn target<'a>(
            buf: &'a mut [u8],
            w: i32,
            h: i32,
            fmt: &'static str,
            stride: usize,
        ) -> SwTarget<'a> {
            SwTarget {
                width: w,
                height: h,
                format: fmt,
                stride,
                pixels: buf,
            }
        }
        // The shape `render.h:353-420` requires: 32x18, 4 bytes/pixel, stride 128.
        let mut buf = vec![0u8; 4 * 32 * 18];
        let ptr_aligned = (buf.as_ptr() as usize).is_multiple_of(64);
        let good = target(&mut buf, 32, 18, "rgb0", 4 * 32);
        assert_eq!(good.pixel_size(), Some(4));
        assert_eq!(good.required_bytes(), Some(128 * 18));
        assert_eq!(
            good.is_fast_aligned(),
            good.stride.is_multiple_of(64) && ptr_aligned
        );
        assert!(
            good.check().is_ok(),
            "a conforming SwTarget must be accepted"
        );

        // "bgra" is not one of the values render.h:366-380 documents.
        let mut buf = vec![0u8; 4 * 32 * 18];
        let bad = target(&mut buf, 32, 18, "bgra", 4 * 32);
        assert_eq!(bad.pixel_size(), None);
        match bad.check() {
            Ok(()) => panic!("an undocumented SW format must be refused"),
            Err(e) => assert_eq!(e.code(), "unsupported_item"),
        }

        // Stride must be a multiple of the pixel alignment (render.h:386-390).
        let mut buf = vec![0u8; 4 * 32 * 18];
        assert!(target(&mut buf, 32, 18, "rgb0", 130).check().is_err());
        // ...and at least one row wide.
        let mut buf = vec![0u8; 4 * 32 * 18];
        assert!(target(&mut buf, 32, 18, "rgb0", 124).check().is_err());
        // Buffer shorter than the `pointer..pointer + stride * h` range libmpv
        // overwrites (render.h:412-420) is refused *before* the call.
        let mut buf = vec![0u8; 4 * 32 * 18];
        let short = target(&mut buf, 32, 18, "rgb0", 4 * 40);
        assert!(short.pixels.len() < short.required_bytes().unwrap_or(0));
        assert!(short.check().is_err());
        // Non-positive dimensions are refused too.
        let mut buf = vec![0u8; 4 * 32 * 18];
        assert!(target(&mut buf, 32, 0, "rgb0", 128).check().is_err());
        // rgb24 (1 byte/pixel, render.h:375-377) has a different alignment rule.
        let mut buf = vec![0u8; 3 * 32 * 18];
        let rgb24 = target(&mut buf, 32, 18, "rgb24", 96);
        assert_eq!(rgb24.pixel_size(), Some(1));
        assert!(
            rgb24.check().is_ok(),
            "rgb24 is documented in render.h:375-377"
        );
    }

    #[test]
    fn sw_wire_types_match_the_header_types() {
        // MPV_RENDER_PARAM_SW_SIZE is `int[2]` (render.h:353-360): the wrapper's
        // array is two C ints, 8 bytes, and libmpv is given &s[0].
        assert_eq!(core::mem::size_of::<[c_int; 2]>(), 8);
        // MPV_RENDER_PARAM_SW_STRIDE is `size_t*` (render.h:386-390): a usize,
        // not an i32 - a 4-byte field would make libmpv read 8 bytes out of it.
        assert!(core::mem::size_of::<usize>() >= core::mem::size_of::<c_int>());
        // SwTarget is a Rust-side description only: nothing borrows its fields,
        // so its layout cannot affect what libmpv reads.
        assert_eq!(
            SwTarget::DOCUMENTED_FORMATS,
            ["rgb0", "bgr0", "0bgr", "0rgb", "rgb24"]
        );
    }

    #[test]
    fn native_display_param_follows_the_raw_handle_variant() {
        use raw_window_handle::{
            WaylandDisplayHandle, WindowsDisplayHandle, XcbDisplayHandle, XlibDisplayHandle,
        };
        use std::ptr::NonNull;
        // Only the pointer value is compared, never dereferenced: a dangling
        // but aligned `()` address stands in for a live wl_display/Display.
        let display = NonNull::from(&());
        let param_of = |kind| param(kind, display.cast::<c_void>().as_ptr());
        assert_eq!(
            native_display_param(RawDisplayHandle::Xlib(XlibDisplayHandle::new(
                Some(display.cast(),),
                0
            ))),
            Some(param_of(
                mpv::mpv_render_param_type_MPV_RENDER_PARAM_X11_DISPLAY
            ))
        );
        assert_eq!(
            native_display_param(RawDisplayHandle::Wayland(WaylandDisplayHandle::new(
                display.cast()
            ))),
            Some(param_of(
                mpv::mpv_render_param_type_MPV_RENDER_PARAM_WL_DISPLAY
            ))
        );
        // Other variants pass nothing: a Windows display, an xcb connection
        // (not a `Display*`), and an Xlib handle whose display pointer is None.
        assert_eq!(
            native_display_param(RawDisplayHandle::Windows(WindowsDisplayHandle::new())),
            None
        );
        assert_eq!(
            native_display_param(RawDisplayHandle::Xcb(XcbDisplayHandle::new(
                Some(display.cast()),
                0
            ))),
            None
        );
        assert_eq!(
            native_display_param(RawDisplayHandle::Xlib(XlibDisplayHandle::new(None, 0))),
            None
        );
    }

    #[test]
    fn defaults_are_the_pacing_expecting_defaults() {
        let opts = RenderOptions::default();
        assert!(opts.block_for_target_time, "default must honour mpv timing");
        assert!(!opts.flip_y, "flip_y default must be the documented no-op");
        assert!(!opts.skip_rendering);
        assert!(opts.fbo.is_none());
    }
}
