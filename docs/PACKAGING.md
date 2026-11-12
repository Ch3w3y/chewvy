# Packaging and releases

| Platform | File | Built by | Status |
|---|---|---|---|
| Linux x86_64 | `Chewvy-<version>-x86_64.AppImage` | `packaging/linux/build-appimage.sh` | built in a Debian 13 container; started and played its test clip headless on the development machine |
| Linux x86_64 | `install.sh` (installs the AppImage per user) | repository root | tested in Debian 13, Ubuntu 24.04, Arch and Alpine containers; see below |
| Windows x86_64 | `Chewvy-<version>-windows-x86_64-setup.exe` | `packaging/windows/build-installer.sh` | built; installed and uninstalled silently under Wine; **not run on real Windows** |
| Windows x86_64 | `Chewvy-<version>-windows-x86_64.zip` | `packaging/windows/build-zip.sh` | built; an earlier build was started in a Windows 11 virtual machine with software OpenGL |
| macOS arm64 | `Chewvy-<version>-macos-arm64.dmg` | `packaging/macos/build-app.sh` | **never built** |

Every release also carries `SHA256SUMS`. None of the packages is code-signed.
A Flatpak is planned and not started.

All of the scripts download nothing themselves: they are handed the tools and
libraries they need, and the release workflow fetches those at pinned versions
and checks their SHA-256 first.

## Linux AppImage

```sh
cargo build --release --locked -p plex-app
export LINUXDEPLOY=$HOME/bin/linuxdeploy-x86_64.AppImage
export APPIMAGE_EXTRACT_AND_RUN=1      # when FUSE is unavailable, as in a container
packaging/linux/build-appimage.sh      # prints the AppImage path
packaging/linux/build-appimage.sh --appdir-only   # only assemble target/appimage/AppDir
```

