#!/usr/bin/env bash
# Pack the directory build-zip.sh staged into a per-user Windows installer with
# NSIS. Run build-zip.sh first; nothing is compiled or downloaded here.
set -euo pipefail

usage() {
    cat <<'USAGE'
Usage: build-installer.sh [output-dir]

  output-dir   directory holding stage/ from build-zip.sh, and where the
               installer goes (default: $OUTPUT_DIR, else <target-dir>/windows)

Needs makensis (Debian package: nsis).
USAGE
}

fail() {
    echo "build-installer.sh: $*" >&2
    exit 1
}

if [[ ${1:-} == -h || ${1:-} == --help ]]; then
    usage
    exit 0
fi

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
ROOT=$(cd -- "$SCRIPT_DIR/../.." && pwd)
OUT=${1:-${OUTPUT_DIR:-${CARGO_TARGET_DIR:-$ROOT/target}/windows}}
OUT=$(cd -- "$OUT" 2>/dev/null && pwd) || fail "no output directory '$OUT' (run build-zip.sh first)"
STAGE=$OUT/stage
VERSION=$(sed -n 's/^version = "\([^"]*\)".*/\1/p' "$ROOT/Cargo.toml" | head -n 1)
[[ -n $VERSION ]] || fail "no workspace version in $ROOT/Cargo.toml"
INSTALLER=$OUT/Chewvy-$VERSION-windows-x86_64-setup.exe
FILELIST=$OUT/installer-files.nsh

command -v makensis >/dev/null || fail "makensis not found (install nsis)"
for required in chewvy.exe libmpv-2.dll chewvy.ico LICENSE.txt; do
    [[ -f $STAGE/$required ]] || fail "no $STAGE/$required (run build-zip.sh first)"
done

# The installer writes exactly these files and the uninstaller deletes exactly
# these, so both lists come from one listing of the staging directory.
{
    echo '!macro InstallFiles'
    find "$STAGE" -maxdepth 1 -type f -printf '%f\n' | LC_ALL=C sort |
        while read -r name; do printf '  File "%s"\n' "$STAGE/$name"; done
    echo '!macroend'
    echo '!macro UninstallFiles'
    find "$STAGE" -maxdepth 1 -type f -printf '%f\n' | LC_ALL=C sort |
        while read -r name; do printf '  Delete "$INSTDIR\\%s"\n' "$name"; done
    echo '!macroend'
} >"$FILELIST"

rm -f "$INSTALLER"
(cd -- "$SCRIPT_DIR" && makensis -V2 -NOCD \
    "-DVERSION=$VERSION" "-DSTAGE=$STAGE" "-DOUTFILE=$INSTALLER" "-DFILELIST=$FILELIST" \
    installer.nsi)
[[ -f $INSTALLER ]] || fail "makensis did not produce $INSTALLER"
echo "build-installer.sh: $(du -h "$INSTALLER" | cut -f1) installer"
printf '%s\n' "$INSTALLER"
