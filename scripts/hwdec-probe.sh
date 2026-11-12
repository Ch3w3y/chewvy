#!/usr/bin/env bash
# Read-only host probe for hardware-decode capability (docs/HARDWARE_DECODE.md).
# Run it once per candidate desktop host/gpu/driver stack and keep the transcript
# beside the capability matrix, since each row is host-specific. It only *reads*
# system state and writes inside the output directory, so undoing a run is just
# deleting that directory. Absent tools are recorded (exit=127), never fatal:
# the script always exits 0 unless the output directory cannot be written.
set -u

OUT_DIR=${1:-target/hwdec-probe}
if ! mkdir -p "$OUT_DIR"; then
    echo "hwdec-probe: cannot write $OUT_DIR" >&2
    exit 1
fi
OUT="$OUT_DIR/hwdec-probe.txt"
exec >"$OUT" 2>&1

if command -v timeout >/dev/null 2>&1; then T="timeout 20"; GT="timeout 180"; else T=""; GT=""; fi

need() { command -v "$1" >/dev/null 2>&1 || { echo "$1 not installed"; return 127; }; return 0; }

# run <label> <shell command>: prints the command, its output and its exit code.
run() {
    local label=$1 cmd=$2 code
    printf '\n### %s\n$ %s\n' "$label" "$cmd"
    eval "$cmd"
    code=$?
    printf '[%s exit=%s]\n' "$label" "$code"
    return 0
}

echo "hwdec-probe (H1): $(date -u '+%Y-%m-%dT%H:%M:%SZ')"
echo "cwd=$(pwd)"

run "kernel" "uname -a"
run "os-release" "cat /etc/os-release 2>/dev/null | grep -E '^(NAME|VERSION_ID|PRETTY_NAME)='"
run "session/env" "env | grep -E '^(XDG_SESSION_TYPE|XDG_CURRENT_DESKTOP|WAYLAND_DISPLAY|WAYLAND_SOCKET|DISPLAY|LIBGL_ALWAYS_SOFTWARE|__EGL_VENDOR_LIBRARY_FILENAMES|MESA_)' | sort || true"

run "dev-dri" "ls -l /dev/dri 2>/dev/null || echo 'no /dev/dri'"
run "dev-nvidia" "ls -l /dev/nvidia* 2>/dev/null || echo 'no /dev/nvidia*'"
run "drm-sysfs" 'for d in /sys/class/drm/card[0-9]; do [ -d "$d" ] || continue; echo "$d vendor=$(cat "$d/device/vendor" 2>/dev/null) device=$(cat "$d/device/device" 2>/dev/null) driver=$(basename "$(readlink "$d/device/driver" 2>/dev/null)" 2>/dev/null)"; done; true'
run "lspci-gpu" "command -v lspci >/dev/null && lspci -nnk 2>/dev/null | grep -iA3 -E 'vga|3d controller|display controller' || echo 'lspci missing or no match'"
run "lsmod-gpu" "command -v lsmod >/dev/null && lsmod 2>/dev/null | grep -E '^(nvidia|i915|xe|amdgpu|radeon|nouveau|virtio_gpu|vmwgfx)' || echo 'no gpu modules matched (or lsmod missing)'"

run "mpv-version" 'need mpv && mpv --version | head -n2'
run "ffmpeg-version" 'need ffmpeg && ffmpeg -version | head -n1'
run "glxinfo" 'need glxinfo && if [ -n "${DISPLAY:-}" ]; then glxinfo -B; else echo "glxinfo found but no DISPLAY (X11 needed)"; fi'
run "eglinfo" 'need eglinfo && eglinfo -B'
run "vulkaninfo" "need vulkaninfo && $T vulkaninfo --summary"
run "nvidia-smi" 'need nvidia-smi && nvidia-smi --query-gpu=name,driver_version --format=csv,noheader'
run "vainfo" "need vainfo && $T vainfo 2>&1 | grep -E 'Driver|Supported profile|VAEntrypointVLD|error' | head -n40"

run "ffmpeg-hwaccels" 'need ffmpeg && ffmpeg -hide_banner -hwaccels'
run "ffmpeg-hwaccel-decoders" 'need ffmpeg && ffmpeg -hide_banner -decoders | grep -Ei "_vaapi|_nvdec|_cuvid|_videotoolbox|_d3d11va|_dxva2|_vdpau|_vulkan|_v4l2m2m" || echo none listed'

# --- codec x hwdec matrix ---------------------------------------------------
# One short 1080p clip per codec, generated locally with lavfi testsrc2 and
# reused on later runs. --vo=null has no GL context of its own and can differ
# from the app's real render-API path (EGL/dmabuf interop in particular), so
# --vo=gpu is probed as well whenever a display server is present.
ROWS=""
MATRIX_NOTE=""
if ! command -v mpv >/dev/null 2>&1 || ! command -v ffmpeg >/dev/null 2>&1; then
    MATRIX_NOTE="mpv and/or ffmpeg not found: codec matrix skipped"
