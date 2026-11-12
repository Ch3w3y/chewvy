//! Adding a source: the Jellyfin add flow, as a state machine plus the few
//! blocking calls its steps need.
//!
//! The address is the only thing this app has to invent for a Jellyfin server:
//! the server announces itself on the LAN, so the screen offers what the
//! broadcast found before the user types anything. From a validated address the
//! user picks a sign-in — Quick Connect, which needs a signed-in device and a
//! six-digit code, or a username and password, offered as the fallback.
//!
//! The stages are data ([`AddFlow`] over [`FlowInput`]) and the only effect is a
//! [`JfCmd`], so a whole sign-in path is testable by answering with what the
//! server would have answered. The blocking calls in this module — each one
//! jellyfin-core already implements — run on the worker's helper threads
//! ([`crate::watch`]), never on the event-loop thread, and their answers come
//! back as [`crate::watch::JfEv`]. A code's wait is the one long call, so it is
//! cancellable: [`keep_waiting`] says when asking again still makes sense.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use plex_core::secret::SecretString;

use jellyfin_core::browse;
use jellyfin_core::device::DeviceInfo;
use jellyfin_core::dto;
use jellyfin_core::error::JfError;
use jellyfin_core::header;
use jellyfin_core::http::Http;
use jellyfin_core::{auth, discovery, quickconnect, server};

use crate::sources::{self, JellyfinServer};
use crate::watch::JfCmd;

/// Where the Jellyfin device identity lives under the app config dir. It is this
/// app's own device id, kept apart from the Plex one so the two server kinds see
/// two devices.
const DEVICE_FILE: &str = "jellyfin-device.json";

/// What this app reports as its version, for a Jellyfin server's device list.
const CLIENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// How long the LAN broadcast waits for answers before this app gives up on it.
const DISCOVERY_WAIT: Duration = Duration::from_millis(1_500);

/// How long this app keeps asking about a Quick Connect code before telling the
/// user it is gone. The code's own lifetime is the server's; this bounds only
/// how long anyone waits to be told.
pub const QUICK_CONNECT_BUDGET: Duration = Duration::from_secs(4 * 60);

/// Where the add flow has got to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Asking for the server's address.
    Address,
    /// Asking that address what it is.
    Checking,
    /// The server is known; choose how to sign in.
    Ready,
    /// Waiting for the six-digit code to be approved.
    Pairing,
    /// Asking for a username and password.
    Password,
    /// Sending that username and password.
    SigningIn,
    /// The server issued a token and this app stored it.
    Added,
}

impl Stage {
    /// Whether the flow is waiting on something this app or the server has to
    /// answer — what makes a spinner honest.
    #[must_use]
    pub fn is_busy(self) -> bool {
        matches!(self, Self::Checking | Self::Pairing | Self::SigningIn)
    }
}

/// Why a step did not get further, in this app's own words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AddFailure {
    /// No address to work with.
    MissingAddress,
    /// No username to sign in with.
    MissingUsername,
    /// Nothing answers the form the text takes.
    BadAddress,
    /// No route, or nothing listening.
    Unreachable,
    /// The server answered, but with that status.
    HttpStatus(u16),
    /// The server is still mid-setup, so it cannot sign anyone in.
    WizardIncomplete,
    /// Quick Connect is switched off on this server.
    QuickConnectDisabled,
    /// The code went away before it was approved.
    Expired,
    /// The server did not accept that username and password.
    Rejected,
    /// Something answered, but not in the shape a Jellyfin server answers in.
    ServerShape,
    /// This app could not keep its device identity on this device.
    Identity,
    /// This app could not keep the sign-in on this device.
    Storage,
}

