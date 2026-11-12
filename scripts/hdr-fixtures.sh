#!/usr/bin/env bash
# Generate short HDR / Dolby Vision test clips with deterministic synthetic
# frames, then verify each one with ffprobe, dovi_tool / hdr10plus_tool, and
# mpv's own detection output. Everything lands under target/ (nothing is
# committed). Run from anywhere; paths are repo-root relative.
#
# Env overrides:
#   OUT_DIR  output directory            (default: target/hdr-fixtures)
#   RES      WxH of the generated video  (default: 1920x1080)
#   FRAMES   frames per clip             (default: 72, i.e. 3 s at 24 fps)
#   PRESET   x265 speed preset           (default: ultrafast)
set -euo pipefail

cd "$(dirname "$0")/.."
OUT_DIR="${OUT_DIR:-target/hdr-fixtures}"
RES="${RES:-1920x1080}"
FRAMES="${FRAMES:-72}"
FPS=24
PRESET="${PRESET:-ultrafast}"

# BT.2020 mastering display (chromaticity x,y in 0.00002 units, luminance in
# 0.0001 cd/m2) and content light level, matching a typical HDR10 static
# metadata payload.
MD_DISPLAY="G(8500,39850)B(6550,2300)R(34000,16450)WP(15635,16450)L(10000000,1)"
MD_CLL="1000,400"

for tool in ffmpeg ffprobe x265 dovi_tool hdr10plus_tool mpv python3; do
    command -v "$tool" >/dev/null || { echo "missing tool: $tool" >&2; exit 127; }
done

