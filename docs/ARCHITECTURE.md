# Architecture

Chewvy is one desktop process: a winit window with an OpenGL context, libmpv
rendering video into that context, and an egui interface painted over it.
Everything that talks to a network or a media engine sits behind a crate
boundary that knows nothing about windows.

## Crates

| Crate | Binary | What it owns |
|---|---|---|
| `plex-core` | | The Plex side with no display dependency: client identity and `X-Plex-*` headers, the HTTP transport, the PIN sign-in state machine, server discovery and connection, library models, progress reports, token storage, secret redaction. |
| `jellyfin-core` | | The Jellyfin side, to the same rules: device identity and the `MediaBrowser` authorization header, server validation, Quick Connect, password sign-in, browsing, playback info, session reports, LAN discovery. |
| `plex-player` | | libmpv through FFI: the bindings, an asynchronous control loop (commands, observed properties, events), track selection, and the OpenGL render-context wrapper. |
| `plex-app` | `chewvy` | The window, the GL context, the interface, the player screen, the list of sources and the worker threads. |
| `plex-cli` | `chewvy-cli` | A display-free helper: `status` prints a JSON report of the native libraries the build found, and a mock player session exercises the control loop without a window. |

`plex-core` and `jellyfin-core` must never depend on a display or FFI crate;
`crates/plex-core/tests/workspace_contract.rs` enforces it. The crate directory
names predate the product name and are internal: nothing a user sees is called
`plex-*`.

`unsafe` is denied workspace-wide and allowed only in the FFI and GL modules
(`plex-player`'s libmpv link and render link, `plex-app`'s `gl/`), each block
with a `SAFETY:` comment.

## Window and rendering

- `crates/plex-app/src/shell/host.rs` creates the window and its GL context
  with glutin (EGL on Linux, WGL on Windows, CGL on macOS) and runs the winit
  event loop. A display that cannot provide OpenGL 2.1 / OpenGL ES 2.0 gets a
  worded error instead of a crash (`gl/support.rs`).
- Each frame, `player::render_behind` lets mpv draw the current video frame
  into the window's default framebuffer through mpv's OpenGL render API
  (`MPV_RENDER_API_TYPE_OPENGL`), then `egui_glow` paints the interface on
  top. Nothing is read back to the CPU, so hardware decoding and GPU scaling
  apply to what is shown.
- On Linux the window's native Wayland or X11 display handle is passed to mpv
  when the render context is created. That is what lets VA-API frames reach
  the GL context without a copy (`docs/HARDWARE_DECODE.md`).
- mpv draws its own subtitles (libass) into the same frame; the app does not
  rasterise subtitles itself.
- The interface is plain egui with a custom theme (`ui/theme.rs`); eframe and
  wgpu are not used. Artwork is decoded on worker threads, cached on disk under
  the cache directory, and uploaded as GL textures within a fixed GPU budget.

### The HDR window

mpv's render API has no `gpu-next` backend, and `gpu-next` is where Dolby
Vision reshaping, HDR10+ curves and real HDR output live. For HDR and Dolby
Vision items the same in-process mpv core can instead be started with
`vo=gpu-next`, which opens a window of its own with mpv's on-screen controls
(`player/hdr.rs`). The Chewvy window stays open as the remote control. This is
the one case where video leaves the app's own window.

The HDR window's keys are mpv's. The keys for the next episode are bound to
set an mpv property the app reads, and the up-next line is shown there with
mpv's own on-screen text.

A compositor draws no frame for a window it is not showing, and the HDR
window usually covers this one. The app's playback logic runs in its frame, so
the window host (`shell/host.rs`) runs the frame without presenting it when a
frame the app asked for has not been drawn for a quarter of a second. That is
what lets the next episode start, and the watched position reach the server,
while only the HDR window is on screen.

## Threads

- The UI thread owns the window, the GL context and the mpv core (a
  thread-local slot), so the code that draws a frame and the code that drives
  playback need no locks.
- Network calls never run on the UI thread. A worker thread per job family
  (`watch.rs` for Plex, `sources/worker.rs` for Jellyfin, `ui/images.rs` for
  artwork) takes commands over a channel and answers with events; each answer
  wakes the event loop.
- Destroying an mpv core can block on network teardown, so it happens off the
  UI thread.

## Sources

`sources.json` in the config directory lists what the app can play from: the
linked Plex account and any number of Jellyfin servers. It holds no secrets.
Every screen reads one item model (`plex_core::library::Item`); a Jellyfin row
carries its server inside its id (`jellyfin:{server}:{item}`), so a row can
always say which source to go back to (`source.rs`, `sources/mapping.rs`).

## Sign-in

### Plex

Plex account sign-in is the PIN flow; the app never sees a password.

1. `POST https://plex.tv/api/v2/pins` with the `X-Plex-*` identity headers
   returns a pin `id` and a four-character `code`.
2. The app shows the code and offers to open `https://plex.tv/link`.
3. It polls `GET https://plex.tv/api/v2/pins/{id}` once a second for up to two
   minutes. An answer carrying `authToken` ends the wait; anything else means
   a new code is needed.
4. `GET https://plex.tv/api/v2/user` confirms the account.
5. `GET https://plex.tv/api/v2/resources?includeHttps=1&includeRelay=1&includeIPv6=1`
   lists the account's servers. Each server is then contacted with **its own**
   `accessToken`, not the account token, trying connections in the order
   local, remote, relay; https before http; IPv4 before IPv6.
