#!/usr/bin/env bash
# Cross-build chewvy.exe + chewvy-cli.exe for x86_64-pc-windows-gnu and assemble
# a distributable zip: the two exes, the runtime libmpv-2.dll from an unpacked
# libmpv development package, every mingw runtime DLL their PE imports demand,
# the icon and the licence texts. Nothing is downloaded here. The staging
# directory it leaves behind is what build-installer.sh packs.
set -euo pipefail
# DLL names are ASCII and Windows compares them case-insensitively; keep the
# lowercasing below byte-wise regardless of the machine locale.
export LC_ALL=C

usage() {
    cat <<'EOF'
Usage: build-zip.sh [options] [binary-dir [output-dir]]

  binary-dir    directory holding chewvy.exe and chewvy-cli.exe
                (default: $BIN_DIR, else <target-dir>/x86_64-pc-windows-gnu/release)
  output-dir    where the staging dir and the zip go
                (default: $OUTPUT_DIR, else <target-dir>/windows)

Options:
  --skip-build  skip cargo build; assemble and verify from binary-dir

Environment:
  MPV_DIR       unpacked libmpv development package in the shinchiro
                mpv-dev-x86_64-*.7z layout (include/mpv/*.h + libmpv.dll.a +
                libmpv-2.dll). Required: it is both what the build links for
                -lmpv and where the runtime libmpv-2.dll comes from.
  MINGW_DLL_DIR colon-separated extra directories searched for mingw runtime
                DLLs, ahead of the built-in candidates
  OBJDUMP       PE-capable objdump to read import tables with
                (default: x86_64-w64-mingw32-objdump if present, else objdump)
  CARGO_TARGET_DIR target directory the exes come from and whose windows/
                subdirectory receives the zip (default: ./target)
  CARGO_BUILD_JOBS parallelism for the cross-build (default: 2)
EOF
}

fail() {
    echo "build-zip.sh: $*" >&2
    exit 1
}

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
ROOT=$(cd -- "$SCRIPT_DIR/../.." && pwd)
TARGET=x86_64-pc-windows-gnu
TARGET_TRIPLE_DIR=${CARGO_TARGET_DIR:-$ROOT/target}
BIN_DIR=${BIN_DIR:-$TARGET_TRIPLE_DIR/$TARGET/release}
OUT=${OUTPUT_DIR:-$TARGET_TRIPLE_DIR/windows}
SKIP_BUILD=0

pos=()
while [[ $# -gt 0 ]]; do
    case $1 in
        --skip-build) SKIP_BUILD=1 ;;
        -h | --help)
            usage
            exit 0
            ;;
        --)
            shift
            pos+=("$@")
            break
            ;;
        -*)
            echo "build-zip.sh: unknown option: $1" >&2
            usage >&2
            exit 2
            ;;
        *) pos+=("$1") ;;
    esac
    shift
done
if [[ ${#pos[@]} -gt 0 ]]; then
    BIN_DIR=${pos[0]}
    if [[ ${#pos[@]} -gt 1 ]]; then OUT=${pos[1]}; fi
fi
# Relative paths are resolved here so that the later `cd` into the stage dir
# cannot change what the produced zip path refers to. On a fresh checkout the
# build has not created the binary dir yet, so it must exist to be resolved.
if [[ $SKIP_BUILD -eq 0 ]]; then mkdir -p -- "$BIN_DIR"; fi
BIN_DIR=$(cd -- "$BIN_DIR" 2>/dev/null && pwd) ||
    fail "no binary directory '$BIN_DIR' (build it first, or pass one)"
OUT=$(mkdir -p "$OUT" && cd -- "$OUT" && pwd)
STAGE=$OUT/stage
UNPACKED=$OUT/unpacked
VERSION=$(sed -n 's/^version = "\([^"]*\)".*/\1/p' "$ROOT/Cargo.toml" | head -n 1)
[[ -n $VERSION ]] || fail "no workspace version in $ROOT/Cargo.toml"
ZIP=$OUT/Chewvy-$VERSION-windows-x86_64.zip
APP_EXE=$BIN_DIR/chewvy.exe
CLI_EXE=$BIN_DIR/chewvy-cli.exe

if [[ $SKIP_BUILD -eq 0 ]]; then
    # Two jobs keep the build machine responsive; scripts/env.sh sets the same
    # default, but this script also runs standalone in CI.
    export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-2}
    if [[ -z ${MPV_DIR:-} ]]; then
        fail "MPV_DIR is not set: point it at an unpacked libmpv dev package \
(the shinchiro mpv-dev-x86_64-*.7z layout); the build needs libmpv.dll.a and \
this script takes libmpv-2.dll from the same package"
    fi
    echo "build-zip.sh: cargo build --release --locked --target $TARGET (jobs=${CARGO_BUILD_JOBS})"
    (cd -- "$ROOT" &&
        cargo build --release --locked --target "$TARGET" -p plex-app -p plex-cli)
fi

# ---------------------------------------------------------------------------
# What goes into the zip
# ---------------------------------------------------------------------------
MPV_DLL=$(find "${MPV_DIR:-/nonexistent}" -maxdepth 3 -iname 'libmpv-2.dll' -type f \
    -print 2>/dev/null | head -n 1 || true)
if [[ -z $MPV_DLL ]]; then
    fail "no libmpv-2.dll under ${MPV_DIR:-<MPV_DIR unset>}: MPV_DIR must name an \
unpacked libmpv development package"
fi

# In-box Windows system DLLs (Windows 10/11 x64). Anything imported that is not
# in-box and not bundled next to the exes is a packaging error. The list covers
# what a real libmpv-2.dll import table names (measured from
# mpv-dev-x86_64-20261110-git-a61f6c3abd.7z) plus what winit/glutin, the Rust
# std and the win32 bindings use; api-*/ext-* API sets are matched by prefix.
INBOX_SYSTEM_DLLS="
advapi32.dll avicap32.dll avrt.dll bcrypt.dll bcryptprimitives.dll
cfgmgr32.dll combase.dll comctl32.dll comdlg32.dll crypt32.dll d2d1.dll dbghelp.dll
dwmapi.dll dwrite.dll gdi32.dll gdi32full.dll imm32.dll iphlpapi.dll
kernel32.dll kernel.appcore.dll msvcrt.dll ncrypt.dll normaliz.dll nsi.dll
ntdll.dll ole32.dll oleaut32.dll oleacc.dll opengl32.dll powrprof.dll
propsys.dll psapi.dll rpcrt4.dll secur32.dll setupapi.dll shcore.dll
shell32.dll shlwapi.dll synchronization.dll twinapi.appcore.dll user32.dll
userenv.dll uxtheme.dll version.dll winhttp.dll wininet.dll winmm.dll
wldap32.dll ws2_32.dll ws2help.dll wtsapi32.dll
"

# Supplied by the GPU driver, not by Windows: the Vulkan loader only lands in
# System32 when a Vulkan-capable driver is installed, and libmpv-2.dll hard-links
# it (measured: it is an ordinary import, the delay-import directory is empty).
# Never bundled - a private copy can shadow the driver's own loader and then find
# no device. Its presence is proven on the real machine by the GUI smoke test.
DRIVER_DLLS="vulkan-1.dll"

declare -A INBOX=()
for name in $INBOX_SYSTEM_DLLS; do INBOX[$name]=1; done
declare -A DRIVER=()
for name in $DRIVER_DLLS; do DRIVER[$name]=1; done

lower() { tr 'A-Z' 'a-z'; }

is_system_dll() {
    local name=$1
    [[ -n ${INBOX[$name]:-} ]] && return 0
    [[ -n ${DRIVER[$name]:-} ]] && return 0
    case $name in
        api-ms-win-* | ext-ms-win-*) return 0 ;;
    esac
    return 1
}