# The directory is wiped before each run, so refuse anything outside target/.
case "$OUT_DIR" in
    target/*) ;;
    *) echo "OUT_DIR must be under target/: $OUT_DIR" >&2; exit 2 ;;
esac
case "$OUT_DIR" in *..*) echo "OUT_DIR must not contain ..: $OUT_DIR" >&2; exit 2 ;; esac
rm -rf "$OUT_DIR"
mkdir -p "$OUT_DIR/hevc" "$OUT_DIR/logs"

W="${RES%x*}"
H="${RES#*x}"

# Synthetic 10-bit 4:2:0 source, generated on the fly as raw I010 (planar,
# little-endian) and piped into x265. testsrc2 is deterministic across runs.
src_raw() {
    ffmpeg -v error -f lavfi -i "testsrc2=size=${W}x${H}:rate=${FPS}" \
        -frames:v "$FRAMES" -pix_fmt yuv420p10le -f rawvideo -
}

# x265 cannot parse 10-bit y4m headers reliably in this build, and raw input
# keeps the pipe simple; VUI colour tags are therefore set explicitly.
x265_enc() { # x265_enc <out.hevc> [extra x265 args...]
    local out="$1"; shift
    src_raw | x265 --input - --input-res "$RES" --fps "$FPS" \
        --input-depth 10 --input-csp i420 -D 10 --preset "$PRESET" \
        --frames "$FRAMES" --range limited --repeat-headers \
        --keyint 144 --min-keyint 24 --hrd-concat \
        "$@" --output "$out" >/dev/null 2>&1
}

mux_mp4() { # mux_mp4 <in.hevc> <out.mp4> <color_trc> <color_primaries>
    ffmpeg -y -v error -f hevc -r "$FPS" -i "$1" -c copy \
        -color_trc "$3" -color_primaries "$4" -colorspace bt2020nc \
        -color_range tv "$2"
}

echo "== generating RPU binaries (dovi_tool generate) =="
cat > "$OUT_DIR/gen-p81.json" <<EOF
{
  "length": $FRAMES,
  "profile": "8.1",
  "level6": {
    "max_display_mastering_luminance": 1000,
    "min_display_mastering_luminance": 1,
    "max_content_light_level": 1000,
    "max_frame_average_light_level": 400
  }
}
EOF
dovi_tool generate -j "$OUT_DIR/gen-p81.json" -o "$OUT_DIR/rpu-p81.bin" 2>&1 | grep -v '^$'

cat > "$OUT_DIR/gen-p5.json" <<EOF
{
  "length": $FRAMES,
  "profile": "5",
  "level2": [{ "target_nits": 600 }]
}
EOF
dovi_tool generate -j "$OUT_DIR/gen-p5.json" -o "$OUT_DIR/rpu-p5.bin" 2>&1 | grep -v '^$'

# hdr10plus_tool has no metadata *generator*; inject consumes the same JSON
# schema that extract produces. Build that payload directly (one HDR10+
# intensity detector + bezier curve item per frame).
python3 - "$FRAMES" > "$OUT_DIR/hdr10plus-meta.json" <<'EOF'
import json, sys
n = int(sys.argv[1])
curve = {"Anchors": [143, 298, 447, 592, 731, 864, 891, 917, 938],
         "KneePointX": 164, "KneePointY": 240}
lum = {"AverageRGB": 263,
       "LuminanceDistributions": {
           "DistributionIndex": [1, 5, 10, 25, 50, 75, 90, 95, 99],
           "DistributionValues": [0, 6080, 92, 1, 4, 107, 726, 1784, 5843]},
       "MaxScl": [7768, 6589, 6912]}
items = [{"BezierCurveData": curve, "LuminanceParameters": lum,
          "NumberOfWindows": 1, "TargetedSystemDisplayMaximumLuminance": 400,
          "SceneFrameIndex": 0, "SceneId": 0, "SequenceFrameIndex": i}
         for i in range(n)]
print(json.dumps({"JSONInfo": {"HDR10plusProfile": "B", "Version": "1.0"},
                  "SceneInfo": items,
                  "SceneInfoSummary": {"SceneFirstFrameIndex": [0],
                                       "SceneFrameNumbers": [n]},
                  "ToolInfo": {"Tool": "hdr10plus_tool", "Version": "1.7.2"}}, indent=1))
EOF

echo "== encoding =="
# HDR10: BT.2020 primaries + SMPTE ST 2084 (PQ) + mastering display + CLL.
x265_enc "$OUT_DIR/hevc/hdr10.hevc" \
    --colorprim bt2020 --transfer smpte2084 --colormatrix bt2020nc \
    --master-display "$MD_DISPLAY" --max-cll "$MD_CLL"
mux_mp4 "$OUT_DIR/hevc/hdr10.hevc" "$OUT_DIR/hdr10.mp4" smpte2084 bt2020

# HLG: BT.2020 primaries + ARIB STD-B67 transfer, no mastering display.
x265_enc "$OUT_DIR/hevc/hlg.hevc" \
    --colorprim bt2020 --transfer arib-std-b67 --colormatrix bt2020nc
mux_mp4 "$OUT_DIR/hevc/hlg.hevc" "$OUT_DIR/hlg.mp4" arib-std-b67 bt2020

# Dolby Vision P8.1: HDR10-compatible base layer + generated RPU. x265
# requires VBV when writing a DV RPU; interleave of the RPU NALs is done by
# x265 itself (--dolby-vision-rpu).
DV_VBV="--vbv-maxrate 16000 --vbv-bufsize 16000"
x265_enc "$OUT_DIR/hevc/dv_p81.hevc" \
    --colorprim bt2020 --transfer smpte2084 --colormatrix bt2020nc \
    --master-display "$MD_DISPLAY" --max-cll "$MD_CLL" \
    --dolby-vision-profile 8.1 --dolby-vision-rpu "$OUT_DIR/rpu-p81.bin" $DV_VBV
mux_mp4 "$OUT_DIR/hevc/dv_p81.hevc" "$OUT_DIR/dv_p81.mp4" smpte2084 bt2020

# Dolby Vision P5: SDR BT.709 base layer (10-bit) + IPT-targeted RPU. The
# synthetic pixels are plain BT.709, so this fixture demonstrates metadata
# and profile handling, not the green/purple shift of real P5 masters.
x265_enc "$OUT_DIR/hevc/dv_p5.hevc" \
    --colorprim bt709 --transfer bt709 --colormatrix bt709 \
    --dolby-vision-profile 5 --dolby-vision-rpu "$OUT_DIR/rpu-p5.bin" $DV_VBV
ffmpeg -y -v error -f hevc -r "$FPS" -i "$OUT_DIR/hevc/dv_p5.hevc" -c copy \
    -color_trc bt709 -color_primaries bt709 -colorspace bt709 \
    -color_range tv "$OUT_DIR/dv_p5.mp4"

# HDR10+: HDR10 base + injected per-frame dynamic metadata.
hdr10plus_tool inject -i "$OUT_DIR/hevc/hdr10.hevc" \
    -j "$OUT_DIR/hdr10plus-meta.json" -o "$OUT_DIR/hevc/hdr10plus.hevc" 2>/dev/null
mux_mp4 "$OUT_DIR/hevc/hdr10plus.hevc" "$OUT_DIR/hdr10plus.mp4" smpte2084 bt2020

echo "== verifying =="
# frame_side_types <file>: unique frame side-data type names (first frame).
frame_side_types() {
    ffprobe -v error -select_streams v:0 -show_frames -show_entries \
        frame=side_data_list -of json "$1" 2>/dev/null \
        | { grep -o '"side_data_type": "[^"]*"' || true; } \
        | sort | uniq -c | sed 's/^ *//' \
        | awk -F'"' '{printf "%s%s", sep, $4; sep="; "} END{print ""}'
}

