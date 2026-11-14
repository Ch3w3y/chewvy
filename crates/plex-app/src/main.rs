//! The `chewvy` desktop binary.
//!
//! With no arguments it runs the app ([`plex_app::watch`]). The flags below are
//! smoke modes, each reporting what it actually did:
//!
//! * `--smoke` - the **surfaceless llvmpipe composite** (Linux only): one EGL context, `MpvGlSession` renders a generated fixture
//!   frame into the app FBO, `egui` draws the video quad + overlay, `egui_glow`
//!   paints that geometry into a framebuffer object with the app video texture
//!   attached, and the result is read back. That readback is the measured
//!   **composite** (video + UI in one image).
//! * `--smoke-frames N` - the **video window host**
//!   (`plex_app::shell::host::run_host_video_frames`, Linux only: it pumps
//!   the same surfaceless player context): a real `winit` window sized to the
//!   player frame that plays the generated fixture through
//!   `shell::window::FramePump` into the window under the egui overlay, then
//!   closes after N presented frames.
//!   stdout is exactly one JSON line
//!   `{"frames":..,"wall_ms":..,"peak_rss_kib":..,"last_composite_hash":..}` and
//!   the host's own `RUN_RESULT`/`FRAME` lines go to stderr.
//! * `--smoke-label-frames N` - the **label-only** window host
//!   (`plex_app::shell::host::run_host_report`): a `winit` window with one
//!   `egui` label, presented N times. stdout stays exactly one JSON line
//!   `{"frames":<presented>,"wall_ms":<elapsed_ms>}`.
//!
//! No smoke mode signs in to an account or
//! prints a credential or token: the window UI is a fixed label written
//! in `plex_app::shell::host`, and `--smoke` plays only the locally generated
//! fixture built by `ensure_fixture`. On Windows and macOS the two EGL-backed
//! modes above say they need the Linux harness and exit with status 2; the
//! window modes and the no-argument app run everywhere.

use std::path::PathBuf;
use std::time::Instant;

#[cfg(target_os = "linux")]
use glow::HasContext;
#[cfg(target_os = "linux")]
use plex_app::gl::SharedGl;
#[cfg(target_os = "linux")]
use plex_app::overlay::VideoTexture;
#[cfg(target_os = "linux")]
use plex_app::render::{peak_rss_kib, MpvGlSession, SessionConfig};

/// The binary's own `log` sink: everything the shell host logs (`log::error!`
/// and `log::info!` in `plex_app::shell::host`) goes to **stderr**, so a failed
/// window/GL setup is visible in the harness log instead of being discarded.
/// `env_logger` is a dev-dependency of this crate, so the binary implements the
/// two traits `log` needs rather than taking on a runtime dependency.
struct StderrLogger;

impl log::Log for StderrLogger {
    fn enabled(&self, meta: &log::Metadata<'_>) -> bool {
        // zbus logs every D-Bus frame at info; only its warnings are useful.
        if meta.target().starts_with("zbus") && meta.level() > log::Level::Warn {
            return false;
        }
        meta.level() <= log::max_level()
    }

    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            eprintln!("LOG {} {}", record.level(), record.args());
        }
    }

    fn flush(&self) {}
}

static STDERR_LOGGER: StderrLogger = StderrLogger;

/// Install [`StderrLogger`]. Default level is `info`; `RUST_LOG=debug|trace|
/// info|warn|error|off` changes it (an unparsable value keeps `info`).
fn init_logging() {
    let level = std::env::var("RUST_LOG")
        .ok()
        .and_then(|v| v.trim().parse::<log::LevelFilter>().ok())
        .unwrap_or(log::LevelFilter::Info);
    // Only one logger may be installed per process; this binary logs once.
    let installed = log::set_logger(&STDERR_LOGGER).is_ok();
    log::set_max_level(level);
    if installed {
        log::debug!("stderr logger installed level={level}");
    }
}

/// Take the next argv slot (a flag's value) without indexing.
fn take_value(argv: &[String], i: &mut usize, flag: &str) -> Result<String, String> {
    *i = i.saturating_add(1);
    argv.get(*i)
        .map(|s| (*s).to_owned())
        .ok_or_else(|| format!("{flag} needs a value"))
}

/// Every CLI value this binary accepts, parsed without any third-party crate.
#[derive(Clone, Debug)]
struct Args {
    fixture: Option<PathBuf>,
    frames: u64,
    /// `Some(n)` selects the **video** window-host mode (`--smoke-frames n`).
    smoke_frames: Option<u32>,
    /// `Some(n)` selects the **label-only** mode (`--smoke-label-frames n`).
    smoke_label_frames: Option<u32>,
}

