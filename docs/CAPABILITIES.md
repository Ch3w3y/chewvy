# Capability matrix

What Chewvy has been seen to do, where, and what nobody has tried. A row says
"yes" only when it was measured; "expected" means the libraries document it
and nothing more. Chewvy does not claim to play every codec.

Decoding is done by mpv and FFmpeg. What they can decode depends on the build
bundled in the package and, for hardware decoding, on the GPU and its driver.
`docs/HARDWARE_DECODE.md` has the per-API detail and sources.

## Where it has run

| Platform | Built | Started | Played video |
|---|---|---|---|
| Linux x86_64, KDE Plasma on Wayland, AMD Radeon RX 9070 XT, Mesa | yes | yes | yes |
| Linux, X11 session | yes | yes, through Xwayland | yes, through Xwayland; a native X11 session with EGL is not re-measured |
| Linux, Intel or NVIDIA graphics | yes | not tried | not tried |
| Windows 11 x86_64, virtual machine, Mesa llvmpipe software OpenGL | yes, cross-built | yes: the window process stays up | not tried |
| Windows on a real GPU | yes, cross-built | **not tried** | **not tried** |
| macOS 15 arm64, GitHub-hosted runner, software OpenGL | yes | yes: the window opened and drew 60 frames | **not tried** |
| macOS on a physical Mac | yes, on the runner | **not tried** | **not tried** |

The Windows virtual-machine run proves that every DLL the executables import
resolves and that the app survives start-up. It does not prove that the window
drew anything, and it says nothing about hardware decoding or sign-in there.

## Video: direct play

Measured on the Linux development machine with mpv 0.41 and FFmpeg 9.0
(`hwdec=auto-safe`), using generated clips. Generated clips are easier to
decode than real films, so the frame rates are a lower bound on difficulty,
not a promise.

| Codec and shape | Result | How it was measured |
|---|---|---|
| H.264, 8-bit, 1080p | plays, hardware decoded (`vaapi`, zero-copy) | through the app's own render path |
| HEVC Main10, 1080p | plays, hardware decoded (`vaapi`, zero-copy) | through the app's own render path |
| H.264, 8-bit, 3840x2160 | decodes in hardware (201 fps) | mpv on the same host, no display |
| HEVC Main10, AV1 10-bit, VP9 10-bit, 3840x2160 | decode in hardware (109, 91 and 113 fps) | mpv on the same host, no display, `vaapi-copy` |
| AV1 and VP9, 1080p | decode in hardware | mpv on the same host, no display |
| MPEG-2, 720x576 and 1920x1080, progressive | plays, decoded in software | through the app, from a Jellyfin test server |
| MPEG-4 part 2, 1280x720 | plays, decoded in software | through the app, from a Jellyfin test server |
| VP8, 1280x720 | plays, decoded in software | through the app, from a Jellyfin test server |
| WMV8 (`wmv2`), 1280x720 | plays in mpv, decoded in software | mpv on the same host, no display |
| 10-bit H.264 | **unmeasured** | |
| Frames larger than 4096x2160 | **unmeasured** | |
| Interlaced MPEG-2 | **unmeasured** | |
| VC-1/WMV3, MJPEG, VVC, ProRes | **unmeasured**. FFmpeg decodes these in software, so they are expected to play, but no row here has been run. FFmpeg has no VC-1 encoder, so no test clip could be generated. | |

Containers the measured files were read from: MKV, MP4, MOV, WebM, MPEG
transport streams (`.ts`, `.m2ts`), MPEG program streams (`.mpg`) and AVI;
WMV/ASF in mpv alone. Disc structures (DVD `VIDEO_TS`, Blu-ray `BDMV`, ISO
images) are not supported.

The older formats were measured with `scripts/format-fixtures.sh`: 16
generated clips, each played for 5 seconds by mpv with no dropped frames, then
14 of them played through the app from a Jellyfin 12.4.0 test server.

## Audio

Decoded in software by FFmpeg and played through the system audio server.

