//! The real `plex_core` client the executor drives on a worker thread.
//!
//! Rule this module exists to hold: the account/server token is only ever given
//! to `plex_core`'s request methods (which send it as the `X-Plex-Token` header)
//! and to [`DirectPlay::headers`] (the same header, for mpv). It is never
//! appended to a URL, never stored in a [`DirectPlay`] label, and never logged.
//!
//! TLS: [`DirectPlay::https`] records whether the chosen connection base is
//! https, so a run can state whether the header token travelled over TLS or
//! over plaintext LAN http. `docs/RUN.md` records this decision.

use std::collections::HashMap;
use std::path::Path;

use plex_core::auth;
use plex_core::client_info::ClientInfo;
use plex_core::http::Http;
use plex_core::library::{
    self, BrowseOptions, Children, Hub, Item, ItemPage, MediaPart, Page, Section, SortOption,
};
use plex_core::progress::{self, Outcome, ProgressUpdate};
use plex_core::resources::{self, ServerResource};
use plex_core::secret::SecretString;
use plex_core::server::Server;

use crate::player::quality::{self, Mode, StreamAsk};
use crate::shell::resources::{CacheKey, Resolved};

/// plex.tv pairing base (`POST /api/v2/pins`, then polling).
pub const PLEX_TV_BASE: &str = "https://plex.tv";
/// Where the user completes the sign-in in their own browser.
pub const LINK_URL: &str = "https://plex.tv/link";

/// Debug builds only: `CHEWVY_DEV_PLEX_TV` points the plex.tv calls at a local
/// mock so the UI can be previewed with sample libraries. Release builds always
/// talk to plex.tv.
fn plex_tv_base() -> String {
    #[cfg(debug_assertions)]
    if let Ok(base) = std::env::var("CHEWVY_DEV_PLEX_TV") {
        return base;
    }
    PLEX_TV_BASE.to_owned()
}

/// A chosen connection plus the part key the server reported for it.
///
/// `Display` is safe to log: base, part key, https flag and token **length**.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectPlay {
    /// `Server::base_url()` of the connection this client actually chose.
    pub base: String,
    /// The part's own key, exactly as the server reported it.
    pub part_key: String,
    /// `true` when `base` is https: the token travelled over TLS.
    pub https: bool,
    token_len: usize,
}

impl DirectPlay {
    /// The `loadfile` argument: base + part key, **no** token in the URL.
    #[must_use]
    pub fn source(&self) -> String {
        format!("{}{}", self.base, self.part_key)
    }

    /// The header mpv must send with the request. The only method here that
    /// returns the credential, and only so the player seam can set it as a header.
    #[must_use]
    pub fn headers(&self, token: &SecretString) -> String {
        format!("X-Plex-Token: {}", token.expose())
    }

    /// Token length, for a report that must not print the token.
    #[must_use]
    pub fn token_len(&self) -> usize {
        self.token_len
    }

    /// A redacted one-liner safe for a log or run report.
    #[must_use]
    pub fn report_line(&self) -> String {
        format!(
            "direct-play base={} part_key={} https={} token=[redacted len={}]",
            self.base, self.part_key, self.https, self.token_len
        )
    }
}

impl std::fmt::Display for DirectPlay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.report_line())
    }
}

/// Sign-in, server connect, library reads and part-key resolution for one worker thread.
///
/// `Clone` copies the account token, server handle and recorded part keys; only
/// the ureq connection pool behind them is shared, so a clone opens no new
/// session. The artwork loader holds one `Arc<LiveClient>` clone from its worker
/// threads, which make `&self` reads only.
#[derive(Clone)]
pub struct LiveClient {
    client: ClientInfo,
    plex_tv: Http,
    pin: Option<auth::Pin>,
    claim: Option<String>,
    account: Option<SecretString>,
    server: Option<Server>,
    /// Part keys the server reported, keyed by (rating key, part index), so
    /// [`LiveClient::resolve`] never invents a key.
    parts: HashMap<CacheKey, String>,
    last_play: Option<(CacheKey, DirectPlay)>,
}

impl LiveClient {
    /// One identity file per install, so Plex sees one device.
    ///
    /// # Errors
    ///
    /// A bounded description when the identity cannot be loaded or created.
    pub fn new(identity: &Path, version: &str) -> Result<Self, String> {
        let client = ClientInfo::load_or_create(identity, version).map_err(|e| e.to_string())?;
        let plex_tv = Http::new(&plex_tv_base(), &client).map_err(|e| e.to_string())?;
        Ok(Self {
            client,
            plex_tv,
            pin: None,
            claim: None,
            account: None,
            server: None,
            parts: HashMap::new(),
            last_play: None,
        })
    }

