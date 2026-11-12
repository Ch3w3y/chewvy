//! Software-render harness: a **real** `mpv_render_context` on the installed libmpv
//! using the documented software backend (`MPV_RENDER_API_TYPE_SW`).
//!
//! This is a display-free runtime test: it creates a real mpv core, a real video
//! fixture (generated with the machine's own ffmpeg into gitignored
//! `CARGO_TARGET_TMPDIR`), a real SW render context, and then really calls
//! `mpv_render_context_update` / `get_info(NEXT_FRAME_INFO)` / `render` /
//! `report_swap` / `free` while hashing the pixels libmpv wrote.
//!
//! It is **not** the GL path (that lives in `crates/plex-app`) and it is **not**
//! a compositor-backed run. Hardware decode is therefore not measured and not claimed
//! anywhere here: everything runs through CPU decode + libswscale.
//!
//! Redraw triggering is callback-driven: the loop blocks on the waker channel
//! that libmpv's update callback feeds, and calls `take_update()` once per wake
//! (`render.h:634-661`) - there is no busy poll loop.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)] // product code keeps these lints on

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use plex_player::commands::{Named, Request, SourceLabel};
use plex_player::link::PlayerLink;
use plex_player::mpv_link::FfiPlayer;
use plex_player::render_link::{RenderApi, RenderContext, SwTarget, MPV_RENDER_UPDATE_FRAME};

/// Header citation for the names this harness sends to libmpv.
fn hdr(name: &'static str, line: u32) -> Named {
    Named {
        name,
        label: SourceLabel::Header {
            file: "client.h",
            line,
        },
    }
}

/// FNV-1a over the rendered buffer: cheap, dependency-free, and enough to prove
/// that libmpv really wrote pixels and that consecutive frames differ.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    hash
}

/// Peak RSS of this process from `/proc/self/status` (`VmHWM`), in KiB.
fn peak_rss_kib() -> u64 {
    let status = fs::read_to_string("/proc/self/status").unwrap_or_default();
    status
        .lines()
        .filter(|l| l.starts_with("VmHWM:"))
        .filter_map(|l| {
            l.split_whitespace()
                .nth(1)
                .and_then(|v| v.parse::<u64>().ok())
        })
        .next()
        .unwrap_or(0)
}

/// Run ffmpeg and fail the test (not panic) with its last stderr lines.
fn ffmpeg(args: &[&str], what: &str) -> Result<(), String> {
    let out = Command::new("ffmpeg")
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .args(args)
        .stderr(Stdio::piped())
        .stdout(Stdio::null())
        .output()
        .map_err(|e| format!("cannot run ffmpeg for {what}: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!(
            "ffmpeg failed for {what} (exit {}): {}",
            out.status
                .code()
                .map_or_else(|| "?".to_string(), |c| c.to_string()),
            err.lines().rev().take(3).collect::<Vec<_>>().join(" / ")
        ));
    }
    Ok(())
}

/// A 2 s, 320x240, 25 fps, video-only fixture in the named codec.
fn video_fixture(dir: &Path, name: &str, codec: &str, extra: &[&str]) -> Result<PathBuf, String> {
    let path = dir.join(name);
    let mut args: Vec<&str> = vec![
        "-y",
        "-f",
        "lavfi",
        "-i",
        "testsrc2=size=320x240:rate=25:duration=2",
        "-c:v",
        codec,
        "-pix_fmt",
        "yuv420p",
        "-an",
    ];
    args.extend(extra);
    args.push(path.to_str().ok_or("non-UTF-8 tmpdir")?);
    ffmpeg(&args, name)?;
    if path.metadata().map(|m| m.len()).unwrap_or(0) == 0 {
        return Err(format!("{name}: ffmpeg produced an empty file"));
    }
    Ok(path)
}

/// The subtitle-burned case: the same source, subtitles rasterized into the
/// pixels by ffmpeg's libass filter *before* mpv ever sees them.
fn subtitle_fixture(dir: &Path) -> Result<PathBuf, String> {
    let srt = dir.join("burn.srt");
    fs::write(
        &srt,
        "1\n00:00:00,200 --> 00:00:01,900\nSUBTITLE-BURN-CASE\n\n",
    )
    .map_err(|e| format!("cannot write burn.srt: {e}"))?;
    let path = dir.join("h264-burned.mkv");
    let srt_arg = format!("subtitles={}", srt.display());
    video_fixture(dir, "h264-burned.mkv", "libx264", &["-vf", &srt_arg])?;
    Ok(path)
}

