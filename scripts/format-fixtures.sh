#!/usr/bin/env bash
# Generate synthetic clips in older and less common containers and codecs, then
# play each one in mpv for a few seconds and print what mpv reported as a
# Markdown table. Everything lands under target/ (nothing is committed). Run
# from anywhere; paths are repo-root relative.
#
# The table is a measurement of this host's mpv and ffmpeg builds, not a
# general statement about the formats: re-run it per host.
#
# Env overrides:
#   OUT_DIR  output directory           (default: target/format-fixtures)
#   DUR      seconds per clip           (default: 65)
#   PLAY     seconds mpv plays each one (default: 5)
set -euo pipefail

cd "$(dirname "$0")/.."
OUT_DIR="${OUT_DIR:-target/format-fixtures}"
DUR="${DUR:-65}"
PLAY="${PLAY:-5}"

for tool in ffmpeg ffprobe mpv; do
    command -v "$tool" >/dev/null || { echo "missing tool: $tool" >&2; exit 127; }
done

# The directory is wiped before each run, so refuse anything outside target/.
case "$OUT_DIR" in
    target/*) ;;
    *) echo "OUT_DIR must be under target/: $OUT_DIR" >&2; exit 2 ;;
esac
case "$OUT_DIR" in *..*) echo "OUT_DIR must not contain ..: $OUT_DIR" >&2; exit 2 ;; esac
rm -rf "$OUT_DIR"
mkdir -p "$OUT_DIR"

FF=(ffmpeg -nostdin -v error -y)
picture() { printf -- '-f lavfi -i testsrc2=size=%s:rate=%s' "$1" "$2"; }
# Six discrete tones, so a 5.1 encoder has six real channels to carry.
SURROUND=(-f lavfi -i "sine=frequency=220:sample_rate=48000,pan=5.1|c0=c0|c1=0.8*c0|c2=0.6*c0|c3=0.4*c0|c4=0.7*c0|c5=0.5*c0")
STEREO=(-f lavfi -i "sine=frequency=440:sample_rate=48000,pan=stereo|c0=c0|c1=0.7*c0")
H264=(-c:v libx264 -preset ultrafast -crf 30 -pix_fmt yuv420p -g 48)

# make <file> <picture size> <fps> <audio input array name> <encoder args...>
make() {
    local file=$1 size=$2 fps=$3 audio=$4
    shift 4
    local -n audio_in=$audio
    # shellcheck disable=SC2046
    "${FF[@]}" $(picture "$size" "$fps") "${audio_in[@]}" -t "$DUR" "$@" "$OUT_DIR/$file" ||
        echo "generation failed: $file" >&2
}

make mpeg2_mp2.ts 720x576 25 STEREO -c:v mpeg2video -b:v 4M -g 12 -c:a mp2 -b:a 192k
make mpeg2_ac3.mpg 720x576 25 STEREO -c:v mpeg2video -b:v 4M -g 12 -c:a ac3 -b:a 192k -f vob
make mpeg2_1080.mkv 1920x1080 25 STEREO -c:v mpeg2video -b:v 12M -g 12 -c:a ac3 -b:a 192k
make h264_ac3.m2ts 1920x1080 24 SURROUND "${H264[@]}" -c:a ac3 -b:a 448k -f mpegts -mpegts_m2ts_mode 1
make h264_aac.ts 1280x720 25 STEREO "${H264[@]}" -c:a aac -b:a 128k
make hevc10_eac3.ts 1920x1080 24 SURROUND -c:v libx265 -preset ultrafast -crf 30 -pix_fmt yuv420p10le \
    -x265-params log-level=error -c:a eac3 -b:a 384k
make mpeg4_mp3.avi 1280x720 25 STEREO -c:v mpeg4 -q:v 5 -g 250 -c:a libmp3lame -b:a 128k
make h264_aac.mov 1280x720 24 STEREO "${H264[@]}" -c:a aac -b:a 128k
make vp8_vorbis.webm 1280x720 24 STEREO -c:v libvpx -b:v 2M -deadline realtime -cpu-used 8 -c:a libvorbis
make wmv2_wmav2.wmv 1280x720 25 STEREO -c:v wmv2 -b:v 2M -c:a wmav2 -b:a 128k
make h264_dts.mkv 1280x720 24 SURROUND "${H264[@]}" -c:a dca -strict -2 -b:a 1536k
make h264_truehd.mkv 1280x720 24 SURROUND "${H264[@]}" -c:a truehd -strict -2
make h264_pcm24.mkv 1280x720 24 STEREO "${H264[@]}" -c:a pcm_s24le
make h264_flac51.mkv 1280x720 24 SURROUND "${H264[@]}" -c:a flac
make h264_alac.mkv 1280x720 24 STEREO "${H264[@]}" -c:a alac

# One styled ASS track: position, colour, outline and an italic override.
cat >"$OUT_DIR/styled.ass" <<'ASS'
[Script Info]
ScriptType: v4.00+
PlayResX: 1280
PlayResY: 720

[V4+ Styles]
Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding
Style: Top,Sans,48,&H0000FFFF,&H000000FF,&H00000000,&H00000000,1,0,0,0,100,100,0,0,1,3,0,8,20,20,30,1
Style: Low,Sans,40,&H00FFFFFF,&H000000FF,&H00800000,&H00000000,0,0,0,0,100,100,0,0,1,2,1,2,20,20,30,1

[Events]
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
Dialogue: 0,0:00:00.00,0:01:05.00,Top,,0,0,0,,Yellow line at the top
Dialogue: 0,0:00:00.00,0:01:05.00,Low,,0,0,0,,White line with {\i1}italics{\i0} below
ASS
"${FF[@]}" -i "$OUT_DIR/h264_alac.mkv" -i "$OUT_DIR/styled.ass" -map 0:v -map 0:a -map 1 \
    -c copy -c:s ass "$OUT_DIR/h264_ass.mkv" || echo "generation failed: h264_ass.mkv" >&2

# mpv prints one line of properties once it has played for $PLAY seconds.
cat >"$OUT_DIR/report.lua" <<LUA
mp.add_timeout($PLAY, function()
    local function get(name) return tostring(mp.get_property(name)) end
    print("REPORT " .. table.concat({
        get("video-format"), get("video-params/w") .. "x" .. get("video-params/h"),
        get("hwdec-current"), get("audio-codec-name"), get("audio-params/channel-count"),
        get("vid"), get("aid"), get("sid"), get("time-pos"), get("frame-drop-count"),
    }, " | "))
    mp.command("quit")
end)
LUA

echo "| fixture | ffprobe container | mpv video | size | hwdec | mpv audio | channels | vid | aid | sid | position after ${PLAY} s | dropped |"
echo "|---|---|---|---|---|---|---|---|---|---|---|---|"
for file in "$OUT_DIR"/*.*; do
    case "$file" in *.lua | *.ass) continue ;; esac
    container=$(ffprobe -v error -show_entries format=format_name -of csv=p=0 "$file" 2>/dev/null || echo "?")
    report=$(timeout 30 mpv --no-config --vo=null --ao=null --hwdec=auto-safe \
        --audio-fallback-to-null=yes --script="$OUT_DIR/report.lua" "$file" </dev/null 2>&1 |
        sed -n 's/.*REPORT //p' | head -1)
    echo "| $(basename "$file") | $container | ${report:-did not play} |"
done