    /// The client identifier this install presents (a device id, not a secret).
    #[must_use]
    pub fn client_identifier(&self) -> &str {
        &self.client.client_identifier
    }

    /// `POST /api/v2/pins`: the 4-character code and the plex.tv/link claim URL.
    ///
    /// # Errors
    ///
    /// A bounded description when plex.tv refuses the pin.
    pub fn start_pairing(&mut self) -> Result<(String, String), String> {
        let pin = auth::create_pin(&self.plex_tv).map_err(|e| e.to_string())?;
        let claim = auth::build_claim_url(&self.client, &pin).map_err(|e| e.to_string())?;
        let code = pin.code.clone();
        self.claim = Some(claim.clone());
        self.pin = Some(pin);
        Ok((code, claim))
    }

    /// The claim URL for the pairing code already displayed.
    #[must_use]
    pub fn claim_url(&self) -> Option<&str> {
        self.claim.as_deref()
    }

    /// One poll of the displayed code: `Ok(Some(token))` when the user finished
    /// the browser sign-in, `Ok(None)` while pending.
    ///
    /// # Errors
    ///
    /// A bounded description when polling fails.
    pub fn poll_pairing(&mut self) -> Result<Option<SecretString>, String> {
        let pin = self.pin.as_ref().ok_or("no pairing attempt in progress")?;
        let token = auth::poll_once(&self.plex_tv, pin).map_err(|e| e.to_string())?;
        if token.is_some() {
            self.account = token.clone();
            if let Some(pin) = self.pin.as_ref() {
                auth::retire_pin(&pin.id);
            }
        }
        Ok(token)
    }

    /// Reuse an account token obtained earlier.
    pub fn set_account(&mut self, token: SecretString) {
        self.account = Some(token);
    }

    /// The account token, when sign-in completed.
    #[must_use]
    pub fn account(&self) -> Option<SecretString> {
        self.account.clone()
    }

    /// The plex.tv account uuid for the current token.
    ///
    /// Non-secret and stable per account; it is the id the token store keys
    /// the Plex slot by, so one read pays for itself across sign-out and
    /// re-auth. Costs one plex.tv round-trip.
    pub fn account_uuid(&self) -> Result<String, String> {
        let token = self.account.as_ref().ok_or("sign in first")?;
        plex_core::api::fetch_account(&self.plex_tv, token)
            .map(|account| account.uuid)
            .map_err(|e| e.to_string())
    }

    /// `GET api/v2/resources`: the servers this install may browse, as labelled rows.
    ///
    /// # Errors
    ///
    /// A bounded description when plex.tv answers badly or no token is held.
    pub fn list_servers(&mut self) -> Result<Vec<ServerResource>, String> {
        let token = self.account.as_ref().ok_or("sign in first")?;
        let list = resources::fetch_resources(&self.plex_tv, token, &self.client.client_identifier)
            .map_err(|e| e.to_string())?;
        Ok(list.servers().to_vec())
    }

    /// Redacted lines describing every `resources` row and why it is or is not
    /// listed, then a connect attempt per listed server. Never prints a token.
    ///
    /// # Errors
    ///
    /// A bounded description when plex.tv answers badly or no token is held.
    pub fn server_report(&mut self) -> Result<Vec<String>, String> {
        let token = self.account.as_ref().ok_or("sign in first")?;
        let list = resources::fetch_resources(&self.plex_tv, token, &self.client.client_identifier)
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for row in list.excluded() {
            out.push(format!(
                "hidden  {:?} reason={:?} provides={:?} owned={} connections={}",
                row.label,
                row.reason,
                row.provides,
                row.owned,
                row.connections().len()
            ));
        }
        let servers = list.servers().to_vec();
        for row in &servers {
            out.push(format!(
                "listed  {:?} present={} connections={} candidates={}",
                row.display_name(),
                row.is_present(),
                row.connections().len(),
                row.candidates().len()
            ));
            for c in row.connections() {
                out.push(format!(
                    "          {:?} local={} relay={} ipv6={} uri={}",
                    c.protocol, c.local, c.relay, c.ip_v6, c.uri
                ));
            }
            match self.connect(row) {
                Ok(base) => out.push(format!("          connect OK via {base}")),
                Err(e) => out.push(format!("          connect FAILED: {e}")),
            }
        }
        Ok(out)
    }

    /// Connect to one listed row, keeping the connection `plex_core` chose.
    ///
    /// # Errors
    ///
    /// A bounded, sanitized failure (never a token, never a response body).
    pub fn connect(&mut self, resource: &ServerResource) -> Result<String, String> {
        let token = self.account.as_ref().ok_or("sign in first")?;
        let server = match plex_core::server::connect(resource, &self.client) {
            Ok(server) => server,
            Err(first) => plex_core::server::connect_shared(resource, &self.client, token)
                .map_err(|f| format!("{}: delegated {}", first.message(), f.message()))?,
        };
        let base = server.base_url().to_owned();
        self.server = Some(server);
        Ok(base)
    }

