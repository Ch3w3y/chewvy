//! The source list as the worker thread owns it: `sources.json`, the token
//! slots, and the Jellyfin add step in flight.
//!
//! Every Jellyfin call blocks, so each step of adding a source runs on its own
//! thread and answers back on the worker's command channel. This desk is the
//! only place those answers, the registry and the token slots meet, so a finished
//! step never rewrites a file another thread is still reading, and the password
//! a step was given is dropped with the step.
//!
//! What the *panel* shows is [`crate::add_source::AddFlow`]'s; what this device
//! keeps is [`Desk`]'s.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::{Duration, Instant};

use plex_core::secret::SecretString;
use plex_core::tokens::{load_plex_token, TokenStore};

use jellyfin_core::quickconnect::QuickConnect;

use crate::add_source::{self, AddFailure, ValidatedServer};
use crate::sources::{self, jellyfin_token_key, JellyfinServer, Sources};
use crate::watch::{Cmd, Ev, JfAnswer, JfCmd, JfEv};

/// How often a waiting Quick Connect code is asked about again.
const CODE_POLL: Duration = Duration::from_millis(1_200);

/// The registry, its token slots, and the one add step in flight.
pub(crate) struct Desk {
    /// Every source this device can play from, as its file left it.
    sources: Sources,
    /// The server the panel is working on, once one of its steps named one.
    pending: Option<ValidatedServer>,
    /// Set to stop waiting on a code the panel walked away from.
    waiting: Option<Arc<AtomicBool>>,
}

impl Desk {
    /// The registry as this install left it, read before the window's first frame.
    #[must_use]
    pub(crate) fn open() -> Self {
        Self {
            sources: sources::load(),
            pending: None,
            waiting: None,
        }
    }

    /// The Jellyfin servers the registry holds, in the order it holds them.
    #[must_use]
    pub(crate) fn jellyfin(&self) -> &[JellyfinServer] {
        &self.sources.jellyfin
    }

    /// One step the Sources panel asked for. A step that needs a server cannot
    /// start before an address has named one.
    pub(crate) fn step(
        &mut self,
        step: JfCmd,
        store: Option<&dyn TokenStore>,
        ev: &Sender<Ev>,
        cmds: &Sender<Cmd>,
    ) {
        match step {
            JfCmd::Discover => {
                let known: Vec<String> = self
                    .sources
                    .jellyfin
                    .iter()
                    .map(|held| held.server_id.clone())
                    .collect();
                spawn(
                    cmds,
                    "jf-discover",
                    JfAnswer::Found(Vec::new()),
                    move || JfAnswer::Found(add_source::discover(&known)),
                );
            }
            JfCmd::Validate { address } => spawn(
                cmds,
                "jf-validate",
                JfAnswer::Checked(Err(AddFailure::Unreachable)),
                move || JfAnswer::Checked(add_source::validate(&address)),
            ),
            JfCmd::QuickConnect => {
                if let Some(server) = self.pending.clone() {
                    let waiting = self.wait_for();
                    let code = ev.clone();
                    spawn(
                        cmds,
                        "jf-quickconnect",
                        JfAnswer::SignedIn(Err(AddFailure::Identity)),
                        move || JfAnswer::SignedIn(quick_connect(&server, &waiting, &code)),
                    );
                }
            }
            JfCmd::SignIn { username, password } => {
                if let Some(server) = self.pending.clone() {
                    spawn(
                        cmds,
                        "jf-signin",
                        JfAnswer::SignedIn(Err(AddFailure::Identity)),
                        move || {
                            JfAnswer::SignedIn(add_source::sign_in_password(
                                &server, &username, &password,
                            ))
                        },
                    );
                }
            }
            JfCmd::Stop => self.abandon(),
            JfCmd::Dismiss => self.finish(),
            JfCmd::Remove { server_id } => self.remove(&server_id, store, ev),
        }
    }

    /// What one of those steps' threads came back with.
    pub(crate) fn answer(
        &mut self,
        answer: JfAnswer,
        store: Option<&dyn TokenStore>,
        ev: &Sender<Ev>,
    ) {
        match answer {
            // A broadcast answer only refreshes the choices it offered.
            JfAnswer::Found(found) => tell(ev, JfEv::Found(found)),
            // An address that named a server names the server the panel works on.
            JfAnswer::Checked(Ok(server)) => {
                self.pending = Some(server.clone());
                tell(ev, JfEv::Validated(server));
            }
            JfAnswer::Checked(Err(failure)) => tell(ev, JfEv::Failed(failure)),
            // A sign-in the panel left behind keeps nothing: the token it issued is
            // dropped rather than stored under a source nobody is adding any more.
            JfAnswer::SignedIn(outcome) if self.pending.is_none() => drop(outcome),
            JfAnswer::SignedIn(Err(failure)) => tell(ev, JfEv::Failed(failure)),
            JfAnswer::SignedIn(Ok((source, token))) => self.keep(source, token, store, ev),
        }
    }