pick_objdump() {
    local candidate
    for candidate in ${OBJDUMP:-} x86_64-w64-mingw32-objdump objdump; do
        if command -v "$candidate" >/dev/null 2>&1; then
            OBJDUMP=$(command -v "$candidate")
            return 0
        fi
    done
    fail "no objdump found (need one that reads PE32+; install binutils for the \
mingw target or set OBJDUMP)"
}

# `objdump -p` prints one "DLL Name: X.dll" line per imported DLL in the PE
# import (and delay-import) tables. A caller has to capture the output with
# $(...): inside a process substitution a failure would be hidden in the subshell
# and an unreadable binary would look like it imports nothing.
pe_imports() {
    local file=$1 out status=0
    out=$("$OBJDUMP" -p -- "$file" 2>/dev/null |
        sed -n 's/^[[:space:]]*DLL Name:[[:space:]]*//p' | lower | sort -u) || status=$?
    # Every PE links at least ntdll/kernel32, so a failed parse or an empty list
    # means nothing was read and the check would pass for the wrong reason.
    if ((status != 0)) || [[ -z $out ]]; then
        return 1
    fi
    printf '%s\n' "$out"
}

# Echo the import list of a PE file, or stop the build.
import_list() {
    local file=$1 out
    out=$(pe_imports "$file") || fail "cannot read the PE import table of '$file' \
with '$OBJDUMP' (not a PE32+ file, or this objdump does not read the format; \
set OBJDUMP to a mingw binutils objdump)"
    printf '%s\n' "$out"
}

