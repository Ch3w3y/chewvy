# Changelog

All notable changes to Chewvy are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and versions follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.0] - 2026-11-14

### Added

- Updates from inside the app. Chewvy checks GitHub for a newer published
  release when it starts (Settings > Updates turns this off, and has "Check
  now"). A notice says when one is out; clicking it opens the update prompt,
  where "Update now" downloads the package this copy was installed from,
  refuses it unless it matches the release's `SHA256SUMS`, and installs it,
  showing the progress of each. "Restart now" then reopens Chewvy in the new
  version. This works for the AppImage, the Windows installer and zip, and
  the macOS app (the macOS one has not been tried yet); a copy built from
  source or installed by a package manager is told a release is out and
  where to get it. The notice also shows before any server is added.
- F11 enters and leaves full screen on every screen.

### Changed

- Home's cards and hero have separate targets: the show's name opens the
  show, the episode's name opens the episode, and the play button over the
  art plays or resumes it. A Continue Watching card's art still resumes it.
  A wide card's second line now names the episode.

### Fixed

- Starting the next episode, changing quality, switching to or from the HDR
  window, or leaving the player no longer drops the window out of full
  screen. While browsing in full screen, the window's middle button leaves it.

## [0.2.0] - 2026-11-14

### Added

- Watch together for Jellyfin items: a "Watch together" button on an item's
  page starts a SyncPlay group or joins one the server lists. In a group,
  play, pause and seek from anyone move everyone; the player waits for a
  member whose connection stalls, and someone joining mid-title starts where
  the group is. The player shows the group's members and state, and Leave
  gives back the usual controls and playback speed.
- Watch together for Plex films and episodes, through Plex's Watch Together
  service. "Watch together" on the title's page starts a room and invites
  people from the account's Home and friends; rooms for the title are listed
  there to join, and while the account is in any room a "Watch together"
  entry under the Plex server's libraries lists them all. Play, pause and
  seek move everyone, here and in Plex's web app. The player shows who is in
  the room and who is ready, and its room panel invites more people; Leave
  keeps playing alone and keeps the room on the list, and Remove takes it
  off. Joining a room whose title is on another of the account's servers
  connects to that server first. The HDR window follows a room as this
  window does, and a pause or seek made with its own keys moves everyone.
  Plex does not check that the people invited can reach the server.

### Fixed

- A request to Plex or to a Jellyfin server could fail once with "could not
  be reached" when it left on a connection the server had just closed for
  being idle. Such a request is now sent once more on a new connection.

## [0.1.4] - 2026-11-13

### Added

- An episode's page links to its show and its season, and a season's page to
  its show, above the title.
- In the HDR window, N (or Enter, or >) plays the next episode, and the
  up-next line is shown over the picture there.
- Jellyfin shows play on into the next episode, as Plex shows do, and from
  the last episode of a season into the first of the next.

### Fixed

- A Plex sign-in was asked for again at every start: the account's token was
  stored but could not be found again. A sign-in made with this version is
  kept; one made with an earlier version has to be made once more.
- "Open plex.tv/link" did nothing on Windows and macOS. It now opens the
  system's default browser on every platform.
- With the HDR window covering Chewvy's own window, the next episode did not
  start by itself, and the watched position stopped reaching the server,
  until Chewvy's window was shown again.

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