elif ! ffmpeg -hide_banner -encoders >/dev/null 2>&1; then
    MATRIX_NOTE="ffmpeg cannot list encoders: codec matrix skipped"
else
    rm -f "$OUT_DIR/sample.mkv"
    AV1ENC=$(ffmpeg -hide_banner -encoders 2>/dev/null | grep -oE 'lib(aom[-_]av1|svtav1|rav1e)' | head -n1)
    case "$AV1ENC" in
        *aom*)   AV1ARGS="-b:v 0 -crf 50 -cpu-used 8 -row-mt 1" ;;
        *svt*)   AV1ARGS="-preset 12" ;;
        *rav1e*) AV1ARGS="-speed 10" ;;
        *)       AV1ARGS="" ;;
    esac

    # make_clip <codec> <encoder> <pix_fmt> [extra ffmpeg args]: leaves CLIP
    # empty (and prints a skip line) when the encoder is absent or the encode
    # fails, so one missing codec never hides the rest of the matrix.
    make_clip() {
        local codec=$1 enc=$2 pix=$3 f="$OUT_DIR/clip-$1.mkv"
        shift 3
        CLIP=""
        if [ -z "$enc" ]; then
            echo "skip $codec: no encoder"
            return 0
        fi
        if [ ! -s "$f" ]; then
            printf '\n### generate %s clip (%s)\n' "$codec" "$enc"
            # shellcheck disable=SC2086
            if ! $GT ffmpeg -v error -nostdin -f lavfi \
                    -i testsrc2=size=1920x1080:rate=25:duration=3 \
                    -c:v "$enc" -pix_fmt "$pix" "$@" -an -y "$f" </dev/null; then
                echo "skip $codec: encode failed"
                rm -f "$f"
                return 0
            fi
        fi
        CLIP="$f"
    }

    # probe_clip <codec> <clip>: runs mpv once per usable vo and records
    # "<codec>|<vo>|<result>" in ROWS; the hwdec log lines are echoed too.
    probe_clip() {
        local codec=$1 clip=$2 vo log code api
        for vo in null gpu; do
            [ "$vo" = gpu ] && [ -z "${WAYLAND_DISPLAY:-}${DISPLAY:-}" ] && continue
            log="$OUT_DIR/.mpv-$codec-$vo.log"
            printf '\n### %s vo=%s\n$ mpv --no-config --ao=null --vo=%s --hwdec=auto-safe --length=2 %s\n' \
                "$codec" "$vo" "$vo" "$clip"
            # shellcheck disable=SC2086
            $T mpv --no-config --really-quiet --ao=null --vo="$vo" --length=2 \
                --hwdec=auto-safe --msg-level=vd=v,vd_lavc=v -- "$clip" \
                >"$log" 2>&1 </dev/null
            code=$?
            grep -Ei 'hardware decoding|software decoding|decoding method after failure' \
                "$log" | head -n 12
            api=$(grep -m1 -oE 'Using hardware decoding \([^)]*\)' "$log" | sed -e 's/.*(\(.*\))/\1/')
            if [ -n "$api" ]; then
                ROWS="$ROWS$codec|$vo|$api
"
            elif [ "$code" -ne 0 ]; then
                ROWS="$ROWS$codec|$vo|error exit=$code
"
            else
                ROWS="$ROWS$codec|$vo|software
"
            fi
            rm -f "$log"
        done
    }

    make_clip h264 libx264 yuv420p -preset veryfast
    [ -n "$CLIP" ] && probe_clip h264 "$CLIP"
    make_clip hevc libx265 yuv420p10le -preset ultrafast
    [ -n "$CLIP" ] && probe_clip hevc "$CLIP"
    # shellcheck disable=SC2086
    make_clip av1 "$AV1ENC" yuv420p $AV1ARGS
    [ -n "$CLIP" ] && probe_clip av1 "$CLIP"
    make_clip vp9 libvpx-vp9 yuv420p -b:v 0 -crf 40 -deadline realtime -cpu-used 8
    [ -n "$CLIP" ] && probe_clip vp9 "$CLIP"
fi

printf '\n### matrix (codec | vo | result)\n'
if [ -n "$MATRIX_NOTE" ]; then
    echo "$MATRIX_NOTE"
elif [ -z "$ROWS" ]; then
    echo "no clip could be played"
else
    printf '%-6s %-4s %s\n' codec vo result
    while IFS='|' read -r c v res; do
        [ -n "$c" ] && printf '%-6s %-4s %s\n' "$c" "$v" "$res"
    done <<ROWS_BLOCK
$ROWS
ROWS_BLOCK
fi

echo
echo "SUMMARY: interpret with docs/HARDWARE_DECODE.md; record mpv/ffmpeg versions,"
echo "the gpu vendor from drm-sysfs/lspci, ffmpeg-hwaccels names, and every matrix"
echo "row (codec + vo + hwdec API or software) as a host-specific [U] entry."
