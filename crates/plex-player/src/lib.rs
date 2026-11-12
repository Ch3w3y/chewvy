//! `plex-player`: the playback engine layer.
//!
//! The engine is libmpv (client API 2.5 or newer) driven through a thin
//! bindgen-generated FFI, not an upstream mpv crate, with no `--wid` window
//! embedding.
//!
//! Layer contract (enforced by this module layout):
//!
//! * one `mpv_handle` per played item ([`engine::PlayerSession`]);
//! * async commands only: `mpv_command_async` / `mpv_set_property_async` with a
//!   `reply_userdata` token allocated by [`engine::TokenAllocator`] and correlated
//!   with `MPV_EVENT_COMMAND_REPLY` in [`engine::PendingReplies`];
//! * state comes only from observed properties ([`state::ObservedState`],
//!   `mpv_observe_property`), so UI code never peeks into mpv internals;
//! * progress is driven by libmpv's wakeup callback + `mpv_wait_event(h, 0)`
//!   drains ([`mpv_link::FfiPlayer`]) - never a blocking wait, never a busy poll;
//! * every libmpv call lives in [`mpv_link`]; [`commands`], [`events`], [`state`],
//!   [`controls`], [`tracks`] and [`engine`] are pure Rust and run against
//!   [`mock::MockLink`] in tests and in `chewvy-cli --smoke-no-display`.
//!
//! # unsafe policy
//!
//! The workspace denies `unsafe_code`. The only modules that use unsafe are the
//! generated bindings ([`ffi`]) and [`mpv_link`], each of which carries a scoped
//! `#[allow(unsafe_code)]` plus a `SAFETY:` note on every block. `cargo clippy
//! --all-targets -- -D warnings` is the gate that proves no other module does.
//!
//! Rendering into an app-owned FBO (OpenGL render API) is milestone M5/M6 work;
//! Is deliberately display-free.

#![warn(missing_docs)]

/// bindgen 0.72.1 libmpv client+render bindings (committed snapshot produced by
/// `build.rs`, regenerated only with `--features plex-player/regenerate-mpv-bindings`).
///
/// The `allow` attributes are deliberate and narrow to this generated module:
/// `unsafe_code` (bindgen emits `unsafe extern "C"` blocks), the C-derived
/// identifier styles, and `missing_docs` (the generated items carry the C header
/// comments as Rust doc comments only where the header has them).
#[cfg(feature = "libmpv-ffi")]
#[allow(
    unsafe_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    dead_code,
    missing_docs,
    clippy::all,
    clippy::pedantic,
    clippy::restriction,
    // bindgen 0.72.1 derives `PartialEq` for structs that hold function pointers
    // (e.g. `mpv_opengl_init_params.get_proc_address`), and rustc's
    // `unpredictable_function_pointer_comparisons` lint fires on that derive
    // under `-D warnings`. Silenced on this module only: an inner `#![allow]`
    // inside `include!` is not a permitted item, and src/mpv_bindings.rs must
    // stay byte-identical to `cargo build --features regenerate-mpv-bindings`.
    // Nothing in this workspace compares `PartialEq` on a struct that holds a
    // function pointer (render_link never compares mpv_opengl_init_params).
    unpredictable_function_pointer_comparisons
)]
pub mod ffi {
    include!("mpv_bindings.rs");
}

pub mod commands;
pub mod events;
pub mod link;
pub mod state;

#[cfg(feature = "libmpv-ffi")]
pub mod mpv_link;

pub mod controls;
pub mod engine;
pub mod mock;
pub mod tracks;

#[cfg(feature = "libmpv-ffi")]
pub mod probe;

/// The display-free smoke session needs the real probe, so it is gated with it.
#[cfg(feature = "libmpv-ffi")]
pub mod smoke;

/// The mpv render-context link: an `mpv_render_context` that
/// draws into an **app-owned** GL context/FBO, or into a caller-owned CPU buffer
/// via the documented `MPV_RENDER_API_TYPE_SW` last resort.
///
/// Everything in it requires a GL context current on the calling thread, so the
/// app (`crates/plex-app`, glow/glutin over winit's `rwh_06`) is the only
/// intended caller of [`render_link::RenderContext`]; the CPU path is usable
/// without a display.
#[cfg(feature = "libmpv-ffi")]
pub mod render_link;

pub use commands::{ItemSource, Named, PropertyValue, Request, SeekMode, SourceLabel, ValueKind};
pub use controls::{
    load_request, pause_request, seek_request, stop_request, volume_request, MAX_VOLUME,
};
pub use engine::{
    PendingReplies, PendingRequest, PlayerSession, PumpReport, PumpTotals, TokenAllocator,
    MAX_TRACKED_EVENTS, MAX_TRACKED_PUMP_REPORTS, NO_TOKEN, UNMATCHED_NO_REQUEST,
    UNMATCHED_NO_TOKEN,
};
pub use events::{
    error_name, is_success, EndReason, EventKind, MpvError, Observed, PlayerError, PlayerEvent,
};
pub use link::{PlayerLink, RawEvent, Scriptable};
pub use mock::MockLink;
pub use state::{observed_format, ObservedState};
pub use tracks::TrackSelection;

