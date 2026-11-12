//! The libmpv link: the only place in this crate that calls libmpv.
//!
//! [`PlayerLink`] is the surface [`crate::engine`] needs, which is why the
//! control loop is testable without libmpv ([`crate::mock`]) and why the smoke
//! report can show exactly what would have been sent. [`FfiPlayer`] is the real
//! implementation: one `mpv_handle` per item, async-only requests, a wakeup
//! callback, and `mpv_wait_event(handle, 0)` (never a blocking wait, never a busy
//! poll of another kind).
//!
//! All `unsafe` in the crate lives in this module (plus the generated [`crate::ffi`]
//! module); every block carries a `SAFETY:` note, and the module-level
//! `#[allow(unsafe_code)]` below is what reconciles that with the workspace's
//! `unsafe_code = "deny"` lint. `cargo clippy --all-targets -- -D warnings` fails
//! if any *other* module grows unsafe code.

#![allow(unsafe_code)]

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

use crate::commands::{Named, PropertyValue, Request, ValueKind};
use crate::events::{EndReason, EventKind, Observed, PlayerError};
use crate::ffi as mpv;

/// libmpv version this build was compiled against (pkg-config fact emitted by
/// `build.rs`; `guard-disabled` when the guard was explicitly bypassed).
#[must_use]
pub fn build_time_version() -> String {
    env!("PLEX_MPV_VERSION").to_owned()
}

/// `mpv_client_api_version()`: the `MPV_CLIENT_API_VERSION` the **loaded**
/// libmpv was compiled with (`client.h:264-266`), as a packed integer.
///
/// This is the only runtime API-version fact available without reading a
/// property, which is why the probe records it instead of assuming the
/// pkg-config string and the loaded library agree.
#[must_use]
pub fn runtime_client_api_version() -> u64 {
    // SAFETY: `mpv_client_api_version` takes no arguments, dereferences nothing
    // and returns the version the loaded library was built with (client.h:266).
    unsafe { mpv::mpv_client_api_version() as u64 }
}

/// Unpack a `MPV_CLIENT_API_VERSION` value.
///
/// `client.h:250` defines the packing as `((major) << 16) | (minor)`; this is
/// our own Rust mirror of that macro (bindgen did **not** emit
/// `MPV_CLIENT_API_VERSION` as a constant, so the comparison the probe reports
/// uses this hand-written unpacking).
#[must_use]
pub fn client_api_parts(packed: u64) -> (u64, u64) {
    (packed >> 16, packed & 0xffff)
}

/// `major << 16 | minor` for the API version this crate's headers declare
/// (`client.h:251`: `MPV_MAKE_VERSION(2, 5)`).
#[must_use]
pub const fn compile_time_client_api() -> u64 {
    (2 << 16) | 5
}

/// Machine-readable runtime fact line for the probe report.
#[must_use]
pub fn version_string() -> String {
    let packed = runtime_client_api_version();
    let (major, minor) = client_api_parts(packed);
    format!("packed=0x{packed:08x} major={major} minor={minor}")
}

// The link trait and the decoded raw-event struct live in [`crate::link`] so the
// mock and the engine compile without libmpv; this module only implements them.
use crate::link::{PlayerLink, RawEvent};

impl RawEvent {
    /// The libmpv event name, from the header-declared `mpv_event_name()`
    /// (a real libmpv string, not our own label).
    #[must_use]
    pub fn mpv_name(&self) -> String {
        // SAFETY: `mpv_event_name` returns a static string that libmpv documents
        // as valid forever and that must not be freed (client.h:1380-1388).
        unsafe {
            let ptr = mpv::mpv_event_name(self.event_id);
            if ptr.is_null() {
                return format!("event#{}", self.event_id);
            }
            CStr::from_ptr(ptr).to_string_lossy().into_owned()
        }
    }
}

/// Slot libmpv hands back to our wakeup callback.
struct WakeSlot {
    /// Set by libmpv's wakeup callback; drained by [`PlayerLink::took_wakeup`].
    flag: AtomicBool,
    /// Optional waker for the UI event loop (wires winit's
    /// `EventLoopProxy` here). Sending never blocks.
    tx: Option<Sender<()>>,
}