mpv_probe() { # mpv_probe <name> <file>: keep full log, print the detection line
    local name="$1" file="$2" log="$OUT_DIR/logs/$name-mpv.log"
    mpv --no-config --msg-level=all=v --vo=null --ao=null --frames=3 \
        "$file" >"$log" 2>&1 || true
    grep -m1 'Decoder format' "$log" | sed 's/^\[vd\] Decoder format: /mpv: /' \
        || echo "mpv: (no decoder format line, see $log)"
}

printf '%s\n' "---"
printf '| fixture | ffprobe colour tags | frame side data | mpv decoder format |\n'
printf '|---|---|---|---|\n'
for name in hdr10 hlg dv_p81 dv_p5 hdr10plus; do
    f="$OUT_DIR/$name.mp4"
    tags=$(ffprobe -v error -select_streams v:0 -show_entries \
        "stream=codec_name,profile,pix_fmt,color_primaries,color_transfer,color_space" \
        -of csv=p=0 "$f" 2>/dev/null)
    side=$(frame_side_types "$f")
    det=$(mpv_probe "$name" "$f" | sed 's/^mpv: //; s/ \[0:1\]//; s/ CL=.*//; s/ crop=.*//; s/ A=none//')
    printf '| %s | %s | %s | %s |\n' "$name" "$tags" "${side:-—}" "$det"
done
printf '%s\n' "---"

# Dolby Vision checks: dovi_tool only reads raw RPU/HEVC, not MP4, so the
# RPU is extracted from the pre-mux elementary stream.
for name in dv_p81 dv_p5; do
    dovi_tool extract-rpu -o "$OUT_DIR/logs/$name.rpu" "$OUT_DIR/hevc/$name.hevc" >/dev/null 2>/dev/null
    echo "dovi_tool RPU summary for $name:"
    dovi_tool info -i "$OUT_DIR/logs/$name.rpu" -s 2>/dev/null | grep -v '^$' | sed 's/^/  /'
done

echo "hdr10plus_tool verify (hevc elementary stream, 1.7.2 rejects MP4):"
hdr10plus_tool --verify extract -i "$OUT_DIR/hevc/hdr10plus.hevc" \
    -o "$OUT_DIR/logs/hdr10plus-extract.json" 2>/dev/null \
    | grep -viE '^\s*$' | tail -4 | sed 's/^/  /'

echo
echo "Fixtures in: $OUT_DIR (mpv logs under $OUT_DIR/logs)"