    /// The connected server's base URL (already sanitized by `plex_core::server`).
    #[must_use]
    pub fn base_url(&self) -> Option<&str> {
        self.server.as_ref().map(|s| s.base_url())
    }

    /// Read-only GET on the connected server for the shape-capture tool.
    ///
    /// # Errors
    ///
    /// A bounded description when no server is connected or the call fails.
    pub fn raw_get(&self, path: &str, query: &[(&str, &str)]) -> Result<(u16, Vec<u8>), String> {
        let server = self.server.as_ref().ok_or("no server connected")?;
        server
            .get(path, query)
            .map(|r| (r.status, r.body))
            .map_err(|e| e.to_string())
    }

    /// `GET /library/sections`.
    ///
    /// # Errors
    ///
    /// A bounded description when no server is connected or the call fails.
    pub fn sections(&self) -> Result<Vec<Section>, String> {
        let server = self.server.as_ref().ok_or("no server connected")?;
        library::list_sections(server).map_err(|e| e.to_string())
    }

    /// `GET /library/sections/{id}/{all}` for one page, with the sort value and
    /// watch-state filter the grid's chips chose.
    ///
    /// # Errors
    ///
    /// A bounded description when the page or section key is invalid.
    pub fn browse(
        &self,
        section_key: &str,
        start: usize,
        size: usize,
        options: &BrowseOptions,
    ) -> Result<ItemPage, String> {
        let server = self.server.as_ref().ok_or("no server connected")?;
        let page = Page::new(start, size).map_err(|e| e.to_string())?;
        library::browse_items(server, section_key, page, options).map_err(|e| e.to_string())
    }

    /// `GET /library/sections/{id}/sorts` — the sorts one section offers, in the
    /// order it named them.
    ///
    /// # Errors
    ///
    /// A bounded description when no server is connected or the call fails.
    pub fn sorts(&self, section_key: &str) -> Result<Vec<SortOption>, String> {
        let server = self.server.as_ref().ok_or("no server connected")?;
        library::sorts(server, section_key).map_err(|e| e.to_string())
    }

    /// `GET /library/metadata/{key}/children` — a show's seasons or a season's
    /// episodes. A captured children answer names no page or offset, so this is
    /// the whole listing this client asks for, not one page of it.
    ///
    /// # Errors
    ///
    /// A bounded description when the rating key is invalid or the call fails.
    pub fn children(&self, rating_key: &str) -> Result<Children, String> {
        let server = self.server.as_ref().ok_or("no server connected")?;
        library::children(server, rating_key).map_err(|e| e.to_string())
    }

    /// `GET /hubs?count=N` — the home shelves, each with the items the server
    /// chose for it (see [`plex_core::library::hubs`]).
    ///
    /// # Errors
    ///
    /// A bounded description when no server is connected, `count` is out of
    /// range, or the call fails.
    pub fn hubs(&self, count: usize) -> Result<Vec<Hub>, String> {
        let server = self.server.as_ref().ok_or("no server connected")?;
        library::hubs(server, count).map_err(|e| e.to_string())
    }

    /// `GET /hubs/search?query=…&limit=N` on the connected server, kept as the
    /// server's own groups (see [`library::search_hubs`]): each [`Hub`] holds its
    /// `title`, `type` and the matching rows in [`Hub::items`].
    ///
    /// # Errors
    ///
    /// A bounded description when no server is connected, the query is invalid or
    /// `limit` is out of range.
    pub fn search_hubs(&self, query: &str, limit: usize) -> Result<Vec<Hub>, String> {
        let server = self.server.as_ref().ok_or("no server connected")?;
        library::search_hubs(server, query, limit).map_err(|e| e.to_string())
    }

    /// `GET /library/metadata/{rating_key}`, recording each part's key for [`Self::resolve`].
    ///
    /// # Errors
    ///
    /// A bounded description when the rating key is invalid or the call fails.
    pub fn metadata(&mut self, rating_key: &str) -> Result<Item, String> {
        let server = self.server.as_ref().ok_or("no server connected")?;
        let item = library::metadata(server, rating_key).map_err(|e| e.to_string())?;
        for (index, part) in item.parts.iter().enumerate() {
            if let Some(key) = &part.key {
                self.parts.insert(
                    CacheKey {
                        rating_key: rating_key.to_owned(),
                        part: index,
                    },
                    key.clone(),
                );
            }
        }
        Ok(item)
    }