impl Args {
    fn parse(argv: &[String]) -> Result<Self, String> {
        let mut args = Self {
            fixture: None,
            frames: 5,
            smoke_label_frames: None,
            smoke_frames: None,
        };
        let mut i = 0usize;
        while let Some(raw) = argv.get(i) {
            let flag = raw.as_str();
            match flag {
                "--smoke" => args.smoke_frames = None,
                "--smoke-frames" => {
                    let raw = take_value(argv, &mut i, flag)?;
                    let n = raw
                        .parse::<u32>()
                        .map_err(|e| format!("--smoke-frames {raw}: {e}"))?;
                    if n == 0 {
                        return Err("--smoke-frames must be at least 1".to_owned());
                    }
                    args.smoke_frames = Some(n);
                }
                "--fixture" => args.fixture = Some(PathBuf::from(take_value(argv, &mut i, flag)?)),
                "--smoke-label-frames" => {
                    let raw = take_value(argv, &mut i, flag)?;
                    let n = raw
                        .parse::<u32>()
                        .map_err(|e| format!("--smoke-label-frames {raw}: {e}"))?;
                    if n == 0 {
                        return Err("--smoke-label-frames must be at least 1".to_owned());
                    }
                    args.smoke_label_frames = Some(n);
                }
                "--frames" => {
                    let raw = take_value(argv, &mut i, flag)?;
                    args.frames = raw
                        .parse::<u64>()
                        .map_err(|e| format!("--frames {raw}: {e}"))?;
                }
                "--help" | "-h" => return Err("usage".to_owned()),
                other => return Err(format!("unknown argument {other}")),
            }
            i = i.saturating_add(1);
        }
        if args.frames == 0 {
            return Err("--frames must be at least 1".to_owned());
        }
        Ok(args)
    }
}

fn usage() -> &'static str {
    "usage: chewvy [--smoke [--fixture PATH] [--frames N]] [--smoke-frames N] [--smoke-label-frames N]\n\
       \x20 --smoke               surfaceless EGL + llvmpipe composite measurement (default)\n\
       \x20 --smoke-frames N       video window host: fixture pumped into a real window, exit after N frames\n\
       \x20 --smoke-label-frames N label-only window host, exit after N presented frames\n\
       \x20 --fixture PATH    generated fixture path (default target/tmp/smoke-fixture.mkv)\n\
       \x20 --frames N        --smoke composite frame target\n\
       \x20 (--smoke and --smoke-frames need Linux's surfaceless EGL harness; other desktops print that and exit 2)\n"
}

/// Generate a small video-only fixture with the same ffmpeg recipe the harness uses (`crates/plex-app/tests/gl_harness.rs`), so the binary runs
/// against a legal, locally generated file and never against real media.
#[cfg(target_os = "linux")]
fn ensure_fixture(path: &std::path::Path) -> Result<(), String> {
    if path.is_file() && path.metadata().map_or(true, |m| m.len() > 1024) {
        return Ok(());
    }
    let out = std::process::Command::new("ffmpeg")
        .arg("-y")
        .arg("-f")
        .arg("lavfi")
        .arg("-i")
        .arg(
            "testsrc2=size=320x240:rate=25:duration=2,\
             hue=s=0.6,colorchannelmixer=rr=0.2:gg=0.7:bb=0.3",
        )
        .arg("-c:v")
        .arg("libx264")
        .arg("-pix_fmt")
        .arg("yuv420p")
        .arg("-preset")
        .arg("ultrafast")
        .arg(path)
        .output()
        .map_err(|e| format!("cannot run ffmpeg to create the fixture: {e}"))?;
    if !out.status.success() {
        let tail = String::from_utf8_lossy(&out.stderr);
        return Err(format!(
            "ffmpeg fixture generation failed (exit {}): {}",
            out.status.code().unwrap_or(-1),
            tail.lines().rev().take(4).collect::<Vec<_>>().join(" | ")
        ));
    }
    let len = path
        .metadata()
        .map_err(|e| format!("fixture stat: {e}"))?
        .len();
    println!("FIXTURE path={} bytes={len}", path.display());
    Ok(())
}

