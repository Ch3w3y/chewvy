#!/bin/bash
# Software-GL render harness.
#
# Runs the real GL render path (`crates/plex-app/tests/gl_harness.rs`: EGL
# surfaceless context -> libmpv core -> MPV_RENDER_API_TYPE_OPENGL render
# context -> app-owned RGBA8 FBO -> per-frame glReadPixels hash) headless on
# Mesa llvmpipe, once per fixture, and saves the whole run verbatim.
#
#   EGL_PLATFORM=surfaceless                          no window system, no display
#   LIBGL_ALWAYS_SOFTWARE=1                           Mesa llvmpipe, never hardware
#   __EGL_VENDOR_LIBRARY_FILENAMES=.../50_mesa.json    pin the Mesa EGL vendor
#
# Hardware decode is not exercised: the recipe forces software GL and
# SessionConfig::hwdec stays "no".
#
# Each case is run as its own `cargo test --exact` process so that VmHWM (peak
# RSS, which is process-lifetime) is a per-fixture number, not one shared peak.
#
# Usage: scripts/mpv-gl-harness.sh [out-file]
#        default out-file: target/mpv-gl-harness.log
set -u -o pipefail
cd "$(dirname "$0")/.."
source scripts/env.sh

OUT="${1:-target/mpv-gl-harness.log}"
mkdir -p "$(dirname "$OUT")"

export EGL_PLATFORM=surfaceless
export LIBGL_ALWAYS_SOFTWARE=1
export __EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/50_mesa.json
export PLEX_GL_HARNESS=1
# One test thread, so a run measures one mpv core at a time.
export RUST_TEST_THREADS=1

CASES=(gl_h264_frame_loop gl_hevc_frame_loop gl_av1_frame_loop gl_subtitle_ownership_case)

{
  echo "MPV_GL_HARNESS_BEGIN utc=$(date -u +%Y-%m-%dT%H:%M:%SZ) script=scripts/mpv-gl-harness.sh"
  echo "HARNESS_ENV EGL_PLATFORM=$EGL_PLATFORM LIBGL_ALWAYS_SOFTWARE=$LIBGL_ALWAYS_SOFTWARE EGL_VENDOR_FILE=$__EGL_VENDOR_LIBRARY_FILENAMES PLEX_GL_HARNESS=$PLEX_GL_HARNESS"
  echo "MACHINE uname=$(uname -srvm)"
  echo "MACHINE cpu=$(grep -m1 -E '^model name' /proc/cpuinfo | cut -d: -f2- | sed 's/^ //')"
  echo "MACHINE cpus=$(nproc) mem_kib=$(grep -m1 MemTotal /proc/meminfo | awk '{print $2}')"
  echo "MACHINE dev_dri=$(ls /dev/dri 2>&1 | tr '\n' ' ')"
  echo "MACHINE egl_vendor_file=$([ -f "$__EGL_VENDOR_LIBRARY_FILENAMES" ] && echo present || echo MISSING)"
  echo "MACHINE mesa_egl=$(ls -l /usr/lib64/libEGL_mesa.so 2>&1 | awk '{print $NF, $5}')"
  echo "MACHINE rustc=$(rustc --version) cargo=$(cargo --version)"
  echo "MACHINE ffmpeg=$(ffmpeg -version 2>&1 | head -1)"
  echo "MACHINE mpv=$(mpv --version 2>&1 | head -1)"
  echo "MACHINE git_base=$(git -C . rev-parse HEAD 2>&1) git_dirty=$(git status --porcelain | wc -l)"
} >"$OUT" 2>&1

failed=0
for name in "${CASES[@]}"; do
  cmd="cargo test --offline --locked --test gl_harness -- --exact $name --nocapture --test-threads=1"
  echo "CASE name=$name command=$cmd" >>"$OUT"
  # tee keeps the console and the file identical.
  eval "$cmd" 2>&1 | tee -a "$OUT"
  rc=${PIPESTATUS[0]}
  echo "CASE_EXIT name=$name exit=$rc" >>"$OUT"
  echo "CASE_EXIT name=$name exit=$rc"
  [ "$rc" -ne 0 ] && failed=$((failed + 1))
done

echo "HARNESS cases=${#CASES[@]} failed=$failed log=$OUT" | tee -a "$OUT"
echo "MPV_GL_HARNESS_END utc=$(date -u +%Y-%m-%dT%H:%M:%SZ) failed=$failed" | tee -a "$OUT"
exit "$failed"
