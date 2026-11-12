//! Software-GL harness: the **real** GL render path, headless, on software GL.
//!
//! What this is: a runtime test that creates a real EGL surfaceless context
//! (`EGL_PLATFORM=surfaceless`, Mesa llvmpipe), a real libmpv core, a real
//! `MPV_RENDER_API_TYPE_OPENGL` render context against that core with our
//! `eglGetProcAddress` resolver, an app-owned RGBA8 FBO, and then really
//! renders h264 / hevc / av1 fixtures into it while hashing the pixels we read
//! back out of our own FBO. It is run by `scripts/mpv-gl-harness.sh`, which
//! exports the software-GL recipe.
//!
//! What this is **not**:
//!
//! * not a hardware-decode run. The recipe forces `LIBGL_ALWAYS_SOFTWARE=1`; `SessionConfig::hwdec` stays
//!   `"no"`. Hardware decode is neither measured nor claimed here.
//! * not the windowed desktop path: window creation (glutin/winit) needs a real
//!   compositor.
//!
//! ## Gate semantics
//!
//! Without `PLEX_GL_HARNESS` every test here prints a machine-readable
//! `SKIP test=... gate_set=false` line and returns **before touching EGL or
//! libmpv**: `cargo test --all-targets` must not depend on the machine's EGL
//! state, and must not pay for llvmpipe shader compilation. With
//! the gate set, a failure to create the context or to render frames is a hard
//! test failure, so the harness log records the real error instead of a silent
//! skip. (The in-lib GL unit tests in `crate::gl` keep the opposite rule - they
//! try EGL even un-gated, because they are cheap.)
#![cfg(target_os = "linux")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "harness/tests may unwrap; product code keeps these lints on"
)]

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use plex_app::gl::SharedGl;
use plex_app::render::{FrameLoopReport, MpvGlSession, SessionConfig};
use plex_player::tracks::TrackSelection;

/// The environment gate `scripts/mpv-gl-harness.sh` exports.
const GATE: &str = "PLEX_GL_HARNESS";

/// Fixture geometry - the same 320x240 the SW harness measures.
const W: i32 = 320;
const H: i32 = 240;
/// Frames asked for per run (the fixtures are 2 s @ 25 fps = 50 frames).
const MAX_FRAMES: usize = 50;
/// Upper bound on one frame loop; llvmpipe compiles its shaders on first use.
const DEADLINE: Duration = Duration::from_secs(30);
/// Media frame period of every fixture used here: 25 fps = 40 ms.
const FRAME_PERIOD_MS: f64 = 40.0;
/// The subtitle window of `m5gl-drawcase.srt` (00:00:00.200 -> 00:00:01.000),
/// in seconds of media time.
///
/// Why the window ends at 1.0 s and not 1.8 s (an earlier revision used 1.8 s):
/// the fixture is 2 s long, but a run does not always display all 50 of its
/// frames - `MpvGlSession::drive` stops at `max_frames`, at the deadline, or
/// after 40 consecutive no-frame updates, and harness runs have shown the
/// subtitle-free control run stopping after 30 rendered frames
/// (`frames=30 idle_waits=41`, cause **[U]**).
/// A window ending beyond what a run actually covers makes the case fail on
/// harness behaviour instead of producing a measurement, so the event is placed
/// where every accepted run (>= 25 frames = 1.0 s of media time) necessarily
/// displayed at least one frame inside it; a 1.0 s event is 20 frame periods
/// wide, so a run that dropped some frames still showed it.
const SUB_WINDOW_START_S: f64 = 0.2;
const SUB_WINDOW_END_S: f64 = 1.0;

/// Mean start-to-start frame gap in milliseconds (measured display pacing).
fn pacing_mean_ms(r: &FrameLoopReport) -> f64 {
    if r.frame_gap_us.is_empty() {
        return 0.0;
    }
    r.frame_gap_us.iter().sum::<u64>() as f64 / r.frame_gap_us.len() as f64 / 1e3
}

