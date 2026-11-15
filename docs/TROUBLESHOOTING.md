# Troubleshooting

Find the symptom, then follow the steps. If a problem is not here, read
[the capability matrix](CAPABILITIES.md) for what has been tested, then
[report a bug](../CONTRIBUTING.md).

## Collect a log first

Chewvy writes its log to standard error. A sign-in token never appears in a
log line.

1. Start Chewvy from a terminal: `~/.local/bin/chewvy` on Linux.
2. For more detail, set the level: `RUST_LOG=debug ~/.local/bin/chewvy`.
3. Attach the output to your bug report.

For a problem with a Plex connection, also run:

```sh
chewvy --diagnose-servers
```

The command prints a redacted report of the servers of the signed-in Plex
account, and how each connection attempt went.

## Chewvy does not start

### Linux: `install.sh` refuses the system

The script names what it found. Chewvy is built for x86_64 with glibc 2.39 or
newer. Ubuntu 24.04, Debian 13, Fedora 40 and current rolling distributions
qualify. On an older or musl-based system, [build from source](BUILD.md).

### Linux: the AppImage fails with a missing library

Releases 0.1.1 and 0.1.3 fixed a missing `libjack.so.0` and a crash in X11
sessions. Install the current release. If a library is still missing, the
message names it. Report the name and your distribution.

### Linux: an error about OpenGL

Chewvy needs a display that provides OpenGL 2.1 or OpenGL ES 2.0, and says so
when it finds none. Install the graphics drivers for your GPU. In a virtual
machine, turn on 3D acceleration or install a software OpenGL driver.

### Windows: SmartScreen blocks the installer

The installer is not code-signed. Choose **More info**, then **Run anyway**.
Compare the download with `SHA256SUMS` first if you want to be sure of it.
See [Check a download](INSTALL.md#check-a-download).

### Windows: `vulkan-1.dll` was not found

The bundled mpv loads the Vulkan loader, which a graphics driver installs.
Install or update the driver for your GPU.

### macOS: "Chewvy cannot be opened"

The app is not notarised. Allow it under **System Settings > Privacy &
Security > Open Anyway**, or run:

```sh
xattr -dr com.apple.quarantine /Applications/Chewvy.app
```

The macOS build has not been run on a physical Mac. Reports are welcome.

## Sign-in

### Plex asks for a new code at every start

Versions before 0.1.4 stored the Plex token and could not find it again.
Update, then sign in once more.

### The Plex code runs out

Chewvy waits two minutes for a code to be entered at plex.tv/link. After
that, click **Get a new code**.

### A Jellyfin server is not found on the network

Type its address instead, for example `http://192.168.1.20:8096`. Discovery
only finds servers that answer on the local network.

### Quick Connect is not offered

Quick Connect must be turned on in the Jellyfin server's settings. Otherwise
choose **Username and password**. Chewvy sends the password once to your
server and does not store it.

### Chewvy asks whether it may store tokens in a file

No secret service is running, so there is no keyring to use. KDE Wallet and
GNOME Keyring both provide one. If you agree, Chewvy stores tokens in a file
that only your user can read. The file is not encrypted.

### "Could not be reached"

1. Check that the server is up and that this computer can reach it.
2. For Plex, run `chewvy --diagnose-servers` and read which address failed.
3. Check the server's certificate. Chewvy has no option to accept a
   certificate that does not verify, so such a server counts as unreachable.

Versions before 0.2.0 could show this error once after the server closed an
idle connection. Update if you see it only now and then.

## Playback

### A file does not play

When mpv cannot play the original file, Chewvy reopens the item at the same
position as a server remux, then as a 1080p 20 Mbps transcode, then stops
with an error that names the last step. You can also choose a step yourself
from the quality menu in the player.

Press **I** in the player to see what is playing and how it is decoded.

### Video stutters or the fans run hard

1. Press **I** and check whether decoding is in hardware.
2. Check that **Settings > Playback > Hardware decoding** is on.
3. If the codec is one that only decodes in software here, choose a
   transcode from the quality menu. [The capability matrix](CAPABILITIES.md)
   lists which codecs decoded in hardware on the development machine.

### Colours look wrong on a Dolby Vision file

Dolby Vision profile 5 needs Dolby Vision processing, which the main window
does not have. The player offers the HDR window for such a file. Turn on
**Settings > HDR & Dolby Vision > Open in the HDR window** to move there
automatically.

### HDR looks washed out or dim

The main window tone-maps HDR to SDR. Real HDR output is in the HDR window,
and needs a compositor with colour management, such as KDE Plasma 6.4 or
newer on Wayland, with HDR turned on for the display. HDR output on Windows
and macOS is untested.

### No Dolby Atmos or DTS:X at the receiver

Chewvy always decodes audio to PCM. Bitstream passthrough is not supported.

### Subtitles sit under the control bar

mpv draws subtitles at the bottom of the picture, and the control bar covers
them while it is showing. The bar hides after a few seconds without input.

### External subtitles are missing on a Jellyfin title

Chewvy does not fetch external subtitle files from a Jellyfin server yet.
Embedded text tracks work.

### Playback does not start and the picture stays empty

If no first frame arrives within 30 seconds, Chewvy counts the attempt as
failed and moves to the next step: a server remux, then a transcode.

In about 130 headless test runs, 4 stopped producing frames just after a file
was requested. The cause is not established. If you see it, please report it
with a log.

## Watch together

### Someone I invited cannot play the title

Plex does not check that the people you invite can reach the server. Share
the library with them in Plex first.

### Playback speed and Next are missing

Both are switched off in a room and in a SyncPlay group, so that everyone
stays in step.

## Updates

### "Update now" is not offered

A copy built from source, or installed by a package manager, cannot replace
itself. The prompt links to the release page instead.

### The update fails its check

Chewvy refuses a download whose SHA-256 is not the one in the release's
`SHA256SUMS`. Try again later. If it keeps failing, download the package from
the release page and compare it by hand.

## Reset Chewvy

1. Sign out of each source under **Settings > Sources > Manage sources**.
2. Close Chewvy.
3. Delete the settings folder and the cache folder listed in
   [Where Chewvy keeps its files](INSTALL.md#where-chewvy-keeps-its-files).