6. Sign-out is `DELETE https://plex.tv/api/v2/users/signout` plus deleting the
   stored token.

Plex publishes no specification for this flow (`docs/PLEX_AUTH.md` has the
detail and the open questions). The shapes above were
corroborated against python-plexapi's `myplex.py`
(<https://github.com/pkkid/python-plexapi/blob/master/plexapi/myplex.py>,
accessed 2026-11-03), a third-party client, and port 32400 as the server's
"primary port" comes from Plex's own container README
(<https://github.com/plexinc/pms-docker/blob/master/README.md>, accessed
2026-11-03). Progress reporting uses the server's `/:/timeline` and
`/:/scrobble` endpoints (`plex-core/src/progress.rs`).

Not handled: device-JWT sign-in (`clients.plex.tv`), Plex Home user switching,
managed-user restrictions, and more than one Plex account at a time.

### Jellyfin

Quick Connect where the server allows it, otherwise a username and password
sent once to the user's own server and never stored. Only the `MediaBrowser`
`Authorization` header is used; the legacy `X-Emby-*` headers and `api_key`
query parameter are not. `docs/JELLYFIN.md` has the endpoints and sources.

## Tokens and secrets

- A token lives in a `SecretString`, whose `Debug` and `Display` print a
  placeholder. Errors and log lines are built from redacted values;
  `crates/plex-core/tests/redaction.rs` and
  `crates/jellyfin-core/tests/redaction.rs` capture real flows and assert the
  token text never appears.
- Tokens are sent as request headers only. They are never put in a URL, so
  they stay out of mpv's logged command line too: mpv receives the header
  through `http-header-fields` for that one file.
- Storage is the XDG Secret Service on Linux (KDE Wallet, GNOME Keyring),
  one item per account or server. If no secret service is running, the app
  explains that the alternative is a `0600` file that is plaintext at rest and
  uses it only after the user agrees. On Windows and macOS the token file
  lives in the per-user application-data folder; the platform keychains are
  not used yet.
- Config lives in `$XDG_CONFIG_HOME/chewvy` (Linux), `%APPDATA%\chewvy`
  (Windows) or `~/Library/Application Support/chewvy` (macOS); the artwork
  cache in the matching cache directory. A folder left by a build from before
  the app was named Chewvy (`plex-rust-client`) is renamed on first start.

## Updates

`update.rs` reads `https://api.github.com/repos/Ch3w3y/chewvy/releases/latest`
(at start unless Settings turns it off, and on "Check now"). A draft or a
pre-release is never offered. The request carries only a `User-Agent` naming
the app and its version. An install downloads the package this copy was set
up from and the release's `SHA256SUMS`, over https only, and refuses a package
whose SHA-256 is not the listed one: the same check `install.sh` makes, and
the same trust in the GitHub release it implies.

How the package is put in place (`update/install.rs`):

- **AppImage** (`$APPIMAGE` is set): written beside the running file and
  renamed over it.
- **Windows installer** (`uninstall.exe` beside the program): Windows lets a
  running program's files be renamed but not overwritten, so the app renames
  its own `.exe` and `.dll` files to `*.old` and runs the new installer
  silently (`/S /D=<this folder>`) while it keeps running. A failed install
  renames them back; after a good one, any file the installer did not write
  again (a library the user put there) is renamed back too.
- **Windows zip**: unpacked with the `tar.exe` Windows ships; each file it
  replaces is renamed to `*.old` and the new one copied in.
- **macOS bundle**: the disk image is attached, its app copied beside this
  one with `ditto`, and the two swapped by renaming.

Either Windows case deletes the `*.old` files on the next start.

The last install step starts the new version once with `--help`. A new,
unsigned program's first start is when Windows Defender scans it: about ten
seconds for `chewvy.exe` in the test VM, against 0.15 s for a start after
that. Doing it under the install progress is what lets "Restart now" open the
new version at once.

Every install goes through the same screens (`ui/app/update.rs`), on every
screen except the player, sign-in included: a notice says a release is out; clicking it opens the update prompt; "Update now"
downloads, then installs, with the progress shown in the prompt (and in small
on the notice if the prompt is hidden); the prompt then offers "Restart now".
The restart closes the window and starts the new version straight away,
before the rest of the shutdown (`watch::run`).

Anything else (a build from source, a distribution package) gets the same
notice and prompt, which say why it cannot update itself and link to the
release page.

What has been run: the AppImage path end to end (the ignored `live_` test in
`update/tests.rs`), and both Windows paths in a Windows 11 virtual machine,
driven through the real screens with `PLEX_APP_DRIVE`: a 0.1.9 build of this
code, installed by the installer and unpacked from the zip, found the
published 0.2.0, downloaded, verified and installed it while running, and
"Restart now" started the published 0.2.0 `chewvy.exe` (same SHA-256). The
macOS path has not been run.

## What the app never does

- It never changes library metadata, deletes media or alters server settings.
  The only writes are playback progress and played state, and
  `crates/plex-core/tests/mutation_surface.rs` pins the exact set of non-GET
  requests the source contains.
- It never sends a token or password anywhere except the service it belongs to.
- Besides the user's own servers and Plex's account service, the only host
  it contacts is GitHub, for the update check and download.