/// One measured `--smoke` run: player frame -> app FBO -> egui overlay ->
/// composite readback.
#[cfg(target_os = "linux")]
#[allow(
    unsafe_code,
    reason = "the composite framebuffer object is created/attached/bound through glow's FFI wrappers, the same pattern crates/plex-app/src/gl/fbo.rs:26 uses (module-level allow there); SAFETY notes at each block"
)]
fn run_smoke(args: &Args) -> Result<u64, String> {
    let started = Instant::now();
    let fixture = args.fixture.clone().unwrap_or_else(default_fixture);
    if let Some(parent) = fixture.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    ensure_fixture(&fixture)?;
    let fixture_str = fixture
        .to_str()
        .ok_or("fixture path is not valid UTF-8")?
        .to_owned();

    let shared = SharedGl::surfaceless().map_err(|e| format!("SharedGl::surfaceless: {e}"))?;
    println!(
        "SMOKE context=surfaceless-EGL recipe=EGL_PLATFORM_SURFACELESS_MESA+llvmpipe \
         frames_target={} fixture={} mode=player+UI",
        args.frames, fixture_str
    );

    let mut session =
        MpvGlSession::open(&shared, SessionConfig::default()).map_err(|e| e.to_string())?;
    let (fb_id, width, height) = {
        let fbo = session.fbo();
        let (w, h) = fbo.size();
        (fbo.target().fbo, w, h)
    };
    println!(
        "SMOKE app_fbo_handle={fb_id} app_fbo_size={width}x{height} fbo_status={:?}",
        session.fbo().status(shared.gl())
    );
    session.load(&fixture_str).map_err(|e| e.to_string())?;

    // The composite target: a framebuffer object of our own with the **app video
    // texture** attached, so egui paints on top of the very texture libmpv drew
    // into. `glow::Texture` is the handle `AppFbo::texture` hands out.
    let gl = shared.gl();
    let composite_fbo = unsafe {
        let fbo = gl
            .create_framebuffer()
            .map_err(|e| format!("glCreateFramebuffer: {e}"))?;
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        gl.framebuffer_texture_2d(
            glow::FRAMEBUFFER,
            glow::COLOR_ATTACHMENT0,
            glow::TEXTURE_2D,
            Some(session.fbo().texture()),
            0,
        );
        let complete = gl.check_framebuffer_status(glow::FRAMEBUFFER);
        gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        if complete != glow::FRAMEBUFFER_COMPLETE {
            return Err(format!("composite FBO incomplete: 0x{complete:04X}"));
        }
        fbo
    };

    let mut painter = egui_glow::Painter::new(shared.gl_arc(), "", None, false)
        .map_err(|e| format!("egui_glow::Painter::new: {e}"))?;
    let egui_ctx = egui::Context::default();
    plex_app::ui::theme::apply(&egui_ctx);
    let mut video: Option<VideoTexture> = None;

    let mut last_composite_hash = 0u64;
    let mut frames_ok = 0u64;
    for n in 1..=args.frames {
        let t_render = Instant::now();
        let step = session
            .step_frame(std::time::Duration::from_millis(50))
            .map_err(|e| e.to_string())?;
        let gate = step.gate_line();
        let (captured, rgba) = (step.summary, step.pixels);
        println!("{gate}");
        let render_us = t_render.elapsed().as_micros() as u64;

        let t_upload = Instant::now();
        let size = [width as usize, height as usize];
        let tex = match video.as_mut() {
            Some(tex) => {
                tex.update(&rgba).map_err(|e| e.to_string())?;
                tex
            }
            None => {
                let tex = VideoTexture::load(&egui_ctx, size, &rgba).map_err(|e| e.to_string())?;
                video = Some(tex);
                video
                    .as_mut()
                    .ok_or_else(|| "video texture missing after load".to_owned())?
            }
        };
        let uploads = tex.uploads();
        let upload_us = t_upload.elapsed().as_micros() as u64;

        let t_overlay = Instant::now();
        let out = egui_ctx.run_ui(egui::RawInput::default(), |ui| {
            tex.paint_at(ui, ui.max_rect());
            ui.label("Chewvy smoke frame: video quad + overlay panel");
            ui.horizontal(|ui| {
                ui.label("player");
                ui.label("overlay");
            });
        });
        let clipped = egui_ctx.tessellate(out.shapes, out.pixels_per_point);
        let mut deltas = out.textures_delta;
        let overlay_us = t_overlay.elapsed().as_micros() as u64;

        let t_present = Instant::now();
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(composite_fbo));
            gl.viewport(0, 0, width, height);
        }
        painter.clear([width as u32, height as u32], [0.0, 0.0, 0.0, 1.0]);
        painter.paint_and_update_textures(
            [width as u32, height as u32],
            out.pixels_per_point,
            &clipped,
            &mut deltas,
        );
        let pixels = session
            .fbo()
            .read_rgba(gl)
            .map_err(|e| format!("composite readback: {e}"))?;
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        }
        let present_us = t_present.elapsed().as_micros() as u64;
        let composite = plex_app::gl::fbo::CapturedFrame::summarize(&pixels);
        last_composite_hash = composite.hash;
        frames_ok += 1;
        println!(
            "FRAME n={n} video_hash={:016x} video_non_black={} composite_hash={:016x} \
             composite_non_black={} overlay_visible={} uploads={} render_us={} upload_us={} \
             overlay_us={} present_us={} steps=[Render,Upload,Overlay,Present]",
            captured.hash,
            captured.non_black,
            composite.hash,
            composite.non_black,
            composite.hash != captured.hash,
            uploads,
            render_us,
            upload_us,
            overlay_us,
            present_us,
        );
    }

    let total_ms = started.elapsed().as_millis();
    let rss = peak_rss_kib();
    println!(
        "SMOKE_RESULT frames_ok={frames_ok} frames_target={} composite_hash={:016x} \
         wall_ms={} peak_rss_kib={} last_frame_ok=true mode=player+UI copy_path=rgba_readback_to_egui_texture",
        args.frames,
        last_composite_hash,
        total_ms,
        rss
    );
    unsafe {
        gl.delete_framebuffer(composite_fbo);
    }
    painter.destroy();
    drop(session);
    Ok(rss)
}