// The wakeup callback is a Rust function handed to libmpv as a C function
// pointer; it is *defined* here (no `extern "C"` declaration block - a
// declaration would be an import of a symbol libmpv does not export) and is
// never called from Rust directly.
#[allow(unused_variables)]
extern "C" fn wake(d: *mut c_void) {
    // SAFETY: `d` is the raw box pointer stored by `FfiPlayer::new`. `from_raw`
    // takes the strong reference libmpv's callback data owns; `mem::forget`
    // hands it straight back so the allocation's strong count is unchanged by
    // the callback and the box stays alive. `FfiPlayer::drop` unregisters the
    // callback before `Arc::from_raw`ing the box exactly once, so no call can
    // happen after the allocation is gone and the count is never leaked.
    unsafe {
        let slot = Arc::from_raw(d as *const WakeSlot);
        slot.flag.store(true, Ordering::SeqCst);
        if let Some(tx) = &slot.tx {
            let _ = tx.send(());
        }
        std::mem::forget(slot);
    }
}

/// Teardown owner for one `mpv_handle`, shared through an `Arc`.
///
/// This is the ownership half of `render.h:122-123` ("You must free the
/// context with `mpv_render_context_free()` before the mpv core is destroyed",
/// `/usr/include/mpv/render.h:122-123`): [`crate::render_link::RenderContext`]
/// keeps a clone of this `Arc`, so while a render context is alive the refcount
/// is >1 and `FfiPlayer::drop` cannot destroy the core. The destroy then runs
/// from [`CorePin::drop`] - i.e. after the render context's own `Drop` has
/// already called `mpv_render_context_free`.
pub(crate) struct CorePin {
    handle: AtomicPtr<c_void>,
}

impl CorePin {
    fn new(handle: *mut mpv::mpv_handle) -> Self {
        Self {
            handle: AtomicPtr::new(handle.cast()),
        }
    }
}

impl Drop for CorePin {
    fn drop(&mut self) {
        let handle = self.handle.get_mut();
        if !handle.is_null() {
            // SAFETY: this pointer comes from `mpv_create` and is destroyed
            // exactly once: `FfiPlayer::drop` nulls it here when it destroys the
            // core itself, and this Drop nulls it when it does
            // (client.h:302-330, `mpv_destroy`).
            unsafe { mpv::mpv_destroy(handle.cast()) };
            *handle = ptr::null_mut();
        }
    }
}

/// The real libmpv-backed link (control path).
pub struct FfiPlayer {
    handle: *mut mpv::mpv_handle,
    /// The core's teardown owner (see [`CorePin`]).
    core: Arc<CorePin>,
    slot: Option<Arc<WakeSlot>>,
    /// Raw pointer we handed to libmpv as the callback data.
    slot_raw: *mut c_void,
    version: String,
    initialized: bool,
    closed: bool,
}

// SAFETY: this value is the sole owner of its `mpv_handle` and is moved only
// once it is no longer used. libmpv documents the client API as "generally fully
// thread-safe, unless otherwise noted" (`/usr/include/mpv/client.h:136`) with no
// per-thread ownership; `Drop` does nothing but unregister the wakeup callback
// and `mpv_destroy`, which may block while the core shuts down
// (`client.h:505-514`). A wakeup still fired from a foreign thread
// (`client.h:1737`) only touches the `Arc<WakeSlot>`'s atomic flag and `Sender`.
// The per-thread rule that does apply here belongs to the render context, which
// `crate::render_link` keeps `!Send` so it cannot move off the GL thread.
unsafe impl Send for FfiPlayer {}

