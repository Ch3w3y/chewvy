//! The worker thread and the messages it exchanges with [`crate::ui::app`].
//!
//! Plex calls are blocking, so they run on one worker thread that owns the
//! [`LiveClient`]; the UI only exchanges [`Cmd`]/[`Ev`] messages with it, and
//! [`crate::shell::connect`] decides which server this thread opens on start.
//! Playback resolves here; the window then plays it with its own embedded
//! player ([`crate::player`]), rendering straight into the app window.
//! The account token only ever reaches `plex_core` requests and mpv's
//! `http-header-fields`; it is never shown, logged or put in a URL.

use std::error::Error;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use plex_core::client_info::default_identity_path;
use plex_core::library::{Hub, Item, SortOption};
use plex_core::progress::{PlayState, ProgressUpdate};
use plex_core::resources::ServerResource;
use plex_core::secret::{SecretString, REDACTED};
use plex_core::tokens::{
    delete_plex_token, load_plex_token, open_store, save_plex_token, Consent, StorePaths, TokenKey,
    TokenStore,
};

use crate::add_source::{self, AddFailure, DiscoveredServer, ValidatedServer};
use crate::player::quality::Mode;
use crate::player::{self, request};
use crate::player::{NextUp, PlayRequest};
use crate::shell::connect;
use crate::shell::host::run_host_ui;
use crate::shell::live::LiveClient;
use crate::shell::resources::CacheKey;
use crate::source::{self, Library, Origin};
use crate::sources::browse;
use crate::sources::worker::Desk;
use crate::sources::{JellyfinServer, Sources};
use crate::together::{self, rooms::Groups};
use crate::ui::app;
use crate::ui::detail;
use crate::ui::grid;

/// Shelves asked of `/hubs` for the Home screen (`plex_core::library::MAX_HUB_ROWS`
/// is the largest count a PMS is asked for).
const HUB_ROWS: usize = 8;
/// How many rows a search hub is asked for (`/hubs/search?limit=N`).
const SEARCH_ROWS: usize = 20;
/// How long a pairing code is polled before asking the user to retry.
const PAIRING_BUDGET: Duration = Duration::from_secs(15 * 60);
/// How often plex.tv is asked whether the code on screen has been entered.
const PAIRING_POLL: Duration = Duration::from_secs(2);

pub(crate) enum Cmd {
    /// Ask plex.tv for a sign-in code and wait for it to be entered.
    Pair,
    /// Stop waiting for the code on screen.
    CancelPair,
    Connect(usize),
    /// (Re)read the home shelves: shown first after connect and from the rail.
    Home,
    /// One page of a section's posters, with the chips that chose it.
    Browse {
        section: String,
        start: usize,
        sort: Option<String>,
        unwatched: bool,
    },
    /// The sorts one section offers: asked before its posters, so the first
    /// page is fetched with the sort the chips will show, and again on every
    /// entry into a library so the sort in effect is one that section names.
    Sorts {
        section: String,
    },
    /// One item's detail page: its own `/metadata` answer, and for a show or
    /// season the `/children` read for it.
    Detail {
        rating_key: String,
    },
    /// The episodes of the season a show's detail page selected.
    Season {
        rating_key: String,
    },
    /// Search the connected server with its own grouped answer, answering
    /// [`Ev::SearchHubs`].
    SearchHubs(String),
    /// Re-read the account's servers so Settings can offer another one.
    Servers,
    Play {
        rating_key: String,
        title: String,
        /// Seconds to open the item at, from its `viewOffset`.
        start: Option<f64>,
        /// Open at this quality for this play only: the step after one that
        /// failed. `None` opens at the quality `Cmd::Quality` last named.
        quality: Option<usize>,
        /// The window's egui context, woken once the item is resolved.
        wakeup: egui::Context,
    },
    /// The quality later plays open at (an index into `player::quality::QUALITIES`).
    Quality(usize),
    /// End a server stream session the player has left. `None` for a file the
    /// server is only storing, where there is no server-side session to close.
    StopStream(Option<String>),
    /// Where a Jellyfin item stands, reported against the session `Cmd::Play`
    /// opened. The first report of an item tells the server it started; the ones
    /// after it tell the server where it is.
    JellyfinProgress {
        /// Milliseconds into the item.
        time_ms: u64,
        /// Whether the player is paused, as its own play state said.
        paused: bool,
        /// The setting in effect. Like a Plex position report, this one states
        /// it: Jellyfin playback is opt-in and off by default.
        enabled: bool,
    },
    /// One playback-position report (`plex_core::progress::report_timeline`).
    /// Today the worker sends these to itself - at play start and when a stream
    /// session ends - because the window does not track a position yet; the
    /// player will when the UI is wired to it.
    Timeline {
        rating_key: String,
        /// The item's library path, sent as the optional `key` when known.
        item_key: Option<String>,
        state: PlayState,
        /// Milliseconds into the item.
        time_ms: u64,
        duration_ms: Option<u64>,
        /// The setting in effect. Playback position is opt-in and off by
        /// default, so every report states it rather than trusting a copy.
        enabled: bool,
    },
    /// Mark one item watched (`plex_core::progress::mark_watched`): asked for
    /// once a tracked position has reached the item's end.
    Scrobble {
        rating_key: String,
    },
    SignOut,
    /// A Jellyfin step the Sources screen asked for.
    Jf(JfCmd),
    /// What one of those steps' helper thread answered with.
    JfAnswer(JfAnswer),
    /// Watching together: the panel's asks, and the player's for its group.
    Together(together::Ask),
    /// The window has closed: leave any group, then answer.
    Quit(Sender<()>),
}

/// A step of adding a Jellyfin source. Every Jellyfin call blocks, so the worker
/// runs each one on its own thread; these say which step to run, and the answers
/// come back as [`JfAnswer`] on the same channel.
#[derive(Clone, PartialEq, Eq)]
pub enum JfCmd {
    /// Ask the LAN which Jellyfin servers answer, skipping the ones already added.
    Discover,
    /// Ask `address` whether it names a Jellyfin this app can sign in to.
    Validate {
        /// The address the panel's field holds, as its owner typed it.
        address: String,
    },
    /// Ask the server being added for a Quick Connect code and wait for it.
    QuickConnect,
    /// Sign in to the server being added with a username and password.
    SignIn {
        /// The account name the panel's field holds.
        username: String,
        /// Bearer input: only its length is ever printable.
        password: String,
    },
    /// Stop the step in flight; the server being added is still being added.
    Stop,
    /// The add panel closed: nothing it was waiting for is wanted any more.
    Dismiss,
    /// Forget a source on this device: its token slot, then its row.
    Remove {
        /// The server whose row and token slot this device is forgetting.
        server_id: String,
    },
}

