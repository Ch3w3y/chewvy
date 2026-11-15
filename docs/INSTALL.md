# Install Chewvy

Packages are on the [Releases](https://github.com/Ch3w3y/chewvy/releases)
page. Each release carries `SHA256SUMS` with the SHA-256 of every package.
[Capabilities](CAPABILITIES.md) has the details for each platform.

## Linux

### Install with the script

```sh
curl -fsSL https://raw.githubusercontent.com/Ch3w3y/chewvy/main/install.sh | sh
```

The script installs for the current user only and never asks for root. It
does these things, in order:

1. It refuses anything that is not Linux on x86_64 with glibc 2.39 or newer,
   and says which it found.
2. It downloads `SHA256SUMS` from the latest published release.
3. It downloads the AppImage and compares its SHA-256 with the listed one. On
   a mismatch it stops and installs nothing.
4. It puts the AppImage at `~/.local/bin/chewvy`, with a menu entry and icons
   under `~/.local/share`.

Read [`install.sh`](../install.sh) first if you prefer. To install one
specific release, pass its version:

```sh
curl -fsSL https://raw.githubusercontent.com/Ch3w3y/chewvy/main/install.sh | sh -s -- --version 0.3.0
```

### Install the AppImage by hand

1. Download `Chewvy-<version>-x86_64.AppImage` from the release page.
2. Make it executable: `chmod +x Chewvy-*.AppImage`.
3. Run it.

The AppImage bundles mpv and FFmpeg. It uses the graphics drivers of your
system.

### What Linux needs

- x86_64. No other architecture is built.
- glibc 2.39 or newer. Ubuntu 24.04, Debian 13, Fedora 40 and current rolling
  distributions qualify. Older releases and musl-based systems such as Alpine
  do not.
- A display that provides OpenGL 2.1 or OpenGL ES 2.0.

### Remove it

```sh
sh install.sh --uninstall
```

The script removes the launcher, the menu entry and the icons. It leaves your
settings and sign-ins in `~/.config/chewvy` and tells you so.

## Windows

1. Download `Chewvy-<version>-windows-x86_64-setup.exe` from the release page.
2. Run it. Windows SmartScreen warns about an unrecognised app because the
   installer is not code-signed. Choose **More info**, then **Run anyway**.

The installer needs no administrator rights. It installs under
`%LOCALAPPDATA%\Programs\Chewvy`, adds a Start Menu entry, and adds an
uninstaller to **Installed apps**.

A portable `Chewvy-<version>-windows-x86_64.zip` is also provided. Unpack it
anywhere and run `chewvy.exe`.

A graphics driver with Vulkan support must be installed, because the bundled
mpv needs `vulkan-1.dll`. The installer warns when it finds none.

The uninstaller removes the files it installed and keeps `%APPDATA%\chewvy`,
which holds your settings and sign-ins.

## macOS

The macOS build is experimental. Nobody has run it on a physical Mac.

1. Download `Chewvy-<version>-macos-arm64.dmg` from the release page.
2. Open it and drag Chewvy to Applications.
3. Open Chewvy. macOS refuses the first launch because the app is not
   notarised by Apple. Allow it under **System Settings > Privacy & Security >
   Open Anyway**, or clear the download flag:

```sh
xattr -dr com.apple.quarantine /Applications/Chewvy.app
```

It needs an Apple Silicon Mac that runs macOS 15 or newer.

## Check a download

Download `SHA256SUMS` from the same release and compare:

```sh
sha256sum -c SHA256SUMS --ignore-missing     # Linux
shasum -a 256 Chewvy-*.dmg                   # macOS: compare with the line in SHA256SUMS
```

```powershell
Get-FileHash .\Chewvy-0.3.0-windows-x86_64-setup.exe -Algorithm SHA256   # Windows
```

## Update

Chewvy checks GitHub for a newer published release when it starts. A notice
says when one is out.

1. Click the notice. The update prompt opens.
2. Click **Update now**. Chewvy downloads the package this copy was installed
   from, refuses it unless it matches the release's `SHA256SUMS`, and
   installs it.
3. Click **Restart now**.

**Settings > Updates** turns the check off and has **Check now**.

In-app updates work for the AppImage, the Windows installer and the Windows
zip. The macOS path exists and has not been tried. A copy that you built from
source, or that a package manager installed, shows the same notice with a
link to the release page.

## Build from source

See [Building and testing](BUILD.md).

## Where Chewvy keeps its files

| Platform | Settings and the list of sources | Artwork cache |
|---|---|---|
| Linux | `$XDG_CONFIG_HOME/chewvy`, usually `~/.config/chewvy` | `$XDG_CACHE_HOME/chewvy`, usually `~/.cache/chewvy` |
| Windows | `%APPDATA%\chewvy` | `%LOCALAPPDATA%\chewvy` |
| macOS | `~/Library/Application Support/chewvy` | `~/Library/Caches/chewvy` |

Sign-in tokens are kept in the system keyring on Linux. See
[Questions and answers](FAQ.md#where-are-my-sign-in-tokens-kept).