The script assembles an AppDir (the binary, the desktop entry, the icon, the
licence texts) and runs [linuxdeploy](https://github.com/linuxdeploy/linuxdeploy),
which copies in libmpv and everything it links: FFmpeg, libass, libplacebo and
their dependencies. It needs `python3` for the crate licence collection.

`packaging/linux/bundled-sources.sh target/appimage/AppDir` then lists the
Debian source package and version of every library that was copied in. A
release publishes that list beside the AppImage, because the bundled libraries
are GPL and LGPL code and the list is how a user finds their exact source.

**Not bundled, on purpose: the graphics stack.** The AppImage
[excludelist](https://github.com/AppImageCommunity/pkg2appimage/blob/master/excludelist),
which linuxdeploy applies by default, already leaves out `libEGL`, `libGL`,
`libGLX`, `libGLdispatch`, `libgbm`, `libdrm`, `libX11` and `libxcb`. The
Vulkan loader and the remaining `libwayland-*` libraries are not on that list,
so the script passes `--exclude-library` for them, and then fails the build if
any of those names, or any `libEGL*`, `libGL*`, `libgbm*` or `libdrm*`, ended
up in `AppDir/usr/lib`. These libraries belong to the driver installed on the
user's machine; a mismatched private copy is a classic cause of a window that
will not open. `libvulkan` is excluded so that a machine without Vulkan loses
only mpv's Vulkan paths rather than failing to start.

Release builds run in a `debian:trixie` container: Debian 13 is the oldest
Debian whose libmpv is new enough, and building on an older distribution keeps
the glibc version the AppImage requires as low as it can be (2.39 at
present). `NO_STRIP=1` is set because
linuxdeploy's bundled `strip` cannot read the `.relr.dyn` sections current
toolchains emit.

## Linux installer script

```sh
curl -fsSL https://raw.githubusercontent.com/Ch3w3y/chewvy/main/install.sh | sh
```

`install.sh` is POSIX `sh`. What it does, in order:

1. Refuses anything that is not Linux on x86_64 with glibc 2.39 or newer,
   saying which it found. (The release workflow fails if that number stops
   matching what the built AppImage actually requires.)
2. Downloads `SHA256SUMS` from the latest published release (or from
   `--version X.Y.Z`), and reads the AppImage's file name from it.
3. Downloads that AppImage to a temporary directory and compares its SHA-256
   with the listed one. On a mismatch it stops and installs nothing.
4. Moves the AppImage to `~/.local/bin/chewvy`, writes
   `~/.local/share/applications/io.github.ch3w3y.chewvy.desktop`, and copies
   the icon out of the verified AppImage to
   `~/.local/share/icons/hicolor/scalable/apps/`.

It never calls `sudo`, never writes outside the user's home, and its whole
body is one function called on the last line, so a truncated download cannot
run half of it. `--uninstall` removes those three files and leaves settings
and sign-ins alone, telling the user where they are.

`CHEWVY_BASE_URL` points it at another directory holding `SHA256SUMS` and the
AppImage, which is how it is tested: serve a locally built AppImage with
`python3 -m http.server` and run the script in a container.

What was run (2026-11-11, as an unprivileged user, against a locally built
0.1.0 AppImage):

| Check | Debian 13 (dash) | Ubuntu 24.04 (dash) | Arch (bash) | Alpine (busybox, musl) |
|---|---|---|---|---|
| piped install; launcher, menu entry and icon in place; checksum matches | pass | pass | pass | refused: no glibc |
| `--version` for an existing and a missing release | pass | pass | pass | |
| corrupted download is rejected and nothing installed | pass | pass | pass | |
| truncated script runs nothing | pass | pass | pass | |
| unsupported architecture is refused by name | pass | pass | pass | |
| `--uninstall` removes everything it installed | pass | pass | pass | |
| `wget` when `curl` is absent | pass | not run | not run | |
| installed AppImage executes (`chewvy --help`) once the desktop's graphics and audio libraries are present | pass | pass | not run | |

A bare container has no `libEGL`, so the AppImage cannot load there until
those libraries are installed; that is a property of the container, and the
script says nothing about it. No container had a display, so no window was
opened in any of them.

## Windows zip

```sh
# once, anywhere outside the repository
curl -fLO https://github.com/shinchiro/mpv-winbuild-cmake/releases/download/20261110/mpv-dev-x86_64-20261110-git-a61f6c3abd.7z
7z x -ompv-dev mpv-dev-x86_64-20261110-git-a61f6c3abd.7z
export MPV_DIR=$PWD/mpv-dev

packaging/windows/build-zip.sh                # cross-builds, prints the zip path
packaging/windows/build-zip.sh --skip-build   # re-assemble and re-verify only
```

It cross-builds `chewvy.exe` and `chewvy-cli.exe` for
`x86_64-pc-windows-gnu` and stages a flat directory: the two executables,
`libmpv-2.dll` from the development package, any MinGW runtime DLL the import
tables actually ask for, the icon and the licence texts. Host requirements:
the MinGW cross compiler and binutils (`gcc-mingw-w64-x86-64`,
`binutils-mingw-w64-x86-64`), the Rust target, `zip`, `unzip` and `python3`.

**The completeness rule.** After zipping, the script unzips the result and
reads the import table of every `.exe` and `.dll` in it. Each imported DLL
must be an in-box Windows 10/11 system DLL (an explicit list plus the
`api-ms-win-*` API sets), driver-provided, or present in the zip. Anything
else fails the build and names the file. A file whose imports cannot be read
is an error too, so the check cannot pass by accident.

**Not bundled: `vulkan-1.dll`.** The pinned `libmpv-2.dll` imports the Vulkan
loader eagerly. The loader is installed into `System32` by the graphics
driver
([Khronos](https://khronos.org/blog/vulkan-runtime-and-loader-security-on-windows-os),
accessed 2026-11-10); a private copy next to the executable could shadow the
driver's own. So a Windows machine with no Vulkan-capable driver cannot start
Chewvy at all, and the installer warns when it finds no loader.

`packaging/windows/smoke.ps1` is the check to run on a real Windows machine
from the unpacked directory: it confirms `chewvy-cli` runs and reports
(which proves every DLL resolved), then starts `chewvy.exe` in the
interactive session through a scheduled task and requires it to still be
running a few seconds later. It proves start-up, not rendering or playback.

## Windows installer

```sh
packaging/windows/build-zip.sh
packaging/windows/build-installer.sh    # needs makensis (Debian package: nsis)
```

`build-installer.sh` packs the staged directory with
[NSIS](https://nsis.sourceforge.io/) using `packaging/windows/installer.nsi`.
The installer:

- installs for the current user under `%LOCALAPPDATA%\Programs\Chewvy`, with
  no administrator prompt;
- adds a Start Menu shortcut and an entry in *Installed apps* with an
  uninstaller;
- warns if no Vulkan loader is present;
- on uninstall removes exactly the files it installed, the shortcut and its
  registry keys, and keeps `%APPDATA%\chewvy` (settings and sign-ins).

Silent install and uninstall work with `/S`. The installer is unsigned, so
Windows SmartScreen shows an "unrecognised app" warning.

The installer has been exercised under Wine only: `/S` installed the seven
files, the Start Menu shortcut and the registry entries; `chewvy-cli.exe`
ran from the installed directory; the uninstaller removed all of it. That is
evidence the script is sound, not a test on Windows. Nobody has run it on a
real Windows machine yet.

## macOS

```sh
brew install mpv pkgconf dylibbundler librsvg
cargo build --release --locked -p plex-app
packaging/macos/build-app.sh            # -> target/macos/Chewvy-<version>-macos-arm64.dmg
```

`build-app.sh` creates `Chewvy.app`, copies libmpv and its dependencies into
`Contents/Frameworks` with
[dylibbundler](https://github.com/auriamg/macdylibbundler), ad-hoc signs the
bundle (mandatory on Apple Silicon after the load commands are rewritten),
checks that no Homebrew path is left in the binary, and builds a compressed
DMG. The result has no Developer ID signature and is not notarised: a first
launch needs right-click > Open, or
`xattr -dr com.apple.quarantine /Applications/Chewvy.app`.

**No macOS build has been produced.** The project has no Mac hardware, so the
script has only ever been read, not run. The release workflow has a macOS job
for a GitHub-hosted runner, switched off unless the repository variable
`CHEWVY_MACOS_BUILD` is `true`.

## Releases

`.github/workflows/release.yml` runs when a `v*` tag is pushed (or by hand for
an existing tag). In one `debian:trixie` container on a self-hosted runner it:

1. checks that the tag equals the workspace version in `Cargo.toml`;
2. builds the AppImage, the Windows zip and the Windows installer;
3. writes the bundled-source list and `SHA256SUMS` over the files;
4. creates a **draft** GitHub Release with the four files attached.

A maintainer reads the draft, edits the notes from `CHANGELOG.md` and
publishes it. Until then `install.sh` does not see it.

To cut a release: update `version` in `Cargo.toml` and `CHANGELOG.md`, run
the gates, commit, tag `vX.Y.Z`, push the tag.

### Runners and pull requests

The workflows run on self-hosted runners labelled `chewvy` and have no
`pull_request` trigger. That is deliberate: a self-hosted runner executes
whatever the workflow in the triggering ref says, so a pull request from a
fork must never be able to start one. Keep it that way, and keep "Require
approval for all outside collaborators" switched on in the repository's
Actions settings.

### Licences in the packages

Each package contains `LICENSE`, `THIRD-PARTY-NOTICES.md` and
`THIRD-PARTY-LICENSES.txt`. The last is generated at package time by
`scripts/gen_third_party_licenses.py` from the licence files of exactly the
crates that target links.

### Icons

`packaging/linux/io.github.ch3w3y.chewvy.svg` is the source. The Windows icon
is generated from it:

```sh
rsvg-convert -w 256 -h 256 packaging/linux/io.github.ch3w3y.chewvy.svg -o icon.png
python3 -c "from PIL import Image; Image.open('icon.png').save('packaging/windows/chewvy.ico', sizes=[(16,16),(24,24),(32,32),(48,48),(64,64),(128,128),(256,256)])"
```