impl std::fmt::Debug for JfCmd {
    /// A command's shape, never its bearer input: a password is named here but
    /// never rendered, so a `{:?}` of a step cannot leak one.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Discover => f.write_str("Discover"),
            Self::Validate { address } => f
                .debug_struct("Validate")
                .field("address", address)
                .finish(),
            Self::QuickConnect => f.write_str("QuickConnect"),
            Self::SignIn { username, .. } => f
                .debug_struct("SignIn")
                .field("username", username)
                .field("password", &REDACTED)
                .finish(),
            Self::Stop => f.write_str("Stop"),
            Self::Dismiss => f.write_str("Dismiss"),
            Self::Remove { server_id } => f
                .debug_struct("Remove")
                .field("server_id", server_id)
                .finish(),
        }
    }
}

/// What one add-flow step's thread came back with.
pub enum JfAnswer {
    /// What the LAN broadcast answered with.
    Found(Vec<DiscoveredServer>),
    /// What checking an address answered with.
    Checked(Result<ValidatedServer, AddFailure>),
    /// What a sign-in answered with: the row it earned and the token to keep.
    SignedIn(Result<(JellyfinServer, SecretString), AddFailure>),
}

/// What the Sources screen needs to hear about its own steps.
#[derive(Clone, Debug)]
pub enum JfEv {
    /// The LAN answered with these servers, none of them already added.
    Found(Vec<DiscoveredServer>),
    /// This address is a Jellyfin ready to sign in to.
    Validated(ValidatedServer),
    /// The code to enter on a device already signed in to that server.
    Code(String),
    /// The source is added, and its token is kept on this device.
    Added(JellyfinServer),
    /// A step did not work, in words the panel can act on.
    Failed(AddFailure),
}

/// The item the worker is currently reporting playback for.
#[derive(Clone, Debug)]
struct Playing {
    rating_key: String,
    /// The item's library path (`/library/metadata/{ratingKey}`), as recorded.
    item_key: Option<String>,
    /// Milliseconds into the item, as last reported.
    time_ms: u64,
    duration_ms: Option<u64>,
    /// The progress setting this item was started under, sent with each report.
    enabled: bool,
}

impl Playing {
    /// The report that tells the server where this item stands.
    fn timeline(&self, state: PlayState) -> Cmd {
        Cmd::Timeline {
            rating_key: self.rating_key.clone(),
            item_key: self.item_key.clone(),
            state,
            time_ms: self.time_ms,
            duration_ms: self.duration_ms,
            enabled: self.enabled,
        }
    }
}

/// The Jellyfin item the worker is reporting playback for. A Jellyfin session is
/// the app's own invention — item id, `MediaSourceId`, `PlaySessionId` — so what
/// the resolve step learned has to outlive the command that opened it.
struct JfPlaying {
    /// What to report: where the item came from, its ids and length, and where
    /// it was resumed from.
    item: browse::Playback,
    /// Milliseconds into the item, as last reported.
    time_ms: u64,
    /// Whether `/Sessions/Playing` has been sent. A position before that first
    /// report would be a position in a session the server never heard of.
    started: bool,
}

/// Whether a report says the item is over. Plex marks an item watched at **90%**
/// of its duration, but that threshold is server configuration and is not
/// echoed back, so this reports the end it can see and asks for the watched flag
/// instead of assuming the server inferred it.
fn finished(state: PlayState, time_ms: u64, duration_ms: Option<u64>) -> bool {
    matches!(state, PlayState::Stopped | PlayState::Paused)
        && duration_ms.is_some_and(|duration| time_ms >= duration)
}

/// Which browsing an answer belongs to, carried on every card, sorts and search
/// answer so a late answer fills the list that asked for it instead of whatever
/// screen the user has moved to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Ask {
    /// One page of this section's posters, asked at `start` with `sort` and
    /// `unwatched`.
    Section {
        section: String,
        start: usize,
        sort: Option<String>,
        unwatched: bool,
    },
}

pub(crate) enum Ev {
    Status(String),
    Error(String),
    Pairing {
        code: String,
        url: String,
    },
    Servers(Vec<String>),
    Sections {
        /// The connected server's own label (`display_name`).
        server: String,
        sections: Vec<(String, String)>,
    },
    /// The connected server's home shelves, with their resume offsets.
    Hubs(Vec<Hub>),
    /// Poster-grid cards: one page of one section's posters.
    Cards {
        cards: Vec<grid::Card>,
        append: bool,
        more: bool,
        /// The container's `totalSize`, when its answer named one.
        total: Option<u64>,
        /// The answer's [`Ask`], so it fills the list that asked for it.
        asked: Ask,
    },
    /// The sorts one section offered, in the order it named them.
    Sorts {
        section: String,
        options: Vec<SortOption>,
    },
    /// A connected session the artwork loader may read artwork from.
    Artwork(std::sync::Arc<LiveClient>),
    /// One item's detail page, read and ready to draw.
    Detail(Box<detail::Page>),
    /// The episodes of `season_key`, answering [`Cmd::Season`].
    Episodes {
        season_key: String,
        items: Vec<Item>,
    },
    /// Search results for `query`, as the server grouped them.
    SearchHubs {
        query: String,
        hubs: Vec<Hub>,
    },
    /// An item resolved and ready: the window starts it in its own player.
    PlayReady(Box<PlayRequest>),
    /// Every source this install can play from, as its file keeps them.
    Sources(Sources),
    /// The Jellyfin side of the browse screens: the servers that answered, the
    /// libraries they listed, and what a server said instead of a list.
    Jellyfin {
        pool: browse::Jellyfin,
        libraries: Vec<Library>,
        notes: Vec<String>,
    },
    /// What one of the Sources screen's own steps answered with.
    Jf(JfEv),
    /// What a watch-together group said.
    Together(together::News),
    SignedOut,
    /// No Plex account is linked and none is being linked: the window decides
    /// between the first-run choice and the Jellyfin servers already on file.
    NoPlex,
}

/// The window's context, once its first frame has run.
static WINDOW: std::sync::OnceLock<egui::Context> = std::sync::OnceLock::new();

/// Remember the window's context so background threads can wake it.
pub(crate) fn register_window(ctx: &egui::Context) {
    let _ = WINDOW.set(ctx.clone());
}

/// Ask the window for one more frame: a background result is ready to show.
pub(crate) fn wake() {
    if let Some(ctx) = WINDOW.get() {
        ctx.request_repaint();
    }
}