impl FfiPlayer {
    /// `mpv_create` + register the wakeup callback.
    ///
    /// `waker` is sent to once per wakeup (used by the UI loop in milestone M5);
    /// pass `None` for headless use.
    ///
    /// Fails with [`PlayerError::Unavailable`] when the version guard was skipped
    /// or `mpv_create` returned NULL, so a build that bypassed the guard cannot
    /// quietly pretend it has a player.
    pub fn new(waker: Option<Sender<()>>) -> Result<Self, PlayerError> {
        let version = build_time_version();
        if version == "guard-disabled" {
            return Err(PlayerError::Unavailable {
                version,
                reason: "the libmpv version guard was skipped via PLEX_DISABLE_MPV_GUARD"
                    .to_owned(),
            });
        }
        // SAFETY: `mpv_create` allocates the handle and takes no arguments; the
        // returned pointer is owned by this struct and freed in `Drop` after
        // `mpv_destroy` (client.h:272-300).
        let handle = unsafe { mpv::mpv_create() };
        if handle.is_null() {
            return Err(PlayerError::Unavailable {
                version,
                reason: "mpv_create returned NULL".to_owned(),
            });
        }
        let slot = Arc::new(WakeSlot {
            flag: AtomicBool::new(false),
            tx: waker,
        });
        let slot_raw = Arc::into_raw(Arc::clone(&slot)) as *mut c_void;
        // SAFETY: we pass libmpv the raw pointer of a live `Arc<WakeSlot>`
        // (`slot_raw`), and keep our own `slot` strong ref. mpv guarantees the
        // callback is not called after it is unregistered (see `Drop`).
        unsafe {
            mpv::mpv_set_wakeup_callback(handle, Some(wake), slot_raw);
        }
        Ok(Self {
            handle,
            core: Arc::new(CorePin::new(handle)),
            slot: Some(slot),
            slot_raw,
            version,
            initialized: false,
            closed: false,
        })
    }

    /// `mpv_initialize`. Call after setting options.
    pub fn initialize_link(&mut self) -> Result<(), PlayerError> {
        // SAFETY: `self.handle` is a live handle created by `mpv_create` and is
        // not destroyed until `Drop`.
        let code = unsafe { mpv::mpv_initialize(self.handle) };
        if code != mpv::mpv_error_MPV_ERROR_SUCCESS {
            return Err(PlayerError::InitFailed {
                code,
                name: crate::error_name(code).to_owned(),
            });
        }
        self.initialized = true;
        Ok(())
    }

    /// Synchronous `mpv_command` with string arguments (`args[0]` is the command).
    ///
    /// For the app's player controls, which address mpv's documented command
    /// and property names directly rather than through the provenance table.
    pub fn command_sync(&self, args: &[&str]) -> Result<(), PlayerError> {
        let owned = args
            .iter()
            .map(|a| cstring(a))
            .collect::<Result<Vec<_>, _>>()?;
        let mut ptrs: Vec<*const c_char> = owned.iter().map(|c| c.as_ptr()).collect();
        ptrs.push(ptr::null());
        // SAFETY: `ptrs` is a NULL-terminated array of C strings that outlive the
        // call; mpv copies what it needs; the handle is live until Drop.
        let code = unsafe { mpv::mpv_command(self.handle, ptrs.as_mut_ptr()) };
        check(
            code,
            "mpv_command",
            args.first().copied().unwrap_or_default(),
        )
    }

    /// `mpv_set_property_string` (synchronous).
    pub fn set_str(&self, name: &str, value: &str) -> Result<(), PlayerError> {
        let (n, v) = (cstring(name)?, cstring(value)?);
        // SAFETY: both C strings outlive the call, which copies them; the handle
        // is live until Drop.
        let code = unsafe { mpv::mpv_set_property_string(self.handle, n.as_ptr(), v.as_ptr()) };
        check(code, "mpv_set_property_string", name)
    }

    /// `mpv_get_property_string`: `None` when the property is unavailable now.
    #[must_use]
    pub fn get_str(&self, name: &str) -> Option<String> {
        let n = cstring(name).ok()?;
        // SAFETY: the C string outlives the call; the returned pointer is owned
        // by us and released with `mpv_free` right after it is copied.
        unsafe {
            let raw = mpv::mpv_get_property_string(self.handle, n.as_ptr());
            if raw.is_null() {
                return None;
            }
            let out = std::ffi::CStr::from_ptr(raw).to_string_lossy().into_owned();
            mpv::mpv_free(raw.cast());
            Some(out)
        }
    }

