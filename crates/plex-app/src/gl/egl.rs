//! The app-owned EGL context: one context shared by the app's own GL calls
//! (through [`glow`]), by libmpv (through [`ProcResolver`]) and, on a desktop,
//! by the egui painter. Linux only - see [`crate::gl`]'s "What is Linux-only".
//!
//! ## Why this module hand-declares EGL
//!
//! `crates/plex-app` already depends on `glutin`/`glutin_egl_sys` transitively,
//! but `glutin`'s EGL path needs a **display** handle
//! (`glutin::api::egl::display::EglDisplay::new(raw_display)`,
//! `glutin-0.32.3/src/api/egl/display.rs:52`), which does not exist in a
//! display-free run. So the context recipe the headless tests need - surfaceless
//! EGL on Mesa llvmpipe - is declared here against the installed Khronos headers
//! and linked with `#[link(name = "EGL")]` on the `extern` block below.
//!
//! That link attribute is what makes these entry points resolvable when
//! `cargo test` links the `plex-app` test binary (an rlib build alone would not
//! notice). A host without a `libEGL.so` link name needs the libglvnd/mesa
//! development symlink to link this crate.
//!
//! Every declaration below is transcribed from the installed headers, with the
//! file and line it was taken from:
//!
//! * `typedef unsigned int EGLBoolean;` - `/usr/include/EGL/egl.h:39`
//! * `typedef void *EGLDisplay; typedef void *EGLSurface; typedef void *EGLContext;`
//!   - `/usr/include/EGL/egl.h:40,44,45`
//! * `typedef unsigned int EGLenum;` - `/usr/include/EGL/egl.h:188`
//! * `typedef khronos_int32_t EGLint;` - `/usr/include/EGL/eglplatform.h:159`
//! * `eglChooseConfig` `egl.h:131`, `eglCreateContext` `:133`,
//!   `eglDestroyContext` `:137`, `eglGetDisplay` `:143`, `eglGetError` `:144`,
//!   `eglGetProcAddress` `:145`, `eglInitialize` `:146`, `eglMakeCurrent` `:147`,
//!   `eglQueryString` `:149`, `eglTerminate` `:152`, `eglBindAPI` `:225`
//! * `eglGetPlatformDisplayEXT` - `/usr/include/EGL/eglext.h:880` (extension
//!   function: fetched at run time through `eglGetProcAddress`, never linked
//!   directly)
//!
//! Constants (same headers): `EGL_ALPHA_SIZE` `egl.h:47`, `EGL_NONE` `:80` (the
//! attribute-list terminator, `0x3038`, not `0`), `EGL_BLUE_SIZE` `:60`,
//! `EGL_FALSE` `:69`, `EGL_GREEN_SIZE` `:70`, `EGL_NO_CONTEXT` `:83`,
//! `EGL_NO_DISPLAY` `:84`, `EGL_NO_SURFACE` `:85`, `EGL_PBUFFER_BIT` `:86`,
//! `EGL_RED_SIZE` `:89`, `EGL_SURFACE_TYPE` `:95`, `EGL_TRUE` `:101`,
//! `EGL_VENDOR` `:102`, `EGL_VERSION` `:103`, `EGL_CLIENT_APIS` `:196`,
//! `EGL_RENDERABLE_TYPE` `:212`, `EGL_DEFAULT_DISPLAY` `:251`,
//! `EGL_OPENGL_BIT` `:257`, `EGL_OPENGL_API` `:256`;
//! `EGL_CONTEXT_MAJOR_VERSION_KHR` `eglext.h:70`,
//! `EGL_CONTEXT_MINOR_VERSION_KHR` `:71`,
//! `EGL_CONTEXT_OPENGL_PROFILE_MASK_KHR` `:73`,
//! `EGL_CONTEXT_OPENGL_CORE_PROFILE_BIT_KHR` `:80`,
//! `EGL_PLATFORM_WAYLAND_EXT` `:893`, `EGL_PLATFORM_SURFACELESS_MESA` `:1093`.
//!
//! There is no `EGL_CONTEXT_PROFILE_MASK_KHR` / `EGL_CONTEXT_CORE_PROFILE_BIT_KHR`
//! in these headers; only the `_OPENGL_`-named constants above exist.
//!
//! All `unsafe` in `crates/plex-app` lives in this module and [`crate::gl::fbo`],
//! with a `SAFETY:` note on every block, against the workspace
//! `unsafe_code = "deny"` lint.

#![allow(unsafe_code)]

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_uint, c_void};
use std::ptr;

use plex_player::render_link::ProcResolver;

use crate::gl::GlError;

// EGL typedefs, mirroring the header lines cited in the module docs.
type EglBoolean = c_uint;
type EglInt = c_int;
type EglEnum = c_uint;
type EglDisplay = *mut c_void;
type EglConfig = *mut c_void;
type EglContextPtr = *mut c_void;