/// Run the app window until it is closed.
///
/// # Errors
///
/// When the window cannot be created or the worker cannot start.
pub fn run() -> Result<(), Box<dyn Error>> {
    let (cmd_tx, cmd_rx) = mpsc::channel();
    let (ev_tx, ev_rx) = mpsc::channel();
    // The worker's answers pass through one forwarding thread that wakes the
    // window as each lands, so the UI sleeps until there is something to show.
    let (worker_ev, answers) = mpsc::channel::<Ev>();
    let self_cmd = cmd_tx.clone();
    std::thread::Builder::new()
        .name("plex-wake".into())
        .spawn(move || {
            for answer in answers {
                if ev_tx.send(answer).is_err() {
                    break;
                }
                wake();
            }
        })?;
    std::thread::Builder::new()
        .name("plex-worker".into())
        .spawn(move || {
            // The worker sends some commands to itself, on the same channel the
            // window uses.
            worker(&cmd_rx, &worker_ev, &self_cmd)
        })?;
    let quit = cmd_tx.clone();
    let mut ui = app::Ui::new(cmd_tx, ev_rx);
    let shown = run_host_ui(Box::new(move |root| ui.draw(root)));
    // The window is gone: an update the user restarted for opens now, while
    // this process finishes closing.
    crate::update::relaunch();
    // A group this device leaves without saying so would wait for its player.
    let (left, done) = mpsc::channel();
    if quit.send(Cmd::Quit(left)).is_ok() {
        let _ = done.recv_timeout(together::jellyfin::CLOSE_WAIT);
    }
    shown
}

/// `chewvy --diagnose-servers`: print a redacted server report and exit.
///
/// # Errors
///
/// A description when there is no stored sign-in or plex.tv cannot be read.
pub fn diagnose_servers() -> Result<(), String> {
    let mut client = LiveClient::new(&default_identity_path(), env!("CARGO_PKG_VERSION"))?;
    let store = token_store().ok_or("the token store could not be opened")?;
    let token = load_plex_token(&*store, plex_token_key().as_ref())
        .map_err(|e| format!("could not read the stored sign-in: {e}"))?
        .ok_or("no stored sign-in: run chewvy and sign in first")?;
    client.set_account(token);
    for line in client.server_report()? {
        println!("{line}");
    }
    if client.base_url().is_none() {
        return Ok(());
    }
    // Read-only library check on the last connected server.
    let sections = client.sections()?;
    println!("sections: {}", sections.len());
    for section in &sections {
        let items = client.browse(
            &section.key,
            0,
            3,
            &plex_core::library::BrowseOptions::default(),
        )?;
        println!(
            "  {:?} kind={:?} first items: {:?}",
            section.title.as_deref().unwrap_or("?"),
            section.kind,
            items
                .items
                .iter()
                .map(|i| i.title.as_deref().unwrap_or("?"))
                .collect::<Vec<_>>()
        );
        if let Some(key) = items
            .items
            .iter()
            .find(|i| i.kind.as_deref() == Some("movie"))
            .and_then(|i| i.rating_key.clone())
        {
            let item = client.metadata(&key)?;
            println!(
                "    metadata {:?}: {} part(s)",
                item.title,
                item.parts.len()
            );
        }
    }
    Ok(())
}

/// `chewvy --capture GET-PATH [QUERY]`: print one read-only server answer
/// (JSON) to stdout, for capturing real response shapes. Never prints a token.
///
/// # Errors
///
/// A description when there is no sign-in, no server, or the call fails.
pub fn capture(path: &str, query: &str) -> Result<(), String> {
    let mut client = LiveClient::new(&default_identity_path(), env!("CARGO_PKG_VERSION"))?;
    let store = token_store().ok_or("the token store could not be opened")?;
    let token = load_plex_token(&*store, plex_token_key().as_ref())
        .map_err(|e| format!("could not read the stored sign-in: {e}"))?
        .ok_or("no stored sign-in")?;
    client.set_account(token);
    let servers = client.list_servers()?;
    let first = servers.first().ok_or("no servers")?;
    client.connect(first)?;
    let pairs: Vec<(&str, &str)> = query
        .split('&')
        .filter_map(|kv| kv.split_once('='))
        .collect();
    let (status, body) = client.raw_get(path, &pairs)?;
    eprintln!("status {status}");
    println!("{}", String::from_utf8_lossy(&body));
    Ok(())
}

// ------------------------------------------------------------------ worker