    /// `true` after `mpv_initialize` succeeded (options must be set before that).
    #[must_use]
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// The raw `mpv_handle` this value owns.
    ///
    /// Needed by [`crate::render_link`]: `mpv_render_context_create` takes this
    /// handle (`render.h:122-123`) and the render context must be freed before
    /// the player it draws for. Not public API: nothing outside this crate
    /// should get a live libmpv handle.
    #[must_use]
    pub(crate) fn mpv_handle_ptr(&self) -> *mut mpv::mpv_handle {
        self.handle
    }

    /// A hold on this core for a render context created from it.
    ///
    /// `mpv_render_context_create` takes the raw handle (`render.h:578`), which
    /// Rust cannot tie to the render context's lifetime. Handing the render
    /// context this clone is what ties them: the core is destroyed when the last
    /// clone drops, which is always after the render context that borrowed it.
    pub(crate) fn core_pin(&self) -> Arc<CorePin> {
        Arc::clone(&self.core)
    }

    /// `mpv_request_log_messages(level)`, so a failed item load can be reported
    /// with libmpv's own words (the smoke run requests `error`).
    pub fn request_log(&mut self, level: &str) -> Result<(), PlayerError> {
        let c = cstring(level)?;
        // SAFETY: `c` outlives this call (mpv copies the string), and the handle
        // is live.
        let code = unsafe { mpv::mpv_request_log_messages(self.handle, c.as_ptr()) };
        check(code, "mpv_request_log_messages", level)
    }

    /// `mpv_command_string` is deliberately **not** used anywhere: control only
    /// goes through the async functions below.
    fn send_command(
        &mut self,
        token: u64,
        args: &[String],
        label: &str,
    ) -> Result<(), PlayerError> {
        let mut cargs: Vec<CString> = Vec::with_capacity(args.len());
        for a in args {
            cargs.push(cstring(a)?);
        }
        // client.h:986-991: `args` is a **NULL-terminated** list of strings, so
        // the terminator is a NULL *pointer* entry in `ptrs`, not an extra empty
        // C string (an empty string would be a bogus command argument).
        let mut ptrs: Vec<*const c_char> = cargs.iter().map(|s| s.as_ptr()).collect();
        ptrs.push(ptr::null());
        // SAFETY: `ptrs` is a NULL-terminated array of C strings that all outlive
        // the call (mpv copies the arguments when queuing the command), and the
        // handle is live (client.h:982-991). `as_mut_ptr` matches the bindgen
        // signature `*mut *const c_char`; mpv only reads the array.
        let code = unsafe { mpv::mpv_command_async(self.handle, token, ptrs.as_mut_ptr()) };
        check(code, "mpv_command_async", label)
    }

