#!/bin/sh
# Chewvy installer for Linux.
#
#   curl -fsSL https://raw.githubusercontent.com/Ch3w3y/chewvy/main/install.sh | sh
#   curl -fsSL .../install.sh | sh -s -- --version 0.1.0
#   sh install.sh --uninstall
#
# Installs the AppImage for the current user only: nothing here needs or asks
# for root. The download is checked against the release's SHA256SUMS before
# anything is installed.
#
# Environment:
#   CHEWVY_BASE_URL  directory URL holding SHA256SUMS and the AppImage
#                    (a mirror or a local test server) instead of GitHub
#   XDG_BIN_HOME     where the launcher goes   (default ~/.local/bin)
#   XDG_DATA_HOME    menu entry and icon root  (default ~/.local/share)
set -eu

# Everything runs from main, called on the last line, so a download cut off
# part-way cannot execute half of this script.
main() {
    repo="Ch3w3y/chewvy"
    app_id="io.github.ch3w3y.chewvy"
    version=""
    uninstall=0

    while [ $# -gt 0 ]; do
        case "$1" in
            --version)
                [ $# -ge 2 ] || die "--version needs a value, for example --version 0.1.0"
                version="${2#v}"
                shift
                ;;
            --version=*) version="${1#--version=}"; version="${version#v}" ;;
            --uninstall) uninstall=1 ;;
            -h | --help) usage; exit 0 ;;
            *) usage >&2; die "unknown option: $1" ;;
        esac
        shift
    done

    [ -n "${HOME:-}" ] || die "HOME is not set, so there is no user to install for"
    bin_dir="${XDG_BIN_HOME:-$HOME/.local/bin}"
    data_dir="${XDG_DATA_HOME:-$HOME/.local/share}"
    target="$bin_dir/chewvy"
    desktop="$data_dir/applications/$app_id.desktop"
    icon="$data_dir/icons/hicolor/scalable/apps/$app_id.svg"

    if [ "$uninstall" -eq 1 ]; then
        do_uninstall
        exit 0
    fi

    os="$(uname -s)"
    [ "$os" = "Linux" ] || die "this installer is for Linux; $os has its own package on the releases page"
    machine="$(uname -m)"
    case "$machine" in
        x86_64 | amd64) arch="x86_64" ;;
        *) die "Chewvy has no build for $machine yet; only x86_64 is published" ;;
    esac
    # The AppImage is linked against glibc; say so before downloading it.
    min_glibc="2.39"
    libc="$(getconf GNU_LIBC_VERSION 2>/dev/null || true)"
    case "$libc" in
        "glibc "*)
            version_at_least "${libc#glibc }" "$min_glibc" ||
                die "Chewvy needs glibc $min_glibc or newer; this system has ${libc#glibc }"
            ;;
        *) die "Chewvy needs a glibc-based Linux (glibc $min_glibc or newer); this system reports none" ;;
    esac
    if [ "$(id -u)" -eq 0 ]; then
        say "note: running as root installs Chewvy for the root user only ($target)"
    fi

    if command -v curl >/dev/null 2>&1; then
        fetch() { curl -fsSL --proto '=https,http' -o "$2" "$1"; }
    elif command -v wget >/dev/null 2>&1; then
        fetch() { wget -q -O "$2" "$1"; }
    else
        die "curl or wget is needed to download Chewvy"
    fi
    if command -v sha256sum >/dev/null 2>&1; then
        sha256() { sha256sum "$1" | cut -d ' ' -f 1; }
    elif command -v shasum >/dev/null 2>&1; then
        sha256() { shasum -a 256 "$1" | cut -d ' ' -f 1; }
    else
        die "sha256sum or shasum is needed to verify the download"
    fi

    if [ -n "${CHEWVY_BASE_URL:-}" ]; then
        base="${CHEWVY_BASE_URL%/}"
    elif [ -n "$version" ]; then
        base="https://github.com/$repo/releases/download/v$version"
    else
        base="https://github.com/$repo/releases/latest/download"
    fi

    work="$(mktemp -d "${TMPDIR:-/tmp}/chewvy-install.XXXXXX")"
    trap 'rm -rf "$work"' EXIT INT TERM

    say "fetching the release checksums"
    fetch "$base/SHA256SUMS" "$work/SHA256SUMS" ||
        die "could not download $base/SHA256SUMS (is that release published?)"

    # The checksum file names the AppImage, which is how "latest" learns its version.
    if [ -n "$version" ]; then
        asset="Chewvy-$version-$arch.AppImage"
    else
        asset="$(sed -n "s/^[0-9a-f]\{64\}[ *]*\(Chewvy-[0-9A-Za-z.+-]*-$arch\.AppImage\)\$/\1/p" "$work/SHA256SUMS" | head -n 1)"
        [ -n "$asset" ] || die "SHA256SUMS lists no $arch AppImage"
    fi
    expected="$(sed -n "s/^\([0-9a-f]\{64\}\)[ *]*$asset\$/\1/p" "$work/SHA256SUMS" | head -n 1)"
    [ -n "$expected" ] || die "SHA256SUMS has no entry for $asset"

    say "downloading $asset"
    fetch "$base/$asset" "$work/$asset" || die "could not download $base/$asset"
    actual="$(sha256 "$work/$asset")"
    if [ "$actual" != "$expected" ]; then
        die "checksum mismatch for $asset: expected $expected, got $actual. Nothing was installed."
    fi
    say "checksum verified"

    mkdir -p "$bin_dir" "$(dirname "$desktop")" "$(dirname "$icon")"
    chmod 755 "$work/$asset"
    # Take the icon out of the verified AppImage; a system that cannot run it
    # here (no exec on the temp directory) still gets a working install.
    if (cd "$work" && "./$asset" --appimage-extract "usr/share/icons/hicolor/scalable/apps/$app_id.svg" >/dev/null 2>&1) &&
        [ -f "$work/squashfs-root/usr/share/icons/hicolor/scalable/apps/$app_id.svg" ]; then
        cp "$work/squashfs-root/usr/share/icons/hicolor/scalable/apps/$app_id.svg" "$icon"
    else
        say "note: could not read the icon from the AppImage; the menu entry will use a generic one"
    fi
    # Move into place last, in one step, so an interrupted run leaves no half-written program.
    mv "$work/$asset" "$bin_dir/.chewvy.new"
    mv "$bin_dir/.chewvy.new" "$target"

    cat >"$desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Chewvy
