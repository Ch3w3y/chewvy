# Jellyfin integration notes

How Chewvy talks to a Jellyfin server: sign-in, discovery, browsing, playback
and progress reporting. Endpoints and field names were read from the Jellyfin
server source on the `master` branch (12.x line; 12.4 was the latest stable
release) unless noted, and the browse and playback shapes were then pinned
against captures from a disposable local 12.4.0 server (§3.1). Access date for
every URL: **2026-11-11**.

---

## 1. Auth

### Authorization header (the one thing that always works)

Jellyfin's primary auth mechanism is the `Authorization` header in the
`MediaBrowser` scheme:

```
Authorization: MediaBrowser Client="Chewvy", Device="Linux desktop",
              DeviceId="<stable per-install uuid>", Version="0.1.0", Token="<access token>"
```

- `Client`, `Device`, `DeviceId`, `Version` identify the *device*; `Token` is the
  per-device access token and is omitted on the unauthenticated handshake calls.
- The server extracts App/AppVersion/DeviceId/DeviceName from this header and
  creates a session behind them (UserController's `AuthenticateByName` does
  exactly that; `TryConnect` throws `ArgumentException` if any of the four
  device fields is missing — they are mandatory even before login).
- Legacy mechanisms — `X-Emby-Token`/`X-Emby-Authorization` headers and the
  `api_key` query parameter — are being removed. The `EnableLegacyAuthorization`
  option shipped default-off in 10.11 and PR #15559 flipped it to `false` for
  all installs; removal of the code itself was planned for a later release, so
  on current 12.x servers a legacy-only client simply gets 401s.
  **The client only ever sends the `MediaBrowser` header.** Because the client
  fetches all HTTP (including artwork and stream bytes) through its own HTTP
  client or mpv, a header can always be attached; `api_key`-in-URL is never needed.

Source: nielsvanvelzen, *Jellyfin auth: api keys vs tokens*
<https://gist.github.com/nielsvanvelzen/ea047d9028f676185832e51ffaf12a6f>;
jellyfin/jellyfin PR #15559
<https://github.com/jellyfin/jellyfin/pull/15559>;
`UserController.cs`, `QuickConnectController.cs`,
`Emby.Server.Implementations/QuickConnect/QuickConnectManager.cs`
<https://github.com/jellyfin/jellyfin/blob/master/Jellyfin.Api/Controllers/UserController.cs>