    fn set_property(
        &mut self,
        token: u64,
        name: Named,
        value: &PropertyValue,
    ) -> Result<(), PlayerError> {
        let cname = cstring(name.name)?;
        let label = format!("set {} = {}", name.name, value.kind());
        // Typed scalars live on the stack for the duration of the call; mpv
        // copies the value when the request is queued.
        let flag: c_int = matches!(value, PropertyValue::Flag(true)) as c_int;
        let int64: i64 = match value {
            PropertyValue::Int64(i) => *i,
            _ => 0,
        };
        let double: f64 = match value {
            PropertyValue::Double(d) => *d,
            _ => 0.0,
        };
        let string = match value {
            PropertyValue::Str(s) => Some(cstring(s)?),
            _ => None,
        };
        // MPV_FORMAT_STRING is the one format whose `data` is **not** the value's
        // own bytes: the header example writes `char *value = "...";` and then
        // passes `&value` - i.e. the address of a `char*` variable
        // (client.h:663-667 "yep, you pass the address to the variable", and
        // client.h:655 for the symmetric read). Handing mpv the bytes of the
        // string instead makes it dereference arbitrary memory (SIGSEGV), which
        // is what `--smoke-no-display` reproduced. The
        // indirection target lives for the whole call.
        let string_ptr: *const c_char = string.as_ref().map(|s| s.as_ptr()).unwrap_or(ptr::null());
        let (format, data): (mpv::mpv_format, *const c_void) = match value {
            PropertyValue::Flag(_) => (
                mpv::mpv_format_MPV_FORMAT_FLAG,
                &flag as *const c_int as *const c_void,
            ),
            PropertyValue::Int64(_) => (
                mpv::mpv_format_MPV_FORMAT_INT64,
                &int64 as *const i64 as *const c_void,
            ),
            PropertyValue::Double(_) => (
                mpv::mpv_format_MPV_FORMAT_DOUBLE,
                &double as *const f64 as *const c_void,
            ),
            PropertyValue::Str(_) => (
                mpv::mpv_format_MPV_FORMAT_STRING,
                // `&string_ptr`: pointer to the `char*` storage, not the bytes.
                &string_ptr as *const *const c_char as *const c_void,
            ),
            PropertyValue::None => (mpv::mpv_format_MPV_FORMAT_NONE, ptr::null()),
        };
        // SAFETY: `format` is one of the header `mpv_format` constants and `data`
        // points at a variable of exactly that format that outlives the call:
        // `&flag`/`&int64`/`&double` for the scalar formats, and `&string_ptr`
        // (a `char*` variable whose bytes - the `CString` in `string` - also
        // outlive the call) for MPV_FORMAT_STRING. mpv copies the value when the
        // request is queued and never writes through `data`
        // (client.h:1096-1110 `@param[in] data`).
        let code = unsafe {
            mpv::mpv_set_property_async(
                self.handle,
                token,
                cname.as_ptr(),
                format,
                data as *mut c_void,
            )
        };
        check(code, "mpv_set_property_async", &label)
    }
}

impl PlayerLink for FfiPlayer {
    fn set_option(&mut self, name: &str, value: &str) -> Result<(), PlayerError> {
        let (n, v) = (cstring(name)?, cstring(value)?);
        // SAFETY: both C strings outlive the call, which copies them; the handle
        // is live and still uninitialized for these option names.
        let code = unsafe { mpv::mpv_set_option_string(self.handle, n.as_ptr(), v.as_ptr()) };
        check(code, "mpv_set_option_string", name)
    }

    fn initialize(&mut self) -> Result<(), PlayerError> {
        self.initialize_link()
    }

    fn request(&mut self, token: u64, request: &Request) -> Result<(), PlayerError> {
        match request {
            Request::Command { name, args } => {
                let mut full = Vec::with_capacity(args.len() + 1);
                full.push(name.name.to_owned());
                full.extend(args.iter().cloned());
                self.send_command(token, &full, &request.label())
            }
            Request::SetProperty { name, value } => self.set_property(token, *name, value),
        }
    }

    fn observe(&mut self, property: Named, token: u64) -> Result<(), PlayerError> {
        let cname = cstring(property.name)?;
        let kind = crate::state::observed_format(property);
        let format = match kind {
            ValueKind::Flag => mpv::mpv_format_MPV_FORMAT_FLAG,
            ValueKind::Int64 => mpv::mpv_format_MPV_FORMAT_INT64,
            ValueKind::Double => mpv::mpv_format_MPV_FORMAT_DOUBLE,
            ValueKind::String => mpv::mpv_format_MPV_FORMAT_STRING,
            ValueKind::None => mpv::mpv_format_MPV_FORMAT_NONE,
        };
        // SAFETY: `cname` outlives the call; `format` is a header constant; mpv
        // owns any memory it later delivers through this observation.
        let code = unsafe { mpv::mpv_observe_property(self.handle, token, cname.as_ptr(), format) };
        check(code, "mpv_observe_property", property.name)
    }

