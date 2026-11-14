//! The Jellyfin servers this install can browse, and the reads its screens ask
//! for.
//!
//! [`Server`] is one signed-in server: a transport, the device identity the
//! install paired with it, and the user id every query is scoped by.
//! [`jellyfin_core::browse::Client`] borrows all three, so it is built per read
//! and the answers are handed to [`super::mapping`] on the way out. Everything
//! here blocks, so it runs on the worker thread ([`crate::watch`]) and on the
//! artwork loader's own threads ([`crate::ui::images`]).

// As in the other modules of this source, the lints product code keeps on are
// relaxed for the tests, which unwrap answers they just proved exist.
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::indexing_slicing,
        reason = "tests may unwrap; product code keeps these lints on"
    )
)]

use std::sync::Arc;

use jellyfin_core::browse::{Client, ItemKind, ME_PATH};
use jellyfin_core::device::DeviceInfo;
use jellyfin_core::dto::{Item as JfItem, QueryResult, User};
use jellyfin_core::header::media_browser_header;
use jellyfin_core::http::Http;
use jellyfin_core::playback::{Delivery, SessionBody};
use jellyfin_core::socket::Socket;
use jellyfin_core::JfError;
use plex_core::library::{Hub, Item};
use plex_core::secret::SecretString;
use plex_core::tokens::{load_plex_token, TokenStore};

use crate::player::quality::Mode;
use crate::source::{self, jellyfin_id, Library};
use crate::sources::mapping;
use crate::sources::{jellyfin_token_key, JellyfinServer};

/// What happened to a playing item, as a server hears about it. The three
/// session endpoints take the same body and differ only in what the server
/// makes of it, so the choice travels with the call instead of in three
/// look-alike methods.
#[derive(Clone, Copy)]
pub(crate) enum Report {
    /// It started.
    Playing,
    /// It is somewhere else now.
    Progress,
}

/// A resolved Jellyfin source: enough to open the player, and enough to keep
/// the server's idea of this session current.
pub(crate) struct Playback {
    /// The row the app shows while the item plays.
    pub row: Item,
    /// The URL the player loads. No token in it.
    pub source: String,
    /// The `MediaBrowser` header that authorises that URL (never logged).
    pub header: SecretString,
    /// Where the server says this user had got to, in seconds.
    pub start: Option<f64>,
    /// The item's length, in seconds, when the source named one.
    pub duration: Option<f64>,
    /// Which server answered, because every report goes back to it.
    pub server: String,
    /// The item, as that server ids it.
    item: String,
    /// Which copy of the file this is, as the resolve step named it.
    media_source: String,
    /// The session the server named for this playback, when it named one.
    session: Option<String>,
    /// Whether the server's own stream plays rather than the file as it sits.
    pub transcoding: bool,
}

impl Playback {
    /// The body of a session report about this source, at one position.
    pub(crate) fn body(&self, position_ms: u64, paused: bool) -> SessionBody<'_> {
        SessionBody {
            item_id: &self.item,
            media_source_id: Some(&self.media_source),
            play_session_id: self.session.as_deref(),
            position: std::time::Duration::from_millis(position_ms),
            paused,
            duration: self.duration.map(std::time::Duration::from_secs_f64),
            transcoding: self.transcoding,
        }
    }

    /// The server-side session behind a stream the server is making, for the
    /// player to hand back when it leaves the item. `None` for a file.
    pub(crate) fn stream_session(&self) -> Option<String> {
        self.transcoding
            .then(|| self.session.clone().unwrap_or_default())
    }
}

/// What a quality-menu choice asks a Jellyfin server for.
pub(crate) fn delivery(mode: Mode) -> Delivery {
    match mode {
        Mode::DirectPlay => Delivery::ServerChoice,
        Mode::DirectStream => Delivery::Remux,
        Mode::Transcode { resolution, kbps } => {
            let (width, height) = resolution.split_once('x').unwrap_or_default();
            Delivery::Transcode {
                max_bitrate: u64::from(kbps) * 1000,
                max_width: width.parse().unwrap_or(1920),
                max_height: height.parse().unwrap_or(1080),
            }
        }
    }
}