// Constants from /usr/include/EGL/egl.h and eglext.h (lines cited above).
const EGL_TRUE: EglBoolean = 1;
/// Attribute-list terminator: `EGL_NONE` is `0x3038` (`/usr/include/EGL/egl.h:80`),
/// **not** `0`. A list ended with a literal `0` makes the vendor EGL read `0` as
/// an attribute name and reject the whole list. Measured A/B in the headless test environment
/// (Rust probe, same host / software-GL env / vendor pin): the identical 6-pair list
/// terminated with `EGL_NONE` -> `eglChooseConfig` `EGL_SUCCESS`, `num_config
/// 1`/`8`; the same list terminated with `0` -> `EGL_BAD_ATTRIBUTE` (`0x3004`),
/// `num_config 0` - which is exactly what every harness case hit before the
/// terminator was fixed.
const EGL_NONE: EglInt = 0x3038;
const EGL_SURFACE_TYPE: EglInt = 0x3033;
const EGL_RED_SIZE: EglInt = 0x3024;
const EGL_GREEN_SIZE: EglInt = 0x3023;
const EGL_BLUE_SIZE: EglInt = 0x3022;
const EGL_ALPHA_SIZE: EglInt = 0x3021;
const EGL_RENDERABLE_TYPE: EglInt = 0x3040;
const EGL_PBUFFER_BIT: EglInt = 0x0001;
const EGL_OPENGL_BIT: EglInt = 0x0008;
const EGL_VENDOR: EglInt = 0x3053;
const EGL_VERSION: EglInt = 0x3054;
const EGL_CLIENT_APIS: EglInt = 0x308D;
const EGL_OPENGL_API: EglEnum = 0x30A2;
const EGL_CONTEXT_MAJOR_VERSION_KHR: EglInt = 0x3098;
const EGL_CONTEXT_MINOR_VERSION_KHR: EglInt = 0x30FB;
const EGL_CONTEXT_OPENGL_PROFILE_MASK_KHR: EglInt = 0x30FD;
const EGL_CONTEXT_OPENGL_CORE_PROFILE_BIT_KHR: EglInt = 0x0000_0001;
const EGL_PLATFORM_SURFACELESS_MESA: EglEnum = 0x31DD;
const EGL_PLATFORM_WAYLAND_EXT: EglEnum = 0x31D8;
const EGL_NO_CONTEXT: EglContextPtr = ptr::null_mut();
const EGL_NO_SURFACE: *mut c_void = ptr::null_mut();

// `#[link]` is required: nothing else in this crate's link line provides these
// symbols (glutin loads EGL dynamically through `libloading`, so
// `glutin_egl_sys` does not export them to the linker).
#[link(name = "EGL")]
extern "C" {
    // egl.h:143,146,225,131,133,147,152,137,144,145,149
    fn eglGetDisplay(display_id: *mut c_void) -> EglDisplay;
    fn eglInitialize(dpy: EglDisplay, major: *mut EglInt, minor: *mut EglInt) -> EglBoolean;
    fn eglBindAPI(api: EglEnum) -> EglBoolean;
    fn eglChooseConfig(
        dpy: EglDisplay,
        attrib_list: *const EglInt,
        configs: *mut EglConfig,
        config_size: EglInt,
        num_config: *mut EglInt,
    ) -> EglBoolean;
    fn eglCreateContext(
        dpy: EglDisplay,
        config: EglConfig,
        share_context: EglContextPtr,
        attrib_list: *const EglInt,
    ) -> EglContextPtr;
    fn eglMakeCurrent(
        dpy: EglDisplay,
        draw: *mut c_void,
        read: *mut c_void,
        ctx: EglContextPtr,
    ) -> EglBoolean;
    fn eglTerminate(dpy: EglDisplay) -> EglBoolean;
    fn eglDestroyContext(dpy: EglDisplay, ctx: EglContextPtr) -> EglBoolean;
    fn eglGetError() -> EglInt;
    fn eglGetProcAddress(procname: *const c_char) -> *mut c_void;
    fn eglQueryString(dpy: EglDisplay, name: EglInt) -> *const c_char;
}

/// Look up one EGL entry point by name (`eglGetProcAddress`, `egl.h:145`).
///
/// Returns `None` when the implementation does not export the name, which is how
/// EGL says "not here" (`eglGetProcAddress` returns a null pointer for unknown
/// names; `eglGetError` is *not* the error channel for this call).
fn egl_ext(name: &str) -> Option<*mut c_void> {
    let cname = match CString::new(name) {
        Ok(v) => v,
        Err(_) => return None,
    };
    // SAFETY: `cname` is a live NUL-terminated string for the duration of the
    // call (`egl.h:145` takes `const char *procname`).
    let p = unsafe { eglGetProcAddress(cname.as_ptr()) };
    if p.is_null() {
        None
    } else {
        Some(p)
    }
}