    fn next_event(&mut self) -> Option<RawEvent> {
        // Non-blocking drain: timeout 0 returns NULL when the queue is empty.
        // SAFETY: the returned `mpv_event*` is owned by libmpv and is valid until
        // the *next* mpv_wait_event()/mpv_terminate_destroy() call, so every
        // field is read here and copied into safe Rust before returning
        // (client.h:1181-1190, 1242-1420).
        unsafe {
            let event = mpv::mpv_wait_event(self.handle, 0.0);
            if event.is_null() {
                return None;
            }
            let event_id = (*event).event_id;
            if event_id == mpv::mpv_event_id_MPV_EVENT_NONE {
                return None;
            }
            let kind = EventKind::from_raw(event_id);
            let error = (*event).error;
            let reply_userdata = (*event).reply_userdata;
            let mut property = None;
            let mut end_file = None;
            if event_id == mpv::mpv_event_id_MPV_EVENT_PROPERTY_CHANGE {
                let prop = &*((*event).data as *mut mpv::mpv_event_property);
                let name = if prop.name.is_null() {
                    String::new()
                } else {
                    CStr::from_ptr(prop.name).to_string_lossy().into_owned()
                };
                let value = read_property_value(prop);
                // Label against our own table: header-backed names are backed by
                // a string literal in the installed headers; names we use but
                // which are not header symbols are labelled as observed-at-run-
                // time; anything else is 'foreign' (None).
                let header_backed =
                    crate::commands::lookup_property(&name).map(|named| named.is_header_backed());
                property = Some(Observed {
                    name,
                    value,
                    header_backed,
                });
            } else if event_id == mpv::mpv_event_id_MPV_EVENT_END_FILE {
                let end = &*((*event).data as *mut mpv::mpv_event_end_file);
                end_file = Some((EndReason::from_raw(end.reason), end.error));
            }
            Some(RawEvent {
                event_id,
                kind,
                error,
                reply_userdata,
                property,
                end_file,
            })
        }
    }

    fn took_wakeup(&mut self) -> bool {
        self.slot
            .as_ref()
            .map(|s| s.flag.swap(false, Ordering::SeqCst))
            .unwrap_or(false)
    }

    fn libmpv_version(&self) -> String {
        self.version.clone()
    }

    /// A real libmpv session queues controls until `MPV_EVENT_FILE_LOADED`.
    ///
    /// Why the conservative gate and not "send immediately": a display-free run
    /// in the headless test environment answered `MPV_ERROR_COMMAND` for controls issued early in
    /// an item run (the `--probe-json` report records it), and
    /// this repository has **not** established which lifecycle event is the
    /// earliest one that accepts a control (`docs/CAPABILITIES.md`). Sending
    /// immediately would reproduce that rejection in every real session; queueing
    /// until `FILE_LOADED` is a safe point, and any rejection that still happens
    /// is surfaced by [`crate::engine::PlayerSession::rejected_replies`] rather
    /// than hidden. A caller with better evidence can override it with
    /// [`crate::engine::PlayerSession::set_control_gate`].
    fn default_control_gate(&self) -> crate::engine::ControlGate {
        crate::engine::ControlGate::FileLoaded
    }

    fn is_closed(&self) -> bool {
        self.closed
    }
}

impl Drop for FfiPlayer {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: unregistering the callback first means libmpv cannot call
            // `wake` with our slot pointer after this point; mpv_destroy waits
            // for async work and stops delivering events (client.h:302-330).
            unsafe {
                mpv::mpv_set_wakeup_callback(self.handle, None, ptr::null_mut());
            }
            // A render context handed out by this player holds its own clone of
            // `self.core`; a count above 1 means at least one is still alive.
            if Arc::strong_count(&self.core) > 1 {
                // A render context made from this player is still alive, so the
                // core must not be destroyed yet (`render.h:122-123`). Its
                // `Drop` frees the render context first, and this `Arc` clone is
                // the last reference, so `CorePin::drop` destroys the core next.
                self.handle = ptr::null_mut();
            } else {
                // SAFETY: no other holder exists; this is the single destroy.
                unsafe { mpv::mpv_destroy(self.handle) };
                self.handle = ptr::null_mut();
                self.core.handle.store(ptr::null_mut(), Ordering::Release);
            }
        }
        if !self.slot_raw.is_null() {
            // SAFETY: this is the matching raw pointer for the `Arc::into_raw` in
            // `FfiPlayer::new`, and the callback that could dereference it was
            // just unregistered.
            unsafe {
                drop(Arc::from_raw(self.slot_raw as *const WakeSlot));
            }
            self.slot_raw = ptr::null_mut();
        }
        self.slot = None;
        self.closed = true;
    }
}