/// The per-fixture measurement this harness reports.
#[derive(Debug)]
struct Run {
    fixture: &'static str,
    frames: usize,
    /// Render-call intervals in microseconds (block-for-target-time mode).
    intervals: Vec<u64>,
    /// Wall time spent inside `mpv_render_context_render`.
    render_us: u64,
    frame_hashes: BTreeSet<u64>,
    first_hash: u64,
    last_hash: u64,
    /// Largest byte seen in the last frame (0 => pure black buffer).
    max_byte: u8,
    /// Non-zero bytes in the last frame.
    lit_bytes: usize,
    callbacks: u64,
    peak_rss_kib: u64,
}

impl Run {
    fn pct(&self, p: u64) -> u64 {
        if self.intervals.is_empty() {
            return 0;
        }
        let mut v = self.intervals.clone();
        v.sort_unstable();
        let idx = ((v.len() as u64).saturating_sub(1)) * p / 100;
        v[usize::try_from(idx).unwrap_or(0)]
    }

    fn mean_us(&self) -> u64 {
        if self.intervals.is_empty() {
            return 0;
        }
        self.intervals.iter().sum::<u64>() / self.intervals.len() as u64
    }

    /// One machine-readable line per fixture, so the numbers can be pasted into
    /// a later evidence file without
    /// reinterpreting them.
    fn line(&self) -> String {
        format!(
            "MPV-SW fixture={} frames={} frames_per_s={:.2} mean_ms={:.2} p50_ms={:.2} p95_ms={:.2} max_ms={:.2} render_ms={:.2} distinct_frame_hashes={} first_hash={:016x} last_hash={:016x} lit_bytes={} max_byte={} update_callbacks={} peak_rss_kib={}",
            self.fixture,
            self.frames,
            if self.frames > 1 {
                (self.frames - 1) as f64 * 1e6
                    / self.intervals.iter().sum::<u64>().max(1) as f64
            } else {
                0.0
            },
            self.mean_us() as f64 / 1e3,
            self.pct(50) as f64 / 1e3,
            self.pct(95) as f64 / 1e3,
            self.intervals.iter().max().copied().unwrap_or(0) as f64 / 1e3,
            self.render_us as f64 / 1e3,
            self.frame_hashes.len(),
            self.first_hash,
            self.last_hash,
            self.lit_bytes,
            self.max_byte,
            self.callbacks,
            self.peak_rss_kib,
        )
    }
}

/// A resolver the SW backend never calls (no GL is involved).
struct NoGl;

impl plex_player::render_link::ProcResolver for NoGl {
    fn resolve(&self, _name: &std::ffi::CStr) -> *mut std::os::raw::c_void {
        std::ptr::null_mut()
    }
}

/// Drive one fixture through a real SW render context.
fn run_fixture(dir: &Path, name: &'static str, codec: &str, extra: &[&str]) -> Result<Run, String> {
    let path = video_fixture(dir, name, codec, extra)?;
    run_render(name, &path)
}

