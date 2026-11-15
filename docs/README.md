# Chewvy documentation

Start with the [project README](../README.md) for what Chewvy is and how to
get it. The pages below go further.

## Use Chewvy

| Page | What it answers |
|---|---|
| [Install Chewvy](INSTALL.md) | How to install, update and remove it on Linux, Windows and macOS |
| [Keyboard shortcuts](KEYBOARD.md) | Every key the app answers, by screen |
| [Troubleshooting](TROUBLESHOOTING.md) | What to do when it will not start, sign in or play |
| [Questions and answers](FAQ.md) | Cost, privacy, formats, platforms |
| [Capability matrix](CAPABILITIES.md) | What has been measured, where, and what nobody has tried |

## Build and change Chewvy

| Page | What it covers |
|---|---|
| [Building and testing](BUILD.md) | Requirements per platform, the gates a change must pass, conventions |
| [Architecture](ARCHITECTURE.md) | Crates, threads, rendering, sign-in, tokens, updates |
| [Packaging and releases](PACKAGING.md) | AppImage, `install.sh`, Windows installer and zip, macOS disk image, icons |
| [Dependencies and licences](DEPENDENCIES.md) | Every crate and native component with its licence |
| [Contributing](../CONTRIBUTING.md) | How to report a bug and send a change |
| [Security](../SECURITY.md) | How to report a security problem |

## Protocol and decoding notes

These record what the code relies on, with sources and open questions.

| Page | What it covers |
|---|---|
| [Plex sign-in and server discovery](PLEX_AUTH.md) | The PIN flow, headers, tokens |
| [Plex Watch Together](PLEX_WATCH_TOGETHER.md) | Rooms, the sync socket, measured timing |
| [Jellyfin integration](JELLYFIN.md) | Sign-in, browsing, playback, progress, SyncPlay |
| [Hardware decoding and video output](HARDWARE_DECODE.md) | Why OpenGL, codec coverage per API, HDR and Dolby Vision |

## Plans

| Page | What it covers |
|---|---|
| [Mobile port plan](MOBILE_PORT.md) | A plan for Android and iOS. No work on it has started. |

## Screenshots and the website

The screenshots in `docs/media/` and the website in `site/` are made by
`site/tools/capture.sh` from a demo library of freely licensed films.
[`site/CREDITS.md`](../site/CREDITS.md) lists every film and image with its
source and licence. [`site/DEPLOY.md`](../site/DEPLOY.md) says how the website
is published.