| Codec | Result |
|---|---|
| AAC, MP3, FLAC (stereo and 5.1), AC-3, E-AC-3, Opus, Vorbis | play, measured per clip |
| MP2, DTS core 5.1, Dolby TrueHD 5.1, 24-bit PCM, ALAC | play, measured per clip |
| DTS-HD MA, DTS:X, TrueHD with Atmos, WMA, other PCM layouts | **unmeasured**. FFmpeg has no DTS-HD encoder, so no test clip could be generated; whether its lossless extension decodes here is unknown. |
| What reaches the speakers | **unmeasured**: the clips prove decoding, with the audio output discarded. Multichannel audio is downmixed or passed on by mpv's defaults. |
| Bitstream passthrough to a receiver (Dolby Atmos, DTS:X) | **not supported**: audio is always decoded to PCM |

## Subtitles

| Kind | Result |
|---|---|
| Embedded text tracks (SRT, ASS) selected in the player | rendered by mpv/libass; a styled ASS track drew with its position, colour, outline and italics |
| Image-based tracks (PGS, VobSub) | **unmeasured**: no test clip could be generated |
| External subtitle files listed by a Plex server | loaded beside the file and offered as tracks; VobSub pairs are skipped |
| External subtitle files on a Jellyfin server | **not fetched** |
| Server-side subtitle burn-in | **not requested** |

## HDR and Dolby Vision

| Case | Result |
|---|---|
| HDR10, HLG in the main window | tone-mapped to SDR by mpv |
| Dolby Vision profile 8 in the main window | plays through its HDR10-compatible base layer, tone-mapped |
| Dolby Vision profile 5 in the main window | plays, but colours are wrong without Dolby Vision processing; the player offers the HDR window, and moves there by itself when that setting is on |
| HDR10, HLG, HDR10+, Dolby Vision 5 and 8 in the HDR window (`vo=gpu-next`) | plays with Dolby Vision reshaping; HDR output on a colour-managed Wayland compositor (KDE Plasma 6.4 or newer). Detection of each format was verified with generated fixtures (`scripts/hdr-fixtures.sh`). Light output at the display has **not** been measured. |
| Dolby Vision profile 7 enhancement layer | needs mpv 0.42 or newer; **unmeasured** |
| HDR output on Windows or macOS | **unmeasured** |
| A television's own Dolby Vision mode | **not possible** from a desktop window: the picture is processed here and sent as HDR10 |

## By source

### Plex

| Capability | State |
|---|---|
| Direct Play (the original file) | default for every item. Whatever mpv can decode plays; nothing is checked against a profile first. |
| Direct Stream (server remuxes to HLS) | available from the quality menu |
| Server transcode | available from the quality menu: 1080p at 20, 12 and 8 Mbps; 720p at 4 and 2 Mbps; 480p; 240p |
| Automatic fallback when direct play fails | yes: the item reopens at the position it reached as a server remux, then as the 1080p 20 Mbps transcode, then stops with an error naming the last step. "Fails" means mpv ended the file with an error, dropped a video or audio track it could not decode, or showed no first frame within 30 seconds. Not detected: a decoder that starts and then fails on every frame, and a picture that decodes with wrong colours. **Measured against a mock server only** (unreadable file, undecodable video, undecodable audio, nothing served); what a real Plex Media Server's remux does for each case is unmeasured. |
| Resume position, watched state, progress reports | yes, through `/:/timeline` and `/:/scrobble`; can be switched off |
| Audio and subtitle track choice | yes, applied in the player |
| Watch together (Plex Watch Together) | yes, for films and episodes: start a room from the title's page and invite Home users and friends, join a room listed there or under the server's libraries, invite more from the room's panel in the player, leave it, or remove it from the list. Joining a room whose title is on another of the account's servers connects to that server first; a room on a server the account does not have is listed without Join. Play, pause and seek from anyone move everyone, here and in Plex's web app; someone joining a room under way starts where it is. Plex does not check that the people invited can reach the server. A room is one title for three hours. Playback speed and the next-episode hand-off are off in a room. The HDR window follows a room, and a pause or seek made with its own keys moves everyone. Plex's other apps no longer offer the feature. **Measured against Plex's live service** with disposable servers: a pause, play or seek reaches the other side 0.1 to 1 s later, from Chewvy or from Plex's web app, because each sends a change with its once-a-second answer to the service; against a scripted participant Chewvy held within 0.18 s (median under 0.01 s), and a Windows and a Linux Chewvy played 0.04 s apart. Plex's web app ran a steady 0.95 s ahead and left it, inside its own looser tolerance; see [PLEX_WATCH_TOGETHER.md](PLEX_WATCH_TOGETHER.md) 4. |
| Several Plex accounts, Plex Home users | **not implemented** |
| Music and photo libraries | **not implemented**: films and television only |

