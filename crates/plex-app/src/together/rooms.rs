//! Watching together, as the worker holds it: the Plex rooms this account is
//! in, the session for whichever group or room this device has joined, and
//! the routing of each [`Ask`] to the one it is for.
//!
//! While a Plex account is signed in a thread looks at its rooms every
//! [`POLL`], so an invitation shows without a restart; nothing is polled
//! while no Plex account is. Creating, inviting and removing are single
//! calls on threads of their own, and each has the list read again.

use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::Duration;

use plex_core::client_info::ClientInfo;
use plex_core::error::CoreError;
use plex_core::http::Http;
use plex_core::secret::SecretString;
use plex_core::together::people::{self, Friend, HomeUser, COMMUNITY_BASE};
use plex_core::together::room::{self, TOGETHER_BASE};

use super::jellyfin::{self, CLOSE_WAIT};
use super::plex::{Entry, Session};
use super::{Ask, Elsewhere, News, Person, Room};
use crate::sources::browse;
use crate::watch::Ev;

/// How often the room list is read while a Plex account is signed in: soon
/// enough for an invitation to show while someone waits for it, rare enough
/// to cost the service nothing.
const POLL: Duration = Duration::from_secs(30);

/// What the room calls need of the signed-in Plex account.
pub(crate) struct Account {
    pub(super) token: SecretString,
    pub(super) client: ClientInfo,
    /// This account's plex.tv user id, when plex.tv said.
    pub(super) me: Option<u64>,
    /// The connected Plex server's machine identifier.
    pub(super) machine: Option<String>,
    /// Every server the account has, as machine identifier and name.
    servers: Vec<(String, String)>,
    pub(super) together: Http,
    plex_tv: Http,
    community: Http,
}

impl Account {
    /// The account behind `token`, connected to the server whose machine
    /// identifier is `machine`, with `servers` all it has.
    ///
    /// # Errors
    ///
    /// When a transport cannot be built.
    pub(crate) fn new(
        token: SecretString,
        client: &ClientInfo,
        plex_tv: Http,
        me: Option<u64>,
        machine: Option<String>,
        servers: Vec<(String, String)>,
    ) -> Result<Self, CoreError> {
        Ok(Self {
            together: Http::new(TOGETHER_BASE, client)?,
            community: Http::new(COMMUNITY_BASE, client)?,
            plex_tv,
            token,
            client: client.clone(),
            me,
            machine,
            servers,
        })
    }
}

/// The Plex side: the signed-in account, its room poller and the room this
/// device is in.
#[derive(Default)]
pub(crate) struct Plex {
    account: Option<Arc<Account>>,
    /// Wakes the poller for an early look; dropping it stops the poller.
    poll: Option<Sender<()>>,
    session: Option<Session>,
}

/// Every group and room this device can be in, at most one at a time.
#[derive(Default)]
pub(crate) struct Groups {
    jellyfin: Option<jellyfin::Session>,
    plex: Plex,
}

impl Groups {
    /// A Plex account signed in, or another server connected (`Some`), or
    /// signed out (`None`): the room list follows it.
    pub(crate) fn plex_account(&mut self, account: Option<Account>, ev: &Sender<Ev>) {
        let plex = &mut self.plex;
        plex.poll = None;
        plex.account = account.map(Arc::new);
        match &plex.account {
            Some(account) => plex.poll = poll(Arc::clone(account), ev),
            None => {
                if let Some(session) = plex.session.take() {
                    session.close(CLOSE_WAIT);
                }
                tell(ev, News::Rooms(Ok(Vec::new())));
            }
        }
    }

    /// One ask from the screens or the player.
    pub(crate) fn handle(&mut self, pool: &browse::Jellyfin, ask: Ask, ev: &Sender<Ev>) {
        if self.plex.session.as_ref().is_some_and(|s| !s.alive()) {
            self.plex.session = None;
        }
        match ask {
            Ask::Rooms => self.plex.look(),
            Ask::People => self.plex.people(ev),
            Ask::Create {
                rating_key,
                title,
                people,
            } => self.enter(
                Entry::Create {
                    rating_key,
                    title,
                    people,
                },
                ev,
            ),
            Ask::Enter { room, .. } => self.enter(Entry::Join(room), ev),
            Ask::Invite { room, people } => self.plex.invite(room, people, ev),
            Ask::Remove { room } => self.plex.remove(room, ev),
            Ask::Start { .. } | Ask::Join { .. } => {
                self.plex.close();
                jellyfin::handle(&mut self.jellyfin, pool, ask, ev);
            }
            Ask::Groups { .. } => jellyfin::handle(&mut self.jellyfin, pool, ask, ev),
            other => match &self.plex.session {
                Some(session) => session.ask(other),
                None => jellyfin::handle(&mut self.jellyfin, pool, other, ev),
            },
        }
    }