/// Read the value out of an `mpv_event_property`.
///
/// Every value is **copied** into owned Rust types; nothing libmpv allocated is
/// freed (see the MPV_FORMAT_STRING arm below for why `mpv_free` is wrong here).
///
/// # Safety
///
/// `prop` must come from a live `MPV_EVENT_PROPERTY_CHANGE` event delivered by
/// `mpv_wait_event` and must not be used after the next such call.
unsafe fn read_property_value(prop: &mpv::mpv_event_property) -> PropertyValue {
    match prop.format {
        mpv::mpv_format_MPV_FORMAT_FLAG => {
            if prop.data.is_null() {
                PropertyValue::None
            } else {
                PropertyValue::Flag(*(prop.data as *const c_int) != 0)
            }
        }
        mpv::mpv_format_MPV_FORMAT_INT64 => {
            if prop.data.is_null() {
                PropertyValue::None
            } else {
                PropertyValue::Int64(*(prop.data as *const i64))
            }
        }
        mpv::mpv_format_MPV_FORMAT_DOUBLE => {
            if prop.data.is_null() {
                PropertyValue::None
            } else {
                PropertyValue::Double(*(prop.data as *const f64))
            }
        }
        mpv::mpv_format_MPV_FORMAT_STRING | mpv::mpv_format_MPV_FORMAT_OSD_STRING => {
            if prop.data.is_null() {
                PropertyValue::None
            } else {
                // `data` points at a `char*` that **libmpv owns**: it lives in the
                // arena that backs this event, and the client copies it.
                //
                // We deliberately do **not** call `mpv_free` here. client.h:393-399
                // ("Call this only if it's explicitly documented as allowed. Calling
                // this on mpv memory not owned by the caller will lead to undefined
                // behavior") is the rule, and nothing in the property-observation
                // docs (client.h:1170-1214) or in `mpv_event_property` itself
                // (client.h:1390-1413, which only says how to *read* the value:
                // `char *value = *(char **)(event_property->data);`) transfers that
                // block to the client. `mpv_wait_event`'s own docs are explicit the
                // other way (client.h:1709-1714): "You must not write to the struct,
                // and all memory referenced by it will be automatically released by
                // the API on the next mpv_wait_event() call, or when the context is
                // destroyed."
                //
                // Measured with a small C program against mpv 2.5.0:
                // the same call sequence (observe 13 properties + async loadfile +
                // drain) with `mpv_free(*(char**)p->data)` aborts with
                //   mpv/ta/ta.c:291: ta_dbg_check_header: Assertion
                //     `h->canary == CANARY' failed
                // i.e. the block is inside mpv's talloc arena, not ours; without
                // the free the identical sequence exits 0, and with it the process
                // SIGSEGVs later inside `mpv_destroy` - which is what made
                // `--smoke-no-display` crash.
                let raw = *(prop.data as *mut *mut c_char);
                let text = if raw.is_null() {
                    String::new()
                } else {
                    // SAFETY: `raw` is the NUL-terminated string libmpv put in this
                    // event; the struct and everything it references stay valid
                    // until the next mpv_wait_event call / handle destruction
                    // (client.h:1709-1714), and we only copy it here.
                    CStr::from_ptr(raw).to_string_lossy().into_owned()
                };
                PropertyValue::Str(text)
            }
        }
        mpv::mpv_format_MPV_FORMAT_NONE => PropertyValue::None,
        // Nodes/byte arrays are not decoded (see docs/CAPABILITIES.md).
        _ => PropertyValue::None,
    }
}