impl AddFailure {
    /// What to show the user. Every message names what to do next.
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::MissingAddress => "Type the server's address first.".to_owned(),
            Self::MissingUsername => "Type the username you sign in with.".to_owned(),
            Self::BadAddress => {
                "That is not an address this app can use: try 192.168.1.20:8096 or \
                 jellyfin.home:8096."
                    .to_owned()
            }
            Self::Unreachable => {
                "Nothing answered at that address. Is the server running, and is the port \
                 right?"
                    .to_owned()
            }
            Self::HttpStatus(status) => format!(
                "The server answered HTTP {status} rather than Jellyfin. Is that the server's \
                 own port?"
            ),
            Self::WizardIncomplete => {
                "This server has not finished its setup wizard. Open its web interface, finish \
                 the setup, then try again."
                    .to_owned()
            }
            Self::QuickConnectDisabled => {
                "Quick Connect is off on this server. Turn it on under Dashboard > Networking > \
                 Quick Connect, or sign in with a username and password."
                    .to_owned()
            }
            Self::Expired => {
                "That code is no longer live. Start Quick Connect again and use it within a few \
                 minutes."
                    .to_owned()
            }
            Self::Rejected => {
                "The server did not accept that username and password. Try again, or sign in \
                 another way."
                    .to_owned()
            }
            Self::ServerShape => {
                "That address answered, but not like a Jellyfin server.".to_owned()
            }
            Self::Identity => {
                "This app could not create its device identity for this server.".to_owned()
            }
            Self::Storage => {
                "The server issued a token, but this app could not keep it on this device. Check \
                 the keychain or this app's config folder."
                    .to_owned()
            }
        }
    }
}

impl From<JfError> for AddFailure {
    fn from(error: JfError) -> Self {
        match error {
            JfError::Transport | JfError::Io => Self::Unreachable,
            JfError::HttpStatus { status } => Self::HttpStatus(status),
            JfError::ResponseShape => Self::ServerShape,
            JfError::InvalidInput => Self::BadAddress,
            JfError::Identity => Self::Identity,
            JfError::Storage(_) => Self::Storage,
            JfError::AuthRejected { .. } => Self::Rejected,
            JfError::QuickConnectDisabled => Self::QuickConnectDisabled,
        }
    }
}

/// Why an address did not name a usable server, where a rejection can only ever
/// be a status: asking a public endpoint needs no sign-in of ours.
fn refused(error: JfError) -> AddFailure {
    match error {
        JfError::AuthRejected { status } => AddFailure::HttpStatus(status),
        other => AddFailure::from(other),
    }
}

/// A server the address the user typed turned out to name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidatedServer {
    /// The server's system uuid — the token slot's key.
    pub server_id: String,
    /// The name the server was asked for.
    pub name: String,
    /// The API base the address normalised to.
    pub base_url: String,
    /// The server's own version string.
    pub version: String,
}

/// A server the LAN broadcast answered with, offered as a one-click choice.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredServer {
    /// The name the server was asked for.
    pub name: String,
    /// The address to use if the user chooses it.
    pub address: String,
}

/// What the user or the server just answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FlowInput {
    /// The address field now holds this.
    Address(String),
    /// The user asked for that address to be checked.
    SubmitAddress,
    /// The user asked what the network answers with.
    Discover,
    /// What the LAN broadcast answered with.
    Discovered(Vec<DiscoveredServer>),
    /// What checking the address answered with.
    Validated(ValidatedServer),
    /// The code the user must enter on a signed-in device.
    Code(String),
    /// The user chose username and password instead of a code.
    ChoosePassword,
    /// The user chose a code again.
    ChooseQuickConnect,
    /// The username field now holds this.
    Username(String),
    /// The password field now holds this.
    Password(String),
    /// The user asked for that username and password to be sent.
    SubmitPassword,
    /// A source was added, and this is its row.
    Added(JellyfinServer),
    /// The step that was waiting did not work.
    Failed(AddFailure),
}