/// The two home shelves Jellyfin answers feed, under the ids Home already
/// recognises: `homecontinue` and `home.onDeck` name Plex's own two, so a
/// Jellyfin shelf of the same kind joins it rather than starting a third row.
/// Home's hero comes from `homecontinue`, so a Jellyfin row can be that hero.
pub(crate) const RESUME_ID: &str = "home.continue";
pub(crate) const RESUME_TITLE: &str = "Continue Watching";
pub(crate) const NEXTUP_ID: &str = "home.onDeck";
pub(crate) const NEXTUP_TITLE: &str = "Next Up";
/// The group a server's search results arrive in.
const SEARCH_ID: &str = "search";
const SEARCH_TITLE: &str = "Results";

/// Every Jellyfin server this install can browse, as its registry row left it.
///
/// Cheap to clone: a screen, a worker and the artwork loader hold the same list.
#[derive(Clone, Default)]
pub(crate) struct Jellyfin {
    servers: Arc<Vec<Arc<Server>>>,
}

impl Jellyfin {
    /// Open one handle per registry row this install can actually ask: a token
    /// on file, and a user the server will name for it.
    ///
    /// A row that fails is named in the returned notes and left out — the
    /// sources that do answer still browse.
    pub(crate) fn open(
        servers: &[JellyfinServer],
        store: Option<&dyn TokenStore>,
        device: &DeviceInfo,
    ) -> (Jellyfin, Vec<String>) {
        let mut opened = Vec::new();
        let mut notes = Vec::new();
        for row in servers {
            match Server::open(row, store, device) {
                Ok(server) => opened.push(Arc::new(server)),
                Err(note) => notes.push(format!("{}: {note}", row.name)),
            }
        }
        (
            Jellyfin {
                servers: Arc::new(opened),
            },
            notes,
        )
    }

    /// The rail's library rows: one per library of one server, with that
    /// server's label for the header it sits under.
    pub(crate) fn libraries(&self) -> (Vec<Library>, Vec<String>) {
        let mut rows = Vec::new();
        let mut notes = Vec::new();
        for server in self.servers.iter() {
            match server.view_rows() {
                Ok(views) => rows.extend(views.into_iter().map(|(id, title)| Library {
                    origin: crate::source::Origin::Jellyfin(server.id.clone()),
                    server: server.name.clone(),
                    id: jellyfin_id(&server.id, &id),
                    title,
                })),
                Err(note) => notes.push(format!("{}: {note}", server.name)),
            }
        }
        (rows, notes)
    }

    /// Home's Jellyfin shelves: what this user is watching, then what is next,
    /// per server. A shelf with no rows is left out, and a server that will not
    /// answer is named rather than assumed empty.
    pub(crate) fn shelves(&self, rows: usize) -> (Vec<Hub>, Vec<String>) {
        let mut shelves = Vec::new();
        let mut notes = Vec::new();
        for server in self.servers.iter() {
            let asked = [
                (RESUME_ID, RESUME_TITLE, server.resume()),
                (NEXTUP_ID, NEXTUP_TITLE, server.next_up()),
            ];
            for (hub_id, label, answer) in asked {
                match answer {
                    Ok(mut answer) => {
                        answer.items.truncate(rows);
                        if !answer.items.is_empty() {
                            shelves.push(shelf(server, hub_id, label, &answer));
                        }
                    }
                    Err(note) => notes.push(format!("{}: {note}", server.name)),
                }
            }
        }
        (shelves, notes)
    }

    /// What every server has for `query`: one group per server that found
    /// something. A server that will not answer is left out, so the search
    /// still shows what the others found.
    pub(crate) fn search(&self, query: &str, rows: usize) -> Vec<Hub> {
        let mut found = Vec::new();
        for server in self.servers.iter() {
            let Ok(mut answer) = server.search(query, rows) else {
                continue;
            };
            answer.items.truncate(rows);
            if !answer.items.is_empty() {
                found.push(shelf(server, SEARCH_ID, SEARCH_TITLE, &answer));
            }
        }
        found
    }

