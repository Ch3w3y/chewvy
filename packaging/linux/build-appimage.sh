#!/usr/bin/env bash
# Assemble an AppDir for Chewvy and turn it into an AppImage with linuxdeploy.
# Nothing is downloaded here: the linuxdeploy binary comes from $LINUXDEPLOY.
set -euo pipefail

usage() {
    cat <<'EOF'
Usage: build-appimage.sh [options] [binary [output-dir]]

  binary        built chewvy binary     (default: $CHEWVY_BIN, else target/release/chewvy)
  output-dir    where the AppImage goes (default: $OUTPUT_DIR, else target/appimage)

Options:
  --appdir-only   only assemble the AppDir; do not run linuxdeploy
                  (lets the binary be missing: a placeholder is used)
  -h, --help      this help

Environment:
  LINUXDEPLOY     path to the linuxdeploy binary (required unless --appdir-only);
                  run it with APPIMAGE_EXTRACT_AND_RUN=1 when it is itself an
                  AppImage inside a container without FUSE
  LINUXDEPLOY_OUTPUT_VERSION
                  version in the AppImage file name (default: the workspace
                  version in Cargo.toml)
EOF
}

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
REPO_ROOT=$(cd -- "$SCRIPT_DIR/../.." && pwd)

APP_ID=io.github.ch3w3y.chewvy
BIN=${CHEWVY_BIN:-target/release/chewvy}
OUT=${OUTPUT_DIR:-target/appimage}
APPDIR_ONLY=0

pos=()
while [[ $# -gt 0 ]]; do
    case $1 in
        --appdir-only) APPDIR_ONLY=1 ;;
        -h | --help) usage; exit 0 ;;
        --) shift; pos+=("$@"); break ;;
        -*)
            echo "build-appimage.sh: unknown option: $1" >&2
            usage >&2
            exit 2
            ;;
        *) pos+=("$1") ;;
    esac
    shift
