# Changelog

All notable changes to Chewvy are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and versions follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
