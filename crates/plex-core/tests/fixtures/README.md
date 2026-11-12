# Mock Plex fixtures

The files in this directory are **hand-written mock responses**, using the
field names the sign-in flow reads (`id`, `code`, `authToken`, `uuid`,
`username`, `email`, `rememberExpiresAt`, `twoFactorEnabled`). They are not
captures of a real account, and no test here signs in to one.

* `pin_created.json` / `pin_created_second.json`: a `POST /api/v2/pins`
  answer, with the pin `id` and the user-visible `code`.
* `pin_pending.json`: a poll answer with no `authToken` yet.
* `pin_claimed.json` / `pin_claimed_second.json`: a poll answer carrying
  `authToken` = `MOCKTOKEN-9f4c1b7ae2d84c0f`.
* `user.json` / `user_2fa.json`: `GET /api/v2/user` answers, the second with
  `twoFactorEnabled: true` and `rememberExpiresAt: 0`.

`MOCKTOKEN-9f4c1b7ae2d84c0f` is a distinctive fake bearer token, so the
redaction tests can prove that exact string never reaches a log, an error
message or `Debug` output. It is not a real credential.

The files under `real/` have the shape of library and metadata answers
recorded from a Plex Media Server: the field names, nesting and value types
are what the server sent, and they pin the wire shapes the library models
parse. Their contents are not a real library. Every title, person, character,
studio, synopsis, tagline, external identifier and timestamp was replaced with
an invented value, and file paths and section identifiers are redacted.