/// Media seconds a run covered, from its frame count at the fixtures' frame
/// period.
///
/// This is the coverage basis the harness uses, and it is a *derivation from a
/// measurement*, not an assertion: with `block_for_target_time=true`,
/// `mpv_render_context_render` blocks until the frame's display target time, so
/// a run whose measured start-to-start pacing matches the fixture's 40 ms frame
/// period displayed media at the fixture's rate. `target_time_us` itself is
/// **not** used for coverage: on this host (mpv v0.41.0-dirty) the reported
/// target-time deltas are ~1000x the measured pacing gap (a 2 s fixture spans
/// `target_time_span_s=1960.085` for 50 frames), cause `[U]`.
fn media_covered_s(r: &FrameLoopReport) -> f64 {
    f64::from(r.frames as u32) * FRAME_PERIOD_MS / 1e3
}

/// Return the harness GL context, or skip the test when the gate is not set.
///
/// Un-gated: skip *before* creating any EGL context (see module docs).
/// Gated: a context failure is a panic that carries the real `GlError` text.
fn harness_gl(test: &str) -> Option<SharedGl> {
    let gated = env::var_os(GATE).is_some();
    if !gated {
        println!("SKIP test={test} gate={GATE} gate_set=false reason=harness-gate-not-set");
        return None;
    }
    match SharedGl::surfaceless() {
        Ok(gl) => {
            let s = gl.gl_strings();
            let e = gl.egl_info();
            println!(
                "GLCONTEXT test={test} gl_version={} gl_renderer={} glsl={} egl_vendor={} egl_version={} surfaceless=true",
                s.version, s.renderer, s.glsl, e.vendor, e.version
            );
            Some(gl)
        }
        Err(err) => panic!("GL harness gate is set but no EGL context was created: {err}"),
    }
}

/// Run ffmpeg and report its last stderr lines instead of panicking opaque.
fn ffmpeg(args: &[&str], what: &str) -> Result<(), String> {
    let out = Command::new("ffmpeg")
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
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

fn fixture_size(path: &Path) -> u64 {
    path.metadata().map(|m| m.len()).unwrap_or(0)
}

/// A 2 s, 320x240, 25 fps, video-only Matroska fixture.
fn video_fixture(
    dir: &Path,
    name: &str,
    lavfi: &str,
    codec: &str,
    extra: &[&str],
) -> Result<PathBuf, String> {
    let path = dir.join(name);
    let dst = path.to_str().ok_or("non-UTF-8 fixture path")?.to_string();
    let mut args: Vec<&str> = vec![
        "-y", "-f", "lavfi", "-i", lavfi, "-c:v", codec, "-pix_fmt", "yuv420p", "-an",
    ];
    args.extend(extra);
    args.push(dst.as_str());
    ffmpeg(&args, name)?;
    if fixture_size(&path) < 1024 {
        return Err(format!(
            "{name}: ffmpeg produced {} bytes",
            fixture_size(&path)
        ));
    }
    println!(
        "FIXTURE name={name} codec={codec} bytes={} path={}",
        fixture_size(&path),
        path.display()
    );
    Ok(path)
}

/// The static-content + embedded-subtitle fixture used by the ownership case:
/// a *mostly black* video with one bright box (so every undecorated frame has
/// the same, deliberately **unsaturated** lit-pixel count) and one Matroska
/// `subrip` track whose text lives only in the subtitle layer.
///
/// Why the base is not a solid colour: an earlier revision of this fixture was
/// `color=c=0x3f7f3f` (whole frame lit), so `max_non_black` was already
/// 76800/76800 in every case and `max_non_black_delta` was pinned at 0 - the
/// lit-pixel discriminator could not respond to drawn glyphs at all. The base here lights a 240x120 box (~37% of the frame),
/// leaving ~48000 pixels of headroom for any glyph layer to show up in.
fn subtitle_fixture(dir: &Path) -> Result<(PathBuf, PathBuf), String> {
    let video = video_fixture(
        dir,
        "m5gl-static-base.mkv",
        "color=c=black:size=320x240:rate=25:duration=2,\
         drawbox=x=40:y=60:w=240:h=120:color=white:t=fill",
        "libx264",
        &["-preset", "ultrafast"],
    )?;
    let srt = dir.join("m5gl-drawcase.srt");
    fs::write(
        &srt,
        "1\n00:00:00.200 --> 00:00:01.000\nLIBASS SUBTITLE\nDRAW CASE\n\n",
    )
    .map_err(|e| format!("cannot write {}: {e}", srt.display()))?;
    let path = dir.join("m5gl-static-srt.mkv");
    ffmpeg(
        &[
            "-y",
            "-i",
            video.to_str().ok_or("non-UTF-8 path")?,
            "-f",
            "srt",
            "-i",
            srt.to_str().ok_or("non-UTF-8 path")?,
            "-map",
            "0:v",
            // The subrip stream comes from the **second** input (the .srt), so
            // it is `1:s`. An earlier M5b session observed `-map 0:s` failing on
            // this host with "Failed to set value '0:s' for option 'map':
            // Invalid argument" (exit 234) because the base video has no
            // subtitle stream; that session's intermediate log was **not
            // retained**, so it is quoted as history, not as evidence in this
            // tree. What the retained harness logs do show is `1:s` succeeding:
            // `FIXTURE name=m5gl-static-srt.mkv codec=libx264+subrip` plus
            // `PROBE subtitle ...;codec_name=subrip;codec_type=subtitle`.
            "-map",
            "1:s",
            "-c",
            "copy",
            "-c:s",
            "srt",
            path.to_str().ok_or("non-UTF-8 path")?,
        ],
        "m5gl-static-srt.mkv mux",
    )?;
    if fixture_size(&path) < 1024 {
        return Err(format!(
            "subtitle fixture mux produced {} bytes",
            fixture_size(&path)
        ));
    }
    println!(
        "FIXTURE name=m5gl-static-srt.mkv codec=libx264+subrip bytes={} path={} control={}",
        fixture_size(&path),
        path.display(),
        video.display()
    );
    // Both paths are returned: the subtitle-free variant of the *same* video
    // stream is the control the ownership comparison needs, and it does not
    // depend on libmpv accepting a `sub-id` selection (the rerun log shows this
    // host's libmpv accepting it - see `run_case`).
    Ok((path, video))
}

/// ffprobe the fixture's streams so the harness states what it actually ran.
fn ffprobe_streams(path: &Path) -> String {
    let out = Command::new("ffprobe")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-show_entries",
            "stream=codec_name,codec_type",
            "-of",
            "default=noprint_wrappers=1:nokey=0",
        ])
        .arg(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();
    match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
            .lines()
            .map(|l| l.trim().replace(' ', "_"))
            .collect::<Vec<_>>()
            .join(";"),
        Ok(o) => format!(
            "ffprobe-failed(exit {}) {}",
            o.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&o.stderr)
                .lines()
                .next()
                .unwrap_or("")
        ),
        Err(e) => format!("ffprobe-unavailable {e}"),
    }
}