fn run_render(name: &'static str, path: &Path) -> Result<Run, String> {
    let (tx, rx) = mpsc::channel::<()>();
    let mut player = FfiPlayer::new(Some(tx.clone())).map_err(|e| format!("core: {e}"))?;
    // Options must be set before mpv_initialize (client.h `mpv_set_option`).
    //
    // `audio` and `hwdec` were accepted pre-init by the installed libmpv 2.5.0.
    // `vid` is **not**: this harness observed
    // `mpv_set_option_string("vid", "yes") -> -7 (MPV_ERROR_OPTION_ERROR)`, i.e.
    // libmpv treats `vid` (like `aid`/`sid`) as a per-playback option that cannot
    // be set through the pre-init option list. Video is selected by default and
    // the fixtures contain video streams only, so the harness does not need it and
    // does not pretend it was set. `sub` is left unset for the same reason: the
    // burned-in case has its subtitles rasterized into the pixels by ffmpeg before
    // mpv sees them, and no fixture carries a subtitle stream.
    player
        .set_option("audio", "no")
        .map_err(|e| format!("set_option audio: {e}"))?;
    player
        .set_option("hwdec", "no")
        .map_err(|e| format!("set_option hwdec: {e}"))?;
    // `vo=libmpv` is what makes the core route video to this render context.
    //
    // Without it the core builds its normal display VO chain (`gpu`, `gpu-next`,
    // `vdpau`, `xv`, `sdl`, `dmabuf-wayland`, `vaapi`, `x11`), every member of
    // that chain fails in the headless test environment, and the core then disables video:
    //   `cplayer/fatal: Error opening/initializing the selected video_out (--vo) device.`
    //   `cplayer/info: Video: no video`
    // and `mpv_render_context_update()` returns 0 forever (measured with
    // `--msg-level=all=v` and `PROP current-vo=(none)`). Creating the render context
    // before
    // `mpv_initialize` does **not** change that - it was measured both ways.
    //
    // With the option set, the core logs `VO: [libmpv] 320x240 yuv420p` /
    // `VO: Description: render API for libmpv` and `current-vo=libmpv`, and no
    // window, X11 display, Wayland display or DRM node is contacted.
    player
        .set_option("vo", "libmpv")
        .map_err(|e| format!("set_option vo: {e}"))?;
    player
        .initialize_link()
        .map_err(|e| format!("initialize: {e}"))?;

    let mut ctx = RenderContext::create(
        &player,
        RenderApi::Software,
        Box::new(NoGl),
        // The wake closure only schedules: it sends on a channel, it never calls
        // libmpv or GL (render.h:634-661).
        Some(Box::new(move || {
            let _ = tx.send(());
        })),
        true,
        // The SW backend ignores the native-display parameter.
        None,
    )
    .map_err(|e| format!("mpv_render_context_create(SW): {e}"))?;

    if !ctx.advanced_control() {
        return Err("advanced control was not recorded as enabled".to_string());
    }

    let source = path.display().to_string();
    player
        .request(
            1,
            &Request::Command {
                name: hdr("loadfile", 0),
                args: vec![source.clone(), "replace".to_string()],
            },
        )
        .map_err(|e| format!("loadfile {source}: {e}"))?;

    // 320x240 rgba8, 64-byte-aligned stride (render.h:392-399 hint).
    let (w, h) = (320i32, 240i32);
    let stride = (usize::try_from(w).map_err(|e| e.to_string())? * 4).div_ceil(64) * 64;
    let mut pixels = vec![0u8; stride * usize::try_from(h).map_err(|e| e.to_string())?];

    let mut run = Run {
        fixture: name,
        frames: 0,
        intervals: Vec::new(),
        render_us: 0,
        frame_hashes: BTreeSet::new(),
        first_hash: 0,
        last_hash: 0,
        max_byte: 0,
        lit_bytes: 0,
        callbacks: 0,
        peak_rss_kib: 0,
    };

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut last_render: Option<Instant> = None;
    let mut idle_waits = 0u32;

    while Instant::now() < deadline && run.frames < 60 {
        // Drain libmpv's event queue so replies/end-file are actually consumed.
        while player.next_event().is_some() {}

        let update = ctx.take_update().map_err(|e| format!("take_update: {e}"))?;
        run.callbacks = ctx.callback_count();
        if !update.has_frame() {
            if update.flags & !MPV_RENDER_UPDATE_FRAME != 0 {
                return Err(format!(
                    "unexpected update flags from libmpv: {:#x}",
                    update.flags
                ));
            }
            idle_waits = idle_waits.saturating_add(1);
            if idle_waits > 40 {
                break;
            }
            // Wait to be woken by the callback instead of polling. `recv_timeout`
            // is a blocking receive with a deadline (`std::sync::mpsc`), not a
            // `try_recv`: the timeout is only the upper bound on one wake.
            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(()) => {}
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            continue;
        }
        let info = ctx
            .next_frame_info()
            .map_err(|e| format!("next_frame_info: {e}"))?;
        if info.as_ref().is_some_and(|f| f.is_empty()) {
            continue;
        }
        let started = Instant::now();
        {
            let mut target = SwTarget {
                width: w,
                height: h,
                format: "rgb0",
                stride,
                pixels: &mut pixels,
            };
            ctx.render_sw(&mut target)
                .map_err(|e| format!("render_sw: {e}"))?;
        }
        let render_dt = started.elapsed();
        run.render_us += u64::try_from(render_dt.as_micros()).unwrap_or(u64::MAX);
        if let Some(prev) = last_render {
            run.intervals
                .push(u64::try_from(started.duration_since(prev).as_micros()).unwrap_or(0));
        }
        last_render = Some(started);
        ctx.report_swap().map_err(|e| format!("report_swap: {e}"))?;

        let hash = fnv1a(&pixels);
        run.frame_hashes.insert(hash);
        if run.frames == 0 {
            run.first_hash = hash;
        }
        run.last_hash = hash;
        run.max_byte = pixels.iter().copied().max().unwrap_or(0);
        run.lit_bytes = pixels.iter().filter(|b| **b != 0).count();
        run.frames += 1;
        idle_waits = 0;
    }

    run.peak_rss_kib = peak_rss_kib();
    // RenderContext::free runs in Drop, before the core is destroyed
    // (render.h:122-123).
    drop(ctx);
    drop(player);
    Ok(run)
}

