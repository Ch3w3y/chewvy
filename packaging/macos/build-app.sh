#!/usr/bin/env bash
# Bundle the built chewvy binary into "Chewvy.app" plus a DMG.
# Ad-hoc codesigning is mandatory here, not cosmetic: on Apple Silicon the
# kernel kills any arm64 binary whose signature was invalidated by
# dylibbundler's install_name_tool rewrites.
set -euo pipefail

usage() {
    cat <<'EOF'
Usage: build-app.sh [binary [output-dir [version]]]

  binary       built chewvy binary    (default: target/release/chewvy)
  output-dir   bundle + DMG location  (default: target/macos)
  version      bundle/DMG version     (default: workspace version in Cargo.toml)
EOF
}

if [[ ${1:-} == -h || ${1:-} == --help ]]; then
    usage
    exit 0
fi

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
REPO_ROOT=$(cd -- "$SCRIPT_DIR/../.." && pwd)

BIN=${1:-target/release/chewvy}
OUT=${2:-target/macos}
VERSION=${3:-$(sed -n 's/^version = "\([^"]*\)".*/\1/p' "$REPO_ROOT/Cargo.toml" | head -n1)}

APP_NAME="Chewvy"
APP="$OUT/$APP_NAME.app"
EXE="$APP/Contents/MacOS/chewvy"
PLIST="$APP/Contents/Info.plist"
DMG="$OUT/Chewvy-${VERSION}-macos-arm64.dmg"

if [[ ! -f "$BIN" ]]; then
    echo "build-app.sh: no binary at $BIN (run: cargo build --release --locked -p plex-app --bin chewvy)" >&2
    exit 1
fi
if [[ -z "$VERSION" ]]; then
    echo "build-app.sh: no version: pass one as the third argument" >&2
    exit 1
fi
if ! command -v dylibbundler >/dev/null; then
    echo "build-app.sh: dylibbundler not found (brew install dylibbundler)" >&2
    exit 1
fi

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" "$APP/Contents/Frameworks"
cp "$BIN" "$EXE"

# Optional: never fail the build over the icon.
ICON_SRC="$REPO_ROOT/packaging/linux/io.github.ch3w3y.chewvy.svg"
ICON_FILE="__none__"
if [[ -f "$ICON_SRC" ]] && command -v rsvg-convert >/dev/null && command -v iconutil >/dev/null; then
    iconset=$(mktemp -d)
    for spec in "16x16:16" "16x16@2x:32" "32x32:32" "32x32@2x:64" \
        "128x128:128" "128x128@2x:256" "256x256:256" "256x256@2x:512" \
        "512x512:512" "512x512@2x:1024"; do
        rsvg-convert -w "${spec##*:}" -h "${spec##*:}" \
            -o "$iconset/icon_${spec%%:*}.png" "$ICON_SRC"
    done
    iconutil -c icns -o "$APP/Contents/Resources/chewvy.icns" "$iconset"
    rm -rf "$iconset"
    ICON_FILE="chewvy"
else
    echo "build-app.sh: warning: skipping icon (need $ICON_SRC, rsvg-convert and iconutil)" >&2
fi

cp "$REPO_ROOT/LICENSE" "$REPO_ROOT/THIRD-PARTY-NOTICES.md" "$APP/Contents/Resources/"
python3 "$REPO_ROOT/scripts/gen_third_party_licenses.py" \
    --target aarch64-apple-darwin --out "$APP/Contents/Resources/THIRD-PARTY-LICENSES.txt"

sed -e "s|@VERSION@|$VERSION|g" -e "s|@ICON_FILE@|$ICON_FILE|" "$SCRIPT_DIR/Info.plist" >"$PLIST"
if [[ "$ICON_FILE" == "__none__" ]]; then
    grep -v -e '<key>CFBundleIconFile</key>' -e "<string>$ICON_FILE</string>" "$PLIST" >"$PLIST.tmp"
    mv "$PLIST.tmp" "$PLIST"
fi

# Copy libmpv and friends into Frameworks and rewrite load commands so the
# binary finds them relative to itself instead of the Homebrew prefix.
dylibbundler -od -b -x "$EXE" -d "$APP/Contents/Frameworks/" -p '@executable_path/../Frameworks/'

codesign --force --deep --sign - "$APP"
codesign --verify --deep --strict "$APP"

if otool -L "$EXE" | grep -E '/opt/homebrew|/usr/local' >&2; then
    echo "build-app.sh: binary still references Homebrew paths (matched lines above)" >&2
    exit 1
fi

hdiutil create -volname "$APP_NAME" -srcfolder "$APP" -ov -format UDZO "$DMG"
echo "$DMG"