/// Drive one fixture through the real GL session and print its report line.
///
/// Returns the report plus, when a subtitle selection was requested, a
/// measurement note saying whether libmpv accepted it. A rejected selection is
/// **recorded**, not fatal: earlier captured libmpv runs in this repository
/// rejected `set sub-id = ...` with `MPV_ERROR_PROPERTY_NOT_FOUND` (cause
/// `[U]`, `docs/CAPABILITIES.md`), and a harness that aborted on
/// that rejection would produce no measurement at all for the subtitle case.
/// Measured in the retained GL run of 2026-11-05T16:07Z the request *is*
/// accepted (`SUBCONTROL ... outcome=accepted sub_id=no|auto`), so the earlier
/// rejection is history, not a current constraint of this host's libmpv.
fn run_case(
    gl: &SharedGl,
    label: &str,
    path: &Path,
    subtitle: Option<&TrackSelection>,
    osd_level: &'static str,
) -> Result<(FrameLoopReport, Option<String>), String> {
    let cfg = SessionConfig {
        width: W,
        height: H,
        max_frames: MAX_FRAMES,
        deadline: DEADLINE,
        osd_level,
        ..SessionConfig::default()
    };
    // `open` takes the config by value, so the measured settings are named here
    // before it moves (a run must always state the knobs it ran with).
    let (hwdec, audio, advanced_control, osd_level) =
        (cfg.hwdec, cfg.audio, cfg.advanced_control, cfg.osd_level);
    let mut session = MpvGlSession::open(gl, cfg).map_err(|e| format!("open: {e}"))?;
    println!(
        "GLSESSION fixture={label} api={:?} advanced_control={} hwdec={} audio={} osd_level={} block_for_target_time=true fbo={}x{} sub_id={}",
        session.render_context().api(),
        advanced_control,
        hwdec,
        audio,
        osd_level,
        W,
        H,
        subtitle.map_or("untouched".to_string(), TrackSelection::value),
    );
    session
        .load(&path.display().to_string())
        .map_err(|e| format!("loadfile {}: {e}", path.display()))?;
    let sub_note = subtitle.map(|sel| match session.apply_subtitle(sel, 2) {
        Ok(()) => format!("accepted sub_id={}", sel.value()),
        Err(e) => format!("REJECTED sub_id={} error={e}", sel.value()),
    });
    println!(
        "SUBCONTROL case={label} selection={} outcome={}",
        subtitle.map_or("none-requested".to_string(), TrackSelection::value),
        sub_note
            .clone()
            .unwrap_or_else(|| "no selection request issued (libmpv load-time default)".to_string()),
    );
    let report = session.drive(label).map_err(|e| format!("drive: {e}"))?;
    println!("{}", report.report_line());
    Ok((report, sub_note))
}

