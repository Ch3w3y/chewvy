# Third-party notices

Chewvy itself is licensed under the GNU General Public License, version 3 or
later (`LICENSE`). It is built from, and its packages contain, work by other
people under the licences below.

## Bundled artwork and fonts

These are compiled into the `chewvy` binary.

| Work | Author | Licence | Notes |
|---|---|---|---|
| Backdrop photograph "Aurora" | Russo Francesco | [CC BY 3.0](https://creativecommons.org/licenses/by/3.0/) | Source: <https://commons.wikimedia.org/wiki/File:Aurora_(140959963).jpeg>. **Modified:** cropped to 16:9, resized, colour-graded toward violet, darkened and blurred (`crates/plex-app/assets/backdrop/`). |
| Plus Jakarta Sans typeface | The Plus Jakarta Sans Project Authors | [SIL Open Font License 1.1](crates/plex-app/assets/fonts/OFL-PlusJakartaSans.txt) | Source: <https://github.com/tokotype/PlusJakartaSans>. Unmodified. |
| Jellyfin icon | The Jellyfin project | [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/) | Source: <https://github.com/jellyfin/jellyfin-ux>. Unmodified; identifies Jellyfin sources. |
| Interface icons | Chewvy contributors | GPL-3.0-or-later | Drawn for this project (`crates/plex-app/assets/icons/`). |

## Trademarks and logos

The following marks appear in the app only to label the format or age rating
of a title, or to say which service a library belongs to. They are the
property of their owners, who do not sponsor or endorse Chewvy. The image
files come from Wikimedia Commons, where they are marked as public-domain
text logos; trademark rights are unaffected by that. Sources are listed in
`crates/plex-app/assets/marks/SOURCES.md`.

- Plex wordmark: Plex, Inc.
- Dolby Vision, Dolby Atmos, Dolby Digital, Dolby Digital Plus, Dolby TrueHD:
  Dolby Laboratories
- DTS, DTS-HD Master Audio: DTS, Inc.
- HDR10 and HDR10+ logos: their respective owners (HDR10+ Technologies, LLC
  for HDR10+)
- BBFC age rating symbols: British Board of Film Classification

Chewvy is an independent project, not affiliated with Plex, Inc. or the
Jellyfin project.

## Libraries in the packages

The Chewvy program links libmpv at run time. Each package carries a copy of
libmpv and of what libmpv itself needs.

### Linux AppImage

Copied by linuxdeploy from Debian 13 (trixie) packages at build time. The
principal components:

| Component | Licence |
|---|---|
| mpv (`libmpv.so.2`) | GPL-2.0-or-later |
| FFmpeg (`libavcodec`, `libavformat`, `libavfilter`, `libavutil`, `libswscale`, `libswresample`) | GPL-2.0-or-later as configured by Debian (LGPL-2.1-or-later parts included) |
| libplacebo | LGPL-2.1-or-later |
| libass | ISC |
| the roughly 190 other libraries those depend on (for example dav1d, x264, x265, libvpx, libaom, Opus, libjxl, zimg, Rubber Band, the VA-API and VDPAU loaders, the PulseAudio client, OpenSSL, GnuTLS and Lua) | each under its own licence, as recorded in the Debian package's `copyright` file |

The complete list is the contents of `usr/lib` inside the AppImage
(`./Chewvy-*.AppImage --appimage-extract`). Graphics drivers (EGL, OpenGL,
Vulkan, Mesa, libdrm), the display libraries (Wayland, X11) and the usual
desktop base (fontconfig, FreeType, ALSA, PipeWire) are deliberately not
bundled and come from your system.

Corresponding source for every Debian-built library is available from Debian.
Each release carries `Chewvy-<version>-linux-bundled-sources.txt`, which names
the Debian source package and exact version behind every bundled library; that
version can be fetched from
`https://snapshot.debian.org/package/<source package>/<version>/`.

### Windows installer and zip

| Component | Licence |
|---|---|
| `libmpv-2.dll`: mpv with FFmpeg, libplacebo, libass and their dependencies linked in, from [shinchiro/mpv-winbuild-cmake](https://github.com/shinchiro/mpv-winbuild-cmake) | GPL-3.0-or-later as a whole (FFmpeg is configured with `--enable-gpl --enable-version3`) |
| MinGW-w64 runtime DLLs, when the import tables require them | Zope Public License 2.1 and the GCC Runtime Library Exception |

Corresponding source: the library is taken unmodified from the
mpv-winbuild-cmake release named in `.github/workflows/release.yml`
(`MPV_DEV_URL`), and that repository holds the scripts that built it. Those
scripts pin mpv to the commit in the file name but fetch most dependencies at
the head of their upstream branch on the build date, so the revisions are read
from the library itself: for Chewvy 0.1.0 it reports mpv
`v0.42.0-19-ga61f6c3ab` (<https://github.com/mpv-player/mpv>) and FFmpeg
`N-128718-gb2b543c07` (<https://github.com/FFmpeg/FFmpeg>). The other
libraries linked into it are the upstream sources listed in that repository's
`packages/` directory as of the release date.

### macOS disk image

No macOS package has been published. The packaging script copies libmpv and
its dependencies from Homebrew, whose `mpv` formula is GPL-2.0-or-later and
whose `ffmpeg` formula is GPL-3.0-or-later.

## Rust crates

The `chewvy` and `chewvy-cli` binaries are statically linked with the Rust
crates listed in `docs/DEPENDENCIES.md`, Table A, each under the licence shown
there (most are `MIT OR Apache-2.0`). Every package includes
`THIRD-PARTY-LICENSES.txt`, generated at build time by
`scripts/gen_third_party_licenses.py`, with the licence and copyright texts
those crates ship.

`egui`'s default font data (`epaint_default_fonts`) includes fonts under the
SIL Open Font License 1.1 and the Ubuntu Font Licence 1.0.

## Source code for Chewvy

The source for any released version is the matching tag at
<https://github.com/Ch3w3y/chewvy>. If you received a Chewvy binary without
its source, that address is the written offer: the complete corresponding
source for the version you have is available there at no charge.