/// Generate the three codec fixtures and drive each through the SW path.
fn three_codecs(dir: &Path) -> Result<Vec<Run>, String> {
    Ok(vec![
        run_fixture(dir, "m5-h264.mkv", "libx264", &["-preset", "ultrafast"])?,
        run_fixture(
            dir,
            "m5-hevc.mkv",
            "libx265",
            &["-preset", "ultrafast", "-x265-params", "log-level=none"],
        )?,
        run_fixture(
            dir,
            "m5-av1.mkv",
            "libsvtav1",
            &["-preset", "8", "-cpu-used", "4"],
        )?,
    ])
}

#[test]
fn real_libmpv_sw_render_produces_non_black_frames_for_three_codecs() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    fs::create_dir_all(&dir).expect("cannot create the fixture dir");

    let runs = match three_codecs(&dir) {
        Ok(v) => v,
        Err(e) => panic!("fixture/libmpv SW run failed: {e}"),
    };

    for r in &runs {
        println!("{}", r.line());
        assert!(
            r.frames >= 10,
            "{} produced only {} frames",
            r.fixture,
            r.frames
        );
        assert!(
            r.frame_hashes.len() > 1,
            "{} produced identical frames - nothing moved (hashes={})",
            r.fixture,
            r.frame_hashes.len()
        );
        assert!(
            r.max_byte > 32,
            "{} frame is black (max byte {})",
            r.fixture,
            r.max_byte
        );
        assert!(r.lit_bytes > 1000, "{} frame is empty", r.fixture);
        assert!(r.callbacks >= 1, "libmpv never called the update callback");
    }
    assert_eq!(runs.len(), 3, "expected h264, hevc and av1 runs");
}

#[test]
fn subtitle_burned_fixture_renders_differently_from_the_plain_case() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    fs::create_dir_all(&dir).expect("cannot create the fixture dir");
    let plain = match video_fixture(
        &dir,
        "m5-h264-subs-plain.mkv",
        "libx264",
        &["-preset", "ultrafast"],
    ) {
        Ok(p) => p,
        Err(e) => panic!("{e}"),
    };
    let burned = match subtitle_fixture(&dir) {
        Ok(p) => p,
        Err(e) => panic!("{e}"),
    };

    let a = match run_render("h264-plain", &plain) {
        Ok(r) => r,
        Err(e) => panic!("{e}"),
    };
    let b = match run_render("h264-subtitle-burned", &burned) {
        Ok(r) => r,
        Err(e) => panic!("{e}"),
    };
    println!("{}", a.line());
    println!("{}", b.line());

    assert_ne!(
        a.last_hash, b.last_hash,
        "the burned-in subtitle did not change any rendered pixel"
    );
    assert!(b.max_byte > 32, "subtitle-burned frame is black");
    assert!(
        b.frames >= 10,
        "subtitle-burned run produced {} frames",
        b.frames
    );
}

#[test]
fn sw_target_contract_is_refused_before_libmpv_is_asked_to_write() {
    let (tx, _rx) = mpsc::channel::<()>();
    let mut player = FfiPlayer::new(Some(tx)).expect("core");
    player.set_option("audio", "no").expect("set_option audio");
    player.set_option("hwdec", "no").expect("set_option hwdec");
    player.initialize_link().expect("initialize");
    let mut ctx = RenderContext::create(
        &player,
        RenderApi::Software,
        Box::new(NoGl),
        None,
        true,
        None,
    )
    .expect("SW render context");

    let mut short = vec![0u8; 320 * 4 * 10];
    let mut target = SwTarget {
        width: 320,
        height: 240,
        format: "rgb0",
        stride: 320 * 4,
        pixels: &mut short,
    };
    let err = ctx
        .render_sw(&mut target)
        .expect_err("a 10-row buffer must not be handed to libmpv as 240 rows");
    assert_eq!(err.code(), "unsupported_item", "guard did not fire: {err}");

    // A GL-target call on a SW context is refused too (SW params are backend-specific).
    let gl_only = plex_player::render_link::RenderOptions {
        fbo: Some(plex_player::render_link::GlFbo {
            fbo: 1,
            width: 320,
            height: 240,
            internal_format: 0,
        }),
        ..plex_player::render_link::RenderOptions::default()
    };
    let err2 = ctx
        .render(&gl_only)
        .expect_err("SW context must not accept a GL FBO");
    assert_eq!(err2.code(), "unsupported_item");

    // The documented SW formats are the only ones accepted.
    let mut ok = vec![0u8; 320 * 4 * 240];
    let mut good = SwTarget {
        width: 320,
        height: 240,
        format: "rgb0",
        stride: 320 * 4,
        pixels: &mut ok,
    };
    assert!(good.check().is_ok());
    good.format = "bgra";
    assert!(
        good.check().is_err(),
        "\"bgra\" is not one of the formats render.h:361-385 documents"
    );
}