dll_search_dirs() {
    local extra=()
    IFS=':' read -r -a extra <<<"${MINGW_DLL_DIR:-}"
    local dirs=("${extra[@]}")
    if command -v rustc >/dev/null 2>&1; then
        local sysroot
        sysroot=$(rustc --print sysroot 2>/dev/null || true)
        if [[ -n $sysroot ]]; then
            # Rust ships the mingw CRT objects (and, with the rust-mingw
            # component, the runtime DLLs) under the target's rustlib dir;
            # std-<hash>.dll lives there too when std is linked dynamically.
            dirs+=("$sysroot/lib/rustlib/$TARGET/bin"
                "$sysroot/lib/rustlib/$TARGET/lib"
                "$sysroot/lib/rustlib/$TARGET/lib/self-contained")
        fi
    fi
    dirs+=(/usr/x86_64-w64-mingw32/bin /usr/share/mingw-w64/bin
        /usr/lib/gcc/x86_64-w64-mingw32)
    local globbed
    for globbed in /usr/lib/gcc/x86_64-w64-mingw32/*/; do
        dirs+=("${globbed%/}")
    done
    local dir
    for dir in "${dirs[@]}"; do
        [[ -d $dir ]] && printf '%s\n' "$dir"
    done
}

# Echo the first directory that holds a file with this (already lowercase) name.
find_source_dll() {
    local name=$1 dir hit
    while read -r dir; do
        [[ -z $dir ]] && continue
        hit=$(find "$dir" -maxdepth 1 -iname "$name" -type f -print 2>/dev/null |
            head -n 1 || true)
        if [[ -n $hit ]]; then
            printf '%s\n' "$hit"
            return 0
        fi
    done < <(dll_search_dirs)
    return 1
}

# ---------------------------------------------------------------------------
# Assemble
# ---------------------------------------------------------------------------
rm -rf "$STAGE"
mkdir -p "$STAGE"
for required in "$APP_EXE" "$CLI_EXE"; do
    [[ -f $required ]] ||
        fail "no '$required' (cargo build --release --target $TARGET produces it, \
or pass a binary directory)"
    install -m 0755 "$required" "$STAGE/"
done
install -m 0755 "$MPV_DLL" "$STAGE/"

pick_objdump

declare -A BUNDLED=()
declare -A QUEUED=()
pending=()

bundled_note() { printf 'build-zip.sh: %s\n' "$*"; }

bundled_add() {
    local name
    name=$(basename -- "$1" | lower)
    BUNDLED[$name]=1
}
for staged in "$STAGE"/*; do bundled_add "$staged"; done

queue() {
    local name=$1
    [[ -n ${QUEUED[$name]:-} ]] && return 0
    QUEUED[$name]=1
    pending+=("$name")
}

queue_imports() {
    local file=$1 name
    while read -r name; do
        [[ -n $name ]] && queue "$name"
    done <<<"$(import_list "$file")"
}

for staged in "$STAGE"/*; do queue_imports "$staged"; done

# Every import that is neither in-box nor already staged has to be a mingw
# runtime DLL (or another bundled DLL's own dependency): copy it in and walk its
# imports too, until the closure stops growing.
round=0
index=0
while ((index < ${#pending[@]})); do
    name=${pending[index]}
    index=$((index + 1))
    round=$((round + 1))
    if ((round > 200)); then
        fail "dependency closure did not settle after $round imports: a DLL cycle \
or an unexpected import graph"
    fi
    [[ -n ${BUNDLED[$name]:-} ]] && continue
    is_system_dll "$name" && continue
    src=$(find_source_dll "$name") || {
        echo "build-zip.sh: cannot satisfy import '$name' (needed by the staged \
exes/DLLs or their dependencies). Looked in:" >&2
        dll_search_dirs | sed 's/^/  /' >&2
        echo "  Put the directory that holds it in MINGW_DLL_DIR." >&2
        exit 1
    }
    install -m 0755 "$src" "$STAGE/"
    bundled_note "bundled $(basename -- "$src") for import '$name'"
    bundled_add "$src"
    queue_imports "$STAGE/$(basename -- "$src")"
done

# The icon the shortcuts use, and the licences of everything in the package.
install -m 0644 "$SCRIPT_DIR/chewvy.ico" "$STAGE/"
install -m 0644 "$ROOT/LICENSE" "$STAGE/LICENSE.txt"
install -m 0644 "$ROOT/THIRD-PARTY-NOTICES.md" "$STAGE/"
python3 "$ROOT/scripts/gen_third_party_licenses.py" \
    --target "$TARGET" --out "$STAGE/THIRD-PARTY-LICENSES.txt"

# ---------------------------------------------------------------------------
# Zip, then check what was actually zipped
# ---------------------------------------------------------------------------
rm -f "$ZIP"
rm -rf "$UNPACKED"
mkdir -p "$UNPACKED"
(cd -- "$STAGE" && find . -maxdepth 1 -type f -printf '%P\n' |
    zip -X -q "$ZIP" -@)
unzip -q "$ZIP" -d "$UNPACKED"

declare -A UNPACKED_NAMES=()
for file in "$UNPACKED"/*; do
    UNPACKED_NAMES[$(basename -- "$file" | lower)]=1
done

problems=0
files_checked=0
dlls_bundled=0
for file in "$UNPACKED"/*; do
    base=$(basename -- "$file" | lower)
    case $base in
        *.exe | *.dll) ;;
        *) continue ;;
    esac
    files_checked=$((files_checked + 1))
    if [[ $base != *.exe ]]; then dlls_bundled=$((dlls_bundled + 1)); fi
    while read -r name; do
        [[ -z $name ]] && continue
        is_system_dll "$name" && continue
        if [[ -z ${UNPACKED_NAMES[$name]:-} ]]; then
            echo "build-zip.sh: '$base' imports '$name', which is neither an \
in-box Windows DLL nor bundled in the zip" >&2
            problems=$((problems + 1))
        fi
    done <<<"$(import_list "$file")"
done
if ((problems > 0)); then
    echo "build-zip.sh: $problems unresolved import(s); the zip would not start" >&2
    exit 1
fi

size=$(du -h "$ZIP" | cut -f1)
bundled_note "$files_checked PE file(s) in the zip, $dlls_bundled runtime DLL(s), \
every non-system import present, zip $size"
printf '%s\n' "$ZIP"