fn worker(cmds: &Receiver<Cmd>, ev: &Sender<Ev>, out: &Sender<Cmd>) {
    let send = |e: Ev| {
        let _ = ev.send(e);
    };
    let mut client = match LiveClient::new(&default_identity_path(), env!("CARGO_PKG_VERSION")) {
        Ok(c) => c,
        Err(e) => {
            return send(Ev::Error(format!(
                "could not create the client identity: {e}"
            )))
        }
    };
    let store = token_store();
    let mut servers: Vec<ServerResource> = Vec::new();
    let mut quality = 0;
    // Playback position is opt-in and off by default (`plex_core::progress`);
    // nothing turns it on until the UI offers it, so the default means the
    // progress commands below answer without touching a connection.
    let mut progress_reporting = false;
    let mut playing: Option<Playing> = None;
    // The same, for the source whose session the app has to name itself.
    let mut jellyfin_playing: Option<JfPlaying> = None;

    let stored = store.as_ref().and_then(|s| {
        load_plex_token(&**s, plex_token_key().as_ref())
            .ok()
            .flatten()
    });
    // The source registry is read before anything blocks, so the Settings screen
    // has a list to show while sign-in is still waiting for a code.
    let mut desk = Desk::open();
    desk.sync_plex(store.as_deref(), plex_account_id(), ev);
    // Jellyfin browse opens from the registry the Settings screen keeps, before
    // anything blocks: a server that does not answer is named, not waited on.
    let mut jellyfin = open_jellyfin(&desk, store.as_deref(), ev);
    // Whether a Plex server is connected. Without one, Home and Search are
    // the Jellyfin servers' alone.
    let mut plex = false;
    // When the sign-in code on screen was issued, while one is waiting.
    let mut pairing: Option<Instant> = None;
    // The watch-together group or room this device is in, and the Plex rooms.
    let mut groups = Groups::default();
    match stored {
        Some(token) => {
            // Resolve the account id once so a pre-keying token migrates into
            // the keyed slot on the next start; failures are invisible here.
            resolve_plex_token_key(&client);
            client.set_account(token);
            send(Ev::Status("Signed in. Finding your servers…".into()));
            plex = list_servers(&mut client, ev, &mut servers);
            groups.plex_account(plex_rooms(&client, &servers), ev);
        }
        // Signing in to Plex is the user's choice, not the app's first move.
        None => send(Ev::NoPlex),
    }

    loop {
        // A code on screen is asked about between commands, so the window
        // stays answerable while it waits: the wait can be left, and a
        // Jellyfin server can be added during it.
        let cmd = match pairing {
            None => match cmds.recv() {
                Ok(cmd) => cmd,
                Err(_) => break,
            },
            Some(started) => match cmds.recv_timeout(PAIRING_POLL) {
                Ok(cmd) => cmd,
                Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => {
                    match poll_pairing(&mut client, store.as_deref(), ev, started) {
                        Pairing::Waiting => {}
                        Pairing::Over => pairing = None,
                        Pairing::SignedIn => {
                            pairing = None;
                            desk.sync_plex(store.as_deref(), plex_account_id(), ev);
                            servers.clear();
                            plex = list_servers(&mut client, ev, &mut servers);
                            groups.plex_account(plex_rooms(&client, &servers), ev);
                        }
                    }
                    continue;
                }
            },
        };
        match cmd {
            Cmd::Pair => pairing = start_pairing(&mut client, ev),
            Cmd::CancelPair => pairing = None,
            Cmd::Connect(i) => {
                plex |= connect(&mut client, ev, &servers, i);
                groups.plex_account(plex_rooms(&client, &servers), ev);
            }
            Cmd::Home => {
                let mut hubs = Vec::new();
                if plex {
                    match client.hubs(HUB_ROWS) {
                        Ok(shelves) => hubs = shelves,
                        Err(e) => {
                            send(Ev::Error(format!("could not load the home shelves: {e}")));
                            continue;
                        }
                    }
                }
                // Jellyfin's shelves join Plex's at the end of Home; a server
                // that answers nothing adds no row and says nothing.
                let (shelves, _notes) = jellyfin.shelves(HUB_ROWS);
                hubs.extend(shelves);
                send(Ev::Hubs(hubs));
            }
            Cmd::Browse {
                section,
                start,
                sort,
                unwatched,
            } => match source::split(&section) {
                (Origin::Plex, id) => browse(&client, ev, id, start, sort, unwatched),
                (Origin::Jellyfin(server), id) => {
                    match jellyfin.page(&server, id, start, grid::PAGE) {
                        Ok((items, more, total)) => send(Ev::Cards {
                            cards: items.iter().map(grid::Card::from_item).collect(),
                            append: start > 0,
                            more,
                            total,
                            asked: Ask::Section {
                                section,
                                start,
                                sort,
                                unwatched,
                            },
                        }),
                        Err(e) => send(Ev::Error(format!("could not load the library: {e}"))),
                    }
                }
            },
            // A Jellyfin library lists its rows in its own one order, so it
            // answers with the empty list the screen already treats as "the
            // server's own order" and draws no chips.
            Cmd::Sorts { section } if source::split(&section).0.is_jellyfin() => {
                send(Ev::Sorts {
                    section,
                    options: Vec::new(),
                });
            }
            Cmd::Sorts { section } => match client.sorts(&section) {
                Ok(options) => send(Ev::Sorts {
                    section: section.clone(),
                    options,
                }),
                Err(e) => {
                    send(Ev::Error(format!(
                        "could not read this library's sorts: {e}"
                    )));
                    // An empty list still answers: the screen asks its first
                    // page in the server's own order instead of waiting forever.
                    send(Ev::Sorts {
                        section: section.clone(),
                        options: Vec::new(),
                    });
                }
            },
            Cmd::Detail { rating_key } => match source::split(&rating_key) {
                (Origin::Plex, id) => detail(&mut client, ev, id),
                (Origin::Jellyfin(server), id) => {
                    jellyfin_detail(&jellyfin, ev, &server, id);
                }
            },
            Cmd::Season { rating_key } => match source::split(&rating_key) {
                (Origin::Plex, id) => season(&client, ev, id),
                (Origin::Jellyfin(server), id) => match jellyfin.listed(&server, "season", id) {
                    Ok(items) => send(Ev::Episodes {
                        season_key: rating_key,
                        items,
                    }),
                    Err(e) => {
                        send(Ev::Error(format!("could not load this season: {e}")));
                    }
                },
            },
            // One search asks every source: the Plex server's own groups
            // first, then one group per Jellyfin server that found something.
            Cmd::SearchHubs(q) => {
                let mut hubs = Vec::new();
                if plex {
                    match client.search_hubs(&q, SEARCH_ROWS) {
                        Ok(found) => hubs = found,
                        Err(e) => {
                            send(Ev::Error(format!("search failed: {e}")));
                            continue;
                        }
                    }
                }
                hubs.extend(jellyfin.search(&q, SEARCH_ROWS));
                send(Ev::SearchHubs { hubs, query: q });
            }
            // Settings' server switch: the account's servers as names the
            // Settings panel offers, without connecting to the remembered one
            // again. `servers` keeps the order `Cmd::Connect(index)` indexes.
            Cmd::Servers => {
                if let Some(names) = account_servers(&mut client, ev, &mut servers) {
                    send(Ev::Servers(names));
                }
            }
            Cmd::Play {
                rating_key,
                title,
                start,
                quality: step,
                wakeup,
            } => {
                let fell_back = step.is_some();
                let quality = step.unwrap_or(quality);
                if source::split(&rating_key).0.server_id().is_some() {
                    // A Jellyfin row plays the file the server says this client
                    // can take. The session it opens is the app's own, so the
                    // resolve step's answer is what later reports name.
                    let at = (quality, fell_back);
                    jellyfin_playing =
                        start_jellyfin(&jellyfin, &rating_key, &title, start, at, &send);
                } else if let Some(tracked) = start_playback(
                    &mut client,
                    &rating_key,
                    title,
                    start,
                    (quality, fell_back),
                    progress_reporting,
                    ev,
                ) {
                    let _ = out.send(tracked.timeline(PlayState::Playing));
                    playing = Some(tracked);
                }
                wakeup.request_repaint();
            }
            Cmd::Quality(index) => quality = index,
            Cmd::StopStream(session) => {
                // A Jellyfin session the server never heard of — the switch was
                // off the whole time — has no report to close, only an encode.
                if let Some(tracked) = jellyfin_playing.take() {
                    if let Err(e) = jellyfin.close(&tracked.item, tracked.time_ms, tracked.started)
                    {
                        send(Ev::Error(format!("could not close playback: {e}")));
                    }
                } else if let Some(session) = session {
                    client.stop_stream(&session);
                }
                // The server keeps an item on "Continue Watching" until it is
                // told playback stopped, so the report that closes a session is
                // sent even though the window does not track a position yet.
                if let Some(tracked) = playing.take() {
                    let _ = out.send(tracked.timeline(PlayState::Stopped));
                }
            }
            Cmd::JellyfinProgress {
                time_ms,
                paused,
                enabled,
            } => {
                // Reporting is opt-in and off by default, and the playing screen
                // ticks whatever is in front of it, so a position with no
                // Jellyfin item behind it is a no-op rather than an error.
                let Some(tracked) = jellyfin_playing.as_mut() else {
                    continue;
                };
                if enabled {
                    // The report that opens a session is a position like any
                    // other; it only has to be the first one sent.
                    let report = if tracked.started {
                        browse::Report::Progress
                    } else {
                        tracked.started = true;
                        browse::Report::Playing
                    };
                    tracked.time_ms = time_ms;
                    let body = tracked.item.body(time_ms, paused);
                    if let Err(e) = jellyfin.report(&tracked.item.server, report, &body) {
                        send(Ev::Error(format!("could not report playback: {e}")));
                    }
                }
            }
            Cmd::Timeline {
                rating_key,
                item_key,
                state,
                time_ms,
                duration_ms,
                enabled,
            } => {
                progress_reporting = enabled;
                // A position of 0 is a real position: it is how a restarted item
                // clears its resume point, so it is sent rather than omitted.
                let mut update = ProgressUpdate::new(&rating_key, state)
                    .with_position_ms(Some(time_ms), duration_ms);
                if let Some(path) = item_key {
                    update = update.with_item_key(path);
                }
                if enabled {
                    update = update.opt_in();
                }
                match client.report_timeline(&update) {
                    Ok(_) => {}
                    Err(e) => send(Ev::Error(format!("could not report playback: {e}"))),
                }
                // A position that reached the end is watched: ask for the
                // scrobble rather than assuming the server inferred it.
                if enabled && finished(state, time_ms, duration_ms) {
                    let _ = out.send(Cmd::Scrobble { rating_key });
                }
            }
            Cmd::Scrobble { rating_key } => {
                if let Err(e) = client.mark_watched(&rating_key) {
                    send(Ev::Error(format!(
                        "could not tell the server this item is watched: {e}"
                    )));
                }
            }
            Cmd::SignOut => {
                if let Some(s) = store.as_ref() {
                    let _ = delete_plex_token(&**s, plex_token_key().as_ref());
                }
                desk.sync_plex(store.as_deref(), None, ev);
                servers.clear();
                playing = None;
                plex = false;
                pairing = None;
                groups.plex_account(None, ev);
                send(Ev::SignedOut);
            }
            Cmd::Jf(step) => {
                let held = desk.jellyfin().len();
                desk.step(step, store.as_deref(), ev, out);
                if desk.jellyfin().len() != held {
                    // A source this install just gained or lost is browsed from
                    // now on, without waiting for the window to open again.
                    jellyfin = open_jellyfin(&desk, store.as_deref(), ev);
                }
            }
            Cmd::JfAnswer(answer) => {
                let held = desk.jellyfin().len();
                desk.answer(answer, store.as_deref(), ev);
                if desk.jellyfin().len() != held {
                    jellyfin = open_jellyfin(&desk, store.as_deref(), ev);
                }
            }
            // A room whose title is on another of the account's servers:
            // that server is connected first, as the switcher would.
            Cmd::Together(together::Ask::Enter {
                room,
                server: Some(machine),
            }) => {
                let index = servers
                    .iter()
                    .position(|server| server.client_identifier() == machine);
                if index.is_some_and(|index| connect(&mut client, ev, &servers, index)) {
                    plex = true;
                    groups.plex_account(plex_rooms(&client, &servers), ev);
                    groups.handle(&jellyfin, together::Ask::Enter { room, server: None }, ev);
                } else {
                    send(Ev::Together(together::News::Left {
                        why: Some(
                            "The server this room's title is on could not be reached.".into(),
                        ),
                    }));
                }
            }
            Cmd::Together(ask) => groups.handle(&jellyfin, ask, ev),
            Cmd::Quit(done) => {
                groups.close();
                let _ = done.send(());
            }
        }
    }
}

