# Plex Watch Together

Watch Together is Plex's own service for watching one item in step with other
people. A room lives on `together.plex.tv`; it names the item, the people
invited and a Syncplay server, and every participant's player keeps in step
through that server.

`plex_core::together` holds all of it: `room` (the room calls), `people`
(who can be invited), `message` (the Syncplay frames), `socket` (the sync
socket) and `exchange` (what to answer each state with, and what to follow).
Everything below was observed on 2026-11-13 with one account, a managed Home
user of that account and a disposable Plex Media Server, against Plex's live
service and against Plex's own web app 4.163.0, which still ships the
feature. A web app session is kept, with every id replaced, as
`crates/plex-core/tests/fixtures/together/web_session.jsonl`, and
`crates/plex-core/tests/together_live.rs` repeats the two-participant flow
against the live service.

## 1. Rooms

Every call is authorised by the account token in `X-Plex-Token` with the
usual `X-Plex-*` headers.

| Call | Body | Answer |
|---|---|---|
| `GET https://together.plex.tv/rooms` | | `{rooms:[room]}` |
| `GET /rooms/<id>` | | the room |
| `POST /rooms` | `{sourceUri, title, users}` | `201` with the room |
| `POST /rooms/<id>/invite` | `{users}` | `200` |
| `DELETE /rooms/<id>` | | `204` |

A room is `{id, title, sourceUri, source, type:"watch", createdBy, startsAt,
endsAt, updatedAt, syncplayHost, syncplayPort, users:[{id, uuid, title,
username, thumb}]}`. Times are Unix seconds and a room lasts three hours.
`createdBy` and each user `id` are numeric plex.tv user ids. A managed Home
user's `username` is `null`.

- `sourceUri` is
  `server://<machineIdentifier>/com.plexapp.plugins.library/library/metadata/<ratingKey>`.
  The service does not check it: a room for a server nobody in it can reach is
  accepted. Offering only people who can play the item is the client's job.
- `users` is a list of numeric user ids, or `null` for a room with only its
  creator; `invite` adds people later, and an invitee then lists the room.
- A room shows for its creator and every invitee. Plex's web app offers it
  on its home screen under "Watch Together", one click to join, and a
  managed Home user can be either side.
- `DELETE /rooms/<id>` takes the caller out of the room, creator or not. The
  room stays for whoever is left until they leave too or it ends. There is no
  separate "leave" call and no "end the room for everyone".
- The product named in the headers need not be Plex's own.

## 2. People who can be invited

- **Home users:** `GET https://plex.tv/api/v2/home/users` answers
  `{users:[{id, uuid, title, restricted, admin, thumb, …}]}`, the account
  itself included (`admin: true`).
- **Friends:** `POST https://community.plex.tv/api` with
  `{"query": "query GetAllFriends { allFriendsV2 { user { avatar displayName id username idRaw } createdAt } }", "variables": {}}`
  answers `{data:{allFriendsV2:[{user, createdAt}]}}`. `idRaw` is the numeric
  id the room calls take; `id` is the user's uuid. The query changes nothing.
  The older `plex.tv/api/v2/friends` answers `410 Gone`.

Neither list says who can reach a given server.

## 3. The sync socket

- **Address.** `wss://<syncplayHost>:<syncplayPort>/ws` (a host under
  `syncplay.plex.services`, port 7776). No token and no `X-Plex-*` header
  travel on it: knowing the room id is what lets a client in.
- **Server.** Stock Syncplay, announcing `realversion` 1.6.5; Plex's web app
  says it speaks 1.6.4. Every frame is one JSON text object with a single key
  naming its type.
- **Names.** The Syncplay user name is the JSON text of
  `{deviceIdentifier, deviceName, userID}` (the client identifier, a display
  name and the plex.tv user id as text). A name already in the room comes
  back with underscores appended, so they are stripped before reading it.
  The Syncplay file name is the JSON text of
  `{ads:{playing:false}, uri:<sourceUri>}`.
- **Keep-alive.** None of its own: the server sends a state about once a
  second.
- **Transport.** rustls with the ring provider and the same Mozilla root
  bundle as the HTTP transport.

### 3.1 Messages