    /// One server, if this install has it.
    pub(crate) fn server(&self, id: &str) -> Option<Arc<Server>> {
        self.servers.iter().find(|s| s.id == id).cloned()
    }

    /// One page of one library, by the rail's ids, with the `more` the source
    /// gives and its own row count when it names one.
    pub(crate) fn page(
        &self,
        server: &str,
        view_id: &str,
        start: usize,
        limit: usize,
    ) -> Result<(Vec<Item>, bool, Option<u64>), String> {
        let server = self.server(server).ok_or_else(|| server.to_owned())?;
        // A recursive listing without a type filter also answers the library's
        // folders, seasons and episodes; a library page shows only its titles.
        let answer = server.view_rows_of(view_id, &[ItemKind::Movie, ItemKind::Series])?;
        Ok(paginate(&server.id, answer, start, limit))
    }

    /// One row of one server.
    pub(crate) fn item(&self, server: &str, id: &str) -> Result<Item, String> {
        let server = self.server(server).ok_or_else(|| server.to_owned())?;
        server.item(id).map(|row| mapping::item(&server.id, &row))
    }

    /// The source this server will hand this client for one item — or the
    /// reason it will not. Both halves come from the same two calls: the row the
    /// app shows, and what the server says about the item's file for the profile
    /// this player claims.
    pub(crate) fn playback(
        &self,
        server: &str,
        id: &str,
        delivery: Delivery,
    ) -> Result<Playback, String> {
        let server = self.server(server).ok_or_else(|| server.to_owned())?;
        let item = server.item(id)?;
        let claimed = crate::player::profile::mpv_device_profile(&server.device);
        let info = server.playback_info(id, &claimed, delivery)?;
        let target =
            jellyfin_core::playback::resolve(&item, &info, delivery).map_err(|w| w.to_string())?;
        Ok(Playback {
            server: server.id.clone(),
            item: id.to_owned(),
            row: mapping::item(&server.id, &item),
            source: target.stream_url(server.base()),
            header: crate::player::profile::stream_header(&server.device, &server.token),
            start: target.resume_at.map(|at| at.as_secs_f64()),
            duration: target.duration.map(|end| end.as_secs_f64()),
            media_source: target.media_source_id,
            session: target.play_session_id,
            transcoding: target.transcoding,
        })
    }

    /// The player has left `playing`: say where it stopped when the server was
    /// hearing about the session, and end the encode behind a stream. The
    /// encode is ended whatever the reporting setting says: it is work this
    /// client started on the server, not a record of what was watched.
    pub(crate) fn close(
        &self,
        playing: &Playback,
        position_ms: u64,
        reported: bool,
    ) -> Result<(), String> {
        let server = self
            .server(&playing.server)
            .ok_or_else(|| playing.server.clone())?;
        let stopped = match reported {
            true => server.stopped(&playing.body(position_ms, false)),
            false => Ok(()),
        };
        match playing.stream_session() {
            Some(session) => server.stop_encoding(&session).and(stopped),
            None => stopped,
        }
    }

    /// Tell a server something about a session it is already hearing about. A
    /// server counts what it is told and nothing else, which is where
    /// "Continue Watching" for a Jellyfin item is won or lost.
    pub(crate) fn report(
        &self,
        server: &str,
        report: Report,
        body: &jellyfin_core::playback::SessionBody<'_>,
    ) -> Result<(), String> {
        let server = self.server(server).ok_or_else(|| server.to_owned())?;
        match report {
            Report::Playing => server.playing(body),
            Report::Progress => server.progress(body),
        }
    }

    /// The level below one row: its seasons when it is a series, its contents
    /// when it is a season.
    pub(crate) fn listed(&self, server: &str, kind: &str, id: &str) -> Result<Vec<Item>, String> {
        let server = self.server(server).ok_or_else(|| server.to_owned())?;
        let answer = if kind == "show" {
            server.seasons(id)?
        } else {
            server.view_rows_of(id, &[ItemKind::Episode])?
        };
        Ok(answer
            .items
            .iter()
            .map(|row| mapping::item(&server.id, row))
            .collect())
    }