done
if [[ ${#pos[@]} -gt 0 ]]; then
    BIN=${pos[0]}
    [[ ${#pos[@]} -gt 1 ]] && OUT=${pos[1]}
fi
APPDIR=${OUT}/AppDir

# Upstream list of libraries that must come from the host system; linuxdeploy
# applies it by default. Everything below that is NOT on that list still has to
# be excluded explicitly, see the candidate list further down.
# https://github.com/AppImageCommunity/pkg2appimage/blob/master/excludelist
EXCLUDELIST_URL=https://github.com/AppImageCommunity/pkg2appimage/blob/master/excludelist

# libmpv links against GPU-adjacent libraries (Vulkan loader, libwayland) that
# are absent from the upstream excludelist. The GPU driver and the graphics
# stack (EGL/GL/GLX/GLdispatch/gbm/drm/Mesa/vulkan/wayland/X11/libxcb) always
# belong to the host, so they must never end up in usr/lib. Every candidate is
# always passed to --exclude-library: excluding a library the binary does not
# link is harmless, and linuxdeploy has no way to undo a copy after the fact.
GPU_CANDIDATES=(
    libvulkan.so.1
    libwayland-cursor.so.0
    libwayland-egl.so.1
    libwayland-server.so.0
)

rm -rf "$APPDIR"
mkdir -p \
    "$APPDIR/usr/bin" \
    "$APPDIR/usr/share/applications" \
    "$APPDIR/usr/share/icons/hicolor/scalable/apps"

if [[ -f "$BIN" ]]; then
    install -m 0755 "$BIN" "$APPDIR/usr/bin/chewvy"
elif [[ $APPDIR_ONLY -eq 1 ]]; then
    # Dry run without a compiled binary (e.g. checking packaging layout).
    printf '#!/bin/sh\nexit 0\n' >"$APPDIR/usr/bin/chewvy"
    chmod 0755 "$APPDIR/usr/bin/chewvy"
else
    echo "build-appimage.sh: no binary at '$BIN' (build it first, or use --appdir-only)" >&2
    exit 1
fi

# The desktop file and its icon share the application id, which is also the
# window's app id, so desktops match a running window to this entry.
install -m 0644 "$SCRIPT_DIR/$APP_ID.desktop" "$APPDIR/usr/share/applications/$APP_ID.desktop"
install -m 0644 "$SCRIPT_DIR/$APP_ID.svg" "$APPDIR/usr/share/icons/hicolor/scalable/apps/$APP_ID.svg"

# The licences travel inside the package, next to what they cover.
DOC_DIR=$APPDIR/usr/share/doc/chewvy
mkdir -p "$DOC_DIR"
install -m 0644 "$REPO_ROOT/LICENSE" "$REPO_ROOT/THIRD-PARTY-NOTICES.md" "$DOC_DIR/"

if [[ $APPDIR_ONLY -eq 1 ]]; then
    echo "AppDir (not packed, --appdir-only): $APPDIR"
    exit 0
fi

python3 "$REPO_ROOT/scripts/gen_third_party_licenses.py" \
    --target x86_64-unknown-linux-gnu --out "$DOC_DIR/THIRD-PARTY-LICENSES.txt"
if [[ -z ${LINUXDEPLOY_OUTPUT_VERSION:-} ]]; then
    LINUXDEPLOY_OUTPUT_VERSION=$(sed -n 's/^version = "\([^"]*\)".*/\1/p' "$REPO_ROOT/Cargo.toml" | head -n 1)
fi
export LINUXDEPLOY_OUTPUT_VERSION

if [[ -z ${LINUXDEPLOY:-} ]]; then
    echo "build-appimage.sh: LINUXDEPLOY is not set (path to linuxdeploy; this script downloads nothing)" >&2
    exit 1
fi

mkdir -p "$OUT"
OUT=$(cd -- "$OUT" && pwd)
APPDIR=$OUT/AppDir
# Stale AppImages from an earlier run would make the final glob ambiguous.
rm -f "$OUT"/*.AppImage
cmd=("$LINUXDEPLOY" --appdir "$APPDIR"
    --executable "$APPDIR/usr/bin/chewvy"
    --desktop-file "$APPDIR/usr/share/applications/$APP_ID.desktop"
    --icon-file "$APPDIR/usr/share/icons/hicolor/scalable/apps/$APP_ID.svg"
    --output appimage)

echo "build-appimage.sh: running linuxdeploy (upstream excludelist: $EXCLUDELIST_URL)"
for lib in "${GPU_CANDIDATES[@]}"; do
    cmd+=(--exclude-library "$lib")
done
# linuxdeploy writes the AppImage into the working directory.
(cd -- "$OUT" && "${cmd[@]}")

# Belt and braces: if the host graphics stack sneaked into usr/lib anyway, the
# AppImage could load its own copy against the host driver and fail to start or
# render, so refuse to ship it rather than hope the search order works out.
if [[ -d "$APPDIR/usr/lib" ]]; then
    host_only=("${GPU_CANDIDATES[@]}" 'libEGL*' 'libGL*' 'libGLX*' 'libgbm*' 'libdrm*')
    find_args=(-name "${host_only[0]}")
    for pattern in "${host_only[@]:1}"; do
        find_args+=(-o -name "$pattern")
    done
    if ! leaked=$(find -L "$APPDIR/usr/lib" -type f \( "${find_args[@]}" \) -print); then
        echo "build-appimage.sh: failed to scan $APPDIR/usr/lib" >&2
        exit 1
    fi
    if [[ -n $leaked ]]; then
        echo "build-appimage.sh: host graphics library bundled in usr/lib:" >&2
        sed 's/^/  /' <<<"$leaked" >&2
        exit 1
    fi
fi

for required in AppRun "$APP_ID.desktop" "$APP_ID.svg"; do
    if [[ ! -e "$APPDIR/$required" ]]; then
        echo "build-appimage.sh: linuxdeploy did not create $APPDIR/$required" >&2
        exit 1
    fi
done

produced=$(ls -t "$OUT"/*.AppImage 2>/dev/null | head -n 1 || true)
if [[ -z "$produced" ]]; then
    echo "build-appimage.sh: no .AppImage found in $OUT" >&2
    exit 1
fi
echo "$produced"