The header parser (`Jellyfin.Server.Implementations/Security/AuthorizationContext.cs`,
accessed 2026-11-11,
<https://github.com/jellyfin/jellyfin/blob/master/Jellyfin.Server.Implementations/Security/AuthorizationContext.cs>)
splits on top-level commas outside quotes, takes `key=value` at the first `=`,
then trims surrounding `"` and URL-decodes each value. Values containing `,`,
`"` or `%` must therefore be percent-escaped by the client (and a literal `+`
must be sent as `%2B`, because decode turns `+` into a space); the client escapes every
byte outside RFC 3986's unreserved set to be safe.

### Password sign-in

`POST /Users/AuthenticateByName` with JSON body `{"Username": "...", "Pw": "..."}`
→ `AuthenticationResult`:

```
{ "User": UserDto, "SessionInfo": SessionInfoDto, "AccessToken": "...", "ServerId": "..." }
```

Plaintext password over the wire → treat HTTPS/reverse-proxy or trusted LAN as a
precondition (same trust stance as Plex local auth). Only the `AccessToken` is
stored, never the password. After sign-in, `GET /Users/Me` (authenticated) returns the current
`UserDto` for refresh/validation.

Tokens are revocable: `POST /Sessions/Logout` ends the session and invalidates
the access token.

### Quick Connect (Plex-PIN-equivalent, preferred UX)

Route surface (all read from `QuickConnectController.cs` on master):

| Call | Notes |
|---|---|
| `GET /QuickConnect/Enabled` | bare `true`/`false`; unauthenticated |
| `POST /QuickConnect/Initiate` | needs `MediaBrowser` header with device fields (no Token); returns `QuickConnectResult {Secret, Code, Authenticated, DeviceId, DeviceName, AppName, AppVersion}`; `401 "Quick connect is disabled"` if the server option is off |
| `GET /QuickConnect/Connect?secret=...` | unauthenticated poll; same DTO; 404 `Unknown secret` after expiry |
| `POST /QuickConnect/Authorize?code=...&userId=...` | called by the *already-logged-in* device (authenticated); returns `true` |
| `POST /Users/AuthenticateWithQuickConnect` | body `{"Secret": "..."}` → `AuthenticationResult` with `AccessToken` |

Client sequence: `Initiate` → show the 6-character `Code` → poll `Connect`
(1–2 s interval) until `Authenticated: true` → `AuthenticateWithQuickConnect`
→ store `AccessToken` + `User.Id`.

Server semantics from `QuickConnectManager.cs`: requests expire after **10
minutes** (`Timeout = 10`); codes are 6 chars; once authorized, the request is
kept visible ~1 extra minute so the polling client notices before expiry, and
the authorized secret survives for the same 10-minute window.

If `Enabled` is false, fall back to username/password with an actionable error
("enable Quick Connect in server settings, or sign in with a password").

### Server handshake / validation

`GET /System/Info/Public` (unauthenticated) returns
`{ LocalAddress, ServerName, Version, ProductName, OperatingSystem (deprecated, empty), Id, StartupWizardCompleted }`.
Use it for "Add server" URL validation, version display, and identity (its `Id`
is the server's stable GUID). `GET /Ping` is the cheap liveness check.

`StartupWizardCompleted` is what "Add server" refuses an unfinished server on:
a `false` means the address answers but there is no user to sign in to yet, so
the app says so instead of failing a sign-in. A server that omits the key is read
as finished, not unfinished — only an explicit `false` refuses; the live 12.4
fixture carries `true`, and the omitted case is pinned in the DTO test.

---

## 2. Discovery

- **LAN broadcast:** UDP port **7359**. Client broadcasts the literal payload
  `Who is JellyfinServer?`; every willing server answers on each bound interface
  with JSON `{"Address": "http://...", "Id": "<server guid>", "Name": "<server name>"}`
  (`ServerDiscoveryInfo`). The responder (`AutoDiscoveryHost`, a
  `BackgroundService`) is gated by the network setting `AutoDiscovery` and skips
  link-local addresses; multiple replies mean multiple servers — collect replies
  until a short timeout (e.g. 1–2 s) instead of assuming one.
- **No cloud relay.** Unlike Plex (plex.tv `resources`), Jellyfin has no hosted
  "my servers" service; server identity is the local `Id`, and remote access is
  just a different base URL to the same server. Manual URL entry must therefore
  be a first-class path, with discovery as a convenience on top.
- **Dedup rule:** resolve every candidate (discovered or manually entered) via
  `/System/Info/Public` and key stored servers by `Id`, not URL — the same
  server is frequently reachable via several URLs (LAN IP, mDNS, reverse proxy).

Sources: `src/Jellyfin.Networking/AutoDiscoveryHost.cs`,
`MediaBrowser.Model/ApiClient/ServerDiscoveryInfo.cs`,
`MediaBrowser.Model/System/PublicSystemInfo.cs` (paths on master; MediaBrowser.*
projects moved to repo root in 12.x).

---

## 3. Browse

12.x moved `userId` from path segment to `?userId=` query on essentially every
user-scoped route (legacy `/Users/{userId}/...` routes still exist but are
marked `[Obsolete]`). An empty/absent `userId` resolves from the token.

| Need | Endpoint (Jellyfin) | Plex analogue |
|---|---|---|
| Library sections | `GET /UserViews?userId=&IncludeMediaTypes=&DisableEmbeddedImages=&EnableImages=&ImageTypeLimit=` (`QueryResult<BaseItemDto>`) | `GET /library/sections` |
| Section contents | `GET /Items?userId=&ParentId=&IncludeItemTypes=&Recursive=true&SortBy=&SortOrder=&StartIndex=&Limit=&Fields=` | `/library/sections/{id}/all` |
| Item metadata | `GET /Items/{itemId}` (UserLibraryController; single `BaseItemDto` resolved in user context; `UserData` is always included, §3.1) — list/search via `GET /Items` which supports `Fields=People,Genres,Overview,ProviderIds,...`, `enableUserData=true`, `searchTerm=`, `Filters=IsUnplayed,IsPlayed,IsFavorite,IsResumable,...` | `/library/metadata/{ratingKey}` |
| Item by path (latest) | `GET /Items/Latest?userId=&parentId=&limit=&groupItems=` | `/library/sections/{id}/recentlyAdded` |
| Continue watching | `GET /UserItems/Resume?userId=&limit=&fields=&mediaTypes=&excludeActiveSessions=` (ItemsController); also `GET /Shows/NextUp?userId=&limit=` | `/library/metadata?continue=1` / `/library/sections/{id}/next` |
| Seasons / episodes | `GET /Shows/{seriesId}/Seasons?userId=`, `GET /Shows/{seriesId}/Episodes?userId=&season=` | `/library/metadata/{id}/children` |
| Search | `GET /Search/Hints?userId=&searchTerm=&IncludeItemTypes=&Limit=` | `/library/search` |
| Root folder | `GET /Items/Root` | `/library/sections` (browse root) |

`BaseItemDto` fields the UI needs map onto the `plex-core` item model:
`Name`, `Overview`, `ProductionYear`, `CommunityRating`, `RunTimeTicks`,
`Genres`, `ProviderIds` (imdb/tmdb — useful for artwork cross-ref), item kind
(`Movie|Series|Episode|...` — serialised `Type`; the filter enum is
`BaseItemKind`) ≈ Plex `library.type` semantics, `UserData`
(`PlaybackPositionTicks`, `PlayCount`, `Played`, `IsFavorite` ≈ Plex
`LeafViewerMetadata` viewer state), `ImageTags`/`BackdropImageTags`.

**Images:** `GET /Items/{itemId}/Images/{imageType}?maxWidth=&maxHeight=&width=&height=&quality=&fillWidth=&fillHeight=&tag=&format=`
(image types `Primary`, `Thumb`, `Backdrop`, `Banner`, `Logo`, `Box`, …; plus
`GET /Items/{itemId}/Images` for the available image-info/tag list). Role
mapping for the existing UI slots: `Primary` ↔ poster, `Thumb` ↔ Plex
`snapshot`, `Backdrop` ↔ Plex `art`/background (Jellyfin returns several
backdrops per item, richer than Plex's one). The `tag` param (from
`ImageTags`) selects/validates the cached image version; the client always
sends the `MediaBrowser` header with image requests, so authorization is a
non-issue regardless of the route's anonymous handling. A `Backdrop` request
takes an index segment (`/Images/Backdrop/0`) because one item has several;
`/UserViews` also accepts `IncludeMediaTypes`/`DisableEmbeddedImages`/
`EnableImages`/`ImageTypeLimit`, so a section's artwork set is configurable
per call.

JSON casing: the server serializes CLR property names as-is (PascalCase —
confirmed in the discovery responder, which uses `JsonSerializer` defaults).
Rust models pin PascalCase (serde `rename_all = "PascalCase"`). A live 12.4
capture confirmed this for every browse payload and the DTOs are **case-
sensitive**, not case-insensitive: see §3.1 for the one asymmetry that buys us
(case insensitivity would have accepted a first-run server's answer as a
signed-in one).

### 3.1 What a live 12.4 server answered

Recorded against a disposable local Jellyfin **12.4.0** server serving generated
test media (the fixtures under
`crates/jellyfin-core/tests/fixtures/live-12.4/` pin these shapes). Nothing here
is inferred from documentation where the capture disagrees; where the capture is
silent, that is said too.

- **PascalCase, pinned case-sensitively.** Every captured browse payload
  (`/System/Info/Public`, `/Users/Me`, `/UserViews`, `/Items`,
  `/Items/{id}`, `/Shows/…/Seasons|Episodes`, `/UserItems/Resume`,
  `/Shows/NextUp`, `/Search/Hints`, `/Items/{id}/Images`,
  `POST /Items/{id}/PlaybackInfo`) answers PascalCase. A first-run server
  answers `/System/Info/Public` and `/System/Info` in **camelCase**
  (`serverName`), which is why the DTOs reject camelCase rather than tolerate
  it: a wizard-stage answer is a shape error for a signed-in browse, not a
  silently half-parsed server. The asymmetry serde gives us is one-directional
  and documented in the test suite: a camelCase **envelope** with an empty
  `items` array parses (the field defaults), while a camelCase **row** does
  not, because `Id` is required. So "camelCase is rejected" is true of rows
  and of a non-empty `items` list, and false of an empty one.
- **`userId` is a query parameter on every user-scoped route the client calls**, as §3
  said; the capture confirms the legacy path form is nowhere required.
- **Ids are 32-char hex** (`5197d80dc0c848286caeb3d345930689`), but
  `UserData.Key` is the **hyphenated** form of the same id
  (`5197d80d-c0c8-4828-6cae-b3d345930689`). Anything that compares a
  `UserData` key to an item id has to normalise, so the DTO keeps both raw and
  the client never synthesises one from the other.
- **A library section is a folder row.** `/UserViews` answers
  `BaseItemDto`s whose `Type` is `CollectionFolder` (with `CollectionType`
  `movies`/`tvshows` — a lower-case value inside a PascalCase payload; the DTO
  keeps it as text and the UI decides what it means), carrying their
  own `ImageTags.Primary` — and a section's artwork tag is **not** the tag of
  an item inside it (the Movies view's tag differs from the movie's). A UI
  that reuses one thumbnail cache key per "show" will mix a section poster
  with an item poster.
- **`Overview` is absent, not `null`.** The detail answers of these generated
  items omit the key entirely (the server sends `ChannelId: null` and friends,
  but no `Overview` at all), and the list rows omit it too. So a description
  is modelled as a missing field, and the DTO must not demand it; an empty
  string would be a description, `null` none, and the server draws neither of
  those lines here.
- **`/UserItems/Resume` can answer folder rows.** The captured resume list
  contains a `Season` folder with `PlaybackPositionTicks: 0` (its episode is
  the resumable leaf, but the season is what the server hands back). A
  "continue watching" card built only from non-folders silently drops the
  first row of that answer; the DTO keeps `IsFolder` for exactly this.
- **`/Search/Hints` answers its own envelope**: `SearchHints`,
  `TotalRecordCount`, more (`/Search/Hints` is not a `QueryResult`), and a
  hint carries `PrimaryImageTag` (not `ImageTags`) plus `Series` — not
  `SeriesName` — for an episode hint. Hint `Id`s are item ids, so a hint is
  directly openable.
- **`/Items/{id}/Images` is a list of image info**, one entry per available
  image type (`ImageType`, `ImageTag`, `Path`, `Height`, `Width`) — `Path` is
  a **server-side filesystem path** (`/config/metadata/library/…/poster.jpg`),
  not a URL; the URL is built from the item id, type and tag.
- **No `Logo`, `Backdrop`, `Box`, `Banner`, audio or subtitle rows exist in
  any capture** for this fixture library: `ImageTags` carries only `Primary`
  and `BackdropImageTags` is empty on every item. The image builders for
  those types are therefore pinned by request shape, not by a live
  successful fetch, and a `Logo` request against this library would be a
  404 — the client returns `None` rather than guessing a tag.
- **Ticks**: `RunTimeTicks: 300000000` is a 30-second generated clip, i.e.
  10,000,000 ticks/second as §5 says, and `PlaybackPositionTicks` is the
  resume position in the same unit. Conversion lives in one place.

**The `SupportsTranscoding` contradiction.** For the same item, the same
server, the same source (`MediaSources[0].Id` identical), two endpoints answer
differently:

| Where | `SupportsTranscoding` |
|---|---|
| `GET /Items/{id}` (and `GET /Items` rows) | `true` |
| `POST /Items/{id}/PlaybackInfo` (empty `DeviceProfile`) | `false` |

Both are faithful captures. `PlaybackInfo` answers the *requesting* client's
`DeviceProfile` — an empty profile declares no encoder, so the server says the
client cannot transcode — while `/Items` reports the source's intrinsic
capability. It is not a bug to fix in the client; it is a rule: **only the
`PlaybackInfo` answer is a statement about what this device can play**, so
the client sends a real `DeviceProfile` and reads the flags from that response
rather than caching the browse row's version. The corollary is that
`SupportsDirectPlay`/`SupportsDirectStream` happened to agree here (`true`,
`true`) and must not be assumed to; they are pinned per tested capability.

Other captured facts that narrow §3's guesses: `/Items/{id}` needs no
`enableUserData` to include `UserData` (it is always present for an
authenticated user); `ParentId` is present on children (so the back-pointer is
free); `SeasonId`/`SeriesId`/`SeasonName`/`SeriesName` are all present on
episode rows; `Etag` appears only on the single-item answer, never in list
rows; `Container` is sometimes a comma list (`mkv,webm`).

---

## 4. Playback

**Implemented: direct play and the server's HLS stream (§4.4).** The
sequence below is what `jellyfin_core::playback` + `plex_app::player::profile`
+ the worker's `Cmd::Play` arm do against a 12.x server: a real `DeviceProfile`
is sent, the first source the server says reaches this player as it sits
(`SupportsDirectPlay` or `SupportsDirectStream`) is streamed with
`static=true`, resuming at the row's `PlaybackPositionTicks`, and a source that
is not direct-playable plays as the server's own stream of it. `Ping`,
`NowPlayingQueue` and subtitle extraction are still open (§4.1, §5.1).

Sequence:

1. `POST /Items/{itemId}/PlaybackInfo?userId=` (GET variant also exists) with
   body fields `MaxStreamingBitrate`, `StartTimeTicks`, `AudioStreamIndex`,
   `SubtitleStreamIndex`, `MaxAudioChannels`, `MediaSourceId`,
   `AutoOpenLiveStream`, `EnableDirectPlay`, `EnableDirectStream`,
   `EnableTranscoding`, `AllowVideoStreamCopy`, `AllowAudioStreamCopy`,
   `DeviceProfile` (optional; empty ⇒ server defaults).
   Response: `{ MediaSources: [...], PlaySessionId, ErrorCode }`. Each
   `MediaSourceInfo` carries `Id` (the **MediaSourceId** — required on every
   subsequent URL), `Container`, `Protocol` (only `File` is handled),
   `MediaStreams` (codec/profile/level/language/default flags), `Bitrate`,
   `Size`, and the capability flags `SupportsDirectPlay` /
   `SupportsDirectStream` / `SupportsTranscoding` + `RequiresOpening`
   (true mainly for Live TV/in-progress transcodes).
2. Build the URL ourselves from item + media source (no `PlaybackInfo`-supplied
   full URL for the direct path):
   `GET /Videos/{itemId}/stream?static=true&mediaSourceId=...&playSessionId=...`
   — `static=true` is the direct-play byte pipe (server may still issue HTTP
   Range). `/Videos/{itemId}/stream.{container}` pins the container.
3. When the source is not direct-playable, or direct play fails, the server's
   own HLS stream is played instead: a remux where the codecs allow it, a
   transcode otherwise (§4.4).
4. Subtitles (embedded-extract or external):
   `GET /Videos/{itemId}/{mediaSourceId}/Subtitles/{index}/Stream.{format}`
   (also a `.../{startPositionTicks}/Stream.{format}` variant and
   `subtitles.m3u8` for HLS) — same `MediaBrowser` header required.

All playback/subtitle/image requests carry the `MediaBrowser` header with
`Token`; nothing relies on `api_key` in the URL. mpv fetches the stream itself,
so the header is added with the `change-list http-header-fields append`
command after `mpv_initialize` (one whole item: the `MediaBrowser` value
carries four commas, and mpv's list parser splits on them; the
`http-header-fields-append` option suffix is CLI-only, and libmpv 0.41 answers
it `MPV_ERROR_OPTION_NOT_FOUND`) for that file only — a plain `http-header-fields`, or a global config,
would send Plex's `X-Plex-Token` on this request or the Jellyfin token on
Plex's. The token is never in the stream URL, so it stays out of mpv's logged
command line.

### 4.1 What the client sends, and what it will not pretend

`DeviceProfile` is a *declaration of this player*, so it is built from
measured facts (`docs/HARDWARE_DECODE.md`,
`crates/plex-app/src/player/profile.rs`), not from a wishlist: only codecs
the development host has been seen decoding appear, and only at the frame size and bit
depth that was measured (H.264 at 8-bit; HEVC, AV1 and VP9 at 10-bit; all up
to 4096x2160). Two consequences are deliberate:

- A file this profile cannot prove it plays is not handed to mpv on faith: the
  server answers it not direct-playable and its own stream plays instead
  (§4.4). Nothing larger than 4096x2160 is claimed, whatever the GPU is
  nominally able to do.
- `MaxStaticBitrate` (the direct-play ceiling in `DeviceProfile`) is set
  explicitly to 1 Gbps rather than left alone: the server model's default is
  8 Mbps, which would describe a bandwidth-limited client, which this is not — the
  fixture file alone is 14.5 Mbps — and `static=true` does not ask the server
  to limit anything anyway.
- `MaxStreamingBitrate` is set to the same 1 Gbps. Measured live against
  12.4.0 on 2026-11-11: with it absent, `PlaybackInfo` answered
  `SupportsDirectPlay=false` and `SupportsDirectStream=false` (no
  `TranscodeReasons`) for a 1080p H.264 MKV the profile names, with or without
  `MaxStaticBitrate`, `CodecProfiles` or the `Enable*` flags; adding
  `MaxStreamingBitrate` alone, in the profile or the request, flipped both to
  `true`. The mock-backed tests did not catch this: they replay a captured
  answer instead of asking a server to decide.

Whether this client will accept direct play, direct stream or a transcode is
the **request's** business, not the profile's: `EnableDirectPlay`,
`EnableDirectStream`, `EnableTranscoding`, `AllowVideoStreamCopy`,
`AllowAudioStreamCopy` are `bool?` fields of the request body
(`Jellyfin.Api/Models/MediaInfoDtos/PlaybackInfoDto.cs`), each defaulting to
true server-side. What this client sends for each delivery is in §4.4.
`DeviceProfile` itself has no `Enable*` fields at all (it
does carry `MaxStreamingBitrate`, `MaxStaticBitrate`,
`MusicStreamingTranscodingBitrate` and `MaxStaticMusicBitrate`, all defaulting
to 8 Mbps except the 128 kbps music transcode ceiling).

### 4.2 The `DeviceProfile` wire shape (read from server source, not guessed)

`jellyfin_core::playback` freezes the DTO shape from the server's own model
(`MediaBrowser.Model/Dlna/{DeviceProfile,CodecProfile,ProfileCondition}.cs`,
`Jellyfin.Api/Models/MediaInfoDtos/PlaybackInfoDto.cs`, read at server source
tag 10.9.6-era paths; 12.x moved controllers, not these DTOs — and the shapes
were then confirmed against the live 12.4 capture in
`crates/jellyfin-core/tests/fixtures/live-12.4/`):

- `ProfileCondition` is `{Condition: enum, Property: enum, Value: string}`.
  **Both** `Condition` and `Property` are server-side enums serialized by name,
  so an unknown spelling is a rejected request rather than a tolerated string.
  `ProfileCondition.IsRequired` defaults to `true` in the server's
  parameterless constructor; for a `LessThanEqual` ceiling on a property an
  optional stream may not carry (bit depth, HDR format) this client sends
  `false`, so an SDR stream is not refused for being silent.
- `CodecProfile` is `{Codec, Type: enum, Container, AsCodec, AsContainer,
  MaxProfileLevel, Conditions[]}`. `Codec`/`AsCodec`/`Container` are comma
  lists; `Type` is `CodecType{Video=0, VideoAudio=1, Audio=2}`, serialized by
  name (`"Video"`).
- `DirectPlayProfile` is `{Container, AudioCodec, VideoCodec, Resolution,
  AspectRatio, Type}` — all comma lists; the container entry names the audio
  codecs and the video codecs that may sit in it, which is what makes a
  source's own `Supports*` flags meaningful for *this* device.
- Request-level, not profile-level: `EnableDirectPlay`, `EnableDirectStream`,
  `EnableTranscoding`, `AllowVideoStreamCopy`, `AllowAudioStreamCopy`.
- Not sent at all, and therefore server-defaulted: `TranscodingProfiles`,
  `ContainerProfiles`, `SubtitleProfiles`, `MaxStreamingBitrate`,
  `MusicStreaming*`. The empty `SubtitleProfiles` is a live consequence, not
  only a shape: a file whose embedded or external subtitles the server would
  rather deliver some other way can answer `SupportsDirectPlay=false`, which
  this client reports as a refusal rather than guessing a subtitle capability
  it has not measured (§4.3).

### 4.3 Capability rows for Jellyfin direct play

Direct play is claimed only where the codec was measured on the development
host (AMD Radeon RX 9070 XT, Mesa) —
hardware decode of mpv's `auto-safe` cascade and, for H.264/HEVC, through this
app's own `vo=libmpv` path at zero-copy `vaapi` (`docs/HARDWARE_DECODE.md`,
"Host measurements"), and audio decode by libavcodec. The
profile is that record; the server's `Supports*` answer to it is what gates the
request:

| Item shape | Claimed in `DeviceProfile` | Why |
|---|---|---|
| H.264 ≤4096x2160 8-bit in mkv/mp4 | yes | 1080p measured on the host and through the app's own render path; 3840x2160 measured on the host (vaapi, 201 fps) |
| HEVC Main10, AV1 ≤10-bit, VP9 ≤10-bit, each ≤4096x2160 | yes | `vainfo` + `auto-safe` measurements at 1080p; 3840x2160 10-bit measured on the host 2026-11-11 (vaapi-copy: HEVC 109 fps, AV1 91 fps, VP9 113 fps; synthetic clips, so a lower bound on difficulty, not real film) |
| Anything above 4096x2160 (8K) | **no** | never measured here; the refusal names the shape |
| 10-bit H.264 | **no** | `vainfo` listed H.264 at 8-bit only |
| HDR / Dolby Vision inside the ceilings above | not excluded | the profile carries no dynamic-range condition, so the player treats such an item like any HDR file (`docs/CAPABILITIES.md`); no HDR item has been played from a Jellyfin server yet |
| MPEG-2 ≤1920x1080, MPEG-4 part 2 ≤1280x720, VP8 ≤1280x720, all 8-bit | yes | software decode, each at the largest generated clip that played (`docs/CAPABILITIES.md`, "Older and less common formats") |
| VC-1/WMV3, WMV8, MJPEG, VVC | **no** | FFmpeg has no VC-1 encoder, so no test clip could be generated; WMV8 plays but is left out with its family; the rest unmeasured |
| `mkv`,`mp4`,`mov`,`webm` (H.264, HEVC, AV1, VP9, MPEG-2, VP8), `ts` (H.264, HEVC, MPEG-2), `mpeg` (MPEG-2), `avi` (MPEG-4 part 2) | yes | one profile row per container family, naming only the codecs played from it |
| `mp3`,`flac`,`m4a`,`ogg`,`opus` (audio files) | yes | the containers those measured files were read from |
| `aac,mp3,flac,ac3,eac3,opus,vorbis,mp2,dts,truehd,pcm_s24le,alac` | yes | libavcodec software audio, measured per clip; `dts` was measured as the 5.1 core only |
| Other PCM layouts, WMA, DTS-HD as such | **no** | outside the probed audio rows (a DTS-HD file is described by the `dts` name, so the server hands it over as it sits) |
| `asf`/`wmv`, `flv` | **no** | containers whose usual contents were not measured |

The claim is deliberately narrower than the machine. An item this profile
cannot name plays as the server's stream (§4.4) rather than being handed to mpv
on faith; mpv's software fallback would probably play the file itself, and
raising a ceiling is a measurement to make, not a number to guess.

### 4.4 The server's own stream: remux and transcode over HLS

Measured against the throwaway 12.4.0 server on 2026-11-11 (generated clips:
MPEG-2 + MP2 in MPEG-TS, 4K HEVC Main10 in MKV, H.264 + AAC in MKV), first with
curl and the mpv CLI, then through the app in the headless harness.

**What is sent.** The profile gains one `TranscodingProfiles` row:
`Type=Video`, `Protocol=hls`, `Context=Streaming`, `Container=ts`,
`VideoCodec=h264,hevc`, `AudioCodec=aac,ac3,eac3`, `MaxAudioChannels=6`. The
server copies a source stream whose codec is in those lists and encodes to the
first entry otherwise. The request then names one of three deliveries:

| Delivery (quality menu) | Request flags | What 12.4 answered |
|---|---|---|
| Server's choice ("Original · Direct Play") | all defaults (`EnableTranscoding` now true) | H.264/HEVC MKV: `SupportsDirectPlay=true`, no `TranscodingUrl`. MPEG-2 TS: both direct flags false, `TranscodingUrl` set, `TranscodeReasons=ContainerNotSupported,VideoCodecNotSupported,AudioCodecNotSupported`; played as H.264 + AAC |
| Remux ("Original · Direct Stream") | `EnableDirectPlay=false`, `EnableDirectStream=false` | `TranscodingUrl` with `TranscodeReasons=DirectPlayError`; the server ran ffmpeg with `-codec:v:0 copy` and the 3840x2160 HEVC stream played from MPEG-TS segments |
| Transcode (the bitrate presets) | also `AllowVideoStreamCopy=false`, `AllowAudioStreamCopy=false`, `MaxStreamingBitrate=<preset>` | `libx264`; the server chose the frame size from the bitrate alone (20 Mbps → 3840x2160, 12 → 2560x1440, 8 → 1920x1080, 4/2/1.5 → 1280x720, 0.32 → 768x432) |

**The stream URL.** `MediaSources[].TranscodingUrl` is
`/videos/{id}/master.m3u8?DeviceId=…&MediaSourceId=…&PlaySessionId=…&ApiKey=<token>&…`.
The token is cut out as the answer is parsed (`jellyfin_core::dto`), so it
never reaches a log, mpv's command line or a `Debug` print. Without it and
without the header the server answered `401`; with the `Authorization` header
alone the master playlist, the media playlist and the segments all answered,
and mpv sends that header on each of them. For a transcode the client appends
`&MaxWidth=…&MaxHeight=…` from the preset, which the stream endpoint honoured
(1280x720 asked, 1280x720 answered at every bitrate above it), so "1080p · 20
Mbps" means 1080p here as it does on Plex.

**Seeking and position.** The playlist is the whole item (`duration` read by
mpv equalled the file's), so mpv seeks in it and starts at the resume position
itself; the client does not restart the stream at an offset as it does on Plex.

**Ending it.** Leaving the item sends
`DELETE /Videos/ActiveEncodings?deviceId=…&playSessionId=…` (`204`; the server
logged the partial stream files deleted). This is sent whatever the progress
setting says: a `Stopped` report would also end the encode, but that report is
opt-in. Session reports name `PlayMethod=Transcode` for the server's stream,
copied or not, which is the server's own word for it.

**Not measured:** audio copy of AC-3/E-AC-3 and anything above stereo (the
clips carry AAC, MP2 or no audio), subtitles of any kind in a stream, HDR
sources (the server tone-maps or not by its own settings), fMP4 segments,
a server with transcoding disabled for the user (the refusal text for it is
written, not observed), and hardware encoding on the server.

---

## 5. Progress

Playstate reporting lives in `PlaystateController` (10.x called this surface
"Sessions"); session inspection/remote control in `SessionController`.

| Event | Endpoint | Key body fields |
|---|---|---|
| Start | `POST /Sessions/Playing` | `ItemId`, `MediaSourceId`, `PlayMethod` (`DirectPlay`/`Transcode`), `PositionTicks`, `PlaySessionId`, `NowPlayingQueue` |
| Progress | `POST /Sessions/Playing/Progress` | `PositionTicks`, `IsPaused`, `IsMuted`, `VolumeLevel`, `PlayMethod`, `PlaySessionId`, `NowPlayingQueue` |
| Stop | `POST /Sessions/Playing/Stopped` | `ItemId`, `PositionTicks`, `PlaySessionId` (this is what marks played at ≥ threshold) |
| Keep transcode warm | `POST /Sessions/Playing/Ping?playSessionId=` | only while transcoding |
| Mark played / unplayed | `POST` / `DELETE /UserPlayedItems/{itemId}?userId=` | |
| Revoke session | `POST /Sessions/Logout` | on "sign out" in Sources UI |

All progress endpoints answer `204 No Content`; failures should be non-fatal to
playback (same stance as the Plex timeline reporter in `plex-core`).

**Ticks**: Jellyfin times are .NET ticks — 10,000,000 per second (100 ns
resolution), used in `PositionTicks`/`StartTimeTicks`/`RunTimeTicks`. One
central conversion helper (ticks ↔ ms) in the Jellyfin module, like
`plex_core::library`'s `duration_secs` for Plex. The `NowPlayingQueue`
(now a list of `{ItemId, PlaylistIndex}` per 12.x model) should be sent with
every event so multi-item playback resolves correctly server-side.

`UserData.PlaybackPositionTicks` is the resume position surfaced by browse
endpoints — no separate fetch needed (unlike Plex, where `ViewerMetadata` comes
per item).

### 5.1 What the client reports

`jellyfin_core::playback::SessionBody` is the one body shape, sent by
`Client::report_playing` / `report_progress` / `report_stopped` (the app's
`Server::playing/progress/stopped` wrap them; `/Sessions/*` is not user-scoped
— the session belongs to the access token, so these three need no `userId`).

- The body carries `ItemId`, `MediaSourceId` (the source the resolve step
  chose), `PlaySessionId` (the server's own, from `PlaybackInfo`),
  `PositionTicks`, `RuntimeTicks` when the source states a length, `CanSeek`,
  `IsPaused`/`IsPlaying`, `PlayMethod: "DirectPlay"` and `EventName`.
- `Playing` is the *first* report of an item rather than a separate event: a
  session the server never heard of cannot take a progress update, so the
  worker sends `Playing` once and `Progress` after that.
- `Stopped` is sent when playback ends or the user leaves, at the position the
  player was last at — which is what crosses the server's Played threshold.
- Only the client knows an item has reached its end: `Completed` is set when
  the reported position is at or past the source's length, and never on
  `Playing`.
- `Ping` is not sent: it exists to keep a *transcoding* session warm, and this
  client only direct-plays.
- The position is read from the player (`player::position()`) on a fixed tick
  while the playing screen is up, and once more just before the player stops —
  quitting mid-item costs at most that tick of Continue Watching.
- All three reports obey the existing Settings switch for progress sync, the
  same switch Plex's timeline uses, so opting out means the server never hears
  about the session at all — the closing `Stopped` included.
- `NowPlayingQueue` is not sent (single-item playback); `IsMuted`/
  `VolumeLevel` are not sent because player volume is not modelled here yet.

---

## Open questions

- **Whether a real 12.x server records what this client reports.** The three
  `/Sessions/*` calls are pinned against a mock and the captured 12.4
  fixtures; the live capture has no session answers recorded. Whether Jellyfin
  writes the resume point and marks the item played from a `Stopped` at a
  position under its threshold, and whether it tolerates a `Progress` for a
  session it answered `204` to, is a live-server observation.
- **Which `MediaSource` to take when a server offers several** (true alternate
  versions): the client takes the first the server marks direct-playable, which is
  what the single-source fixture exercises.
- **`PlaySessionId` lifetime**: the id from `PlaybackInfo` is echoed on every
  report, but nothing re-requests it after a pause longer than the server's
  session timeout. Whether a stale id is dropped silently or answered `404` is
  unmeasured; either way the report is non-fatal, so the symptom is lost
  progress rather than a stalled item.
- **Does `static=true` respect a `MaxStaticBitrate` the profile declares**?
  The profile sets 1 Gbps deliberately above every fixture file, so the field is
  exercised only as a wire shape.

## Sources (accessed 2026-11-11)

- jellyfin/jellyfin (master, 12.x) — `Jellyfin.Api/Controllers/`:
  `UserController.cs` (AuthenticateByName / AuthenticateWithQuickConnect /
  Users/Me), `QuickConnectController.cs`, `SystemController.cs`,
  `UserViewsController.cs`, `UserLibraryController.cs`, `ItemsController.cs`,
  `TvShowsController.cs`, `VideosController.cs`, `MediaInfoController.cs`,
  `ImageController.cs`, `PlaystateController.cs`, `SessionController.cs`,
  `SearchController.cs`, `SubtitleController.cs`
  <https://github.com/jellyfin/jellyfin/tree/master/Jellyfin.Api/Controllers>
- `MediaBrowser.Controller/Authentication/AuthenticationResult.cs`
  <https://github.com/jellyfin/jellyfin/blob/master/MediaBrowser.Controller/Authentication/AuthenticationResult.cs>
- `MediaBrowser.Model/ApiClient/ServerDiscoveryInfo.cs`,
  `MediaBrowser.Model/System/PublicSystemInfo.cs`
  <https://github.com/jellyfin/jellyfin/tree/master/MediaBrowser.Model>
- `src/Jellyfin.Networking/AutoDiscoveryHost.cs` (UDP 7359 responder)
  <https://github.com/jellyfin/jellyfin/blob/master/src/Jellyfin.Networking/AutoDiscoveryHost.cs>
- `Emby.Server.Implementations/QuickConnect/QuickConnectManager.cs` (10-min
  timeout, 6-char code, authorize/expiry semantics)
  <https://github.com/jellyfin/jellyfin/blob/master/Emby.Server.Implementations/QuickConnect/QuickConnectManager.cs>
- jellyfin/jellyfin PR #15559, "Disable legacy authentication by default" (part
  2/3; option added default-off in #13306 for 10.11; removal planned later)
  <https://github.com/jellyfin/jellyfin/pull/15559>
- nielsvanvelzen, *MediaBrowser auth explained*
  <https://gist.github.com/nielsvanvelzen/ea047d9028f676185832e51ffaf12a6f>
- .NET `TimeSpan.TicksPerSecond` = 10,000,000 (ticks convention)
  <https://learn.microsoft.com/en-us/dotnet/api/system.timespan.tickspersecond>

Additional source consulted for §3.1:

- `Jellyfin.Api/Controllers/VideosController.cs` (`GET /Videos/{itemId}/stream`
  — `static` "the original file will be streamed statically without any
  encoding", `mediaSourceId` "the media version id, if playing an alternate
  version"), `UserViewService.cs` and `ItemLookupController.cs` (PlaybackInfo
  `DeviceProfile` handling; the empty profile is the reason §3.1's
  `SupportsTranscoding` answers differ)
  <https://github.com/jellyfin/jellyfin/blob/master/Jellyfin.Api/Controllers/VideosController.cs>
  <https://github.com/jellyfin/jellyfin/blob/master/Jellyfin.Api/Controllers/UserViewService.cs>
  <https://github.com/jellyfin/jellyfin/blob/master/Jellyfin.Api/Controllers/ItemLookupController.cs>

## Sources (stream additions, accessed 2026-11-11)

- `Jellyfin.Api/Helpers/MediaInfoHelper.cs` (`SetDeviceSpecificData`: where
  `enableDirectPlay`, `enableDirectStream`, `allowVideoStreamCopy` and
  `allowAudioStreamCopy` turn the direct flags off, and where `TranscodingUrl`
  is built with the caller's token)
  <https://raw.githubusercontent.com/jellyfin/jellyfin/master/Jellyfin.Api/Helpers/MediaInfoHelper.cs>
- `MediaBrowser.Model/Dlna/TranscodingProfile.cs` (`Container`, `Type`,
  `VideoCodec`, `AudioCodec`, `Protocol`, `Context`, `MaxAudioChannels`, …)
  <https://raw.githubusercontent.com/jellyfin/jellyfin/master/MediaBrowser.Model/Dlna/TranscodingProfile.cs>

Both were read from `master`, which is not the 12.4.0 the tests ran against;
every behaviour §4.4 relies on was observed on the 12.4.0 server itself.

## Sources for playback and session reports (accessed 2026-11-11)

DTO shapes the `DeviceProfile` JSON is frozen against — read from the server's
own model, because a wrong field name is a request the server rejects or
ignores silently:

- `MediaBrowser.Model/Dlna/DeviceProfile.cs` (`Name`, `Id`,
  `MaxStreamingBitrate`/`MaxStaticBitrate` (both defaulting to 8 Mbps),
  `MusicStreamingTranscodingBitrate` (128 kbps), `MaxStaticMusicBitrate`,
  `DirectPlayProfiles`, `TranscodingProfiles`, `ContainerProfiles`,
  `CodecProfiles`, `SubtitleProfiles`)
  <https://raw.githubusercontent.com/jellyfin/jellyfin/v10.9.6/MediaBrowser.Model/Dlna/DeviceProfile.cs>
- `MediaBrowser.Model/Dlna/ProfileCondition.cs` (`Condition`/`Property` are
  enums serialized by name; the parameterless constructor sets `Value = ""` and
  `IsRequired = true`), `MediaBrowser.Model/Dlna/ProfileConditionType.cs`,
  `MediaBrowser.Model/Dlna/ProfileConditionRule.cs`,
  `MediaBrowser.Model/Dlna/ProfileProperty.cs` — the property names in
  `player/profile.rs` (`Width`, `Height`, `VideoBitDepth`, `VideoFramerate`, …)
  are this enum's spellings.
  <https://raw.githubusercontent.com/jellyfin/jellyfin/v10.9.6/MediaBrowser.Model/Dlna/ProfileCondition.cs>
  <https://raw.githubusercontent.com/jellyfin/jellyfin/v10.9.6/MediaBrowser.Model/Dlna/ProfileProperty.cs>
- `MediaBrowser.Model/Dlna/CodecProfile.cs` (`Type: CodecType`, `Language`,
  `Codec`, `Container`, `AsCodec`, `AsContainer`, `MaxProfileLevel`,
  `Conditions[]`) and `MediaBrowser.Model/Dlna/CodecType.cs`
  (`Video=0, VideoAudio=1, Audio=2`), `MediaBrowser.Model/Dlna/DirectPlayProfile.cs`
  (`Container`, `AudioCodec`, `VideoCodec`, `Resolution`, `AspectRatio`,
  `Type`).
  <https://raw.githubusercontent.com/jellyfin/jellyfin/v10.9.6/MediaBrowser.Model/Dlna/CodecProfile.cs>
  <https://raw.githubusercontent.com/jellyfin/jellyfin/v10.9.6/MediaBrowser.Model/Dlna/DirectPlayProfile.cs>
- `Jellyfin.Api/Models/MediaInfoDtos/PlaybackInfoDto.cs` — the request body:
  `StartTimeTicks`, `SubtitleIndex`, `AudioIndex`, `CollectionId`,
  `MediaSourceId`, `LiveStreamId`, `MaxStreamingBitrate`, and the nullable
  `EnableDirectPlay`/`EnableDirectStream`/`EnableTranscoding`/
  `AllowVideoStreamCopy`/`AllowAudioStreamCopy`/`AutoOpenLiveStream`/
  `IsPlayback`/`PlaySessionId`/`PlayMethod`. These are why §4.1's flags are not
  in the profile.
  <https://github.com/jellyfin/jellyfin/blob/master/Jellyfin.Api/Models/MediaInfoDtos/PlaybackInfoDto.cs>

mpv's own list-option semantics, from the mpv 0.40 man page
(`pkg-config --modversion mpv` = 2.5.0):

- `--http-header-fields=<field1,field2>` — "Set custom HTTP fields when
  accessing HTTP stream. This is a string list option. See List Options for
  details." (man page 8330-8334, with a comma-separated CLI example at 8341)
- "List Options": `-append` "Append single item (does not interpret escapes)",
  and "If you want to pass a value without interpreting it for escapes or `,`,
  it is recommended to use the `-append` variant." (807-809, 907-910). Through
  the client API the suffix is not an option name (`mpv_set_option_string`
  returns -5), so `player/mod.rs` uses the `change-list … append` command,
  which applies the same single-item rule.