    /// The bytes behind one artwork request: status and body, as the artwork
    /// loader asks for them. `path` is a [`crate::source`] address; the server
    /// named in it is asked, and the route behind the marker is what it hears.
    pub(crate) fn image(&self, server: &str, path: &str) -> Result<(u16, Vec<u8>), String> {
        let server = self.server(server).ok_or_else(|| server.to_owned())?;
        let route = source::split_artwork(path)
            .map(|(_, route)| route)
            .ok_or_else(|| "that is not an artwork address".to_owned())?;
        server.image(route)
    }
}

/// The listing that holds one model row's children: a series answers its
/// seasons, a season answers its episodes, and nothing else has children to
/// list. The value is Jellyfin's own name for the listing kind.
pub(crate) fn listing_kind(kind: Option<&str>) -> Option<&'static str> {
    match kind {
        Some("show") => Some("show"),
        Some("season") => Some("season"),
        _ => None,
    }
}

/// The Jellyfin server one artwork request names, or `None` when it is a Plex
/// request the connected session answers.
#[must_use]
pub(crate) fn path_server(path: &str) -> Option<&str> {
    source::split_artwork(path).map(|(server, _)| server)
}

/// One page out of a whole answer: Jellyfin hands back a library at once, so
/// the split into pages happens here.
fn paginate(
    server: &str,
    answer: QueryResult<JfItem>,
    start: usize,
    limit: usize,
) -> (Vec<Item>, bool, Option<u64>) {
    let rows = answer.items;
    let items = rows
        .iter()
        .skip(start)
        .take(limit)
        .map(|row| mapping::item(server, row))
        .collect();
    (items, start + limit < rows.len(), answer.total_record_count)
}

/// One shelf's rows as a hub, labelled with its server so a shelf of two servers
/// can be told apart.
fn shelf(server: &Server, hub_id: &str, label: &str, answer: &QueryResult<JfItem>) -> Hub {
    let title = format!("{label} · {}", server.name);
    mapping::shelf(&server.id, hub_id, &title, answer)
}

/// One signed-in Jellyfin server.
pub(crate) struct Server {
    /// The id `sources.json` keys this server by, and the one its browse ids carry.
    id: String,
    /// The server's own label, as its registry row holds it.
    name: String,
    http: Http,
    device: DeviceInfo,
    token: SecretString,
    user_id: String,
    /// The signed-in user's name, as the registry row recorded it.
    user_name: Option<String>,
}

impl Server {
    /// The id `sources.json` keys this server by.
    pub(crate) fn id(&self) -> &str {
        &self.id
    }

    /// The signed-in user's name, when the registry row recorded one.
    pub(crate) fn user_name(&self) -> Option<&str> {
        self.user_name.as_deref().filter(|name| !name.is_empty())
    }

