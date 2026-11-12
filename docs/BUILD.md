# Building and testing

## Requirements

- **Rust 1.99.1.** `rust-toolchain.toml` pins it, so `rustup` selects it for
  you.
- **A C toolchain and `pkg-config`.**
- **libmpv with client API 2.5 or newer** (mpv 0.40 and later), with its
  headers. `crates/plex-player/build.rs` fails the build when it is missing or
  older, and says so.
- **Linux:** EGL, OpenGL, Wayland and xkbcommon development files; X11/xcb
  ones for X11 sessions.

### Linux

```sh
# Debian 13 (trixie)
sudo apt install build-essential pkg-config libclang-dev libmpv-dev \
  libegl-dev libgl-dev libgles-dev libgbm-dev libvulkan-dev \
  libx11-dev libx11-xcb-dev libxcb1-dev libxkbcommon-dev \
  libwayland-dev wayland-protocols

# Arch Linux
sudo pacman -S base-devel clang mpv wayland libxkbcommon mesa

cargo build --release --locked -p plex-app
target/release/chewvy
```

Debian 12 ships mpv 0.35, which is too old. The Debian list above is what the
release build installs; the Arch one is what the development machine uses.

### macOS

```sh
brew install mpv pkgconf
cargo build --release --locked -p plex-app
```

Homebrew's formula ships `mpv.pc`, so pkg-config finds it. This recipe is what
the project's macOS workflow runs on a GitHub-hosted Apple Silicon runner; the
result has not been run on a physical Mac.

### Windows

libmpv has no `.pc` file on Windows. Unpack a development package such as
[shinchiro's `mpv-dev-x86_64-*.7z`](https://github.com/shinchiro/mpv-winbuild-cmake/releases)
(`include/mpv/*.h`, `libmpv.dll.a`, `libmpv-2.dll`) and point `MPV_DIR` at it:

```powershell
7z x mpv-dev-x86_64-20261110-git-a61f6c3abd.7z -ompv
$env:MPV_DIR = "$PWD\mpv"
cargo build --release --locked -p plex-app
```

The build script reads the API version from `$MPV_DIR\include\mpv\client.h`
and links `-lmpv`, which resolves `libmpv.dll.a` with MinGW and `mpv.lib` with
MSVC. `libmpv-2.dll` must be next to `chewvy.exe` or on `PATH` at run time.
The release packages are cross-built from Linux instead; see
`docs/PACKAGING.md`.

`MPV_DIR` works on every platform and is consulted before pkg-config.

## Binaries

| Binary | Crate | Purpose |
|---|---|---|
| `chewvy` | `plex-app` | the application |
| `chewvy-cli` | `plex-cli` | `chewvy-cli status` prints a JSON report of the native libraries found at build time; `--require-native` exits 3 if one is missing |

`chewvy --diagnose-servers` prints a redacted report of the signed-in Plex
account's servers and how each connection attempt went, which is the first
thing to attach to a connection bug.

## Gates

Every change must pass five checks. They run offline, so fetch the crates
once first:

```sh
cargo fetch --locked
scripts/check.sh fmt      # cargo fmt --all -- --check
scripts/check.sh check    # cargo clippy --all-targets -- -D warnings
scripts/check.sh test     # cargo test --all-targets
scripts/check.sh build    # cargo build
scripts/xcheck.sh windows # clippy for x86_64-pc-windows-gnu, no linking
```

`scripts/env.sh` sets the environment they share: two build jobs, offline
cargo, and `target/` as the output directory. It uses a toolchain unpacked at
`.tools/toolchain` and a crate cache at `.cargo/` when those exist, and the
rustup toolchain otherwise.

`scripts/xcheck.sh windows` needs `clang`, `llvm-ar`, the MinGW-w64 headers
and the `x86_64-pc-windows-gnu` Rust target. It sets
`PLEX_DISABLE_MPV_GUARD=1`, which skips the libmpv version check so the
Windows target can be type-checked on a machine without a Windows libmpv. Do
not set that variable for a real build: nothing links without libmpv.

The tests need no network, no display and no media server. HTTP behaviour is
tested against local mock servers; the Jellyfin tests replay responses
captured from a disposable Jellyfin 12.4.0 server serving generated media.
Tests that need a real libmpv playback boundary are marked `#[ignore]` and run
with `cargo test -- --ignored`.

## Conventions

- `unsafe` is denied workspace-wide. It is allowed only in the libmpv and GL
  modules that already carry the `allow`, and every block has a `SAFETY:`
  comment.
- Clippy runs with `unwrap_used`, `expect_used` and `indexing_slicing` as
  warnings, and warnings are errors.
- Dependencies are pinned in the workspace `Cargo.toml` and frozen in
  `Cargo.lock`. After adding one, regenerate the licence tables:
  `python3 scripts/gen_dependencies.py --write docs/DEPENDENCIES.md`.
  A crate must offer a licence option outside the GPL family; the test in
  `crates/plex-core/tests/dependency_licensing.rs` enforces it.
- A token or password must never reach a log line, an error message, a URL or
  a process argument. The redaction tests fail if one does.
- The app must never change library metadata or server settings.
  `crates/plex-core/tests/mutation_surface.rs` pins every non-GET request in
  the source; a new one has to be added there deliberately.

## Regenerating the libmpv bindings

`crates/plex-player/src/mpv_bindings.rs` is committed bindgen output. To
regenerate it from the installed headers:

```sh
cargo build --locked --workspace --features plex-player/regenerate-mpv-bindings
```

It needs libclang, which the ordinary build does not.

## Useful scripts

| Script | What it does |
|---|---|
| `scripts/hwdec-probe.sh` | read-only report of what this machine's GPU, drivers and mpv can decode |
| `scripts/hdr-fixtures.sh` | generates short HDR10, HLG, HDR10+ and Dolby Vision clips and verifies them |
| `scripts/mpv-gl-harness.sh` | runs the render path headless on software OpenGL |
| `scripts/gen_dependencies.py` | regenerates the tables in `docs/DEPENDENCIES.md` |

Packaging is described in `docs/PACKAGING.md`.