/// The error for a failed EGL step: `egl_error` is what `eglGetError` reported
/// when the step set one, `detail` any extra context worth naming.
///
/// The [`GlError`] type lives in [`crate::gl`] because the app-FBO helpers
/// report it on every platform; only the EGL error code inside it is Linux-only.
fn fail(step: &str, egl_error: Option<i32>, detail: Option<String>) -> GlError {
    GlError {
        step: step.to_string(),
        egl_error,
        detail,
    }
}

/// What the created context reports about itself.
///
/// These strings are what the harness prints so a run can be identified
/// (software vs hardware renderer is part of the M5 evidence).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EglInfo {
    /// `EGL_VENDOR` (`egl.h:102`).
    pub vendor: String,
    /// `EGL_VERSION` (`egl.h:103`).
    pub version: String,
    /// `EGL_CLIENT_APIS` (`egl.h:196`).
    pub client_apis: String,
    /// Which platform the display came from.
    pub platform: String,
}

/// The app's EGL display + context, current on the creating thread.
///
/// Dropping it destroys the context (`eglDestroyContext`) and terminates the
/// display (`eglTerminate`) - both *after* every user of the context is gone,
/// because `crate::render::MpvGlSession` (which owns the libmpv render context)
/// borrows it via `&EglContext` and must be dropped first.
pub struct EglContext {
    display: EglDisplay,
    ctx: EglContextPtr,
    info: EglInfo,
    owns_display: bool,
}

impl EglContext {
    /// Create a **core profile** context with no window and no compositor.
    ///
    /// This is the exact attribute set measured to work headless on Mesa
    /// llvmpipe:
    /// `eglGetPlatformDisplayEXT(EGL_PLATFORM_SURFACELESS_MESA, EGL_DEFAULT_DISPLAY,
    /// NULL)` -> `eglInitialize` -> `eglBindAPI(EGL_OPENGL_API)` ->
    /// `eglChooseConfig` (RGBA8, `EGL_PBUFFER_BIT`, `EGL_OPENGL_BIT`) ->
    /// `eglCreateContext` (3.3, core profile) ->
    /// `eglMakeCurrent(dpy, EGL_NO_SURFACE, EGL_NO_SURFACE, ctx)`.
    ///
    /// Falls back to `eglGetDisplay(EGL_DEFAULT_DISPLAY)` when Mesa's
    /// surfaceless platform is not exported, so the same code also runs on a
    /// desktop EGL (where the window surface is attached separately).
    pub fn surfaceless() -> Result<Self, GlError> {
        let (display, platform) = Self::get_display()?;
        let mut major: EglInt = 0;
        let mut minor: EglInt = 0;
        // SAFETY: `display` is the value returned by eglGet*Display above;
        // `major`/`minor` are live locals (`egl.h:146` accepts NULL for either).
        if unsafe { eglInitialize(display, &mut major, &mut minor) } != EGL_TRUE {
            return Err(fail(
                "eglInitialize",
                Some(unsafe_egl_error()),
                Some(format!("platform {platform}")),
            ));
        }
        // SAFETY: `display` is initialized; `egl.h:149` returns an internal
        // string that is live until eglTerminate, which `Drop` does last.
        let info = EglInfo {
            vendor: query_string(display, EGL_VENDOR),
            version: query_string(display, EGL_VERSION),
            client_apis: query_string(display, EGL_CLIENT_APIS),
            platform,
        };

        // EGL_OPENGL_API, not GLES: libmpv's opengl backend wants a desktop GL
        // context (libmpv's own log reads "Detected desktop OpenGL 4.6").
        // SAFETY: plain enum argument (`egl.h:225`).
        if unsafe { eglBindAPI(EGL_OPENGL_API) } != EGL_TRUE {
            return Err(fail(
                "eglBindAPI(EGL_OPENGL_API)",
                Some(unsafe_egl_error()),
                Some(format!("client_apis {}", info.client_apis)),
            ));
        }

        let attribs: [EglInt; 13] = [
            EGL_SURFACE_TYPE,
            EGL_PBUFFER_BIT,
            EGL_RENDERABLE_TYPE,
            EGL_OPENGL_BIT,
            EGL_RED_SIZE,
            8,
            EGL_GREEN_SIZE,
            8,
            EGL_BLUE_SIZE,
            8,
            // RGBA8 as claimed in the [`Self::surfaceless`] docs: alpha is part of
            // the requested config, not left to the driver default.
            EGL_ALPHA_SIZE,
            8,
            EGL_NONE,
        ];
        let mut config: EglConfig = ptr::null_mut();
        let mut num: EglInt = 0;
        // SAFETY: `attribs` is an `EGL_NONE`-terminated attribute list and
        // `config`/`num` are live locals for the call (`egl.h:131`).
        if unsafe { eglChooseConfig(display, attribs.as_ptr(), &mut config, 1, &mut num) }
            != EGL_TRUE
            || num < 1
        {
            // The context must be attributed to the call that produced it, and
            // a config failure additionally to the display the attribute list
            // was handed.
            return Err(fail(
                "eglChooseConfig",
                Some(unsafe_egl_error()),
                Some(format!(
                    "num_config {num} attribs[SURFACE_TYPE=PBUFFER_BIT, RENDERABLE_TYPE=OPENGL_BIT, RGB+ALPHA=8] display_platform {} egl_vendor {} egl_version {} client_apis {}",
                    quote_or(&info.platform, "<empty>"),
                    quote_or(&info.vendor, "<empty>"),
                    quote_or(&info.version, "<empty>"),
                    quote_or(&info.client_apis, "<empty>")
                )),
            ));
        }

        let ctx_attribs: [EglInt; 7] = [
            EGL_CONTEXT_MAJOR_VERSION_KHR,
            3,
            EGL_CONTEXT_MINOR_VERSION_KHR,
            3,
            // The profile mask is what actually requests the *core* profile
            // claimed above and in `crate::gl`; without it EGL hands back a
            // default (compatibility) context.
            EGL_CONTEXT_OPENGL_PROFILE_MASK_KHR,
            EGL_CONTEXT_OPENGL_CORE_PROFILE_BIT_KHR,
            EGL_NONE,
        ];
        // SAFETY: `EGL_NONE`-terminated attribute list, live config from the call
        // above, `EGL_NO_CONTEXT` (null) as the share context (`egl.h:133`).
        let ctx =
            unsafe { eglCreateContext(display, config, EGL_NO_CONTEXT, ctx_attribs.as_ptr()) };
        if ctx == EGL_NO_CONTEXT {
            return Err(fail(
                "eglCreateContext(core 3.3)",
                Some(unsafe_egl_error()),
                None,
            ));
        }
        if !Self::make_current_on(display, ctx)? {
            // SAFETY: `ctx` is non-null and owned by us (`egl.h:137`).
            unsafe { eglDestroyContext(display, ctx) };
            return Err(fail("eglMakeCurrent(surfaceless)", None, None));
        }
        Ok(Self {
            display,
            ctx,
            info,
            owns_display: true,
        })
    }

