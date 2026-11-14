//! In-app updates from the project's GitHub releases.
//!
//! A check reads the latest *published* release (drafts and pre-releases are
//! never offered), compares its tag with this build's version and picks the
//! package this install was set up from: the AppImage on Linux, the installer
//! or the zip on Windows, the disk image on macOS. An install downloads that
//! package and the release's `SHA256SUMS`, refuses a package whose digest is not
//! the listed one, and puts it in place ([`install`]). The new version starts
//! when the app is restarted, which [`relaunch`] does after the window closes.
//!
//! An install the app cannot replace itself (a development build, a distro
//! package) still hears that a release is out, and is told why it cannot
//! install it from here.
//!
//! Nothing here sends anything about the user: the requests carry only a
//! `User-Agent` naming the app and its version.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Mutex;
use std::time::Duration;

use serde::Deserialize;

#[path = "update/install.rs"]
mod install;

/// The repository releases are read from, from the workspace manifest.
const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
/// This build's version.
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");
/// The release list's own answer is small; anything bigger is not it.
const MAX_JSON_BYTES: u64 = 4 * 1024 * 1024;
/// The largest package a release is trusted to hold.
const MAX_PACKAGE_BYTES: u64 = 1024 * 1024 * 1024;

/// A version as `major.minor.patch`, with any `-pre` part kept apart: a
/// pre-release sorts before the release it leads up to.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    core: (u64, u64, u64),
    /// `true` for a plain release, so it sorts after its own pre-releases.
    release: bool,
}

impl Version {
    /// `0.2.1`, `v0.2.1` or `0.3.0-rc1`; `None` for anything else.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim().trim_start_matches('v');
        let (core, pre) = match text.split_once(['-', '+']) {
            Some((core, rest)) => (core, !rest.is_empty()),
            None => (text, false),
        };
        let mut parts = core.split('.').map(|part| part.parse::<u64>().ok());
        let major = parts.next()??;
        let minor = parts.next()??;
        let patch = parts.next().unwrap_or(Some(0))?;
        if parts.next().is_some() {
            return None;
        }
        Some(Self {
            core: (major, minor, patch),
            release: !pre,
        })
    }
}

/// What a check found newer than this build.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    /// `0.2.1`, without the tag's `v`.
    pub version: String,
    /// The release's own notes, as its page shows them.
    pub notes: String,
    /// The package for this install, when the release holds one.
    pub package: Option<Asset>,
    /// The release's `SHA256SUMS`.
    pub sums: Option<Asset>,
}

/// One file attached to a release.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Asset {
    /// The file name, as `SHA256SUMS` lists it.
    pub name: String,
    /// Where it downloads from.
    pub url: String,
    /// Its size in bytes, as the release lists it.
    pub size: u64,
}

/// How this copy of the app was installed, which decides what replaces it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The AppImage at this path (Linux).
    AppImage(PathBuf),
    /// The per-user installer's folder (Windows).
    Installer(PathBuf),
    /// The unpacked zip's folder (Windows).
    Portable(PathBuf),
    /// The app bundle at this path (macOS).
    Bundle(PathBuf),
    /// Something the app does not replace itself: why, for the screen.
    Unmanaged(&'static str),
}

impl Kind {
    /// The release file this install is replaced from.
    #[must_use]
    pub fn package_name(&self, version: &str) -> Option<String> {
        match self {
            Kind::AppImage(_) => Some(format!("Chewvy-{version}-x86_64.AppImage")),
            Kind::Installer(_) => Some(format!("Chewvy-{version}-windows-x86_64-setup.exe")),
            Kind::Portable(_) => Some(format!("Chewvy-{version}-windows-x86_64.zip")),
            Kind::Bundle(_) => Some(format!("Chewvy-{version}-macos-arm64.dmg")),
            Kind::Unmanaged(_) => None,
        }
    }
}

/// Where the updater is, for the screen to draw.
#[derive(Clone, Debug, PartialEq)]
pub enum State {
    /// Nothing asked yet.
    Idle,
    /// Asking GitHub for the latest release.
    Checking,
    /// The latest release is this build or older.
    Current,
    /// A newer release this install can take.
    Available(Release),
    /// A newer release this install cannot take from here, and why.
    Manual(Release, String),
    /// Downloading the package: bytes so far, of the size the release lists.
    Downloading {
        /// The release being installed.
        release: Release,
        /// Bytes downloaded so far.
        done: u64,
        /// The package's size.
        total: u64,
    },
    /// Putting the verified download in place.
    Installing {
        /// The release being installed.
        release: Release,
        /// What the install is doing, for the screen.
        step: &'static str,
        /// How far through that step, where it can say.
        fraction: Option<f32>,
    },
    /// Installed: the next start is the new version.
    Ready(Release),
    /// A check or an install failed, and why.
    Failed(String),
}

