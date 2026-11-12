//! `plex-app`: the Chewvy desktop shell (egui + glow/glutin + winit, no eframe/wgpu).
//!
//! egui 0.36.1 with egui-winit, egui_glow, glow 0.17.0, glutin 0.32.3 and
//! glutin-winit 0.5.0 on a single shared GL context per frame.
//!
//! Shell contract:
//!
//! * one window, one GL context; mpv renders video and its own libass subtitles
//!   into an app-owned FBO through the mpv OpenGL render API, egui draws the
//!   overlay controls on top of that texture;
//! * no `--wid`/reparenting embedding; `WL_DISPLAY` is used only for hardware
//!   decode interop, never for embedding the player surface;
//! * network and player work stay off the event-loop thread (channels in and out),
//!   so the UI stays responsive while polling or buffering;
//! * the only state this app persists is the Plex token
//!   (`plex_core::tokens::FileTokenStore`, `0600` under XDG config); egui's
//!   serde/accesskit persistence is not enabled (it needs the cache-absent
//!   `enumn` crate);
//! * the app flow is sign-in -> server select -> library browse -> play
//!   ([`watch`]). Tokens are never rendered into UI text.

#![warn(missing_docs)]

/// The shell layer: the resolved playback source and its redaction helpers
/// ([`shell::resources`]), the live Plex session
/// ([`shell::live`]), the driver injected by the harness ([`shell::drive`]) and
/// the window host with its per-frame pump ([`shell::host`], [`shell::window`]).
pub mod shell;

/// EGL/GL context ownership, the app-owned FBO/texture that mpv renders into, and
/// the GL procedure resolver handed to mpv (the Linux surfaceless path uses
/// `eglGetProcAddress`, the live window uses glutin's display).
pub mod gl;

/// The app-owned GL render session: `MpvGlSession` creates the libmpv OpenGL
/// render context against [`gl::SharedGl`] and renders into the app FBO
/// ([`gl::fbo::AppFbo`]), update-callback driven.
///
/// Linux only: this is the surfaceless-EGL harness path, not the live window.
#[cfg(target_os = "linux")]
pub mod render;

/// egui overlay: transport controls, track/subtitle selection, progress, error and
/// unsupported-codec surfaces, plus the upload of the app FBO texture into egui.
///
pub mod overlay;

/// The embedded player engine: libmpv rendering into the app window, and the
/// state the player controls draw from.
pub mod player;

/// The UI foundation: the dark token set applied to the `egui::Context` and the
/// bounded async artwork loader behind the browse rows.
pub mod ui;

/// What this install can play from: its `sources.json` registry of a Plex
/// account and the Jellyfin servers added to it, the token slots that go with
/// them, and the desk that owns both on the worker thread ([`sources::worker`]).
/// A token is never in the file, only in the keyed store behind [`plex_core::tokens`].
pub mod sources;

/// The state a user goes through to add one Jellyfin server: an address or a
/// discovered broadcast, then a six-digit code or a username and password. The
/// steps themselves run on the worker's side ([`sources::worker`]); this holds
/// only which step the panel is at and what it has been told so far.
pub mod add_source;

/// The one seam between the browse screens and the two kinds of media source:
/// where a row, an image or a request belongs — this app's Plex server or one
/// of its Jellyfin servers — and the ids that say so.
pub mod source;

/// Sign-in, browse and play: the end-to-end app flow.
pub mod watch;