/// The add panel: which stage it is at, and what its fields hold.
///
/// The password lives here, in memory, only until it is sent — and `sources.json`
/// never sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AddFlow {
    /// Where the flow has got to.
    pub stage: Stage,
    /// The address field.
    pub address: String,
    /// The username field.
    pub username: String,
    /// The password field, cleared as soon as it is sent.
    pub password: String,
    /// The server the flow is working on, once an address has named one.
    pub server: Option<ValidatedServer>,
    /// The Quick Connect code, once the server issues one.
    pub code: String,
    /// What the LAN broadcast offered.
    pub discovered: Vec<DiscoveredServer>,
    /// A broadcast is in flight, so the panel says it is looking.
    pub discovering: bool,
    /// Whether the password field shows what it holds.
    pub show_password: bool,
    /// What the last step answered with, if it did not work.
    pub error: Option<AddFailure>,
}

impl Default for AddFlow {
    fn default() -> Self {
        Self {
            stage: Stage::Address,
            address: String::new(),
            username: String::new(),
            password: String::new(),
            server: None,
            code: String::new(),
            discovered: Vec::new(),
            discovering: false,
            show_password: false,
            error: None,
        }
    }
}

impl AddFlow {
    /// A panel opened on the address step, already asking the network what it
    /// answers with — which is the command its caller sends to start.
    #[must_use]
    pub fn open() -> Self {
        Self {
            discovering: true,
            ..Self::default()
        }
    }

    /// Record `input` and answer with the one thing the worker has to do next, if
    /// anything.
    ///
    /// An answer from a step the flow has already left is dropped, not applied: it
    /// belongs to an operation the user walked away from. Every failure returns to
    /// a step the user can act on again, so no stage can trap the panel.
    pub fn apply(&mut self, input: FlowInput) -> Option<JfCmd> {
        match input {
            FlowInput::Address(text) => self.retype(text),
            FlowInput::Username(text) => {
                self.username = text;
                None
            }
            FlowInput::Password(text) => {
                self.password = text;
                None
            }
            FlowInput::Code(code) => {
                if self.stage == Stage::Pairing {
                    self.code = code;
                }
                None
            }
            // A broadcast answer only refreshes the choices — even a late one, or
            // an empty one, is never a reason to move a stage.
            FlowInput::Discovered(found) => {
                self.discovered = found;
                self.discovering = false;
                None
            }
            FlowInput::Discover => {
                self.error = None;
                self.discovering = true;
                Some(JfCmd::Discover)
            }
            FlowInput::SubmitAddress => self.check_address(),
            FlowInput::Validated(server) => self.accept_address(server),
            FlowInput::ChoosePassword => self.use_password(),
            FlowInput::ChooseQuickConnect => self.use_code(),
            FlowInput::SubmitPassword => self.send_password(),
            FlowInput::Added(source) => self.finish(source),
            FlowInput::Failed(failure) => self.give_up(failure),
        }
    }

    /// The address field changed: a new address is a new attempt.
    fn retype(&mut self, text: String) -> Option<JfCmd> {
        self.address = text;
        if self.stage == Stage::Checking {
            // Editing while an answer is on its way restarts the step.
            self.stage = Stage::Address;
            self.error = None;
        }
        None
    }

    /// Ask what the typed address is, if there is one to ask about.
    fn check_address(&mut self) -> Option<JfCmd> {
        if self.address.trim().is_empty() {
            self.stage = Stage::Address;
            self.error = Some(AddFailure::MissingAddress);
            return None;
        }
        self.error = None;
        self.server = None;
        self.code.clear();
        self.stage = Stage::Checking;
        Some(JfCmd::Validate {
            address: self.address.trim().to_owned(),
        })
    }

    /// Record what checking the address answered with, unless the user has
    /// already moved on from that step.
    fn accept_address(&mut self, server: ValidatedServer) -> Option<JfCmd> {
        if self.stage != Stage::Checking {
            return None;
        }
        self.address = server.base_url.clone();
        self.error = None;
        self.server = Some(server);
        self.stage = Stage::Ready;
        None
    }

    /// Fall back to a username and password, abandoning a code still waiting.
    fn use_password(&mut self) -> Option<JfCmd> {
        let was_pairing = self.stage == Stage::Pairing;
        self.error = None;
        self.code.clear();
        self.stage = Stage::Password;
        was_pairing.then_some(JfCmd::Stop)
    }