/// What a background job reports.
enum Event {
    Checked(Result<Option<Release>, String>),
    Progress(u64),
    Installing(&'static str, Option<f32>),
    Installed(Result<(), String>),
}

/// The app's one updater: the state the screen draws, and the jobs it runs
/// off the UI thread.
pub struct Updater {
    kind: Kind,
    state: State,
    /// The newer release the last check found, kept through a failed install
    /// so it can be tried again without another check.
    offered: Option<Release>,
    tx: Sender<Event>,
    rx: Receiver<Event>,
}

impl Default for Updater {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            kind: install::detect(),
            state: State::Idle,
            offered: None,
            tx,
            rx,
        }
    }
}

impl Updater {
    /// The state as of the jobs' last reports.
    pub fn state(&mut self) -> &State {
        while let Ok(event) = self.rx.try_recv() {
            self.apply(event);
        }
        &self.state
    }

    /// Whether a job is running, so another is not started under it.
    #[must_use]
    pub fn busy(&self) -> bool {
        matches!(
            self.state,
            State::Checking | State::Downloading { .. } | State::Installing { .. }
        )
    }

    /// The newer release the last check found, if any.
    #[must_use]
    pub fn offered(&self) -> Option<&Release> {
        self.offered.as_ref()
    }

    fn apply(&mut self, event: Event) {
        self.state = match (event, std::mem::replace(&mut self.state, State::Idle)) {
            (Event::Checked(Ok(None)), _) => {
                self.offered = None;
                State::Current
            }
            (Event::Checked(Ok(Some(release))), _) => {
                self.offered = Some(release.clone());
                self.offer(release)
            }
            (Event::Checked(Err(why)), _) => State::Failed(why),
            (Event::Progress(done), State::Downloading { release, total, .. }) => {
                State::Downloading {
                    release,
                    done,
                    total,
                }
            }
            (
                Event::Installing(step, fraction),
                State::Downloading { release, .. } | State::Installing { release, .. },
            ) => State::Installing {
                release,
                step,
                fraction,
            },
            (Event::Installed(Ok(())), State::Installing { release, .. }) => State::Ready(release),
            (Event::Installed(Err(why)), _) => State::Failed(why),
            (_, state) => state,
        };
    }

    /// A newer release, as this install can take it.
    fn offer(&self, release: Release) -> State {
        if let Kind::Unmanaged(why) = &self.kind {
            return State::Manual(release, (*why).to_owned());
        }
        match (&release.package, &release.sums) {
            (Some(_), Some(_)) => State::Available(release),
            (None, _) => {
                let why = "This release has no package for this system yet.".to_owned();
                State::Manual(release, why)
            }
            (Some(_), None) => {
                let why = "This release lists no checksums, so it cannot be verified.".to_owned();
                State::Manual(release, why)
            }
        }
    }

    /// Ask GitHub for the latest release, in the background.
    pub fn check(&mut self) {
        if self.busy() {
            return;
        }
        self.state = State::Checking;
        let tx = self.tx.clone();
        let kind = self.kind.clone();
        spawn("update-check", move || {
            let _ = tx.send(Event::Checked(latest(&kind, CURRENT)));
            crate::watch::wake();
        });
    }

    /// Download, verify and install the release on offer, in the background.
    pub fn install(&mut self) {
        let State::Available(release) = &self.state else {
            return;
        };
        let (Some(package), Some(sums)) = (release.package.clone(), release.sums.clone()) else {
            return;
        };
        let release = release.clone();
        self.state = State::Downloading {
            release,
            done: 0,
            total: package.size,
        };
        let tx = self.tx.clone();
        let kind = self.kind.clone();
        spawn("update-install", move || {
            let progress = |done| {
                let _ = tx.send(Event::Progress(done));
                crate::watch::wake();
            };
            let stage = |step, fraction| {
                let _ = tx.send(Event::Installing(step, fraction));
                crate::watch::wake();
            };
            let result = install::run(&kind, &package, &sums, &progress, &stage);
            if let Err(why) = &result {
                log::warn!("update: {why}");
            }
            let _ = tx.send(Event::Installed(result));
            crate::watch::wake();
        });
    }

    /// Install the last offer again after an install failed.
    pub fn retry(&mut self) {
        if self.busy() {
            return;
        }
        match self.offered.clone() {
            Some(release) => {
                self.state = self.offer(release);
                self.install();
            }
            None => self.check(),
        }
    }