| Frame | Direction | Content |
|---|---|---|
| `Hello` | to server | `{room:{name:<room id>}, username, version:"1.6.4"}`; the first frame |
| `Hello` | from server | the same, plus `realversion`, `motd`, `features` |
| `List` | to server | `{}` |
| `List` | from server | `{<room>:{<username>:{position, file, isReady, controller, features}}}`; `file` is `{}` before one is set |
| `Set` | to server | `{file:{name}}` or `{ready:{isReady}}` |
| `Set` | from server | `{user:{<username>:{room, file?, event?:{joined\|left:true}}}}`, `{ready:{username, isReady, manuallyInitiated}}`, `{playlistChange:…}`, `{playlistIndex:…}` |
| `State` | both | see 3.2 |
| `Error` | from server | `{message}` |

After `Hello` the server sends readiness and playlist `Set` frames, then its
`Hello`. Plex's web app answers with `List`, then on the first state sends
its file and `ready: false`, and `ready: true` a second later. Another
participant's arrival shows as a `Set.user` with its file, its departure as
`Set.user` with `event: {left: true}`. The server lists each user once
however many devices joined.

### 3.2 States

The server sends, about once a second:

```
{"State": {"ping": {"latencyCalculation", "serverRtt", "clientLatencyCalculation"?},
           "playstate": {"position", "paused", "doSeek", "setBy"},
           "ignoringOnTheFly"?: {"server"?, "client"?}}}
```

and the client answers each one, and only then:

```
{"State": {"ping": {"clientLatencyCalculation", "clientRtt", "serverRtt", "latencyCalculation"},
           "playstate": {"doSeek", "paused", "position", "setBy": null},
           "ignoringOnTheFly"?: {"client"?, "server"?}}}
```

So a local change leaves with the next answer, up to a second later.

- **Latency.** The client stamps each answer with its own clock in seconds
  (`clientLatencyCalculation`). The server sends it back, moved on by the time
  it held it, on its next state; the client's round trip is the time of
  arrival minus that value, and the web app sends the last one as
  `clientRtt`. `latencyCalculation` and `serverRtt` are echoed. A playing
  group has moved on by half the round trip when its state arrives.
- **A local change.** The answer carries the player's new state (`doSeek`
  true for a seek) and raises `ignoringOnTheFly.client` to 1. The next state
  is the server imposing that change on everyone, with
  `ignoringOnTheFly: {server: 1, client: 1}` and `setBy` naming the device
  that made it.
- **A change the server imposes.** A state carrying `ignoringOnTheFly.server`
  is answered by echoing its playstate back, `doSeek` false, with
  `ignoringOnTheFly: {server: <same>}`; the client's own counter goes back to
  zero. Another participant's change arrives this way; the client follows it
  unless `setBy` is its own device.
- **Otherwise** the answer reports the player's position and pause, and a
  client follows the group's state when it is not its own change on the way.
- The web app sends both counters even at zero; leaving zeros out, as
  `Exchange` does, works the same.

`plex_core::together::exchange::Exchange` does exactly this; replayed against
the web app session above it gives the web app's 137 answers, counters
included.

**Tolerance.** The web app seeks when it is 4 s ahead of the group or 1.75 s
behind, and plays at 0.95 speed when more than 1.5 s ahead. Following is the
caller's choice; `Exchange` only says where the group is.

### 3.3 Results

Two participants from one process (the account and its managed Home user)
followed each other's play, pause and seek within about one state each way;
with a 0.2 s round trip to the sync server they ran 0.04 s apart while
playing. Plex's web app as one participant and this client's protocol as the
other followed each other in all six directions (pause, play and seek from
each side), and the web app showed the join and the leave.

## 4. In Chewvy

`together::rooms` (worker side) lists the account's rooms every 30 s while a
Plex account is signed in, and at once when a panel opens; nothing is polled
otherwise. `together::plex` runs one room on a thread of its own: it says
`Hello`, asks for the `List`, sends its file and `ready` once the player has
the title, and answers every state with the player's latest reading through
`Exchange`. What the group wants becomes a command for the same follower the
Jellyfin groups use, so Chewvy holds a Plex room as tightly as a Jellyfin
group.

