# Changelog

All notable changes to Chewvy are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and versions follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.3] - 2026-11-12

### Added

- The first start asks whether to add a Plex account or a Jellyfin server,
  instead of going straight to a Plex sign-in code. A Jellyfin server on its
  own is enough to use the app.
- A source switcher at the top of the sidebar: every Plex and Jellyfin server,
  switching between a Plex account's servers, and adding a source.
- Sources can add a Plex account as well as a Jellyfin server.
- Search covers Jellyfin servers as well as the Plex server.
- The Plex and Jellyfin marks identify each source on the first-start screen,
  the switcher, Sources and the home cards, in place of lettered squares.

### Changed

- Signing out of Plex no longer starts a new Plex sign-in by itself, and a
  Plex sign-in code can be left with Back.

### Fixed

- The hero and the detail page named the connected Plex server, with the
  Plex mark, for an item that came from a Jellyfin server.
- The Sources screen scrolls, and Settings and Sources show a scroll bar when
  they are taller than the window.
- The AppImage crashed at start in an X11 session, including under Xwayland,
  on systems whose libxkbcommon is a different version from the bundled one.
  Its X11 companion library is now bundled with it.

## [0.1.2] - 2026-11-12

### Added

- An experimental macOS disk image for Apple Silicon (macOS 15 or newer),
  built on a GitHub-hosted runner. It has not been run on a physical Mac.

### Changed

- The Plex test fixtures hold invented titles, people and identifiers.

## [0.1.1] - 2026-11-12

### Fixed

- The AppImage failed to start on systems without `libjack.so.0`, which
  includes a default Ubuntu 24.04 desktop. The library is now bundled.

## [0.1.0] - 2026-11-12

First public version.

### Added

- Plex account sign-in with a `plex.tv/link` code, server discovery and
  connection over local, remote and relay addresses.
- Jellyfin servers as a second kind of source: LAN discovery, Quick Connect
  and password sign-in, any number of servers.
- Home screen with Continue Watching and recently added shelves, library
  grids, film and show detail pages with seasons, episodes and cast, and
  search across sources.
- Playback through libmpv inside the app window, with hardware decoding,
  audio and subtitle track selection, playback speed, skip markers and next
  episode.
- Older formats measured and played directly: MPEG-2, MPEG-4 part 2 and VP8
  video; transport streams, program streams, AVI and MOV; MP2, DTS core,
  TrueHD, 24-bit PCM and ALAC audio.
- Quality menu for both kinds of source: Direct Play, Direct Stream and
  server transcode presets.
- Automatic fallback: an item mpv cannot play reopens as a server remux, then
  as a transcode.
- A separate HDR window for HDR10, HLG, HDR10+ and Dolby Vision.
- Watch progress reporting for both kinds of source, with a switch to turn it
  off.
- Token storage in the system keyring on Linux.
- Packages: Linux AppImage with a per-user `install.sh`, Windows installer
  and portable zip. A macOS DMG script exists but no build has been produced.

### Known limitations

- Windows has not been run on a real GPU; macOS has not been built.
- The fallback to a server stream has not been run against a real Plex Media
  Server.
- One Plex account at a time.