### Jellyfin

Tested against Jellyfin 12.4.0. The app sends a device profile that names only
the measured shapes above, and the server's answer to that profile decides.

| Capability | State |
|---|---|
| Direct play of H.264 8-bit, HEVC, AV1 and VP9 up to 10-bit, up to 4096x2160, in MKV, MP4, MOV or WebM with the audio codecs measured above | yes |
| Direct play of MPEG-2 up to 1920x1080 (also in transport and program streams), VP8 up to 1280x720, and MPEG-4 part 2 up to 1280x720 in AVI | yes. A codec is claimed only in the containers it was played from. |
| Anything outside that profile | not handed to mpv on faith: the server answers it not direct-playable and its own HLS stream plays instead. Seen with WMV8 in ASF, which arrived as the server's H.264 stream. A DTS-HD file carries the codec name `dts`, so it is handed over as it sits. |
| Server remux and transcode | yes, automatically as above and from the quality menu. A remux copied 3840x2160 HEVC into MPEG-TS segments; transcodes are H.264, sized by the preset. Unmeasured: AC-3/E-AC-3 copy, more than two audio channels, subtitles in a stream, HDR sources, a server with transcoding disabled. |
| Automatic fallback when direct play fails | yes, the same ladder as Plex; each step was run against a Jellyfin 12.4.0 test server. A failure on the client that the server then repairs was not reproduced. |
| HDR and Dolby Vision items inside those ceilings | not excluded by the profile, so they are direct-played and handled like a Plex HDR item. **No HDR item has been played from a Jellyfin server yet.** |
| Resume position and progress reports | yes (`/Sessions/Playing`, `Progress`, `Stopped`); can be switched off |
| Items with several versions | the first version the server calls direct-playable is used; there is no picker |
| Quick Connect and password sign-in, LAN discovery, several servers | yes |
| Used on its own, with no Plex account | yes: the app opens on the server's libraries. Sign-in from the first-start screen was run with Quick Connect against a Jellyfin 12.4.0 test server; the password path from that screen was not. |
| Search | yes: films, shows and episodes by title (`GET /Items?searchTerm=`), shown as one group per server after the Plex server's groups. People are not searched. |
| Watch together (SyncPlay) | yes: start or join a group from an item's page; play, pause and seek from anyone move everyone; a member that stalls holds the group until it catches up; a member that joins mid-title starts where the group is. One title at a time: no shared queue, repeat, shuffle, next or previous, and no chat. Playback speed is not offered in a group. **Measured on a Jellyfin 12.4.0 test server** against a scripted second member (within 0.05 s, 0.11 s just after moving to the HDR window) and against the Jellyfin web client in a headless browser (within 0.21 s, the web client's own lag included); see [JELLYFIN.md](JELLYFIN.md) 6.6. |
| Live TV, music | **not implemented** |

## Unverified behaviour

Things the code does on purpose without knowing how a real server or libmpv
build answers. Source comments marked **[U]** point here.

### Plex account and server discovery

- **`resources` JSON shape.** Plex publishes no specification; a bare array,
  `MediaContainer.media-server` and `mediaServer` are all accepted, anything
  else is a shape error. Fields that the reference client does not list
  (`ownedBy`, `machineIdentifier`, `httpsEnabled`, `reachable`, a connection
  `secure` flag) are never read.