GenericName=Media Player
Comment=Browse and play your Plex and Jellyfin libraries
Exec="$target"
Icon=$app_id
Terminal=false
Categories=AudioVideo;Video;Player;
Keywords=media;video;film;tv;plex;jellyfin;
StartupWMClass=$app_id
EOF
    if command -v update-desktop-database >/dev/null 2>&1; then
        update-desktop-database "$(dirname "$desktop")" >/dev/null 2>&1 || true
    fi

    say "installed ${asset%-"$arch".AppImage} to $target"
    case ":${PATH:-}:" in
        *":$bin_dir:"*) say "start it from your applications menu, or run: chewvy" ;;
        *) say "start it from your applications menu, or run: $target"
           say "($bin_dir is not on your PATH)" ;;
    esac
    if ! command -v fusermount3 >/dev/null 2>&1 && ! command -v fusermount >/dev/null 2>&1; then
        say "note: no FUSE helper was found. If Chewvy does not start, install fuse3 or run it with APPIMAGE_EXTRACT_AND_RUN=1"
    fi
}

usage() {
    cat <<'EOF'
Usage: install.sh [--version X.Y.Z] [--uninstall]

  --version X.Y.Z   install that release instead of the latest
  --uninstall       remove the launcher, menu entry and icon
  -h, --help        this help

Installs for the current user only. Nothing is installed unless the download
matches the release's SHA256SUMS.
EOF
}

do_uninstall() {
    removed=0
    for path in "$target" "$desktop" "$icon"; do
        if [ -e "$path" ]; then
            rm -f "$path"
            say "removed $path"
            removed=1
        fi
    done
    [ "$removed" -eq 1 ] || say "Chewvy is not installed for this user"
    if command -v update-desktop-database >/dev/null 2>&1; then
        update-desktop-database "$(dirname "$desktop")" >/dev/null 2>&1 || true
    fi
    say "your settings and sign-ins were left in ${XDG_CONFIG_HOME:-$HOME/.config}/chewvy"
    say "and the artwork cache in ${XDG_CACHE_HOME:-$HOME/.cache}/chewvy; delete them to remove everything"
}

# Whether "major.minor" $1 is at least $2.
version_at_least() {
    have_major="${1%%.*}"
    have_minor="${1#*.}"
    have_minor="${have_minor%%.*}"
    want_major="${2%%.*}"
    want_minor="${2#*.}"
    [ "$have_major" -gt "$want_major" ] ||
        { [ "$have_major" -eq "$want_major" ] && [ "$have_minor" -ge "$want_minor" ]; }
}

say() { printf 'chewvy: %s\n' "$*"; }

die() {
    printf 'chewvy: error: %s\n' "$*" >&2
    exit 1
}

main "$@"