    /// Leave whatever this device is in: the app is closing.
    pub(crate) fn close(&mut self) {
        if let Some(session) = self.jellyfin.take() {
            session.close(CLOSE_WAIT);
        }
        self.plex.close();
    }

    /// Into a Plex room, out of anything else first.
    fn enter(&mut self, entry: Entry, ev: &Sender<Ev>) {
        if let Some(session) = self.jellyfin.take() {
            session.close(CLOSE_WAIT);
        }
        self.plex.close();
        let Some(account) = self.plex.account.clone() else {
            return tell(
                ev,
                News::Left {
                    why: Some("Sign in to Plex to watch together.".into()),
                },
            );
        };
        self.plex.session = Session::open(account, entry, self.plex.poll.clone(), ev);
    }
}

impl Plex {
    fn close(&mut self) {
        if let Some(session) = self.session.take() {
            session.close(CLOSE_WAIT);
        }
    }

    /// Read the room list now.
    fn look(&self) {
        if let Some(poll) = &self.poll {
            let _ = poll.send(());
        }
    }

    /// Who can be invited, on a thread of its own.
    fn people(&self, ev: &Sender<Ev>) {
        let Some(account) = self.account.clone() else {
            return tell(ev, News::People(Err("Sign in to Plex first.".into())));
        };
        spawn(ev, "plex-people", move |news| {
            let home = people::home_users(&account.plex_tv, &account.token);
            let friends = people::friends(&account.community, &account.token);
            let listed = match (home, friends) {
                (Err(error), _) => Err(refusal(error)),
                // Friends are a separate service; Home users alone still help.
                (Ok(home), friends) => {
                    Ok(invitable(&home, &friends.unwrap_or_default(), account.me))
                }
            };
            tell(news, News::People(listed));
        });
    }

    fn invite(&self, id: String, people: Vec<u64>, ev: &Sender<Ev>) {
        let Some(account) = self.account.clone() else {
            return;
        };
        let poll = self.poll.clone();
        spawn(ev, "plex-invite", move |news| {
            match room::invite(&account.together, &account.token, &id, &people) {
                Ok(()) => tell(news, News::Notice("Invited".into())),
                Err(error) => tell(
                    news,
                    News::Notice(format!("Could not invite: {}", refusal(error))),
                ),
            }
            if let Some(poll) = poll {
                let _ = poll.send(());
            }
        });
    }

    /// Take a room off this account's list, leaving it first when this
    /// device is in it.
    fn remove(&mut self, id: String, ev: &Sender<Ev>) {
        if self.session.as_ref().and_then(Session::room) == Some(id.as_str()) {
            self.close();
        }
        let Some(account) = self.account.clone() else {
            return;
        };
        let poll = self.poll.clone();
        spawn(ev, "plex-remove", move |news| {
            if let Err(error) = room::leave_room(&account.together, &account.token, &id) {
                tell(
                    news,
                    News::Notice(format!("Could not remove the room: {}", refusal(error))),
                );
            }
            if let Some(poll) = poll {
                let _ = poll.send(());
            }
        });
    }
}