    /// Try a code again, on this server or after a failed attempt.
    fn use_code(&mut self) -> Option<JfCmd> {
        self.error = None;
        self.code.clear();
        self.stage = Stage::Pairing;
        Some(JfCmd::QuickConnect)
    }

    /// Send the username and password this panel is holding.
    fn send_password(&mut self) -> Option<JfCmd> {
        if self.username.trim().is_empty() {
            self.stage = Stage::Password;
            self.error = Some(AddFailure::MissingUsername);
            return None;
        }
        self.error = None;
        self.stage = Stage::SigningIn;
        // The password leaves the flow here: only the request holds it.
        Some(JfCmd::SignIn {
            username: self.username.trim().to_owned(),
            password: std::mem::take(&mut self.password),
        })
    }

    /// A source was added, and its row is what the Sources list will show.
    fn finish(&mut self, source: JellyfinServer) -> Option<JfCmd> {
        self.error = None;
        self.code.clear();
        self.server = Some(ValidatedServer {
            server_id: source.server_id,
            name: source.name,
            base_url: source.base_url,
            version: String::new(),
        });
        self.stage = Stage::Added;
        None
    }

    /// A step failed: say so, and land on the step the user can act on again.
    fn give_up(&mut self, failure: AddFailure) -> Option<JfCmd> {
        self.error = Some(failure);
        self.stage = match self.stage {
            // Either half of a sign-in can simply have been wrong, so both land
            // back where the user can correct it.
            Stage::Checking => Stage::Address,
            Stage::SigningIn => Stage::Password,
            Stage::Pairing => Stage::Ready,
            other => other,
        };
        None
    }
}

/// The server a LAN broadcast answered with, as a choice this app can offer —
/// and only when it advertised an address to offer, which some do not.
fn offered(found: discovery::DiscoveredServer) -> Option<DiscoveredServer> {
    let address = found.address?;
    Some(DiscoveredServer {
        name: found.name,
        address,
    })
}

/// The name this app reports itself as: the machine's own where one can be read,
/// since a device list of "Chewvy"s is no help to anyone.
fn machine_name() -> String {
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .and_then(|text| text.lines().next().map(str::to_owned))
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| jellyfin_core::device::APP_NAME.to_owned())
}

/// This install's Jellyfin device identity, kept in a file so the server's device
/// list sees the same device across restarts.
///
/// # Errors
///
/// [`AddFailure`] naming why no identity could be read or created.
pub fn device_identity() -> Result<DeviceInfo, AddFailure> {
    let path = sources::config_dir()
        .map_or_else(|| PathBuf::from(DEVICE_FILE), |dir| dir.join(DEVICE_FILE));
    DeviceInfo::load_or_create(&path, CLIENT_VERSION, &machine_name()).map_err(AddFailure::from)
}

/// One transport to a server this flow has already validated.
///
/// # Errors
///
/// [`AddFailure`] naming why the stored address cannot be reached at all.
fn transport(server: &ValidatedServer) -> Result<Http, AddFailure> {
    Http::new(&server.base_url).map_err(AddFailure::from)
}

/// Ask the network which Jellyfin servers answer, skipping the ones this app can
/// already play from.
#[must_use]
pub fn discover(exclude: &[String]) -> Vec<DiscoveredServer> {
    discovery::discover(DISCOVERY_WAIT)
        .unwrap_or_default()
        .into_iter()
        .filter(|found| !exclude.contains(&found.id))
        .filter_map(offered)
        .collect()
}

/// Ask an address what it is, refusing a server that has not finished its own
/// setup: it answers, but it cannot sign anyone in yet.
///
/// # Errors
///
/// [`AddFailure`] naming why the address did not name a usable server.
pub fn validate(address: &str) -> Result<ValidatedServer, AddFailure> {
    let base = server::normalize_base(address).map_err(refused)?;
    let http = Http::new(&base).map_err(refused)?;
    let info = server::public_info(&http).map_err(refused)?;
    if info.startup_wizard_completed == Some(false) {
        return Err(AddFailure::WizardIncomplete);
    }
    Ok(ValidatedServer {
        server_id: info.id,
        name: info.server_name,
        base_url: base,
        version: info.version,
    })
}