    /// The Plex row follows the token on file: recorded when one is there, gone
    /// once a sign-out took it away. The list is shown again either way, since
    /// the window needs it on its first frame.
    pub(crate) fn sync_plex(
        &mut self,
        store: Option<&dyn TokenStore>,
        account_id: Option<String>,
        ev: &Sender<Ev>,
    ) {
        let signed_in = plex_token_on_file(store);
        if signed_in {
            self.sources.record_plex(account_id);
        } else {
            self.sources.clear_plex();
        }
        self.publish(ev);
    }

    /// Keep a sign-in this device asked for: its token first, then its row, so a
    /// source is listed only once the secret that makes it playable is stored.
    fn keep(
        &mut self,
        source: JellyfinServer,
        token: SecretString,
        store: Option<&dyn TokenStore>,
        ev: &Sender<Ev>,
    ) {
        let asked_for = self
            .pending
            .as_ref()
            .is_some_and(|held| held.server_id == source.server_id);
        self.pending = None;
        let key = asked_for.then(|| jellyfin_token_key(&source)).flatten();
        // The panel may have left this server behind, or the row may have no id to
        // key a token slot with; either way the token is dropped here.
        let Some(key) = key else { return };
        if !store.is_some_and(|store| store.store(&key, &token).is_ok()) {
            tell(ev, JfEv::Failed(AddFailure::Storage));
            return;
        }
        self.sources.set_jellyfin(source.clone());
        self.publish(ev);
        tell(ev, JfEv::Added(source));
    }

    /// Forget one source: its token slot first, then its row. The server is not
    /// told — a source's own sign-out is what does that, and this device forgets
    /// a server it cannot reach just the same.
    fn remove(&mut self, server_id: &str, store: Option<&dyn TokenStore>, ev: &Sender<Ev>) {
        if let (Some(store), Some(held)) = (store, self.sources.jellyfin(server_id)) {
            if let Some(key) = jellyfin_token_key(held) {
                let _ = store.delete(&key);
            }
        }
        if self.sources.remove_jellyfin(server_id).is_some() {
            self.publish(ev);
        }
    }

    /// Rewrite the registry and show it again.
    fn publish(&mut self, ev: &Sender<Ev>) {
        if let Err(e) = sources::save(&self.sources) {
            let _ = ev.send(Ev::Error(format!("could not save the source list: {e}")));
        }
        let _ = ev.send(Ev::Sources(self.sources.clone()));
    }

    /// A code is being waited on: the flag its poll loop stops at.
    fn wait_for(&mut self) -> Arc<AtomicBool> {
        self.abandon();
        let waiting = Arc::new(AtomicBool::new(false));
        self.waiting = Some(Arc::clone(&waiting));
        waiting
    }

    /// Whatever is in flight is no longer wanted, so it may stop; the server the
    /// panel named is still its own.
    fn abandon(&mut self) {
        if let Some(waiting) = &self.waiting {
            waiting.store(true, Ordering::Relaxed);
        }
    }

    /// The panel closed: it is waiting for nothing and working on nobody.
    fn finish(&mut self) {
        self.abandon();
        self.pending = None;
    }
}

/// Ask for a Quick Connect code and keep asking about it until it is approved,
/// goes away, or the panel stops wanting it.
fn quick_connect(
    server: &ValidatedServer,
    waiting: &AtomicBool,
    ev: &Sender<Ev>,
) -> Result<(JellyfinServer, SecretString), AddFailure> {
    let ticket = add_source::initiate_quick_connect(server)?;
    tell(ev, JfEv::Code(ticket.code.clone()));
    let started = Instant::now();
    loop {
        std::thread::sleep(CODE_POLL);
        if !add_source::keep_waiting(waiting, started.elapsed()) {
            return Err(AddFailure::Expired);
        }
        match add_source::quick_connect_answer(server, &ticket)? {
            QuickConnect::Pending => {}
            QuickConnect::Authorized(issued) => {
                return add_source::issued_server(server, issued.into_token())
            }
            QuickConnect::Expired => return Err(AddFailure::Expired),
        }
    }
}

/// Whether this install holds a Plex token, in the slot its account id names —
/// which is what makes the registry's Plex row a fact rather than a memory.
fn plex_token_on_file(store: Option<&dyn TokenStore>) -> bool {
    store.is_some_and(|store| {
        load_plex_token(store, crate::watch::plex_token_key().as_ref())
            .ok()
            .flatten()
            .is_some()
    })
}

/// Run one step off the worker thread and answer on its command channel when it
/// is done, so waiting on a code never stops the worker answering anything else.
fn spawn(
    cmds: &Sender<Cmd>,
    name: &str,
    on_no_thread: JfAnswer,
    step: impl FnOnce() -> JfAnswer + Send + 'static,
) {
    let answers = cmds.clone();
    let started = std::thread::Builder::new()
        .name(name.to_owned())
        .spawn(move || {
            let answer = step();
            let _ = answers.send(Cmd::JfAnswer(answer));
        });
    if started.is_err() {
        // With no thread to ask, nothing is coming: answer here rather than wait.
        let _ = cmds.send(Cmd::JfAnswer(on_no_thread));
    }
}

/// One answer for the Sources screen.
fn tell(ev: &Sender<Ev>, answer: JfEv) {
    let _ = ev.send(Ev::Jf(answer));
}
