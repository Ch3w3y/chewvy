# Watch Together fixtures

* `web_session.jsonl`: every frame of one sync socket session as Plex's own
  web app sent (`"dir": "out"`) and received (`"dir": "in"`) them, while a
  second participant in the room played, paused and sought, and the web app
  did the same. Field names, nesting, value types, timings and counters are
  as recorded; the room id, machine identifier, device identifiers, user ids
  and device name of the other participant were replaced with obvious fakes.
* `room.json`, `rooms.json`, `home_users.json`, `friends.json`: hand-written
  answers in the shape the services gave, with invented values throughout.