/// Ask the server for a Quick Connect code.
///
/// # Errors
///
/// [`AddFailure`] naming why no code is on its way.
pub fn initiate_quick_connect(
    server: &ValidatedServer,
) -> Result<quickconnect::Ticket, AddFailure> {
    let http = transport(server)?;
    let device = device_identity()?;
    quickconnect::initiate(&http, &device).map_err(AddFailure::from)
}

/// One look at a code, which trades it for a token the first time it is approved.
///
/// # Errors
///
/// [`AddFailure`] naming why the server would not answer about the code.
pub fn quick_connect_answer(
    server: &ValidatedServer,
    ticket: &quickconnect::Ticket,
) -> Result<quickconnect::QuickConnect, AddFailure> {
    let http = transport(server)?;
    let device = device_identity()?;
    quickconnect::check(&http, &device, ticket).map_err(AddFailure::from)
}

/// Sign in with a username and password.
///
/// # Errors
///
/// [`AddFailure`] naming why the server did not issue a token for them.
pub fn sign_in_password(
    server: &ValidatedServer,
    username: &str,
    password: &str,
) -> Result<(JellyfinServer, SecretString), AddFailure> {
    let http = transport(server)?;
    let device = device_identity()?;
    let issued =
        auth::sign_in_password(&http, &device, username, password).map_err(AddFailure::from)?;
    issued_server(server, issued.into_token())
}

/// The row a fresh token belongs to, with the token to keep beside it: the row
/// names the user the token answers as, and the token is what the caller stores.
///
/// # Errors
///
/// [`AddFailure`] naming why the server would not say who the token belongs to.
pub fn issued_server(
    server: &ValidatedServer,
    token: SecretString,
) -> Result<(JellyfinServer, SecretString), AddFailure> {
    source_for(server, &token).map(|source| (source, token))
}

/// The row a token belongs to, with the user the server names for it — which is
/// what a Quick Connect sign-in arrives without.
///
/// # Errors
///
/// [`AddFailure`] naming why the server would not say who the token belongs to.
pub fn source_for(
    server: &ValidatedServer,
    token: &SecretString,
) -> Result<JellyfinServer, AddFailure> {
    let http = transport(server)?;
    let device = device_identity()?;
    let authorization = header::media_browser_header(&device, Some(token));
    let response = http
        .get(browse::ME_PATH, &[], Some(&authorization))
        .map_err(AddFailure::from)?;
    let user: dto::User = response.json().map_err(|_| AddFailure::ServerShape)?;
    Ok(JellyfinServer {
        server_id: server.server_id.clone(),
        name: server.name.clone(),
        base_url: server.base_url.clone(),
        user_id: (!user.id.is_empty()).then_some(user.id),
        user_name: user.name.filter(|name| !name.is_empty()),
        added_at: sources::unix_now(),
    })
}

/// Tell a server this app is done with a token it is dropping. Best effort: the
/// source is already gone from this device by the time this runs.
///
/// # Errors
///
/// [`AddFailure`] naming why the server did not take the notice.
pub fn sign_out(server: &JellyfinServer, token: &SecretString) -> Result<(), AddFailure> {
    let http = Http::new(&server.base_url).map_err(AddFailure::from)?;
    let device = device_identity()?;
    auth::sign_out(&http, &device, token).map_err(AddFailure::from)
}

/// Whether a waiting Quick Connect is still worth asking about.
#[must_use]
pub fn keep_waiting(cancelled: &AtomicBool, waited: Duration) -> bool {
    !cancelled.load(Ordering::Relaxed) && waited < QUICK_CONNECT_BUDGET
}

#[cfg(test)]
#[path = "add_source/tests.rs"]
mod tests;