/// Open the Jellyfin servers this install can ask, and tell the UI what they
/// listed. A server that needs a sign-in, or that will not answer, is reported
/// in `notes` and left out of the rail rather than failing the screen.
fn open_jellyfin(desk: &Desk, store: Option<&dyn TokenStore>, ev: &Sender<Ev>) -> browse::Jellyfin {
    if desk.jellyfin().is_empty() {
        return browse::Jellyfin::default();
    }
    let device = match add_source::device_identity() {
        Ok(device) => device,
        Err(why) => {
            send_jellyfin(
                ev,
                browse::Jellyfin::default(),
                Vec::new(),
                vec![why.message()],
            );
            return browse::Jellyfin::default();
        }
    };
    let (pool, mut notes) = browse::Jellyfin::open(desk.jellyfin(), store, &device);
    let (libraries, unreadable) = pool.libraries();
    notes.extend(unreadable);
    send_jellyfin(ev, pool.clone(), libraries, notes);
    pool
}

/// One answer about the Jellyfin side of the screens.
fn send_jellyfin(
    ev: &Sender<Ev>,
    pool: browse::Jellyfin,
    libraries: Vec<Library>,
    notes: Vec<String>,
) {
    let _ = ev.send(Ev::Jellyfin {
        pool,
        libraries,
        notes,
    });
}

/// One Jellyfin row's detail page, read the way [`detail`] reads a Plex row: the
/// row itself, the level it lists, and the season its page opens on.
fn jellyfin_detail(jellyfin: &browse::Jellyfin, ev: &Sender<Ev>, server: &str, id: &str) {
    let item = match jellyfin.item(server, id) {
        Ok(item) => item,
        Err(e) => {
            let _ = ev.send(Ev::Error(format!("could not load this item: {e}")));
            return;
        }
    };
    // Only a series or a season lists a level below it, so a movie page is read
    // with one call, as the Plex path reads it.
    let listed = browse::listing_kind(item.kind.as_deref())
        .and_then(|kind| match jellyfin.listed(server, kind, id) {
            Ok(items) => Some(items),
            Err(e) => {
                let _ = ev.send(Ev::Error(format!("could not load this item: {e}")));
                None
            }
        })
        .unwrap_or_default();
    let mut page = detail::Page::open(item, &listed);
    // A series opens on one season, and a season page with no episodes still
    // needs its own children: both are read here so the page draws whole.
    if let Some(season) = page.pending().map(str::to_owned) {
        let (season_source, season_id) = source::split(&season);
        let season_server = season_source.server_id().unwrap_or(server);
        if let Ok(items) = jellyfin.listed(season_server, "season", season_id) {
            page.fill(&season, &items);
        }
    }
    let _ = ev.send(Ev::Detail(Box::new(page)));
}