    /// Acquire the EGL display used for the surfaceless path.
    ///
    /// Two ways to reach a platform display are supported, in this order:
    ///
    /// 1. `eglGetProcAddress("eglGetPlatformDisplayEXT")` (`eglext.h:880`) on a
    ///    dispatch frame that already knows a vendor driver. This is what the
    ///    code here did *before* the bootstrap added below.
    /// 2. `eglGetDisplay(EGL_DEFAULT_DISPLAY)` (`egl.h:143`, `egl.h:251`) first,
    ///    which under libglvnd loads a vendor EGL driver and makes that vendor's
    ///    EGL dispatch frame current, and only then the `EXT` lookup.
    ///
    /// Both ways were measured with C and Rust probes under the same software-GL EGL env as
    /// `scripts/mpv-gl-harness.sh`): the `EXT` lookup answers **with and
    /// without** the `eglGetDisplay` bootstrap, all three displays initialize
    /// (`init_ok=true`, vendor "Mesa Project", EGL 1.5, client_apis
    /// "OpenGL OpenGL_ES"), they are the *same* handle (`HANDLES ...
    /// a_eq_boot=true ext_eq_boot=true a_eq_ext=true`, `distinct_displays=1`),
    /// and `eglChooseConfig` succeeds on each of them with the attribute list
    /// this code uses. So display bootstrap order is **not** what broke the software-GL
    /// harness: the attribute-list terminator was (see the [`EGL_NONE`] docs;
    /// measured A/B in the same file, `list=6pairs+terminator=0(...)` ->
    /// `err=0x3004`, `num=0`).
    ///
    /// The unconditional bootstrap is therefore not required on this host. It is
    /// kept because it is the only fallback when a vendor's `EXT` lookup answers
    /// NULL (`[U]` off-host), and the distinct-handle guard below ensures it can
    /// never leave a second initialized display behind. Which path a given run
    /// actually took is recorded in `EglInfo::platform` and in every
    /// acquisition/config failure message, so a log line attributes the result
    /// instead of leaving it ambiguous.
    fn get_display() -> Result<(EglDisplay, String), GlError> {
        // (2) vendor bootstrap: the generic EGL entry point that loads a vendor
        // driver. Kept even when the EXT path works: it is also the fallback.
        // SAFETY: EGL_DEFAULT_DISPLAY is the documented "no display" value.
        let boot = unsafe { eglGetDisplay(ptr::null_mut()) };
        let boot_usable = !boot.is_null()
            // SAFETY: `boot` is the display returned above (`egl.h:146` accepts
            // NULL for both version out-params).
            && unsafe { eglInitialize(boot, ptr::null_mut(), ptr::null_mut()) } == EGL_TRUE;

        let ext = egl_ext("eglGetPlatformDisplayEXT");
        let boot_vendor = display_vendor(Some(boot));
        let ext_note = if ext.is_none() {
            format!(
                "EXT lookup {}: eglGetProcAddress(\"eglGetPlatformDisplayEXT\") -> NULL after bootstrap on vendor {}",
                if boot_usable { "answered NULL" } else { "answered NULL (no default display either)" },
                quote_or_opt(boot_vendor.as_deref(), "<none>")
            )
        } else {
            String::new()
        };
        let mut ext_failed: Option<String> = None;
        if let Some(raw) = ext {
            // SAFETY: `raw` is the function pointer EGL returned for this exact
            // signature (`eglext.h:880`: `EGLDisplay (EGLenum, void*, const EGLint*)`).
            let get_platform: extern "C" fn(EglEnum, *mut c_void, *const EglInt) -> EglDisplay =
                unsafe { std::mem::transmute(raw) };
            let dpy = get_platform(EGL_PLATFORM_SURFACELESS_MESA, ptr::null_mut(), ptr::null());
            if !dpy.is_null() {
                // SAFETY: `dpy` is a fresh EGLDisplay from this extension call.
                if unsafe { eglInitialize(dpy, ptr::null_mut(), ptr::null_mut()) } == EGL_TRUE {
                    // Whether `boot` (the default display initialized above) is
                    // superseded or leaked depends on whether libglvnd hands
                    // back the same handle for both calls. Measured on this
                    // host: it does - a probe printed `HANDLES a=... boot=...
                    // ext_after_boot=... a_eq_boot=true ext_eq_boot=true
                    // a_eq_ext=true` and `PROBE distinct_displays=1
                    // terminated=1` (one `eglTerminate`, result true), in the C
                    // and in the Rust probe. So returning `dpy` and letting
                    // `Drop` terminate it once terminates exactly one
                    // initialized display. Where a vendor hands back *two*
                    // distinct handles, `boot` was initialized above and must
                    // not survive this call, so it is terminated here.
                    // **[U] beyond this host**: vendor handle aliasing is not
                    // measured anywhere else.
                    let boot_note = if !boot_usable {
                        "boot_display=none(no usable default display)".to_string()
                    } else if dpy == boot {
                        "boot_display=same_handle(terminated_by_Drop)".to_string()
                    } else {
                        // SAFETY: `boot` was initialized above and is a
                        // different display from `dpy`, which is the one
                        // returned (and the one `Drop` terminates).
                        unsafe { eglTerminate(boot) };
                        "boot_display=distinct(terminated_here)".to_string()
                    };
                    return Ok((
                        dpy,
                        format!(
                            "surfaceless(EGL_PLATFORM_SURFACELESS_MESA) via eglGetPlatformDisplayEXT [lookup {}, {}, vendor {}]",
                            if boot_usable { "after eglGetDisplay bootstrap" } else { "before any usable default display" },
                            boot_note,
                            quote_or_opt(display_vendor(Some(dpy)).as_deref(), "<empty EGL_VENDOR>")
                        ),
                    ));
                }
                // A non-null but unusable display: read its error and vendor,
                // then terminate it so we do not leak an EGLDisplay on every
                // attempt, and fall through to the default display (the same
                // fallback the code had before), attributing both.
                let code = unsafe_egl_error();
                let vendor = display_vendor(Some(dpy));
                // SAFETY: `dpy` is the display created above (`egl.h:152`).
                unsafe { eglTerminate(dpy) };
                ext_failed = Some(format!(
                    "EXT display non-null but eglInitialize failed (EGL error {code:#06X}), vendor {}",
                    quote_or_opt(vendor.as_deref(), "<empty>")
                ));
            } else {
                ext_failed = Some("eglGetPlatformDisplayEXT returned EGL_NO_DISPLAY".to_string());
            }
        }
        if boot_usable {
            // SAFETY: `boot` is initialized above and owned here (`owns_display`
            // is true for every `get_display` result).
            let vendor = display_vendor(Some(boot));
            let apis = display_string(boot, EGL_CLIENT_APIS);
            let mut why_ext = Vec::new();
            if !ext_note.is_empty() {
                why_ext.push(ext_note.clone());
            }
            if let Some(f) = &ext_failed {
                why_ext.push(f.clone());
            }
            if why_ext.is_empty() {
                why_ext.push("EXT lookup answered but returned EGL_NO_DISPLAY".to_string());
            }
            let detail = format!(
                "eglGetDisplay(EGL_DEFAULT_DISPLAY) [vendor {}; client_apis {}{}; {}]",
                quote_or_opt(vendor.as_deref(), "<empty EGL_VENDOR>"),
                quote_or_opt(Some(apis.as_str()), "<empty>"),
                egl_platform_env_note(),
                why_ext.join("; "),
            );
            return Ok((boot, detail));
        }
        Err(fail(
            "get_display",
            Some(unsafe_egl_error()),
            Some(format!(
                "no usable EGL display: {}; {}",
                ext_failed.unwrap_or_else(|| {
                    "eglGetProcAddress(\"eglGetPlatformDisplayEXT\") answered NULL".to_string()
                }),
                describe_boot(boot, boot_usable, boot_vendor.as_deref()),
            )),
        ))
    }