    /// The client a group session calls through. Its errors stay the
    /// server's own, which is what tells a refusal from a lost connection.
    pub(crate) fn api(&self) -> Result<Client<'_>, JfError> {
        Client::new(&self.http, &self.device, &self.token, &self.user_id)
    }

    /// This device's session socket on the server.
    pub(crate) fn socket(&self) -> Result<Socket, JfError> {
        Socket::connect(self.base(), &self.device, &self.token)
    }

    /// The address this server's own links come back relative to, with no
    /// trailing slash: what a stream URL has to be joined to.
    fn base(&self) -> &str {
        self.http.base()
    }

    /// What the server says about one item's file, for the profile this client
    /// claims. A `POST` that starts nothing.
    fn playback_info(
        &self,
        item_id: &str,
        profile: &jellyfin_core::playback::DeviceProfile,
        delivery: Delivery,
    ) -> Result<jellyfin_core::dto::PlaybackInfo, String> {
        self.client()?
            .playback_info(item_id, profile, delivery)
            .map_err(|error| error.to_string())
    }

    fn stop_encoding(&self, play_session_id: &str) -> Result<(), String> {
        self.client()?
            .stop_encoding(play_session_id)
            .map_err(|error| error.to_string())
    }

    /// One session report. `/Sessions/*` is not user-scoped: the session is the
    /// access token's own, which is why these three need no `userId`.
    fn playing(&self, body: &jellyfin_core::playback::SessionBody<'_>) -> Result<(), String> {
        self.client()?
            .report_playing(body)
            .map_err(|error| error.to_string())
    }

    fn progress(&self, body: &jellyfin_core::playback::SessionBody<'_>) -> Result<(), String> {
        self.client()?
            .report_progress(body)
            .map_err(|error| error.to_string())
    }

    fn stopped(&self, body: &jellyfin_core::playback::SessionBody<'_>) -> Result<(), String> {
        self.client()?
            .report_stopped(body)
            .map_err(|error| error.to_string())
    }

    /// Reach one server with the install's device identity and its stored token.
    ///
    /// A row may not name its user (it is filled in by the sign-in that filled
    /// the row); the server is asked for it, since every browse query needs one.
    fn open(
        row: &JellyfinServer,
        store: Option<&dyn TokenStore>,
        device: &DeviceInfo,
    ) -> Result<Server, String> {
        let key = jellyfin_token_key(row).ok_or("the source has no id to file its token under")?;
        let token = load_plex_token(store.ok_or("no secret store is available")?, Some(&key))
            .map_err(|error| error.to_string())?
            .ok_or("this install has no token saved for it")?;
        let http = Http::new(&row.base_url).map_err(|error| error.to_string())?;
        let authorization = media_browser_header(device, Some(&token));
        let user_id = match row.user_id.as_deref() {
            Some(id) if !id.is_empty() => id.to_owned(),
            // A row may name no user; the server does, and every browse query
            // is scoped by one. Asked here, once, rather than per read.
            _ => {
                let user = http
                    .get(ME_PATH, &[], Some(&authorization))
                    .map_err(|error| error.to_string())?
                    .json::<User>()
                    .map_err(|error| error.to_string())?;
                (!user.id.is_empty())
                    .then_some(user.id)
                    .ok_or_else(|| "the server named no user for its token".to_owned())?
            }
        };
        Ok(Server {
            id: row.server_id.clone(),
            name: row.name.clone(),
            http,
            device: device.clone(),
            token,
            user_id,
            user_name: row.user_name.clone(),
        })
    }

    /// The client every read of this server goes through.
    fn client(&self) -> Result<Client<'_>, String> {
        Client::new(&self.http, &self.device, &self.token, &self.user_id)
            .map_err(|error| error.to_string())
    }

    /// The library rows of the rail.
    fn view_rows(&self) -> Result<Vec<(String, String)>, String> {
        Ok(self
            .client()?
            .views()
            .map_err(|error| error.to_string())?
            .items
            .into_iter()
            .filter_map(|view| Some((view.id, view.name?)))
            .collect())
    }

    fn resume(&self) -> Result<QueryResult<JfItem>, String> {
        self.client()?.resume().map_err(|error| error.to_string())
    }

    fn next_up(&self) -> Result<QueryResult<JfItem>, String> {
        self.client()?.next_up().map_err(|error| error.to_string())
    }

    fn search(&self, query: &str, rows: usize) -> Result<QueryResult<JfItem>, String> {
        let rows = u32::try_from(rows).unwrap_or(u32::MAX);
        self.client()?
            .search_items(query, rows)
            .map_err(|error| error.to_string())
    }

    fn view_rows_of(
        &self,
        view_id: &str,
        kinds: &[ItemKind],
    ) -> Result<QueryResult<JfItem>, String> {
        self.client()?
            .view_items(view_id, kinds)
            .map_err(|e| e.to_string())
    }

    fn seasons(&self, series_id: &str) -> Result<QueryResult<JfItem>, String> {
        self.client()?
            .seasons(series_id)
            .map_err(|error| error.to_string())
    }

    fn item(&self, item_id: &str) -> Result<JfItem, String> {
        self.client()?
            .item(item_id)
            .map_err(|error| error.to_string())
    }

    /// One image route, answered with the server's status and bytes. The token
    /// rides the `Authorization` header the Jellyfin client builds; the path is
    /// sent as it is, size query included.
    fn image(&self, path: &str) -> Result<(u16, Vec<u8>), String> {
        let authorization = media_browser_header(&self.device, Some(&self.token));
        self.http
            .get(path, &[], Some(&authorization))
            .map(|answer| (answer.status, answer.body))
            .map_err(|error| error.to_string())
    }
}