- **No shared clock.** The sync server sends no time of its own, so a play
  starts from the moment its state arrives, its position already moved on by
  half the round trip. The follower is judged against the group state the
  server sent, not against any other participant's picture.
- **Changes made here.** A pause, play or seek in Chewvy moves this player at
  once and leaves with the next answer, up to a second later; a play and a
  seek made before that answer leave together.
- **The group's position is the slowest participant's.** The server keeps the
  room at the position of whoever with a file is furthest behind, and its
  `setBy` names that participant. A participant reporting itself paused does
  not pause the room; a pause from someone does.
- **Sync servers differ by room.** Rooms came back with different hosts under
  `syncplay.plex.services` (`pop-…`), each on port 7776.
- **A dropped socket** is reopened with growing waits up to 8 s, for up to
  2 minutes, unless the room has ended; the player keeps playing meanwhile.
- **Leaving** closes the socket with a normal close frame and keeps playing
  alone; the room stays on the list. **Remove** is `DELETE /rooms/<id>`.
- **In a room** playback speed is 1 and the next episode does not start by
  itself.
- **The HDR window** follows a room as this window does: it is the same
  player. Its keys are mpv's own, so a pause or a seek made there is seen
  by the follower as a change it did not make and asked of the room; the
  room's notices are shown over its picture.
- **A room on another server.** A room's source names a server by its
  machine identifier, and a rating key means something on that server only.
  A room on another server the account lists is joined by connecting to
  that server first, as the server switch would; one on a server the
  account does not list is shown without Join. A session never opens a
  room whose server is not the connected one.
- **Inviting from the room.** The room panel in the player lists the Home's
  members and friends who are not in the room, read once on joining, and
  `POST /rooms/<id>/invite` adds whoever is ticked; the room list is read
  again, which takes them off the panel.

Measured on 2026-11-13 against the live service, a disposable server and
Plex's web app 4.163.0 in a headless browser:

| What | Result |
|---|---|
| Chewvy's pause, play, seek reaching the web app | 0.1–1 s: the change leaves with Chewvy's next answer, and is on the web app's socket 0.02 s after that |
| The web app's pause, play, seek reaching Chewvy | 0.1–1 s: the web app also sends with its next answer; Chewvy follows 0.02 s after |
| Chewvy against the group state, 94 s of play | median under 0.01 s, worst 0.18 s |
| The web app against Chewvy, 211 samples | the web app 0.95 s ahead (0.949–1.076 s), never corrected |
| Joining a room already playing | started at the room's position, 0.2–0.45 s behind while the title loaded, within 0.06 s after 4–8 s at 1.05 speed |
| Socket cut for about 6 s | states back 7 s after the cut; a scripted participant and Chewvy 0.06 s apart afterwards |
| Leave, and closing the app | the web app saw the leave within 0.4 s |

The web app's 0.95 s lead is inside its own tolerance (see 3.2), so it never
moves; Chewvy cannot close that gap, since the web app plays wherever it is.
When the web app joins a room it starts playing by itself.

Measured on 2026-11-14, the same way:

| What | Result |
|---|---|
| A room made in the web app's own interface | listed in a running Chewvy at its next look; the web app waited in its lobby and started playback for both when Chewvy joined and said it was ready |
| A Windows and a Linux Chewvy in one room, made on either | every pause, play and seek followed both ways; the two players a median 0.04 s apart, 95% within 0.4 s |
| The HDR window in a room, 88 samples | median 0.02 s from the group state, worst 0.44 s; a pause, play and seek from a scripted participant followed, and Space and a seek key in the HDR window moved the participant |
| A room on another of the account's servers | Join connected to that server and opened its title, not the connected server's title with the same rating key |
| Inviting from the room's panel | the person appeared in the room on the service and in their own room list |

## Open questions

- Whether an invite reaches a running client at once. Plex's web app listens
  for `notifyWatchTogetherInvite` on its notification socket; polling
  `GET /rooms` works.
- Inviting a real friend, rooms between two full accounts, and an account
  without Plex Pass creating a room.
- Ads in the web app.
- Plex's other apps: they no longer offer the feature.