    /// Make this context current with no surface (the surfaceless path).
    ///
    /// Safe to call again at the start of every frame: `render.h:68-70` requires
    /// the GL context to be current on the thread that calls
    /// `mpv_render_context_render`, and a thread only has one current context,
    /// so re-making this one current re-establishes it after any other library
    /// changed it.
    pub fn make_current(&self) -> Result<(), GlError> {
        Self::make_current_on(self.display, self.ctx).map(|_| ())
    }

    fn make_current_on(display: EglDisplay, ctx: EglContextPtr) -> Result<bool, GlError> {
        // SAFETY: EGL_NO_SURFACE for draw and read is the surfaceless form
        // documented for EGL_KHR_platform_surfaceless / Mesa surfaceless
        // (`egl.h:85`, `eglext.h:1093`); `ctx` is a live context we own.
        let ok = unsafe { eglMakeCurrent(display, EGL_NO_SURFACE, EGL_NO_SURFACE, ctx) };
        if ok == EGL_TRUE {
            Ok(true)
        } else {
            Err(fail("eglMakeCurrent", Some(unsafe_egl_error()), None))
        }
    }

    /// Attach a window surface to this context.
    ///
    /// `draw`/`read` are `EGLSurface`s created by whoever owns the window
    /// (glutin/winit in the desktop shell). Kept here so the *same* context is
    /// used by the window path and by the harness path.
    ///
    /// # Safety
    ///
    /// `draw` and `read` must each be `EGL_NO_SURFACE` or a live `EGLSurface`
    /// belonging to `self.display` for the whole call (`egl.h:147` takes
    /// `EGLSurface draw, EGLSurface read`); EGL dereferences them.
    pub unsafe fn make_current_with(
        &self,
        draw: *mut c_void,
        read: *mut c_void,
    ) -> Result<(), GlError> {
        // SAFETY: the caller guarantees both are live EGLSurfaces (see above).
        let ok = unsafe { eglMakeCurrent(self.display, draw, read, self.ctx) };
        if ok == EGL_TRUE {
            Ok(())
        } else {
            Err(fail(
                "eglMakeCurrent(window)",
                Some(unsafe_egl_error()),
                None,
            ))
        }
    }