/// Assertions every codec run must satisfy, phrased against the report only.
fn assert_rendered(report: &FrameLoopReport) {
    assert!(
        report.frames >= 10,
        "{} produced {} frames",
        report.fixture,
        report.frames
    );
    assert!(
        report.distinct_hashes.len() > 1,
        "{} produced {} distinct frame hashes - nothing moved",
        report.fixture,
        report.distinct_hashes.len()
    );
    let last = report
        .last_frame
        .as_ref()
        .unwrap_or_else(|| panic!("{} produced no captured frame", report.fixture));
    assert!(
        last.non_black > 1000,
        "{} last frame is (near) black: {}",
        report.fixture,
        last.report_line()
    );
    assert!(
        report.max_non_black > last.non_black / 4,
        "{}: max non-black {} is implausibly close to the last frame {}",
        report.fixture,
        report.max_non_black,
        last.non_black
    );
    assert!(
        report.update_callbacks >= 1,
        "libmpv never called the update callback"
    );
    assert!(report.advanced_control, "advanced control was not recorded");
    assert!(report.peak_rss_kib > 0, "peak RSS was not readable");
}

#[test]
fn gl_h264_frame_loop() {
    let Some(gl) = harness_gl("gl_h264_frame_loop") else {
        return;
    };
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    fs::create_dir_all(&dir).expect("fixture dir");
    let path = video_fixture(
        &dir,
        "m5gl-h264.mkv",
        "testsrc2=size=320x240:rate=25:duration=2",
        "libx264",
        &["-preset", "ultrafast"],
    )
    .unwrap_or_else(|e| panic!("{e}"));
    println!("PROBE h264 {}", ffprobe_streams(&path));
    let (report, _) =
        run_case(&gl, "h264", &path, None, "0").unwrap_or_else(|e| panic!("h264 run failed: {e}"));
    assert_rendered(&report);
}

#[test]
fn gl_hevc_frame_loop() {
    let Some(gl) = harness_gl("gl_hevc_frame_loop") else {
        return;
    };
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    fs::create_dir_all(&dir).expect("fixture dir");
    let path = video_fixture(
        &dir,
        "m5gl-hevc.mkv",
        "testsrc2=size=320x240:rate=25:duration=2",
        "libx265",
        &["-preset", "ultrafast", "-x265-params", "log-level=none"],
    )
    .unwrap_or_else(|e| panic!("{e}"));
    println!("PROBE hevc {}", ffprobe_streams(&path));
    let (report, _) =
        run_case(&gl, "hevc", &path, None, "0").unwrap_or_else(|e| panic!("hevc run failed: {e}"));
    assert_rendered(&report);
}

#[test]
fn gl_av1_frame_loop() {
    let Some(gl) = harness_gl("gl_av1_frame_loop") else {
        return;
    };
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    fs::create_dir_all(&dir).expect("fixture dir");
    let path = video_fixture(
        &dir,
        "m5gl-av1.mkv",
        "testsrc2=size=320x240:rate=25:duration=2",
        "libsvtav1",
        &["-preset", "8", "-cpu-used", "4"],
    )
    .unwrap_or_else(|e| panic!("{e}"));
    println!("PROBE av1 {}", ffprobe_streams(&path));
    let (report, _) =
        run_case(&gl, "av1", &path, None, "0").unwrap_or_else(|e| panic!("av1 run failed: {e}"));
    assert_rendered(&report);
}

