//! The shell layer: the window, frame and live-session half of the desktop
//! client.
//!
//! Seven modules, seven jobs:
//!
//! * [`resources`] - the playback source the shell hands the player:
//!   [`CacheKey`](resources::CacheKey) (one media part of one item),
//!   [`Resolved`](resources::Resolved) (its source plus a credential-free
//!   label), and the redaction helpers that label is built from.
//! * [`window`] - the frame side that *drives* the player: the per-frame
//!   [`FramePump`](window::FramePump) order (player render -> texture upload ->
//!   egui overlay -> present), the frame source/sink contracts
//!   ([`FrameSource`](window::FrameSource), [`FrameSink`](window::FrameSink)) and
//!   their GL implementations ([`GlFrameSource`](window::GlFrameSource) over
//!   `MpvGlSession`).
//! * [`host`] - the window host, adapted near-verbatim from the `egui_glow`
//!   example `pure_glow.rs`: a real `winit` window with a glutin GL context, an
//!   `egui` overlay painted by `egui_glow`, and [`run_host`](host::run_host) /
//!   [`run_host_report`](host::run_host_report), which exit after a requested
//!   number of presented frames. [`run_host_video_frames`](host::run_host_video_frames)
//!   runs a [`FramePump`](window::FramePump) frame per window frame.
//! * [`drive`] - the bounded command driver the harness injects
//!   ([`Driver::from_env`](drive::Driver::from_env)).
//! * [`connect`] - which server the app opens on start: the remembered
//!   [`clientIdentifier`](connect::read_last_server), the only one, or the
//!   picker ([`auto_connect`](connect::auto_connect)).
//! * [`live`] - the live Plex session the app flow runs on
//!   ([`LiveClient`](live::LiveClient)): sign-in, servers, libraries, items and
//!   resolved playback sources.
//!
//! The player frame poll is bounded ([`FRAME_POLL_MS`](host::FRAME_POLL_MS)), so
//! one stalled frame cannot outlive a run's watchdog. The app's command and
//! event channels ([`watch`](crate::watch)) are plain unbounded `mpsc` pairs:
//! nothing in this layer caps a queue.

pub mod connect;
pub mod drive;
pub mod host;
pub mod live;
pub mod resources;
/// Linux only: the pump drives the surfaceless-EGL render session, which the
/// live window path does not use.
#[cfg(target_os = "linux")]
pub mod window;