    /// Start the new version once this window has closed.
    pub fn restart(&self) {
        if matches!(self.state, State::Ready(_)) {
            if let Ok(mut pending) = RELAUNCH.lock() {
                *pending = Some(self.kind.clone());
            }
        }
    }

    /// The page of the release on offer, for an install that cannot take it.
    #[must_use]
    pub fn release_page(version: &str) -> String {
        format!("{REPOSITORY}/releases/tag/v{version}")
    }
}

/// The install the closing app hands over to, set by [`Updater::restart`].
static RELAUNCH: Mutex<Option<Kind>> = Mutex::new(None);

/// Start the installed update, when the user asked for a restart. Called once
/// the window has closed, so the old process no longer holds its files.
pub fn relaunch() {
    let kind = RELAUNCH.lock().ok().and_then(|mut pending| pending.take());
    if let Some(kind) = kind {
        if let Err(why) = install::relaunch(&kind) {
            log::warn!("update: could not start the new version: {why}");
        }
    }
}

/// Remove what an earlier update left behind (Windows keeps replaced files
/// until the process that held them has gone).
pub fn tidy() {
    install::tidy(&install::detect());
}

fn spawn(name: &str, job: impl FnOnce() + Send + 'static) {
    if let Err(e) = std::thread::Builder::new().name(name.into()).spawn(job) {
        log::warn!("update: could not start {name}: {e}");
    }
}

/// The release list's answer, as far as an update needs it.
#[derive(Deserialize)]
struct Wire {
    tag_name: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<WireAsset>,
}

#[derive(Deserialize)]
struct WireAsset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    size: u64,
}

/// The newer release in a `releases/latest` answer, or `None` when it is not
/// newer than `current`.
///
/// # Errors
///
/// When the answer is not a release.
pub fn parse_latest(json: &[u8], current: &str, kind: &Kind) -> Result<Option<Release>, String> {
    let wire: Wire =
        serde_json::from_slice(json).map_err(|_| "GitHub's answer was not a release".to_owned())?;
    let (Some(latest), Some(current)) = (Version::parse(&wire.tag_name), Version::parse(current))
    else {
        return Err(format!("could not read the version {}", wire.tag_name));
    };
    if wire.draft || wire.prerelease || latest <= current {
        return Ok(None);
    }
    let version = wire.tag_name.trim_start_matches('v').to_owned();
    let find = |name: &str| {
        wire.assets
            .iter()
            .find(|asset| asset.name == name)
            .map(|asset| Asset {
                name: asset.name.clone(),
                url: asset.browser_download_url.clone(),
                size: asset.size,
            })
    };
    Ok(Some(Release {
        package: kind.package_name(&version).and_then(|name| find(&name)),
        sums: find("SHA256SUMS"),
        notes: wire.body.unwrap_or_default().trim().to_owned(),
        version,
    }))
}

/// The SHA-256 a `SHA256SUMS` file lists for `name`, in lowercase hex.
#[must_use]
pub fn listed_digest(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (digest, file) = line.split_once(char::is_whitespace)?;
        // `sha256sum` marks a binary-mode entry with `*`.
        let file = file.trim_start().trim_start_matches('*').trim_end();
        let hex = digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit());
        (hex && file == name).then(|| digest.to_ascii_lowercase())
    })
}

/// `owner/repo` of [`REPOSITORY`].
fn slug() -> &'static str {
    REPOSITORY
        .trim_end_matches('/')
        .trim_start_matches("https://github.com/")
}

/// The HTTP client the updater uses: https only, so a redirect to plain http
/// is refused, with GitHub's redirects to its download host followed.
fn agent(overall: Option<Duration>) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_global(overall)
        .https_only(true)
        .user_agent(format!("Chewvy/{CURRENT}"))
        .tls_config(ureq::tls::TlsConfig::builder().build())
        .build()
        .into()
}

/// The latest published release, when it is newer than `current`.
fn latest(kind: &Kind, current: &str) -> Result<Option<Release>, String> {
    use std::io::Read as _;
    let url = format!("https://api.github.com/repos/{}/releases/latest", slug());
    let mut answer = agent(Some(Duration::from_secs(30)))
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| match e {
            ureq::Error::StatusCode(404) => "No release has been published yet".to_owned(),
            ureq::Error::StatusCode(code) => format!("GitHub answered {code}"),
            _ => "Could not reach GitHub".to_owned(),
        })?;
    let mut json = Vec::new();
    answer
        .body_mut()
        .with_config()
        .limit(MAX_JSON_BYTES)
        .reader()
        .read_to_end(&mut json)
        .map_err(|_| "GitHub's answer was cut short".to_owned())?;
    parse_latest(&json, current, kind)
}

#[cfg(test)]
#[path = "update/tests.rs"]
mod tests;