/// One page of a section's posters, asked with the chips currently in effect.
fn browse(
    client: &LiveClient,
    ev: &Sender<Ev>,
    section: &str,
    start: usize,
    sort: Option<String>,
    unwatched: bool,
) {
    let options = plex_core::library::BrowseOptions { sort, unwatched };
    match client.browse(section, start, grid::PAGE, &options) {
        Ok(page) => {
            let _ = ev.send(Ev::Cards {
                cards: page.items.iter().map(grid::Card::from_item).collect(),
                append: start > 0,
                more: page.has_more(grid::PAGE),
                total: page.total.map(|total| total as u64),
                asked: Ask::Section {
                    section: section.to_owned(),
                    start,
                    sort: options.sort.clone(),
                    unwatched: options.unwatched,
                },
            });
        }
        Err(e) => {
            let _ = ev.send(Ev::Error(format!("could not load the library: {e}")));
        }
    }
}

/// A show's detail page: its own `/metadata` answer, the `/children` answer
/// holding its seasons, and the episodes of the season the page opens on.
fn detail(client: &mut LiveClient, ev: &Sender<Ev>, rating_key: &str) {
    let item = match client.metadata(rating_key) {
        Ok(item) => item,
        Err(e) => {
            let _ = ev.send(Ev::Error(format!("could not load this item: {e}")));
            return;
        }
    };
    // Only a show's or a season's answer holds the next level down, so a movie
    // page is read with one call.
    let listed = if detail::browsable(&item) {
        match client.children(rating_key) {
            Ok(listed) => listed.items,
            Err(e) => {
                let _ = ev.send(Ev::Error(format!("could not load this item: {e}")));
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };
    let mut page = detail::Page::open(item, &listed);
    // A show opens on one season, and a season page with no episodes still
    // needs its own children: both are read here so the page draws whole.
    if let Some(season) = page.pending().map(str::to_owned) {
        if let Ok(listed) = client.children(&season) {
            page.fill(&season, &listed.items);
        }
    }
    let _ = ev.send(Ev::Detail(Box::new(page)));
}

/// The episodes of the season a show's detail page selected.
fn season(client: &LiveClient, ev: &Sender<Ev>, rating_key: &str) {
    match client.children(rating_key) {
        Ok(listed) => {
            let _ = ev.send(Ev::Episodes {
                season_key: rating_key.to_owned(),
                items: listed.items,
            });
        }
        Err(e) => {
            let _ = ev.send(Ev::Error(format!("could not load this season: {e}")));
        }
    }
}

/// What the Plex rooms need of the signed-in account, or `None` when no
/// account is signed in. Costs one plex.tv read, for the account's user id.
fn plex_rooms(client: &LiveClient, servers: &[ServerResource]) -> Option<together::rooms::Account> {
    let token = client.account()?;
    together::rooms::Account::new(
        token,
        client.client_info(),
        client.plex_tv().clone(),
        client.account_id(),
        client.machine_identifier().map(str::to_owned),
        servers
            .iter()
            .map(|server| (server.client_identifier().to_owned(), server.display_name()))
            .collect(),
    )
    .ok()
}

/// The plex.tv account uuid this install remembered, if plex.tv ever named one:
/// what keys both its token slot and the registry's Plex row.
fn plex_account_id() -> Option<String> {
    StorePaths::resolve()
        .ok()
        .and_then(|paths| connect::read_plex_account(&paths.config_dir))
}

fn token_store() -> Option<Box<dyn TokenStore>> {
    let paths = StorePaths::resolve().ok()?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0));
    open_store(&paths, Consent::granted(), now).ok()
}

/// The Plex token slot, keyed by the remembered plex.tv account uuid — or
/// `None` when no account has been resolved yet, which keeps reads and clears
/// pointed at the pre-keying single-token slot instead.
pub(crate) fn plex_token_key() -> Option<TokenKey> {
    let paths = StorePaths::resolve().ok()?;
    let uuid = crate::shell::connect::read_plex_account(&paths.config_dir)?;
    TokenKey::plex(&uuid).ok()
}

/// The account uuid for `client`'s current token: the remembered one when it
/// exists, otherwise fetched once from plex.tv and remembered. `None` on
/// network failure leaves the token in the pre-keying slot, where the next
/// read will find it.
fn resolve_plex_token_key(client: &LiveClient) -> Option<TokenKey> {
    if let Some(key) = plex_token_key() {
        return Some(key);
    };
    let uuid = client.account_uuid().ok()?;
    if let Ok(paths) = StorePaths::resolve() {
        let _ = crate::shell::connect::remember_plex_account(&paths.config_dir, &uuid);
    }
    TokenKey::plex(&uuid).ok()
}

/// Where a sign-in code on screen stands after one more look.
enum Pairing {
    /// Not entered yet.
    Waiting,
    /// Entered: the account's token is set on the client and kept.
    SignedIn,
    /// The wait is finished without a sign-in, and the window has been told why.
    Over,
}

/// Ask plex.tv for a sign-in code and put it on screen. Answers when the code
/// was issued, or `None` when plex.tv gave none.
fn start_pairing(client: &mut LiveClient, ev: &Sender<Ev>) -> Option<Instant> {
    // A 4-character code is entered at plex.tv/link; the claim URL is for strong pins.
    match client.start_pairing() {
        Ok((code, _claim)) => {
            let url = crate::shell::live::LINK_URL.to_owned();
            let _ = ev.send(Ev::Pairing { code, url });
            Some(Instant::now())
        }
        Err(e) => {
            let _ = ev.send(Ev::Error(format!("could not start sign-in: {e}")));
            None
        }
    }
}

/// Ask plex.tv once whether the code issued at `started` has been entered, and
/// keep the token it answers with.
fn poll_pairing(
    client: &mut LiveClient,
    store: Option<&dyn TokenStore>,
    ev: &Sender<Ev>,
    started: Instant,
) -> Pairing {
    let send = |e: Ev| {
        let _ = ev.send(e);
    };
    if started.elapsed() >= PAIRING_BUDGET {
        send(Ev::Error(
            "The sign-in code expired. Get a new code to try again.".into(),
        ));
        return Pairing::Over;
    }
    match client.poll_pairing() {
        Ok(Some(token)) => {
            if let Some(s) = store {
                // Prefer the keyed slot; the pre-keying slot is only used
                // when plex.tv cannot tell us the account id right now.
                let key = resolve_plex_token_key(client);
                if let Err(e) = save_plex_token(s, key.as_ref(), &token) {
                    send(Ev::Status(format!(
                        "signed in, but the token was not saved: {e}"
                    )));
                }
            }
            send(Ev::Status("Signed in. Finding your servers…".into()));
            Pairing::SignedIn
        }
        Ok(None) => Pairing::Waiting,
        Err(e) => {
            send(Ev::Error(format!("sign-in failed: {e}")));
            Pairing::Over
        }
    }
}