/// Read the room list now and every [`POLL`] until the returned sender is
/// dropped; a message on it reads the list at once.
fn poll(account: Arc<Account>, ev: &Sender<Ev>) -> Option<Sender<()>> {
    let (wake, woken) = mpsc::channel::<()>();
    let news = ev.clone();
    let started = std::thread::Builder::new()
        .name("plex-rooms".into())
        .spawn(move || {
            let mut last: Option<Result<Vec<Room>, String>> = None;
            loop {
                let listed = room::list_rooms(&account.together, &account.token)
                    .map(|rooms| {
                        let machine = account.machine.as_deref();
                        rows(&rooms, machine, &account.servers, account.me)
                    })
                    .map_err(refusal);
                if last.as_ref() != Some(&listed) {
                    tell(&news, News::Rooms(listed.clone()));
                    last = Some(listed);
                }
                match woken.recv_timeout(POLL) {
                    Ok(()) => {
                        // Several wakes at once need one look.
                        while woken.try_recv().is_ok() {}
                        // A look asked for is answered even when unchanged,
                        // so a panel waiting for it stops waiting.
                        last = None;
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
        });
    started.is_ok().then_some(wake)
}

/// The service's rooms as the screens show them: `machine` is the connected
/// server and `servers` every server the account has.
pub(crate) fn rows(
    rooms: &[room::Room],
    machine: Option<&str>,
    servers: &[(String, String)],
    me: Option<u64>,
) -> Vec<Room> {
    rooms
        .iter()
        .map(|found| {
            let source = room::parse_source_uri(&found.source_uri);
            let here = source
                .as_ref()
                .filter(|source| Some(source.machine_identifier.as_str()) == machine);
            let elsewhere = source
                .as_ref()
                .filter(|_| here.is_none())
                .and_then(|source| {
                    let (machine, name) = servers
                        .iter()
                        .find(|(machine, _)| *machine == source.machine_identifier)?;
                    Some(Elsewhere {
                        machine: machine.clone(),
                        name: name.clone(),
                    })
                });
            let mut users: Vec<&room::RoomUser> = found.users.iter().collect();
            // The creator first, then in the service's order.
            users.sort_by_key(|user| Some(user.id) != found.created_by);
            Room {
                id: found.id.clone(),
                title: found.title.clone(),
                rating_key: here.map(|source| source.rating_key.clone()),
                elsewhere,
                people: users
                    .iter()
                    .filter(|user| Some(user.id) != me)
                    .map(|user| name(&user.title, &user.username))
                    .collect(),
                ids: found.users.iter().map(|user| user.id).collect(),
            }
        })
        .collect()
}

/// Who a room can invite: the Home's other members first, then friends,
/// each once and never this account.
pub(crate) fn invitable(home: &[HomeUser], friends: &[Friend], me: Option<u64>) -> Vec<Person> {
    let mut listed: Vec<Person> = Vec::new();
    let others = home
        .iter()
        // Without this account's id the Home's administrator stands for it.
        .filter(|user| me.map_or(!user.admin, |me| user.id != me))
        .map(|user| Person {
            id: user.id,
            name: name(&user.title, ""),
            home: true,
        });
    let friends = friends.iter().map(|friend| Person {
        id: friend.id,
        name: name(&friend.display_name, &friend.username),
        home: false,
    });
    for person in others.chain(friends) {
        if Some(person.id) != me && listed.iter().all(|known| known.id != person.id) {
            listed.push(person);
        }
    }
    listed
}

fn name(title: &str, username: &str) -> String {
    [title, username]
        .into_iter()
        .find(|text| !text.trim().is_empty())
        .unwrap_or("Someone")
        .to_owned()
}

/// A refusal from the room service in words the panel can show as they are.
pub(crate) fn refusal(error: CoreError) -> String {
    match error {
        CoreError::AuthRejected { .. } => "Plex no longer accepts this sign-in.".into(),
        CoreError::HttpStatus { status: 404 | 410 } => "That room no longer exists.".into(),
        CoreError::Transport => "Plex's Watch Together service could not be reached.".into(),
        other => format!("Plex's Watch Together service answered with an error ({other})."),
    }
}

/// Run `work` on a thread of its own with a sender for its news.
fn spawn(ev: &Sender<Ev>, name: &str, work: impl FnOnce(&Sender<Ev>) + Send + 'static) {
    let news = ev.clone();
    let started = std::thread::Builder::new()
        .name(name.into())
        .spawn(move || work(&news));
    if started.is_err() {
        tell(
            ev,
            News::Notice("Watching together is busy; try again.".into()),
        );
    }
}

fn tell(ev: &Sender<Ev>, news: News) {
    let _ = ev.send(Ev::Together(news));
}
