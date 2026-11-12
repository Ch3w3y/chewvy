# Plex sign-in and server discovery notes

What Chewvy's Plex sign-in is built on, and what is still unverified. Plex
publishes no specification for these endpoints, so the shapes below were
corroborated against a third-party client, python-plexapi's `myplex.py`
(<https://github.com/pkkid/python-plexapi/blob/master/plexapi/myplex.py>, read
2026-11-03; `L` numbers are lines of that file at that date). Labels: **[P]**
published by Plex, **[3]** third-party corroborated, **[U]** unverified.

Source comments cite this file by section, item and gap number, so those
numbers are kept stable.

## 1. Chosen flow: PIN sign-in in the browser (the app never sees a password)

1. `POST https://plex.tv/api/v2/pins` with the `X-Plex-*` headers (`PINS`
   L1764). Classic mode sends no `strong` parameter (L1889, L1891) and returns
   `id` (L1895) and `code` (L1896).
2. One pin object has two modes; do not conflate them. L1718-1719: "login can
   be done using the four character PIN which the user must enter at
   https://plex.tv/link or using Plex OAuth".
   (a) Classic PIN: `def pin` L1786; `oauthUrl` raises `BadRequest` in this
   mode (L1794, L1801).
   (b) OAuth/claim: `params={'strong': True}` (L1886-1887); the browser URL is
   `https://app.plex.tv/auth/#!?{urlencode(params)}` (L1817, L2334) with
   `clientID` = `X-Plex-Client-Identifier` (L1805),
   `context[device][product|version|platform|platformVersion|device|deviceName]`
   (L1806-1811), `code` (L1812) and an optional `forwardUrl` (L1814-1815).
   The live URL's behaviour and code length are [U].
3. Poll `GET {PINS}/{id}` (L1907) every second (`POLLINTERVAL`, L1743, L1765)
   for up to 120 seconds (`run(timeout=120)`, L1819). No `authToken` attribute
   means pending (L1912-1914); its presence means success (L1916-1917). Any
   poll exception marks the pin expired (L1875, L1927, L2393, L2448), after
   which a new pin is required; the reference client cannot tell expiry from a
   4xx or 5xx. The expired-pin status and body are [U].
4. A second host exists for the JWT variant [3, L2052-2053]:
   `https://clients.plex.tv/api/v2/pins` and `.../api/v2/auth`. Chewvy uses the
   `plex.tv` pair from items 1 to 3. Which host Plex prefers for a Linux client
   is [U].
5. `GET https://plex.tv/api/v2/user` returns `authToken` (L58), `uuid`,
   `rememberExpiresAt` (L91) and the home, guest and restricted flags;
   `twoFactorEnabled` is documented at L104 and assigned at L192.
6. Discovery:
   `GET https://plex.tv/api/v2/resources?includeHttps=1&includeRelay=1&includeIPv6=1`
   (`key` L1449; class L1413). Attributes confirmed in `_loadData`
   (L1458-1479; docstring L1422-1446): `accessToken`, `clientIdentifier`,
   `createdAt`, `device`, `dnsRebindingProtection`, `home`, `httpsRequired`
   (L1464), `lastSeenAt`, `name`, `natLoopbackSupported`, `owned` (L1468),
   `ownerId`, `platform`, `platformVersion`, `presence` (L1472), `product`,
   `productVersion`, `provides` (L1475), `publicAddressMatches`, `relay`
   (L1477), `sourceTitle`, `synced`, `connections[]` (L1482). Connection
   fields (`ResourceConnection`, L1599-1625): `address`, `port`, `protocol`
   (L1622), `uri`, `local`, `relay` (L1623), `IPv6` (L1619), and a derived
   `httpuri` (L1625). `ownedBy`, `machineIdentifier`, `httpsEnabled`,
   `reachable` and a connection `secure` flag do **not** exist on these rows:
   never code against them. Reachability is found by probing connections
   (L1590-1597); `machineIdentifier` is learned from the server after
   connecting. The reference selects servers with
   `provides == 'server' and owned` (L897), contacts each resource with its
   **own** `accessToken` rather than the account token (L1594), and orders
   candidates local before remote before relay, https before http, IPv4 before
   IPv6 (L1452-1454).