/// Ask the account which servers it has and file them in `servers`, in the
/// order `Cmd::Connect(index)` indexes. Answers their Settings labels, or
/// `None` once the read has failed.
fn account_servers(
    client: &mut LiveClient,
    ev: &Sender<Ev>,
    servers: &mut Vec<ServerResource>,
) -> Option<Vec<String>> {
    match client.list_servers() {
        Ok(list) => {
            *servers = list;
            Some(servers.iter().map(ServerResource::display_name).collect())
        }
        Err(e) => {
            let _ = ev.send(Ev::Error(format!("could not list servers: {e}")));
            None
        }
    }
}

/// List the account's servers, then connect to the remembered or only one, or
/// hand the picker to the UI when the user has to choose. Answers whether a
/// server is connected.
fn list_servers(
    client: &mut LiveClient,
    ev: &Sender<Ev>,
    servers: &mut Vec<ServerResource>,
) -> bool {
    let Some(names) = account_servers(client, ev, servers) else {
        return false;
    };
    let remembered = StorePaths::resolve()
        .ok()
        .and_then(|paths| connect::read_last_server(&paths.config_dir));
    match connect::auto_connect(servers, remembered.as_deref()) {
        Some(index) => connect(client, ev, servers, index),
        None => {
            let _ = ev.send(Ev::Servers(names));
            false
        }
    }
}

/// Connect to `servers[index]`, remember it as the server to reconnect to, and
/// report its libraries. Answers whether the server is connected.
fn connect(
    client: &mut LiveClient,
    ev: &Sender<Ev>,
    servers: &[ServerResource],
    index: usize,
) -> bool {
    let send = |e: Ev| {
        let _ = ev.send(e);
    };
    let Some(resource) = servers.get(index) else {
        return false;
    };
    send(Ev::Status(format!(
        "Connecting to {}…",
        resource.display_name()
    )));
    match client.connect(resource) {
        Ok(_) => {
            // Artwork is fetched by the loader's own threads on a shared
            // read-only session, so it never queues behind library or playback
            // work on this worker thread.
            send(Ev::Artwork(std::sync::Arc::new(client.clone())));
            if let Ok(paths) = StorePaths::resolve() {
                let _ =
                    connect::remember_last_server(&paths.config_dir, resource.client_identifier());
            }
            match client.sections() {
                Ok(sections) => send(Ev::Sections {
                    server: resource.display_name(),
                    sections: sections
                        .iter()
                        .filter_map(|s| {
                            let id = s.id().ok()?;
                            let title = s.title.clone().unwrap_or_else(|| id.clone());
                            Some((id, title))
                        })
                        .collect(),
                }),
                Err(e) => send(Ev::Error(format!("could not list libraries: {e}"))),
            }
            true
        }
        Err(e) => {
            send(Ev::Error(format!("could not connect: {e}")));
            false
        }
    }
}

/// Ask a Jellyfin server how this client may play an item at `quality`, and
/// hand the answer to the player: one read of the item, one `PlaybackInfo`, and
/// then the same hand-off a Plex item gets. `None` leaves the item unplayed,
/// with the reason already on screen — which is what the server's answer is for.
fn start_jellyfin(
    jellyfin: &browse::Jellyfin,
    rating_key: &str,
    title: &str,
    start: Option<f64>,
    (quality, fell_back): (usize, bool),
    send: &dyn Fn(Ev),
) -> Option<JfPlaying> {
    let (origin, item_id) = source::split(rating_key);
    let server = origin.server_id()?;
    let mode = player::quality::quality(quality).mode;
    let item = match jellyfin.playback(server, item_id, browse::delivery(mode)) {
        Ok(item) => item,
        Err(why) => {
            // Either the read or the verdict failed; either way this item is not
            // going to play, and the reason is the server's own.
            send(Ev::Error(format!("could not open {title}: {why}")));
            return None;
        }
    };
    let (title, subtitle) = request::titles(&item.row, title);
    // The resume position the server recorded, unless the caller named a
    // different one: a quality switch re-opens at the position it left off at.
    let start = start.or(item.start);
    send(Ev::PlayReady(Box::new(PlayRequest {
        source: item.source.clone(),
        header: item.header.clone(),
        rating_key: rating_key.to_owned(),
        title,
        subtitle,
        start,
        markers: request::markers(&item.row),
        chapters: request::chapters(&item.row),
        next: next_episode(&item.row, &|key, kind| {
            let (origin, id) = source::split(key);
            jellyfin.listed(origin.server_id()?, kind, id).ok()
        }),
        media: request::facts(&item.row),
        // Asked for the file, the server may still answer with its own stream;
        // the menu then shows the stream, which is what is playing.
        quality: match (mode, item.transcoding) {
            (Mode::DirectPlay, true) => player::quality::REMUX,
            _ => quality,
        },
        // The server's stream is one whole playlist: the player seeks in it,
        // so its clock is the item's own.
        origin: 0.0,
        title_key: rating_key.to_owned(),
        session: item.stream_session(),
        duration: item.duration,
        sidecars: Vec::new(),
        fell_back,
    })));
    Some(JfPlaying {
        item,
        // The position the item was opened at is the first position the server
        // hears about.
        time_ms: start.map(|at| (at * 1000.0) as u64).unwrap_or(0),
        started: false,
    })
}