#[cfg(target_os = "linux")]
fn default_fixture() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("target/tmp/smoke-fixture.mkv")
}

/// The fixture path both window/video modes hand to `loadfile` as an **absolute**
/// path, created if missing.
#[cfg(target_os = "linux")]
fn staged_fixture(args: &Args) -> Result<String, String> {
    let fixture = args.fixture.clone().unwrap_or_else(default_fixture);
    if let Some(parent) = fixture.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    ensure_fixture(&fixture)?;
    let absolute = fixture
        .canonicalize()
        .map_err(|e| format!("fixture canonicalize {}: {e}", fixture.display()))?;
    Ok(absolute
        .to_str()
        .ok_or("fixture path is not valid UTF-8")?
        .to_owned())
}

/// `--smoke-frames N`: the video host - play the fixture in a real window and
/// pump N frames into it.
///
/// Like [`run_window_host`], whether the headless test environment can create a window at all is
/// measured, not assumed: any `winit`/`glutin`/player failure is printed to
/// **stderr** and exits non-zero, while stdout carries exactly one JSON line.
#[cfg(target_os = "linux")]
fn run_window_video_host(frames: u32, fixture: &str) -> Result<(), String> {
    eprintln!(
        "START mode=window-video host_title={} target_frames={} deadline_ms={} fixture={}",
        plex_app::shell::host::HOST_VIDEO_WINDOW_TITLE,
        frames,
        plex_app::shell::host::watchdog_ms(),
        fixture,
    );
    let report = plex_app::shell::host::run_host_video_frames(fixture, frames)
        .map_err(|e| format!("video window host: {e}"))?;
    let json = serde_json::json!({
        "mode": "window-video",
        "frames": report.frames_presented,
        "pump_frames": report.pump_frames,
        "sink_paints": report.sink_paints,
        "frame": format!("{}x{}", report.frame[0], report.frame[1]),
        "surface": format!("{}x{}", report.surface[0], report.surface[1]),
        "surface_matches_frame": report.surface_matches_frame,
        "composite_non_black_frames": report.composite_non_black_frames,
        "last_composite_hash": report
            .composite_hashes
            .last()
            .map_or_else(|| "none".to_owned(), |h| format!("{h:016x}")),
        "last_video_hash": report
            .video_hashes
            .last()
            .map_or_else(|| "none".to_owned(), |h| format!("{h:016x}")),
        "swap_interval_requested": report.swap_interval,
        "wall_ms": report.wall_ms,
        "peak_rss_kib": report.peak_rss_kib,
        "frame_errors": report.frame_errors.len(),
    });
    println!("{json}");
    // A run that presented every frame but composited nothing non-black never
    // put the player frame in the window: that is a failed M6d2 run, not a pass.
    if report.frames_presented < frames
        || !report.frame_errors.is_empty()
        || report.composite_non_black_frames == 0
    {
        return Err(format!(
            "video window host presented {} of {frames} frames (frame_errors={}, composite_non_black_frames={})",
            report.frames_presented,
            report.frame_errors.len(),
            report.composite_non_black_frames,
        ));
    }
    Ok(())
}