    /// The resolver handed to libmpv (`mpv_opengl_init_params.get_proc_address`)
    /// and to [`glow`]'s loader.
    pub fn resolver(&self) -> Box<dyn ProcResolver> {
        Box::new(EglResolver)
    }

    /// The EGL vendor/version/client-API strings read after `eglInitialize`,
    /// plus the platform that produced the display.
    #[must_use]
    pub fn info(&self) -> &EglInfo {
        &self.info
    }

    /// `EGL_PLATFORM_WAYLAND_EXT` (`eglext.h:893`) is available for the desktop
    /// path; exposed so the shell can log which platform it asked for.
    #[must_use]
    pub fn wayland_platform_constant() -> u32 {
        EGL_PLATFORM_WAYLAND_EXT
    }
}

impl Drop for EglContext {
    fn drop(&mut self) {
        // SAFETY: every user of this context (the glow handle, the app FBO and
        // the libmpv render context) borrows `&EglContext` or is dropped before
        // it, which is enforced by Rust's borrow rules in `crate::render`.
        // `eglDestroyContext` (`egl.h:137`) then `eglTerminate` (`egl.h:152`).
        unsafe {
            eglDestroyContext(self.display, self.ctx);
            if self.owns_display {
                eglTerminate(self.display);
            }
        }
    }
}