/// The loader picks a server from the artwork value alone, so a route is told
/// from a Plex path by what it starts with — and a malformed value names nothing.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_artwork_route_names_its_server_and_nothing_else_does() {
        assert_eq!(
            path_server("jellyfin:0f1e/Items/8/Images/Primary?tag=t&maxWidth=60"),
            Some("0f1e")
        );
        assert_eq!(path_server("/photo/:/transcode?url=%2Flibrary%2F1"), None);
        assert_eq!(path_server("jellyfin:/Items/8/Images/Primary"), None);
        assert_eq!(path_server(""), None);
    }

    /// The two kinds of Jellyfin row whose page is a page of other rows: a
    /// series answers its seasons and a season its episodes. Everything else —
    /// a movie, a row of some type the grid has no shape for — is read on its
    /// own, which is what a Plex movie's detail page does too.
    #[test]
    fn only_a_series_or_a_season_lists_other_rows() {
        assert_eq!(listing_kind(Some("show")), Some("show"));
        assert_eq!(listing_kind(Some("season")), Some("season"));
        assert_eq!(listing_kind(Some("movie")), None);
        assert_eq!(listing_kind(Some("trailer")), None);
        assert_eq!(listing_kind(None), None);
    }

    /// One page of a whole listing: Jellyfin answers a library at once, so the
    /// grid's paging happens here — the rows in the server's own order, the
    /// server's own total, and whether any rows follow this page.
    #[test]
    fn one_page_of_a_whole_listing_says_whether_more_follows() {
        fn rows(count: usize) -> QueryResult<JfItem> {
            QueryResult {
                items: (0..count)
                    .map(|n| JfItem {
                        id: format!("i{n}"),
                        ..Default::default()
                    })
                    .collect(),
                total_record_count: Some(9),
                ..Default::default()
            }
        }
        let (items, more, total) = paginate("0f1e", rows(5), 0, 2);
        assert_eq!(items.len(), 2);
        assert!(more);
        assert_eq!(total, Some(9));
        // A row of the page is a Jellyfin row: its id names the server, which is
        // what sends a later tap of it back to the same one.
        let key = items
            .first()
            .and_then(|row| row.rating_key.as_deref())
            .unwrap_or_default();
        assert_eq!(
            source::split(key).0,
            source::Origin::Jellyfin("0f1e".to_owned())
        );
        // A page that reaches the end of the answer says there is no more.
        let (last, more, _) = paginate("0f1e", rows(5), 4, 2);
        assert_eq!(last.len(), 1);
        assert!(!more);
    }

    /// A server whose sign-in this install cannot put on the wire leaves the pool
    /// and arrives as a note naming it, rather than browsing as an empty server.
    #[test]
    fn a_server_that_cannot_be_opened_is_named_rather_than_ignored() {
        let row = JellyfinServer {
            server_id: "0f1e".to_owned(),
            name: "Aurora".to_owned(),
            base_url: "http://192.168.1.20:8096".to_owned(),
            ..Default::default()
        };
        let device = DeviceInfo {
            app: "Chewvy".to_owned(),
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
            device: "desktop".to_owned(),
            device_id: "00000000-0000-0000-0000-000000000000".to_owned(),
        };
        // No secret store at all: the token its row's sign-in saved is nowhere.
        let (pool, notes) = Jellyfin::open(&[row], None, &device);
        assert!(pool.servers.is_empty());
        assert!(
            notes
                .first()
                .is_some_and(|note| note.starts_with("Aurora: ")),
            "{notes:?}"
        );
    }
}