/// `--smoke-label-frames N`: the label-only window host (`plex_app::shell::host`) in a real
/// window, bounded to N presented frames.
///
/// Whether the headless test environment can create a window at all is measured, not assumed: any
/// `winit`/`glutin` failure is reported by the host and printed to **stderr**
/// with a non-zero exit, while stdout carries exactly one JSON line.
fn run_window_host(frames: u32) -> Result<u32, String> {
    let started = Instant::now();
    let pacing = plex_app::shell::host::swap_interval_label(
        plex_app::shell::host::swap_interval_policy(Some(frames)),
    );
    let watchdog_ms = plex_app::shell::host::watchdog_ms();
    // Stage marker on stderr (stdout stays exactly one JSON line): a run that
    // produces no JSON can still be attributed to this call.
    eprintln!(
        "START mode=window host_title={} target_frames={} deadline_ms={} swap_interval_requested={}",
        plex_app::shell::host::HOST_WINDOW_TITLE,
        frames,
        watchdog_ms,
        pacing,
    );
    let presented = plex_app::shell::host::run_host_report(Some(frames))
        .map_err(|e| format!("window host: {e}"))?;
    let wall_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    // `swap_interval_requested` is what this host asked the driver for; a driver
    // that rejects it is reported by the host's own WARN line.
    let report = serde_json::json!({
        "frames": presented,
        "wall_ms": wall_ms,
        "swap_interval_requested": pacing,
        "watchdog_ms": watchdog_ms,
    });
    println!("{report}");
    Ok(presented)
}

fn main() {
    init_logging();
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.is_empty() {
        plex_app::update::tidy();
        if let Err(e) = plex_app::watch::run() {
            eprintln!("chewvy: {e}");
            std::process::exit(1);
        }
        return;
    }
    if argv.first().map(String::as_str) == Some("--capture") {
        let path = argv.get(1).map_or("/", String::as_str);
        let query = argv.get(2).map_or("", String::as_str);
        if let Err(e) = plex_app::watch::capture(path, query) {
            eprintln!("chewvy: {e}");
            std::process::exit(1);
        }
        return;
    }
    if argv.first().map(String::as_str) == Some("--diagnose-servers") {
        if let Err(e) = plex_app::watch::diagnose_servers() {
            eprintln!("chewvy: {e}");
            std::process::exit(1);
        }
        return;
    }
    let args = match Args::parse(&argv) {
        Ok(a) => a,
        Err(e) => {
            if e == "usage" {
                println!("{}", usage());
                std::process::exit(0);
            }
            eprintln!("ARGS error={e}\n{}", usage());
            std::process::exit(2);
        }
    };

    if let Some(n) = args.smoke_label_frames {
        match run_window_host(n) {
            Ok(presented) if presented >= n => return,
            Ok(presented) => {
                eprintln!("RUN_FAILURE label window host presented {presented} of {n} frames");
                std::process::exit(1);
            }
            Err(e) => {
                eprintln!("RUN_FAILURE {e}");
                std::process::exit(1);
            }
        }
    }

    match args.smoke_frames {
        // `--smoke` and `--smoke-frames` both measure the surfaceless player
        // context, which is the Linux-only EGL harness.
        #[cfg(target_os = "linux")]
        Some(n) => {
            let fixture = match staged_fixture(&args) {
                Ok(f) => f,
                Err(e) => {
                    eprintln!("RUN_FAILURE {e}");
                    std::process::exit(1);
                }
            };
            if let Err(e) = run_window_video_host(n, &fixture) {
                eprintln!("RUN_FAILURE {e}");
                std::process::exit(1);
            }
        }
        #[cfg(target_os = "linux")]
        None => match run_smoke(&args) {
            Ok(_) => println!("RUN exit=0"),
            Err(e) => {
                println!("RUN_FAILURE {e}");
                std::process::exit(1);
            }
        },
        #[cfg(not(target_os = "linux"))]
        _ => {
            eprintln!(
                "RUN_FAILURE mode={} needs the Linux surfaceless-EGL harness: not supported \
                 on this platform (fixture={:?} frames={})",
                match args.smoke_frames {
                    Some(n) => format!("--smoke-frames {n}"),
                    None => "--smoke".to_owned(),
                },
                args.fixture,
                args.frames,
            );
            std::process::exit(2);
        }
    }
}