/// The subtitle-ownership measurement.
///
/// Two fixtures built from the **same** constant-colour video stream: one with
/// an embedded Matroska `subrip` track, one without. On a constant-colour
/// stream every undecorated frame has identical pixels, so the difference
/// between the two fixtures is the subtitle layer and nothing else - which is
/// what makes a pixel difference attributable.
///
/// Four runs:
///
/// * `control-no-subtitle-track` - the subtitle-free variant, no selection request;
/// * `srt-sub-off` - the subrip fixture with `sub-id=no` requested;
/// * `srt-sub-auto` - the same fixture with `sub-id=auto`;
/// * `srt-untouched` - the same fixture with no request at all (libmpv's
///   load-time default selection, which is what actually renders subtitles for
///   a real user).
///
/// Controls, in this order:
///
/// 1. **primary**: the subtitle-free variant. It always exists, so the
///    measurement never depends on libmpv accepting a selection property;
/// 2. **secondary**: `srt-sub-off`, and only when the harness *recorded* that
///    libmpv accepted it (`SUBCONTROL ... outcome=accepted`). This repository's
///    captured libmpv runs reject `set sub-id = ...` with
///    `MPV_ERROR_PROPERTY_NOT_FOUND` (cause `[U]`, `docs/CAPABILITIES.md`
///    M4-M7 line), so a rejection is recorded as a measurement instead of
///    aborting the case.
///
/// Attribution (the confound this addresses): `SessionConfig::osd_level` is
/// `"0"` for every run here - visible as `GLSESSION ... osd_level=0`. mpv's own
/// documentation describes level 0 as "OSD completely disabled (subtitles
/// only)" and says `--video-osd` "does not affect subtitles", and mpv's
/// tech overview documents that the VOs call `osd.c` to render OSD *and*
/// subtitle. So the OSD text layer is excluded by **documented
/// semantics**, not by a measurement in this tree: whether mpv's OSD messages
/// would in fact land in our FBO has never been observed here (no GL run
/// reached a frame), and is recorded as **[U]**. With OSD text off by option, a
/// case that shows a hash the control never showed *and* more lit pixels than
/// the control is consistent with mpv drawing the subtitle layer into **our**
/// FBO - consistent with, not proof of: the same difference could come from a
/// different frame count, decoder output or selection outcome, which is why the
/// verdict line names the observation **and the direction of the lit-pixel
/// change** rather than a conclusion. A difference whose lit-pixel count went
/// *down* is reported as such: a glyph layer adds lit pixels, so a decrease is
/// a confounded difference, not evidence of a drawn subtitle layer.
///
/// Whether mpv's own text layer lands in our FBO at all is measured by this
/// case's `control-osd-level-2` positive control, and its `SUBOSD` line is
/// printed in **every** branch (RUN5: 3 distinct hashes / 40981 lit pixels vs
/// the level-0 control's 2 / 40865, i.e. mpv's OSD text did reach this app's
/// FBO). A difference verdict without that calibration would attribute an
/// uncalibrated frame change to subtitles, which is what an earlier revision of
/// this test did.
///
/// Every case's report line, hash set, subtitle-window coverage and control
/// outcome is printed verbatim, plus one `SUBOSD` line and one `SUBOWNERSHIP
/// verdict=...` line. The verdict is one of three recorded measurements (the
/// case exits 0 for all three; the harness records measurements rather than
/// aborting on an inconvenient answer):
///
/// * `measured-difference-lit-pixels-increased` - a case showed frames the
///   controls never showed and/or more lit pixels: the direction a glyph layer
///   would move this metric in;
/// * `measured-difference-lit-pixels-decreased-confounded` - a case showed
///   frames the controls never showed but with **fewer** lit pixels, which is
///   not the direction an added subtitle layer produces (RUN5's only differing
///   case, `srt-sub-auto`, was of this kind: 2 hashes outside the control set,
///   `max_non_black_delta=-12918`). Attribution is `[U]`;
/// * `no-visible-difference-from-either-control` - every subrip run rendered,
///   hashed, covered the subtitle window and still showed nothing the controls
///   did not.
///
/// The harness never prints a decision: the subtitle-ownership decision is
/// recorded in `docs/CAPABILITIES.md`.
/// If no case differs from the controls, the verdict is a
/// **recorded negative measurement** (the case still exits 0): the harness's
/// job is to retain measurements, and a "no difference" answer is a measurement
/// about this host that `docs/CAPABILITIES.md` has to record as `[U]` rather
/// than paper over. The case still fails hard if a
/// run renders too few frames, renders black, or paces away from the fixture's
/// frame period - those are harness/fixture faults, not findings. The fixture
/// is mostly black with one bright box (unsaturated lit-pixel headroom).
/// Coverage is stated per case (`SUBCOVERAGE`): `drive()` renders with
/// `block_for_target_time=true`, so a run whose measured start-to-start pacing
/// matches the fixture's 40 ms frame period displayed the fixture at its own
/// rate, and a run that covers >= 1.0 s of media time displays the 0.2-1.0 s
/// subtitle event (see `SUB_WINDOW_END_S` for why the window is placed there).
/// The comparison itself is between hash sets and lit-pixel counts, not
/// time-matched frames.
#[test]
fn gl_subtitle_ownership_case() {
    let Some(gl) = harness_gl("gl_subtitle_ownership_case") else {
        return;
    };
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    fs::create_dir_all(&dir).expect("fixture dir");
    let (subs, base) = subtitle_fixture(&dir).unwrap_or_else(|e| panic!("{e}"));
    println!("PROBE subtitle {}", ffprobe_streams(&subs));
    println!("PROBE control {}", ffprobe_streams(&base));

    let cases: [(&str, PathBuf, Option<TrackSelection>, &'static str); 6] = [
        // Primary control: the subtitle-free variant at the level every other
        // case uses.
        ("control-no-subtitle-track", base.clone(), None, "0"),
        // Positive control for the *text layer* itself: the same subtitle-free
        // base with `osd-level=2`, which mpv documents as "enabled + current
        // time visible by default" (installed man page line 7449). If mpv
        // composites any text layer of its own into our FBO, this run must
        // differ from the level-0 control; if it does not, the absence of a
        // subtitle layer in the subtitle cases has a much simpler explanation.
        ("control-osd-level-2", base.clone(), None, "2"),
        ("srt-sub-off", subs.clone(), Some(TrackSelection::None), "0"),
        // The explicit track id mpv's own CLI/IPC probe reports for this
        // fixture ("○ Subs --sid=1 (subrip)").
        (
            "srt-sub-id-1",
            subs.clone(),
            Some(TrackSelection::Index(1)),
            "0",
        ),
        (
            "srt-sub-auto",
            subs.clone(),
            Some(TrackSelection::Auto),
            "0",
        ),
        ("srt-untouched", subs.clone(), None, "0"),
    ];
    let mut runs: Vec<(&'static str, FrameLoopReport, Option<String>)> = Vec::new();
    for (label, path, sel, osd) in &cases {
        let (report, note) = run_case(&gl, label, path, sel.as_ref(), osd)
            .unwrap_or_else(|e| panic!("{label} run failed: {e}"));
        assert!(
            report.frames >= 10,
            "{label} produced {} frames",
            report.frames
        );
        assert!(
            report.max_non_black > 1000,
            "{label} rendered only (near) black frames: max_non_black={}",
            report.max_non_black
        );
        // Coverage: a negative comparison is only attributable if the run
        // actually displayed media while the subrip event was on screen, so the
        // window ends where every accepted run reaches (see SUB_WINDOW_END_S).
        let pacing = pacing_mean_ms(&report);
        let covered = media_covered_s(&report);
        println!(
            "SUBCOVERAGE case={label} frames={} pacing_mean_ms={:.2} expected_frame_period_ms={:.1} media_covered_s={:.2} subtitle_window_s={}..{} window_inside_covered={} target_time_frames={} target_time_span_s={:.3} target_time_us_used_for_coverage=false see=docs/RENDER_SPIKE.md",
            report.frames,
            pacing,
            FRAME_PERIOD_MS,
            covered,
            SUB_WINDOW_START_S,
            SUB_WINDOW_END_S,
            covered >= SUB_WINDOW_END_S,
            report.target_time_us.len(),
            (report.target_time_span_us() as f64) / 1e6
        );
        assert!(
            pacing > FRAME_PERIOD_MS * 0.5 && pacing < FRAME_PERIOD_MS * 2.0,
            "{label}: measured pacing {:.2} ms is not near the fixture's {:.1} ms frame period - the loop is not displaying media at the fixture rate, so no run inside the subtitle window can be claimed",
            pacing,
            FRAME_PERIOD_MS
        );
        assert!(
            covered >= SUB_WINDOW_END_S,
            "{label}: run covered only {:.2} s of a 2.0 s fixture - the {}-{} s subtitle window was not necessarily displayed",
            covered,
            SUB_WINDOW_START_S,
            SUB_WINDOW_END_S
        );
        runs.push((label, report, note));
    }

    for (label, r, note) in &runs {
        println!(
            "SUBCASE case={label} frames={} distinct={} max_non_black={} first_hash={:016x} selection_outcome={} hashes={:016x?}",
            r.frames,
            r.distinct_hashes.len(),
            r.max_non_black,
            r.first_hash,
            note.as_deref().unwrap_or("no-selection-request-issued"),
            Vec::from_iter(r.distinct_hashes.iter().copied())
        );
    }

    let control = runs
        .iter()
        .find(|(l, _, _)| *l == "control-no-subtitle-track")
        .map_or_else(|| panic!("control run missing"), |(_, r, _)| r);
    assert!(
        control
            .last_frame
            .as_ref()
            .is_some_and(|f| f.non_black > 1000),
        "the subtitle-free control fixture rendered (near) black - the fixture itself failed"
    );

    let secondary_accepted = runs.iter().any(|(l, _, n)| {
        *l == "srt-sub-off" && n.as_deref().is_some_and(|v| v.starts_with("accepted"))
    });
    let secondary: Option<&FrameLoopReport> = if secondary_accepted {
        runs.iter()
            .find(|(l, _, _)| *l == "srt-sub-off")
            .map(|(_, r, _)| r)
    } else {
        None
    };
    println!(
        "SUBCONTROL primary=subtitle-free-variant secondary-sub-id-no={}",
        if secondary_accepted {
            "accepted: used as secondary control"
        } else {
            "not available (see the SUBCONTROL case=srt-sub-off outcome line; recorded, not fatal)"
        }
    );

    // Rank a difference by how far it moved either discriminator (hash counts
    // are non-negative; a lit-pixel delta can be negative).
    let score = |extra: usize, delta: i64| -> i64 {
        i64::try_from(extra).unwrap_or(i64::MAX) + delta.max(0)
    };
    let mut best: Option<(&str, usize, i64, &str)> = None;
    // The OSD positive control is compared separately below; it has no subtitle
    // track, so ranking it here would attribute an OSD difference to subtitles.
    for (label, r, _) in &runs {
        if *label == "control-no-subtitle-track" || *label == "control-osd-level-2" {
            continue;
        }
        let compare = |ctl: &FrameLoopReport| -> (usize, i64) {
            let extra = r
                .distinct_hashes
                .iter()
                .copied()
                .filter(|h| !ctl.distinct_hashes.contains(h))
                .count();
            (extra, r.max_non_black as i64 - ctl.max_non_black as i64)
        };
        let (extra, delta) = compare(control);
        println!(
            "SUBDIFF case={label} control=subtitle-free-variant hashes_only_in_case={extra} max_non_black_delta={delta} (case {} - control {})",
            r.max_non_black, control.max_non_black
        );
        // A difference in **either** discriminator counts as an observation: the
        // hash set (a frame the control never showed) or the lit-pixel count.
        // Requiring both was an artifact of the old saturated fixture, where
        // `delta` could never exceed 0 no matter what was drawn.
        if (extra > 0 || delta > 0)
            && best.is_none_or(|(_, be, bd, _)| score(extra, delta) > score(be, bd))
        {
            best = Some((label, extra, delta, "subtitle-free-variant"));
        }
        if let Some(sec) = secondary {
            let (extra2, delta2) = compare(sec);
            println!(
                "SUBDIFF case={label} control=sub-id-no hashes_only_in_case={extra2} max_non_black_delta={delta2} (case {} - control {})",
                r.max_non_black, sec.max_non_black
            );
            if (extra2 > 0 || delta2 > 0)
                && best.is_none_or(|(_, be, bd, _)| score(extra2, delta2) > score(be, bd))
            {
                best = Some((label, extra2, delta2, "sub-id-no"));
            }
        }
    }

    // The OSD positive control is computed and printed in **every** branch.
    // It calibrates the subtitle verdict: it says whether anything mpv draws
    // itself (mpv's `osd.c` text layer at osd-level=2) reaches this app's FBO.
    // Omitting it in a run that found a difference leaves that difference
    // uncalibrated - which is exactly what an earlier revision of this test did
    // (RUN5 printed no SUBOSD line although its own data showed the OSD control
    // differing: 3 distinct hashes / 40981 lit pixels vs 2 / 40865).
    let osd = runs
        .iter()
        .find(|(l, _, _)| *l == "control-osd-level-2")
        .map(|(_, r, _)| r);
    let osd_state = osd.map_or_else(
        || "absent (no control-osd-level-2 run recorded)".to_string(),
        |oc| {
            let extra = oc
                .distinct_hashes
                .iter()
                .copied()
                .filter(|h| !control.distinct_hashes.contains(h))
                .count();
            let delta = oc.max_non_black as i64 - control.max_non_black as i64;
            let verdict = if extra > 0 || delta != 0 {
                "mpv-text-layer-did-reach-app-fbo"
            } else {
                "no-difference-vs-osd-level-0-control-mpv-text-layer-not-observed-in-app-fbo"
            };
            println!(
                "SUBOSD case=control-osd-level-2 verdict={} extra_hashes={extra} max_non_black_delta={delta} (osd-level-2 {} lit vs osd-level-0 control {} lit) distinct_case={} distinct_control={} (osd-level-2 documented as 'enabled + current time visible by default', installed man page line 7449)",
                verdict,
                oc.max_non_black,
                control.max_non_black,
                oc.distinct_hashes.len(),
                control.distinct_hashes.len(),
            );
            format!("verdict={verdict} extra_hashes={extra} max_non_black_delta={delta}")
        },
    );
    let osd_differs = osd_state.starts_with("verdict=mpv-text-layer-did-reach-app-fbo");

    match best {
        Some((label, extra, delta, control_mode)) => {
            // The direction of the lit-pixel change is part of the verdict, not
            // a footnote: an added glyph layer adds lit pixels. A case whose
            // frames differ while its lit-pixel count *falls* is a confounded
            // difference and must not be read as a drawn subtitle layer.
            let (verdict, direction) = if delta > 0 {
                (
                    "measured-difference-lit-pixels-increased",
                    "consistent-with-an-added-layer-toward-the-subtitle-hypothesis-not-proof",
                )
            } else if delta == 0 {
                (
                    "measured-difference-new-hash-frames-only",
                    "frame-content-changed-without-lit-pixel-change-attribution-U",
                )
            } else {
                (
                    "measured-difference-lit-pixels-decreased-confounded",
                    "not-the-direction-an-added-subtitle-layer-produces-attribution-U",
                )
            };
            println!(
                "SUBOWNERSHIP verdict={verdict} direction={direction} case={label} control={control_mode} extra_hashes={extra} max_non_black_delta={delta} osd_level=0 app_fbo={}x{} hwdec=no osd_control[{osd_state}] attribution=osd-level-0-documented-as-OSD-completely-disabled-subtitles-only-mpv-man-line-7444-OSD-text-excluded-by-documented-semantics-not-by-a-measurement-in-this-tree decision_printed_by_harness=false see=docs/CAPABILITIES.md",
                W, H
            );
        }
        // A negative measurement, recorded instead of turned into a failure:
        // every subtitle case ran, rendered frames, hashed them, covered the
        // subtitle window and still showed nothing the controls did not. That
        // is data about this host's libmpv + this app's FBO, and the decision
        // it supports is written in docs/CAPABILITIES.md; the harness must
        // keep measuring it on other
        // hosts rather than abort the run because the answer was "no".
        None => println!(
            "SUBOWNERSHIP verdict=no-visible-difference-from-either-control measurement=negative-on-this-host osd_control_differed={osd_differs} osd_control[{osd_state}] app_fbo={}x{} hwdec=no cases=all-4-subrip-runs-rendered-with-hashes decision_printed_by_harness=false see=docs/CAPABILITIES.md",
            W,
            H
        ),
    }
}