    /// The parts recorded for one item (never a key the server did not report).
    #[must_use]
    pub fn recorded_parts(&self, rating_key: &str) -> Vec<(usize, String)> {
        let mut out: Vec<(usize, String)> = self
            .parts
            .iter()
            .filter(|(k, _)| k.rating_key == rating_key)
            .map(|(k, key)| (k.part, key.clone()))
            .collect();
        out.sort_by_key(|(part, _)| *part);
        out
    }

    /// Build the direct-play request for one recorded part.
    ///
    /// The returned [`Resolved`] source carries **no** token; the token is only
    /// available through [`DirectPlay::headers`].
    #[must_use]
    pub fn resolve(&mut self, key: &CacheKey) -> Option<(Resolved, DirectPlay)> {
        let server = self.server.as_ref()?;
        let part_key = self.parts.get(key)?.clone();
        let base = server.base_url().to_owned();
        let token = server.token();
        let play = DirectPlay {
            https: base.starts_with("https://"),
            token_len: token.expose().len(),
            base,
            part_key,
        };
        let resolved = Resolved::new(&key.rating_key, key.part, &play.source());
        self.last_play = Some((key.clone(), play.clone()));
        Some((resolved, play))
    }

    /// The play request this client last resolved (for the player seam).
    #[must_use]
    pub fn last_play(&self) -> Option<(CacheKey, DirectPlay)> {
        self.last_play.clone()
    }

    /// The token that authorises the last resolved play, for the header only.
    #[must_use]
    pub fn play_headers(&self) -> Option<String> {
        let (_, play) = self.last_play.as_ref()?;
        let server = self.server.as_ref()?;
        let token = server.token();
        Some(play.headers(&token))
    }

    /// The server's HLS URL for `mode` (Direct Stream or a transcode), with no
    /// token in it: mpv sends [`Self::play_headers`] with every request.
    #[must_use]
    pub fn stream_url(
        &self,
        rating_key: &str,
        mode: Mode,
        offset: Option<f64>,
        session: &str,
    ) -> Option<String> {
        let server = self.server.as_ref()?;
        let ask = StreamAsk {
            base: server.base_url(),
            rating_key,
            session,
            client_id: &self.client.client_identifier,
            product: &self.client.product,
            offset,
        };
        quality::stream_url(&ask, mode)
    }

    /// End a server stream session. A failure is ignored: the server ends an
    /// abandoned session on its own.
    pub fn stop_stream(&self, session: &str) {
        if let Some(server) = self.server.as_ref() {
            let _ = server.get(STOP_PATH, &[("session", session)]);
        }
    }

    /// Report one playback position (`GET /:/timeline`). `Ok(None)` means no
    /// request left the process: reporting is off, or the update carried nothing
    /// reportable - the normal answer until a user opts in.
    ///
    /// # Errors
    /// A description when the report was sendable but there is no server or the
    /// request failed.
    pub fn report_timeline(&self, update: &ProgressUpdate) -> Result<Option<u16>, String> {
        // Asked here so a disabled report never needs a connection at all.
        if update.refusal().is_some() {
            return Ok(None);
        }
        let server = self
            .server
            .as_ref()
            .ok_or_else(|| "not connected to a server".to_owned())?;
        match progress::report_timeline(server, update) {
            Ok(Outcome::Reported { status }) => Ok(Some(status)),
            // `refusal()` above has already answered the only case that is left.
            Ok(Outcome::NotEnabled { .. }) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    /// Tell the server one item is watched (`GET /:/scrobble`). Unlike a
    /// playback report this is an explicit request, so the opt-in setting does
    /// not gate it.
    ///
    /// # Errors
    /// A description when there is no server, the key is unusable, or the
    /// request failed.
    pub fn mark_watched(&self, rating_key: &str) -> Result<u16, String> {
        let server = self
            .server
            .as_ref()
            .ok_or_else(|| "not connected to a server".to_owned())?;
        progress::mark_watched(server, rating_key).map_err(|e| e.to_string())
    }
}

/// Where a Direct Stream or transcode session is ended.
const STOP_PATH: &str = "/video/:/transcode/universal/stop";

/// The part key a play request needs, from a core part.
#[must_use]
pub fn part_key(part: &MediaPart) -> Option<&str> {
    part.key.as_deref()
}

#[cfg(test)]
mod tests {
    use super::DirectPlay;
    use plex_core::secret::SecretString;

    #[test]
    fn mpv_header_is_name_colon_value_and_source_has_no_token() {
        let play = DirectPlay {
            base: "https://10-0-0-2.abc.plex.direct:32400".to_owned(),
            part_key: "/library/parts/7/1/file.mkv".to_owned(),
            https: true,
            token_len: 3,
        };
        assert_eq!(
            play.headers(&SecretString::new("t0k3n")),
            "X-Plex-Token: t0k3n"
        );
        assert!(!play.source().contains("t0k3n"));
    }
}