/// Resolve one item and hand it to the window's embedded player
/// ([`crate::player`]), with its markers, chapters and the episode after it.
/// `Some` means it resolved and is ready to play, with what the worker should
/// report as its playback position.
fn start_playback(
    client: &mut LiveClient,
    rating_key: &str,
    title: String,
    start: Option<f64>,
    (quality, fell_back): (usize, bool),
    progress_reporting: bool,
    ev: &Sender<Ev>,
) -> Option<Playing> {
    let send = |e: Ev| {
        let _ = ev.send(e);
    };
    let item = match client.metadata(rating_key) {
        Ok(item) => item,
        Err(e) => {
            send(Ev::Error(format!("could not load “{title}”: {e}")));
            return None;
        }
    };
    let key = CacheKey {
        rating_key: rating_key.to_owned(),
        part: 0,
    };
    let Some((_, play)) = client.resolve(&key) else {
        send(Ev::Error(format!("“{title}” has no playable file")));
        return None;
    };
    let Some(header) = client.play_headers() else {
        send(Ev::Error("not signed in".into()));
        return None;
    };
    let (title, subtitle) = request::titles(&item, &title);
    let mode = player::quality::quality(quality).mode;
    let session = (mode != Mode::DirectPlay).then(session_id);
    let source = match &session {
        None => play.source(),
        Some(id) => match client.stream_url(rating_key, mode, start, id) {
            Some(url) => url,
            None => {
                send(Ev::Error("not connected to a server".into()));
                return None;
            }
        },
    };
    let time_ms = start.map_or(0, |secs| (secs * 1000.0) as u64);
    let duration_ms = item.duration;
    send(Ev::PlayReady(Box::new(PlayRequest {
        source,
        header: SecretString::new(header),
        rating_key: rating_key.to_owned(),
        title,
        subtitle,
        // A stream already starts at the server-side offset.
        start: start.filter(|_| session.is_none()),
        markers: request::markers(&item),
        chapters: request::chapters(&item),
        next: next_episode(&item, &|key, _| client.children(key).ok().map(|c| c.items)),
        media: request::facts(&item),
        quality,
        origin: if session.is_some() {
            start.unwrap_or(0.0)
        } else {
            0.0
        },
        title_key: item
            .grandparent_rating_key
            .clone()
            .filter(|_| item.kind.as_deref() == Some("episode"))
            .unwrap_or_else(|| rating_key.to_owned()),
        session,
        duration: item.duration.map(|ms| ms as f64 / 1000.0),
        sidecars: client
            .base_url()
            .map(|base| request::sidecars(&item, base))
            .unwrap_or_default(),
        fell_back,
    })));
    Some(Playing {
        rating_key: rating_key.to_owned(),
        item_key: item.key.clone(),
        time_ms,
        duration_ms,
        enabled: progress_reporting,
    })
}

/// The episode after `item`: later in its season, else the next season's
/// first. `children` lists what is under a `"show"` or a `"season"`, whichever
/// source the item came from.
fn next_episode(item: &Item, children: &dyn Fn(&str, &str) -> Option<Vec<Item>>) -> Option<NextUp> {
    if item.kind.as_deref() != Some("episode") {
        return None;
    }
    let season = children(item.parent_rating_key.as_deref()?, "season")?;
    if let Some(next) = request::next_in(item, &season) {
        return Some(next);
    }
    let seasons = children(item.grandparent_rating_key.as_deref()?, "show")?;
    let following = request::next_season(item.parent_index?, &seasons)?;
    let episodes = children(following.rating_key.as_deref()?, "season")?;
    request::first_of(&episodes)
}

/// A fresh server stream session id (random, hex).
fn session_id() -> String {
    use ring::rand::SecureRandom as _;
    let mut bytes = [0u8; 16];
    if ring::rand::SystemRandom::new().fill(&mut bytes).is_err() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        return format!("{nanos:032x}");
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(kind: &str, key: &str, index: u64) -> Item {
        Item {
            kind: Some(kind.to_owned()),
            rating_key: Some(key.to_owned()),
            title: Some(format!("{kind} {index}")),
            index: Some(index),
            ..Item::default()
        }
    }

    fn episode(key: &str, season: (&str, u64), index: u64) -> Item {
        Item {
            parent_rating_key: Some(season.0.to_owned()),
            parent_index: Some(season.1),
            grandparent_rating_key: Some("show".to_owned()),
            ..row("episode", key, index)
        }
    }

    /// A show of two seasons, listed the way either source lists one: a show
    /// answers its seasons, a season its episodes.
    fn two_seasons(key: &str, kind: &str) -> Option<Vec<Item>> {
        match (key, kind) {
            ("show", "show") => Some(vec![row("season", "s4", 4), row("season", "s5", 5)]),
            ("s4", "season") => Some(vec![
                episode("e9", ("s4", 4), 9),
                episode("e10", ("s4", 4), 10),
            ]),
            ("s5", "season") => Some(vec![episode("e1", ("s5", 5), 1)]),
            _ => None,
        }
    }

    #[test]
    fn the_next_episode_follows_in_its_season_then_opens_the_next_season() {
        let key = |item: &Item| next_episode(item, &two_seasons).map(|next| next.rating_key);
        assert_eq!(key(&episode("e9", ("s4", 4), 9)).as_deref(), Some("e10"));
        // The last of a season leads to the first of the season after it.
        assert_eq!(key(&episode("e10", ("s4", 4), 10)).as_deref(), Some("e1"));
        // The last episode of the last season leads nowhere, and so does a film.
        assert_eq!(key(&episode("e1", ("s5", 5), 1)), None);
        assert_eq!(key(&row("movie", "m", 1)), None);
    }

    fn playing(enabled: bool) -> Playing {
        Playing {
            rating_key: "9".to_owned(),
            item_key: Some("/library/metadata/9".to_owned()),
            time_ms: 41_000,
            duration_ms: Some(300_000),
            enabled,
        }
    }

    #[test]
    fn only_a_report_at_the_end_of_an_item_asks_for_the_watched_flag() {
        assert!(finished(PlayState::Stopped, 300_000, Some(300_000)));
        assert!(finished(PlayState::Paused, 301_000, Some(300_000)));
        // A live position at the very end is still playing, not finished.
        assert!(!finished(PlayState::Playing, 300_000, Some(300_000)));
        assert!(!finished(PlayState::Stopped, 299_000, Some(300_000)));
        // With no duration there is nothing to compare against, so nothing is
        // claimed - and nothing is marked watched by mistake.
        assert!(!finished(PlayState::Stopped, 10, None));
    }

    #[test]
    fn a_timeline_report_carries_the_position_and_the_setting_it_was_made_under() {
        let off = playing(false).timeline(PlayState::Playing);
        let on = playing(true).timeline(PlayState::Stopped);
        for (cmd, state, enabled) in [
            (off, PlayState::Playing, false),
            (on, PlayState::Stopped, true),
        ] {
            match cmd {
                Cmd::Timeline {
                    rating_key,
                    item_key,
                    state: got_state,
                    time_ms,
                    duration_ms,
                    enabled: got_enabled,
                } => {
                    assert_eq!(rating_key, "9");
                    assert_eq!(item_key.as_deref(), Some("/library/metadata/9"));
                    assert_eq!(got_state, state);
                    assert_eq!(time_ms, 41_000);
                    assert_eq!(duration_ms, Some(300_000));
                    assert_eq!(got_enabled, enabled);
                }
                _ => panic!("expected a timeline report"),
            }
        }
    }
}