- **Shared and managed users.** Whether an account token is authorised on a
  server shared with the account is not verified; connecting to a shared
  server is an explicit path. Plex Home user switching is not implemented.
- **Relay connections, `*.plex.direct` certificates.** A relay candidate is
  tried last like any other address. No insecure-TLS or pinning option exists:
  a certificate that does not verify counts as unreachable.
- **What a "probe" is.** The app sends `GET /` without credentials, then
  `GET /identity` with that server's own token. No source names the request
  Plex's own clients use.

### Transport behaviour

- **Redirects are refused.** Any 3xx answer is returned as an error before a
  body is read, and a token is never re-sent to a redirect target (tested
  against a mock). Whether a real server ever redirects these routes is
  unknown.
- **HTTP 429 is surfaced once.** No retry, no backoff, no `Retry-After`
  handling, on every route.
- **A request the server never read is sent once more.** A server closes a
  kept-alive connection it has left idle (Plex's room service after 20 s),
  and a request that leaves at that moment gets no answer. It is sent again
  once, on a new connection, whatever its verb; a request that does get an
  answer is never repeated. Tested against a host that closes the connection
  unanswered. A server that acts on a request and then drops the connection
  before answering would see it twice; none has been observed to.

### Library and metadata shapes

- Field names come from recorded server answers; parsing is tolerant, and an
  absent field is `None` rather than a guess. Paging uses
  `X-Plex-Container-Start` and `X-Plex-Container-Size` with at most 200 rows.

### libmpv control acceptance

- **Which playback event is the earliest that accepts a control** (seek,
  pause, volume) is not established. Controls issued straight after
  `loadfile` were answered `MPV_ERROR_COMMAND` in display-free runs, so the
  wrapper queues every control until `MPV_EVENT_FILE_LOADED`, with
  `PLAYBACK_RESTART` as a fallback flush, and reports any rejection.
  `cargo test -p plex-player --test ffi_native_session -- --ignored`
  re-measures the earlier orders.
- **Track selection names.** `audio-id` and `sub-id` were rejected by libmpv
  in a display-free run on single-stream test files; `aid`/`sid` are recorded
  as candidates only. The app's own player selects tracks from mpv's
  `track-list`.
- **Volume above 100** is refused by the wrapper's own choice.

### Player controls and progress

- **Fullscreen** is a window action, not an mpv property.
- **Progress reports** use `/:/timeline` and `/:/scrobble`, shapes taken from
  third-party clients and server logs (`crates/plex-core/src/progress.rs`),
  and untested against a real Plex Media Server.

### Subtitle ownership

- mpv draws subtitles; the app never rasterises subtitle text itself. In the
  software-GL harness mpv's OSD text reached the app's framebuffer, but no run
  measured a libass glyph layer there, so that path is unproven for the
  off-screen render session (`crates/plex-app/src/render.rs`). The app's
  window path shows subtitles on screen.

### Jellyfin

- See "Open questions" in `docs/JELLYFIN.md`: session lifetime after a long
  pause, which of several versions to prefer, subtitle delivery in a server
  stream.

## Known issues

- The window thread both draws mpv's frames and talks to mpv synchronously,
  which mpv's `render.h` warns can stall. In about 130 headless test runs, 4
  stopped producing frames just after a file was requested; all four happened
  while a second headless session shared the GPU, and the cause is not
  established.
- On Windows and macOS the sign-in screen's "Open plex.tv/link" button does
  nothing; open the address by hand.
- Subtitles are drawn by mpv at the bottom of the picture and can sit under
  the player's control bar while it is showing.

## Reproducing the measurements

```sh
scripts/hwdec-probe.sh          # what this machine's GPU and mpv can decode
scripts/hdr-fixtures.sh         # generate and verify HDR / Dolby Vision clips
scripts/format-fixtures.sh      # generate and play clips in older formats
scripts/mpv-gl-harness.sh       # the render path on software OpenGL, no display
```

All four write under `target/` and change nothing on the system. A new row
in this file should come with the command that produced it.