#[cfg(feature = "libmpv-ffi")]
pub use mpv_link::{build_time_version, FfiPlayer};

/// Error code the mock and the probe use for "no error".
///
/// With `libmpv-ffi` this **is** the header constant
/// `mpv_error_MPV_ERROR_SUCCESS` read out of the installed `client.h` by
/// bindgen, so mock replies and real replies share one numeric space.
/// Without the feature no header constant is compiled in and `0` is returned
/// as a plain convention - explicitly *not* header evidence in that build.
#[must_use]
pub fn mock_success_code() -> i32 {
    #[cfg(feature = "libmpv-ffi")]
    {
        ffi::mpv_error_MPV_ERROR_SUCCESS
    }
    #[cfg(not(feature = "libmpv-ffi"))]
    {
        0
    }
}

/// Header-declared `mpv_error` code by name, for the mock and tests (so they do
/// not import the raw [`ffi`] module).
///
/// Returns `None` for a name the installed headers do not declare, and always
/// `None` without the `libmpv-ffi` feature (there is no header to look in).
#[must_use]
pub fn mock_error_code(name: &str) -> Option<i32> {
    #[cfg(feature = "libmpv-ffi")]
    {
        error_code_of(name)
    }
    #[cfg(not(feature = "libmpv-ffi"))]
    {
        let _ = name;
        None
    }
}

/// Header-declared `mpv_error` code by name, resolved through the bindgen
/// constants (so the mock and tests never import the raw [`ffi`] module).
///
/// Returns `None` for a name the installed headers do not declare.
#[cfg(feature = "libmpv-ffi")]
#[must_use]
pub fn error_code_of(name: &str) -> Option<i32> {
    use ffi as mpv;
    let code = match name {
        "MPV_ERROR_SUCCESS" => mpv::mpv_error_MPV_ERROR_SUCCESS,
        "MPV_ERROR_EVENT_QUEUE_FULL" => mpv::mpv_error_MPV_ERROR_EVENT_QUEUE_FULL,
        "MPV_ERROR_NOMEM" => mpv::mpv_error_MPV_ERROR_NOMEM,
        "MPV_ERROR_UNINITIALIZED" => mpv::mpv_error_MPV_ERROR_UNINITIALIZED,
        "MPV_ERROR_INVALID_PARAMETER" => mpv::mpv_error_MPV_ERROR_INVALID_PARAMETER,
        "MPV_ERROR_OPTION_NOT_FOUND" => mpv::mpv_error_MPV_ERROR_OPTION_NOT_FOUND,
        "MPV_ERROR_OPTION_FORMAT" => mpv::mpv_error_MPV_ERROR_OPTION_FORMAT,
        "MPV_ERROR_OPTION_ERROR" => mpv::mpv_error_MPV_ERROR_OPTION_ERROR,
        "MPV_ERROR_PROPERTY_NOT_FOUND" => mpv::mpv_error_MPV_ERROR_PROPERTY_NOT_FOUND,
        "MPV_ERROR_PROPERTY_FORMAT" => mpv::mpv_error_MPV_ERROR_PROPERTY_FORMAT,
        "MPV_ERROR_PROPERTY_UNAVAILABLE" => mpv::mpv_error_MPV_ERROR_PROPERTY_UNAVAILABLE,
        "MPV_ERROR_PROPERTY_ERROR" => mpv::mpv_error_MPV_ERROR_PROPERTY_ERROR,
        "MPV_ERROR_COMMAND" => mpv::mpv_error_MPV_ERROR_COMMAND,
        "MPV_ERROR_LOADING_FAILED" => mpv::mpv_error_MPV_ERROR_LOADING_FAILED,
        "MPV_ERROR_AO_INIT_FAILED" => mpv::mpv_error_MPV_ERROR_AO_INIT_FAILED,
        "MPV_ERROR_VO_INIT_FAILED" => mpv::mpv_error_MPV_ERROR_VO_INIT_FAILED,
        "MPV_ERROR_NOTHING_TO_PLAY" => mpv::mpv_error_MPV_ERROR_NOTHING_TO_PLAY,
        "MPV_ERROR_UNKNOWN_FORMAT" => mpv::mpv_error_MPV_ERROR_UNKNOWN_FORMAT,
        "MPV_ERROR_UNSUPPORTED" => mpv::mpv_error_MPV_ERROR_UNSUPPORTED,
        "MPV_ERROR_NOT_IMPLEMENTED" => mpv::mpv_error_MPV_ERROR_NOT_IMPLEMENTED,
        "MPV_ERROR_GENERIC" => mpv::mpv_error_MPV_ERROR_GENERIC,
        _ => return None,
    };
    Some(code)
}