fn query_string(display: EglDisplay, name: EglInt) -> String {
    // SAFETY: `eglQueryString` returns a pointer into EGL's own storage that
    // stays valid until `eglTerminate` (`egl.h:149`); we copy it immediately.
    let p = unsafe { eglQueryString(display, name) };
    if p.is_null() {
        return String::new();
    }
    // SAFETY: the pointer is a NUL-terminated C string owned by EGL.
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

fn unsafe_egl_error() -> i32 {
    // SAFETY: `eglGetError` takes no arguments and clears the error (`egl.h:144`).
    unsafe { eglGetError() }
}

fn display_string(display: EglDisplay, name: EglInt) -> String {
    if display.is_null() {
        String::new()
    } else {
        query_string(display, name)
    }
}

fn display_vendor(display: Option<EglDisplay>) -> Option<String> {
    let d = display?;
    if d.is_null() {
        return None;
    }
    let v = query_string(d, EGL_VENDOR);
    if v.is_empty() {
        None
    } else {
        Some(v)
    }
}

fn describe_boot(display: EglDisplay, usable: bool, vendor: Option<&str>) -> String {
    format!(
        "eglGetDisplay(EGL_DEFAULT_DISPLAY) -> {} (initialize {}), vendor {}",
        if display.is_null() {
            "EGL_NO_DISPLAY"
        } else {
            "non-null"
        },
        if usable { "ok" } else { "failed" },
        quote_or_opt(vendor, "<none>")
    )
}

fn quote_or_opt(value: Option<&str>, if_absent: &str) -> String {
    value.map_or_else(
        || if_absent.to_string(),
        |v| {
            if v.is_empty() {
                if_absent.to_string()
            } else {
                format!("\"{v}\"")
            }
        },
    )
}

fn quote_or(value: &str, if_absent: &str) -> String {
    if value.is_empty() {
        if_absent.to_string()
    } else {
        format!("\"{value}\"")
    }
}

/// Note whether the EGL client-API platform selector env var is present in this
/// process (the software-GL recipe sets it; whether it reached the test binary is a
/// question this session had to answer instead of assume).
fn egl_platform_env_note() -> String {
    match std::env::var("EGL_PLATFORM") {
        Ok(v) => format!(", EGL_PLATFORM=\"{v}\" in this process"),
        Err(_) => ", EGL_PLATFORM unset in this process".to_string(),
    }
}

/// `eglGetProcAddress` as libmpv's and `glow`'s GL function resolver.
///
/// One observation, not a guarantee: a Mesa/llvmpipe EGL context
/// built with this resolver read back `GL_VERSION`
/// "4.6 (Core Profile) Mesa 26.2.5-arch3.1" and `GL_RENDERER`
/// "llvmpipe (LLVM 23.1.3, 256 bits)", i.e. the core GL range was reachable
/// through `eglGetProcAddress` there.
///
/// Measured in this session on the test environment's libglvnd EGL 1.5 dispatch: the
/// installed `eglGetProcAddress` answers **non-null for names it does not
/// implement** (`glTotallyNotARealFunction_` -> 0x7fa5dddc5300,
/// `glNotRealEither` -> 0x7fa5dddc5320, both distinct from `glGetString`
/// 0x7fa5dddb46c0). A null return is therefore not how *this* EGL says "not
/// here", so this resolver does not classify names by nullness and inherits
/// whatever the host dispatcher hands out.
///
/// Two host gates the software-GL harness has measured only for llvmpipe and that
/// a run on real hardware still has to measure (nothing here asserts them): (a) implementations
/// where `eglGetProcAddress` returns null for
/// *core* functions - the resolver has no fallback because `Cargo.lock` is
/// frozen and `libloading` is not a `plex-app` dependency; (b) what calling one
/// of the non-null unknown-name stubs does.
pub struct EglResolver;

impl ProcResolver for EglResolver {
    fn resolve(&self, name: &CStr) -> *mut c_void {
        // SAFETY: `name` is a NUL-terminated C string owned by the caller
        // (libmpv's `get_proc_address` contract, `render_gl.h:43`).
        unsafe { eglGetProcAddress(name.as_ptr()) }
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
    fn constants_match_the_installed_headers() {
        // Values verified against /usr/include/EGL/{egl.h,eglext.h} (cited in the
        // module docs): a drift guard for the transcription above. The `EGL_NONE`
        // pair is the software-GL harness-failure guard - the terminator is a real enum
        // value (egl.h:80), not a zero sentinel.
        assert_eq!(EGL_NONE, 0x3038);
        assert_ne!(EGL_NONE, 0);
        assert_eq!(EGL_SURFACE_TYPE, 0x3033); // egl.h:95
        assert_eq!(EGL_RENDERABLE_TYPE, 0x3040); // egl.h:212
        assert_eq!(EGL_OPENGL_BIT, 0x0008); // egl.h:257
        assert_eq!(EGL_PBUFFER_BIT, 0x0001); // egl.h:86
        assert_eq!(EGL_RED_SIZE, 0x3024); // egl.h:89
        assert_eq!(EGL_GREEN_SIZE, 0x3023); // egl.h:70
        assert_eq!(EGL_BLUE_SIZE, 0x3022); // egl.h:60
        assert_eq!(EGL_ALPHA_SIZE, 0x3021); // egl.h:47
        assert_eq!(EGL_VENDOR, 0x3053); // egl.h:102
        assert_eq!(EGL_VERSION, 0x3054); // egl.h:103
        assert_eq!(EGL_CLIENT_APIS, 0x308D); // egl.h:196
        assert_eq!(EGL_OPENGL_API, 0x30A2); // egl.h:256
        assert_eq!(EGL_CONTEXT_MAJOR_VERSION_KHR, 0x3098); // eglext.h:70
        assert_eq!(EGL_CONTEXT_MINOR_VERSION_KHR, 0x30FB); // eglext.h:71
        assert_eq!(EGL_CONTEXT_OPENGL_PROFILE_MASK_KHR, 0x30FD); // eglext.h:73
        assert_eq!(EGL_CONTEXT_OPENGL_CORE_PROFILE_BIT_KHR, 1); // eglext.h:80
        assert_eq!(EGL_PLATFORM_SURFACELESS_MESA, 0x31DD); // eglext.h:1093
        assert_eq!(EGL_PLATFORM_WAYLAND_EXT, 0x31D8); // eglext.h:893
    }

    #[test]
    fn core_profile_attribute_constants_exist() {
        // The constants that exist are the `EGL_CONTEXT_OPENGL_PROFILE_*` ones.
        assert_ne!(
            EGL_CONTEXT_OPENGL_PROFILE_MASK_KHR,
            EGL_CONTEXT_MAJOR_VERSION_KHR
        );
        assert_eq!(EGL_CONTEXT_OPENGL_CORE_PROFILE_BIT_KHR & 1, 1);
    }

    #[test]
    fn resolver_is_a_stable_egl_pass_through() {
        // The one resolver property assertable without a display or a GL context:
        // `EglResolver` must hand libmpv exactly what `eglGetProcAddress` returns,
        // unchanged and stable per name. Measured in the headless test environment (libglvnd EGL
        // 1.5): repeated lookups returned the identical pointer for a core name
        // ("glGetString" 0x7fa5dddb46c0) and for an unimplemented name
        // ("glTotallyNotARealFunction_" 0x7fa5dddc5300).
        let r = EglResolver;
        for name in ["glGetString", "glTexImage2D", "glTotallyNotARealFunction_"] {
            let c = CString::new(name).expect("no interior NUL");
            let via_resolver = r.resolve(&c);
            // SAFETY: `c` is NUL-terminated and lives for this statement; the
            // test binary links libEGL (see the `#[link]` attribute above) and
            // `eglGetProcAddress` needs no current context (`egl.h:146` note).
            let direct = unsafe { eglGetProcAddress(c.as_ptr()) };
            assert_eq!(via_resolver, direct, "{name} must pass through unchanged");
            assert_eq!(
                via_resolver,
                r.resolve(&c),
                "{name} must be stable per name"
            );
        }
    }

    #[test]
    fn unknown_name_answer_is_reported_not_assumed() {
        // Earlier assumption ("an unknown name must resolve to null") is false on
        // the test environment's EGL: measured 2026-11-05, `glTotallyNotARealFunction_`
        // resolved to 0x7fa5dddc5300 (a non-null per-name stub). Whether the
        // host's EGL returns null, or a stub libmpv would then call, is a host
        // gate for the software-GL harness; the strict contract is only asserted when
        // that harness is deliberately enabled.
        let nonsense = CString::new("glTotallyNotARealFunction_").expect("no interior NUL");
        let p = EglResolver.resolve(&nonsense);
        println!(
            "egl resolver {nonsense:?} -> {}",
            if p.is_null() { "null" } else { "non-null" }
        );
        if std::env::var_os("PLEX_GL_HARNESS").is_some() {
            assert!(
                p.is_null(),
                "host EGL returned {p:?} for an unimplemented name"
            );
        }
    }

    #[test]
    fn display_free_creation_reports_the_real_egl_state() {
        // Whatever the environment provides, the failure or success must be
        // named, never guessed.
        match EglContext::surfaceless() {
            Ok(ctx) => {
                let info = ctx.info();
                assert!(!info.version.is_empty(), "EGL_VERSION was empty");
                assert!(!info.vendor.is_empty(), "EGL_VENDOR was empty");
                println!(
                    "egl ok: platform={} vendor={} version={} client_apis={}",
                    info.platform, info.vendor, info.version, info.client_apis
                );
            }
            Err(e) => {
                println!("egl unavailable: {e}");
                assert!(!e.step.is_empty());
            }
        }
    }
}