7. A selected server listens on HTTP port 32400, "the primary port that Plex
   uses for communication" [P, pms-docker README]. `X-Plex-Token` is sent as a
   header (L245) and never in a query string; the reference client logs a
   `?X-Plex-Token=` URL (L2520), which Chewvy must not copy. Unclaimed servers
   are claimed at `plex.tv/link` or with `PUT api/v2/pins/link` [3, L121];
   container installs use `PLEX_CLAIM` [P].

## 2. X-Plex-* headers

`X-Plex-Product`, `X-Plex-Client-Identifier` (a stable per-install UUID),
`X-Plex-Version`, `X-Plex-Platform`, `X-Plex-Platform-Version`,
`X-Plex-Device`, `X-Plex-Device-Name`, `X-Plex-Provides`, `X-Plex-Language`,
`Accept: application/json` (L2467), and `X-Plex-Token` once signed in [3].
Which values Plex accepts from an unregistered desktop client is [U].

## 3. Token security, expiry, cancel and retry

- `authToken` is an account-wide bearer secret; the reference adds it to its
  log-secrets list (L159). It must never reach Git, logs, process arguments,
  crash dumps or the interface, and travels in a header, not a query string.
  Access to one server additionally uses that resource's `accessToken`
  (§1 item 6).
- Storage: the XDG Secret Service over D-Bus first (no plaintext at rest;
  needs a running session daemon). The alternative is a `0600` file in the
  config directory, which is plaintext at rest: any process of the same user,
  a backup or a stolen disk can read it. The file is used only after the user
  has been shown that and agreed.
- Expiry: `rememberExpiresAt` (L91). "Remember the account token for 14 days"
  is a reference docstring only. Pin lifetime and token lifetime are [U].
- Cancel and sign-out: `DELETE https://plex.tv/api/v2/users/signout` (L118)
  plus deleting the local token. Cancelling an in-flight pairing is simply
  stopping the poll and dropping the pin.
- Chewvy's own rules, not observed Plex behaviour [U]: a 401 or 422 clears the
  token and asks for a new pin, with no blind retry and no password fallback;
  a timeout is a stated interface state; two-factor prompts are handled in the
  browser.

## 4. Device-JWT flow (recorded, not implemented)

`POST https://clients.plex.tv/api/v2/auth/jwk` with `{'jwk': ...}`
(L2223-2225), `GET /auth/nonce` (L2230), `POST /auth/token` (L2236),
`GET /auth/keys` (L2243). Pin creation on that host sends a locally generated
JWK (L2401-2409; an Ed25519 key, L2100), so the flow requires generating a key
first. It yields a scoped, verifiable token rather than an account bearer
token; how it bootstraps without a token, and its scopes, are [U]. Chewvy uses
§1.

## 5. Unresolved

1. No Plex-published specification for pins or claims has been found.
2. Whether `strong=True` is appropriate for a desktop client; the live URL
   form and code length.
3. Pin lifetime, and the status returned for an expired pin.
4. Accepted `X-Plex-Platform` values, and the rules behind HTTP 429.
5. `*.plex.direct` TLS against a self-signed server certificate, and pinning.
6. What the relay connection requires.
7. `provides` values, and scoping for shared (`owned = 0`) and managed users.
8. Plex Home and guest users (`plex.tv/api/home/users*`, L109-118).
9. Rate limits and ETag rules.
10. (Settled: token storage is the Secret Service, with a consented file
    fallback; see §3.)
11. How `PLEX_CLAIM` relates to the pins endpoint.

## 6. Sources (accessed 2026-11-03)

- [P] Plex's container README: `PLEX_CLAIM`, and port 32400 as "primary port
  that Plex uses for communication".
  <https://github.com/plexinc/pms-docker/blob/master/README.md>
- [P] Plex support, "Finding an authentication token / X-Plex-Token" (title
  only; the article body was not readable without accepting cookies).
  <https://support.plex.tv/articles/204059436-finding-an-authentication-token-x-plex-token/>
- [P] <https://developer.plex.tv/> was reachable ("Plex Media Server API"),
  but its landing page carries no sign-in specification and its linked pages
  were not enumerated.
- [3] python-plexapi `myplex.py`, cited by line above.
  <https://github.com/pkkid/python-plexapi/blob/master/plexapi/myplex.py>

Progress reporting (`/:/timeline`, `/:/scrobble`) is documented in
`crates/plex-core/src/progress.rs`.