/// `CString` conversion that turns a NUL in a name/argument into an error instead
/// of a truncated argument.
fn cstring(text: &str) -> Result<CString, PlayerError> {
    CString::new(text).map_err(|_| PlayerError::Rejected {
        call: "cstring".to_owned(),
        command: text.to_owned(),
        code: mpv::mpv_error_MPV_ERROR_INVALID_PARAMETER,
        name: crate::error_name(mpv::mpv_error_MPV_ERROR_INVALID_PARAMETER).to_owned(),
    })
}

fn check(code: c_int, call: &str, context: &str) -> Result<(), PlayerError> {
    if code == mpv::mpv_error_MPV_ERROR_SUCCESS {
        return Ok(());
    }
    Err(PlayerError::Rejected {
        call: call.to_owned(),
        command: context.to_owned(),
        code,
        name: crate::error_name(code).to_owned(),
    })
}

/// Re-export so the mock and tests can name header error codes without importing
/// the raw `ffi` module.
#[must_use]
pub fn mpv_error_code(name: &str) -> Option<i32> {
    crate::error_code_of(name)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests may unwrap; product code keeps these lints on"
)]
mod tests {
    use super::*;

    /// The ownership rule behind `render.h:122-123` ("You must free the context
    /// with `mpv_render_context_free()` before the mpv core is destroyed",
    /// `/usr/include/mpv/render.h:122-123`, re-measured this session).
    ///
    /// `crate::render_link::RenderContext` holds a clone of this `Arc<CorePin>`.
    /// While such a clone exists `FfiPlayer::drop` must hand the destroy to
    /// `CorePin::drop` instead of calling `mpv_destroy` itself. That is asserted
    /// here against a real libmpv core: `mpv_create` + `mpv_destroy` only - no
    /// `mpv_initialize`, no file, no display - so the assertion is about
    /// refcounting, not about playback. The end-to-end version (a real
    /// `MPV_RENDER_API_TYPE_SW` context created and freed before its core) is
    /// `crates/plex-player/tests/render_sw_session.rs`.
    #[test]
    fn a_live_render_clone_defers_mpv_destroy_to_corepin() {
        let player = FfiPlayer::new(None).expect("mpv_create must give a core");
        let held = Arc::clone(&player.core); // exactly what RenderContext holds
        assert_eq!(Arc::strong_count(&player.core), 2);
        drop(player);
        assert!(
            !held.handle.load(Ordering::Acquire).is_null(),
            "mpv_destroy ran while a render-context clone of the core was alive"
        );
        // The deferred destroy runs here, exactly once, in `CorePin::drop`.
        drop(held);
    }

    /// The app drops the render context on the GL thread and hands the core to a
    /// short-lived teardown thread, so `FfiPlayer` must be `Send` and
    /// `mpv_destroy` must work on a thread other than the one that created the
    /// core (`client.h:136`). Exercised against a real (uninitialized, empty)
    /// core: only the destroy moves threads here.
    #[test]
    fn a_core_can_be_destroyed_on_another_thread() {
        let player = FfiPlayer::new(None).expect("mpv_create must give a core");
        std::thread::spawn(move || drop(player))
            .join()
            .expect("mpv_destroy must not panic off the creating thread");
    }

    /// The other arm of `FfiPlayer::drop`: with no render context alive the core
    /// is destroyed immediately and the `CorePin` handle is nulled first, so the
    /// `!handle.is_null()` guard in `CorePin::drop` is what keeps that destroy
    /// single. The guard is exercised here with a pin whose handle is already
    /// null (a real handle would be destroyed by `FfiPlayer::drop` itself).
    #[test]
    fn corepin_drop_skips_the_destroy_when_the_handle_was_nulled() {
        let pin = Arc::new(CorePin::new(ptr::null_mut()));
        assert!(pin.handle.load(Ordering::Acquire).is_null());
        drop(pin); // must not call `mpv_destroy`; reaching this line is the point
    }
}
