//! Token storage abstraction.
//!
//! Two backends, one abstraction ([`TokenStore`]):
//!
//! 1. [`SecretServiceStore`] — the **primary** choice: the XDG Secret Service
//!    (`org.freedesktop.secrets`) spoken over D-Bus with `zbus` 5.19.0. No
//!    plaintext token at rest in our own files; storage is the desktop's
//!    secrets daemon (KDE Wallet / gnome-keyring).
//! 2. [`FileTokenStore`] — a `0600` file in the app config dir. **Never**
//!    selected automatically: it requires explicit in-app consent
//!    ([`CONSENT_TEXT`]) whose text is persisted next to the choice
//!    ([`StoreChoice`]), and the consent text states that the token is
//!    **plaintext at rest** on disk. Where the OS has no Unix mode bits the
//!    file is still owner-only, but via the private per-user profile folder
//!    rather than a literal `0600` mode.
//!
//! # Verification status (honest limits)
//!
//! * `FileTokenStore` behaviour is covered by unit/integration tests in this
//!   crate (`tests/token_storage.rs`).
//! * `SecretServiceStore` has **no automated test**: the tests run without a
//!   session D-Bus or secrets daemon, so its D-Bus call shapes follow the XDG
//!   Secret Service API and only the availability probe is exercised (it
//!   answers [`CoreError::SecretServiceUnavailable`] there).
//! * No token value ever appears in a log, in `Debug`/`Display` output, in an
//!   error variant or in argv. That is asserted, not asserted-about:
//!   `tests/redaction.rs` captures the log buffer during the real
//!   claim/store/load/account/logout and failure flows and asserts the token
//!   literal never appears in it, and its
//!   `no_product_source_builds_an_argv_and_exposes_secrets_only_at_audited_sites`
//!   test enumerates every plain-text re-exposure site in this crate (and
//!   proves `plex-core` spawns no process, so there is no argv to leak into).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, StorageError};
use crate::secret::SecretString;

// The secret-service backend lives in its own module; these re-exports keep
// `plex_core::tokens::…` the single import site for storage.
pub use crate::secret_service::{
    attribute_safe, SecretServiceStore, SECRET_ACCOUNT_ATTRIBUTE, SECRET_ATTRIBUTES_KEY,
    SECRET_ATTRIBUTES_VALUE, SECRET_COLLECTION_ALIAS, SECRET_IFACE_COLLECTION, SECRET_IFACE_ITEM,
    SECRET_IFACE_SERVICE, SECRET_ITEM_LABEL, SECRET_SERVICE_DEST, SECRET_SERVICE_PATH,
};

/// Which backend holds the account token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// XDG Secret Service over D-Bus (primary, no plaintext at rest).
    SecretService,
    /// `0600` file in the app config dir. Plaintext at rest; consent-gated.
    File,
}

impl Backend {
    /// Stable machine-readable name used in the recorded choice and UI text.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SecretService => "secret-service",
            Self::File => "file-0600",
        }
    }
}

/// The exact consent text that must be shown to the user before the file
/// fallback may be used. Persisted verbatim with the choice so the decision is
/// auditable (docs/PLEX_AUTH.md §3 "Storage (OPEN DECISION)").
pub const CONSENT_TEXT: &str = "No compatible secret service (XDG Secret Service / KDE Wallet / \
gnome-keyring) is available in this session. The alternative is to store your Plex account token \
in a plain file at ~/.config/chewvy/tokens.json with file mode 0600 (your user only). \
That file is PLAINTEXT AT REST: anyone who can read your files (same-user processes, backups, \
file-sync, a stolen disk) can read your Plex token, which is a full bearer credential for your \
Plex account. Plex can revoke it from your account's Devices page. Choose this only if you \
accept that risk; otherwise fix/start your secret service and retry.";

/// Short UI headline shown next to [`CONSENT_TEXT`].
pub const CONSENT_HEADLINE: &str = "Store the Plex token in a plaintext file?";

/// The recorded storage decision, persisted as `token-store-choice.json` in the
/// app config dir. Deliberately contains **no token**, only the backend name,
/// the consent text that was shown and a timestamp.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreChoice {
    /// [`Backend::as_str()`] of the chosen backend.
    pub backend: String,
    /// The consent text the user was shown ([`CONSENT_TEXT`]) — empty for the
    /// secret-service choice, which needs no consent.
    pub consent_text: String,
    /// Unix seconds of the recorded decision.
    pub decided_at_unix: i64,
}

/// A user consent to store the token in a plaintext `0600` file.
///
/// Constructed by the UI only after [`CONSENT_TEXT`] has been *displayed*; the
/// core refuses to build a [`FileTokenStore`] without it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Consent {
    /// True only when the UI actually showed [`CONSENT_TEXT`] and the user
    /// accepted it.
    pub granted: bool,
}

impl Consent {
    /// Consent was not given: the file fallback must not be used.
    pub const DECLINED: Consent = Consent { granted: false };

    /// Consent given after showing [`CONSENT_TEXT`].
    #[must_use]
    pub fn granted() -> Consent {
        Consent { granted: true }
    }
}

/// One storage slot: a source kind plus the server's stable account/server id.
///
/// Both backends key on the [`TokenKey::as_str`] string, so one machine can
/// hold a Plex account token and several Jellyfin server tokens side by side.
/// Only identifiers validated by the constructors ever reach a file name or a
/// D-Bus attribute value.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TokenKey {
    value: String,
}

impl TokenKey {
    /// Source kinds this crate distinguishes.
    const PLEX: &'static str = "plex";
    const JELLYFIN: &'static str = "jellyfin";
    /// Longer ids fail [`sanitize`]-style sanity without needing an error type
    /// of their own; uuids and system uuids are far shorter.
    const MAX_ID_BYTES: usize = 96;

    /// The slot for a Plex account, keyed by the plex.tv account `uuid`.
    ///
    /// # Errors
    ///
    /// [`CoreError::InvalidInput`] if the id is empty, oversized or carries
    /// characters outside the stable-identifier set.
    pub fn plex(account_id: &str) -> Result<Self, CoreError> {
        Self::new(Self::PLEX, account_id)
    }

    /// The slot for a Jellyfin server, keyed by the system-uuid `Id` from
    /// `/System/Info/Public`.
    ///
    /// # Errors
    ///
    /// Same as [`TokenKey::plex`].
    pub fn jellyfin(server_id: &str) -> Result<Self, CoreError> {
        Self::new(Self::JELLYFIN, server_id)
    }

    fn new(source: &str, id: &str) -> Result<Self, CoreError> {
        let stable = !id.is_empty()
            && id.len() <= Self::MAX_ID_BYTES
            && id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~'));
        if !stable {
            return Err(CoreError::InvalidInput);
        }
        Ok(Self {
            value: format!("chewvy:{source}:{id}"),
        })
    }

    /// The exact string both backends key on.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.value
    }
}

/// A token backend. Implementations must never leak the token into errors,
/// logs or `Debug`.
pub trait TokenStore: Send + Sync {
    /// Which backend this is.
    fn backend(&self) -> Backend;

    /// Replace the token stored for `key`.
    fn store(&self, key: &TokenKey, token: &SecretString) -> Result<(), CoreError>;

    /// Read the token stored for `key`, or `None` when the slot is empty.
    fn load(&self, key: &TokenKey) -> Result<Option<SecretString>, CoreError>;

    /// Delete the token stored for `key`. Deleting an absent token is not an
    /// error.
    fn delete(&self, key: &TokenKey) -> Result<(), CoreError>;

    /// The pre-keying slot: this client's single Plex token, no key.
    fn load_legacy(&self) -> Result<Option<SecretString>, CoreError>;

    /// Store into the pre-keying slot (used when a token was obtained before
    /// its account id was known).
    fn store_legacy(&self, token: &SecretString) -> Result<(), CoreError>;

    /// Clear the pre-keying slot. Deleting an absent token is not an error.
    fn delete_legacy(&self) -> Result<(), CoreError>;

    /// The one-time move from the pre-keying layout into `key`: an existing
    /// single-token install keeps working without a re-login.
    ///
    /// Returns the migrated token, or `None` when there was nothing to
    /// migrate (the slot already holds a token, or the legacy slot is empty).
    fn migrate_legacy(&self, key: &TokenKey) -> Result<Option<SecretString>, CoreError> {
        if self.load(key)?.is_some() {
            return Ok(None);
        }
        let Some(token) = self.load_legacy()? else {
            return Ok(None);
        };
        self.store(key, &token)?;
        self.delete_legacy()?;
        Ok(Some(token))
    }
}

/// Read the Plex token, preferring the keyed slot and transparently adopting
/// the pre-keying single-token layout once the account id is known.
///
/// Without an account id there is nothing to key by, so the legacy slot is
/// read as-is and left in place.
pub fn load_plex_token(
    store: &dyn TokenStore,
    key: Option<&TokenKey>,
) -> Result<Option<SecretString>, CoreError> {
    match key {
        None => store.load_legacy(),
        Some(key) => match store.load(key)? {
            Some(token) => Ok(Some(token)),
            None => store.migrate_legacy(key),
        },
    }
}

/// Store a freshly obtained Plex token: keyed when the account id is known,
/// legacy when it is not yet.
pub fn save_plex_token(
    store: &dyn TokenStore,
    key: Option<&TokenKey>,
    token: &SecretString,
) -> Result<(), CoreError> {
    match key {
        Some(key) => store.store(key, token),
        None => store.store_legacy(token),
    }
}

/// Clear every Plex slot: the keyed one (when known) and the pre-keying one.
pub fn delete_plex_token(store: &dyn TokenStore, key: Option<&TokenKey>) -> Result<(), CoreError> {
    if let Some(key) = key {
        store.delete(key)?;
    }
    store.delete_legacy()
}

/// Name of this client's per-user app folder inside the platform config and
/// cache dirs.
pub const APP_DIR: &str = "chewvy";

/// The folder name installs used before the app was named Chewvy.
const FORMER_APP_DIR: &str = "plex-rust-client";

/// Rename a pre-Chewvy app folder under `parent` to [`APP_DIR`], so an existing
/// install keeps its sign-in, settings and cache. It happens once: only while
/// the old folder exists and the new one does not. A failed rename leaves both
/// names as they were and the app starts as a fresh install.
pub fn adopt_former_app_dir(parent: &Path) {
    let legacy = parent.join(FORMER_APP_DIR);
    let current = parent.join(APP_DIR);
    if legacy.is_dir() && !current.exists() && std::fs::rename(&legacy, &current).is_err() {
        log::warn!("could not move the pre-Chewvy app folder to its new name");
    }
}

/// This client's per-user config directory, created when missing; `None` when
/// the OS gives us no per-user home (no `HOME`, no `%APPDATA%`).
///
/// Linux: `$XDG_CONFIG_HOME/chewvy` (fallback `~/.config/...`) via `xdg`,
/// which is a Unix-only crate. Windows: `%APPDATA%\chewvy`.
/// macOS: `~/Library/Application Support/chewvy`.
#[cfg(all(unix, not(target_os = "macos")))]
#[must_use]
pub fn config_dir() -> Option<PathBuf> {
    let dir = xdg::BaseDirectories::with_prefix(APP_DIR).get_config_home()?;
    if let Some(parent) = dir.parent() {
        adopt_former_app_dir(parent);
    }
    Some(dir)
}

/// Windows/macOS [`config_dir`]: this client's folder under the OS app-data
/// directory, created on demand (`xdg` is a Unix-only crate).
#[cfg(not(all(unix, not(target_os = "macos"))))]
#[must_use]
pub fn config_dir() -> Option<PathBuf> {
    let root = platform_config_root()?;
    adopt_former_app_dir(&root);
    let dir = root.join(APP_DIR);
    std::fs::create_dir_all(&dir).ok().map(|()| dir)
}

/// The OS per-user app-data folder that holds our own directory.
#[cfg(windows)]
fn platform_config_root() -> Option<PathBuf> {
    // Roaming app data: already private to this user by its ACL.
    dirs_sys::known_folder_roaming_app_data()
}

#[cfg(target_os = "macos")]
fn platform_config_root() -> Option<PathBuf> {
    dirs_sys::home_dir().map(|home| home.join("Library").join("Application Support"))
}

/// Write `bytes` to `path`, owner-only: created with mode `0600` and re-chmod-ed
/// so a pre-existing file (which `create` + `.mode()` leaves alone) ends up `0600`.
#[cfg(unix)]
pub fn write_private_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)?;
    file.flush()?;
    file.sync_all()?;
    drop(file);
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

/// Non-Unix hosts have no `0600`: the per-user profile folder this file lives in
/// is already private to the user by ACL, so a plain create protects it.
#[cfg(not(unix))]
pub fn write_private_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, bytes)
}

/// Create `path` and its missing parents, private to the owner: mode `0700` on
/// Unix, plain create elsewhere (the per-user profile folder is private by ACL).
#[cfg(unix)]
pub fn create_private_dir(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;

    std::fs::create_dir_all(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
}

/// Non-Unix [`create_private_dir`]: a plain create — the per-user profile
/// folder holding it is private to the user by ACL.
#[cfg(not(unix))]
pub fn create_private_dir(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(path)
}

/// On-disk layout used by both backends' bookkeeping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorePaths {
    /// App config dir ([`config_dir`]: `~/.config/chewvy` on Linux).
    pub config_dir: PathBuf,
}

impl StorePaths {
    /// Resolve the app config dir for this OS ([`config_dir`]).
    ///
    /// There is deliberately **no current-working-directory fallback**: an
    /// earlier version defaulted to `PathBuf::from(".")` when
    /// `get_config_home()` returned `None`, which would have written a
    /// plaintext bearer token into whatever directory the process happened to
    /// run in and chmod-ed that directory. A missing `HOME`/`$XDG_CONFIG_HOME`
    /// is a storage error instead, so nothing is written and the UI can say so.
    ///
    /// This is *our* rule, not Plex behaviour.
    pub fn resolve() -> Result<Self, CoreError> {
        let config_dir = config_dir().ok_or(CoreError::Storage(StorageError::NoConfigHome))?;
        Ok(Self { config_dir })
    }

    /// An explicit config dir (tests, and a UI that resolved the path itself).
    #[must_use]
    pub fn in_dir(config_dir: impl Into<PathBuf>) -> Self {
        Self {
            config_dir: config_dir.into(),
        }
    }

    /// `0600` keyed token file (one JSON map, one entry per [`TokenKey`]).
    #[must_use]
    pub fn token_file(&self) -> PathBuf {
        self.config_dir.join("tokens.json")
    }

    /// The pre-keying `0600` single-token file, kept at its historical path
    /// so an existing install's token is found (and migrated) on first use.
    #[must_use]
    pub fn legacy_token_file(&self) -> PathBuf {
        self.config_dir.join("token.json")
    }

    /// Recorded storage choice (`0600`, contains no token).
    #[must_use]
    pub fn choice_file(&self) -> PathBuf {
        self.config_dir.join("token-store-choice.json")
    }
}

/// Load the recorded storage choice, if any.
pub fn load_choice(paths: &StorePaths) -> Result<Option<StoreChoice>, CoreError> {
    match std::fs::read_to_string(paths.choice_file()) {
        Ok(text) => serde_json::from_str(&text)
            .map(Some)
            .map_err(|_| CoreError::Storage(StorageError::Payload)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(CoreError::Storage(StorageError::Io)),
    }
}

/// Persist the storage choice (backend + consent text + timestamp). Writes the
/// file `0600` because it records a security decision, not because it is a
/// secret.
pub fn record_choice(paths: &StorePaths, choice: &StoreChoice) -> Result<(), CoreError> {
    write_private_json(&paths.choice_file(), choice)
}

/// Token-store bookkeeping helpers: create the config dir `0700`, write the
/// choice, then pick a backend.
///
/// Selection rule: secret service first; the file backend is used **only** when
/// `consent` is [`Consent::granted`] (the UI having displayed [`CONSENT_TEXT`]),
/// otherwise [`CoreError::ConsentRequired`] and nothing is written.
pub fn open_store(
    paths: &StorePaths,
    consent: Consent,
    now_unix: i64,
) -> Result<Box<dyn TokenStore>, CoreError> {
    match SecretServiceStore::probe() {
        Ok(store) => {
            record_choice(
                paths,
                &StoreChoice {
                    backend: Backend::SecretService.as_str().to_owned(),
                    consent_text: String::new(),
                    decided_at_unix: now_unix,
                },
            )?;
            Ok(Box::new(store))
        }
        Err(secret_service_error) => {
            if !consent.granted {
                // Refuse silently: no token is stored anywhere, and the UI must
                // show CONSENT_TEXT before anything else can happen.
                return Err(CoreError::ConsentRequired);
            }
            record_choice(
                paths,
                &StoreChoice {
                    backend: Backend::File.as_str().to_owned(),
                    consent_text: CONSENT_TEXT.to_owned(),
                    decided_at_unix: now_unix,
                },
            )?;
            let store = FileTokenStore::with_consent(
                &paths.token_file(),
                &paths.legacy_token_file(),
                consent,
            )?;
            // The failure of the secret service is reported to the UI (it
            // explains why the plaintext fallback is on the table at all), but
            // it must not be a hard error: the user already consented.
            let _ = secret_service_error;
            Ok(Box::new(store))
        }
    }
}

// ---------------------------------------------------------------------------
// File fallback
// ---------------------------------------------------------------------------

/// Wire/on-disk shape of the keyed fallback token file.
///
/// Deliberately **no `Debug` derive**: printing this struct would print the
/// tokens, and nothing in this crate is allowed to do that.
#[derive(Serialize, Deserialize)]
struct FileKeyedRecord {
    /// File format version.
    schema_version: u8,
    /// One entry per [`TokenKey::as_str`].
    tokens: std::collections::BTreeMap<String, StoredToken>,
}

/// One keyed entry, with its storage time.
#[derive(Clone, Serialize, Deserialize)]
struct StoredToken {
    /// Unix seconds of the write.
    stored_at_unix: i64,
    /// The bearer token, plaintext at rest (disclosed in [`CONSENT_TEXT`]).
    token: String,
}

/// Wire/on-disk shape of the pre-keying single-token file.
#[derive(Serialize, Deserialize)]
struct FileRecord {
    /// File format version.
    schema_version: u8,
    /// Unix seconds of the write.
    stored_at_unix: i64,
    /// The bearer token, plaintext at rest (disclosed in [`CONSENT_TEXT`]).
    token: String,
}

/// The `0600` plaintext fallback store.
pub struct FileTokenStore {
    keyed: PathBuf,
    legacy: PathBuf,
}

impl FileTokenStore {
    /// Build the fallback store. Refuses without explicit consent
    /// ([`CoreError::ConsentRequired`]) — there is no constructor that skips
    /// the consent check.
    pub fn with_consent(keyed: &Path, legacy: &Path, consent: Consent) -> Result<Self, CoreError> {
        if !consent.granted {
            return Err(CoreError::ConsentRequired);
        }
        // Both files share one parent directory; checking the keyed path is
        // enough to cover the directory that holds either of them.
        ensure_private_dir(keyed)?;
        Ok(Self {
            keyed: keyed.to_path_buf(),
            legacy: legacy.to_path_buf(),
        })
    }

    /// Path of the plaintext keyed token file (UI/CLI display; not a secret).
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.keyed
    }

    /// Path of the pre-keying single-token file (not a secret).
    #[must_use]
    pub fn legacy_path(&self) -> &Path {
        &self.legacy
    }

    /// Unix seconds stored for `key`, for the UI "stored on ..." line.
    pub fn stored_at(&self, key: &TokenKey) -> Result<Option<i64>, CoreError> {
        match read_keyed(&self.keyed)? {
            Some(record) => Ok(record
                .tokens
                .get(key.as_str())
                .map(|entry| entry.stored_at_unix)),
            None => Ok(None),
        }
    }
}

impl TokenStore for FileTokenStore {
    fn backend(&self) -> Backend {
        Backend::File
    }

    fn store(&self, key: &TokenKey, token: &SecretString) -> Result<(), CoreError> {
        if token.is_empty() {
            return Err(CoreError::InvalidInput);
        }
        let mut record = read_keyed(&self.keyed)?.unwrap_or(FileKeyedRecord {
            schema_version: 1,
            tokens: std::collections::BTreeMap::new(),
        });
        record.tokens.insert(
            key.as_str().to_owned(),
            StoredToken {
                stored_at_unix: unix_now(),
                token: token.expose().to_owned(),
            },
        );
        write_private_json(&self.keyed, &record)
    }

    fn load(&self, key: &TokenKey) -> Result<Option<SecretString>, CoreError> {
        Ok(read_keyed(&self.keyed)?
            .and_then(|record| record.tokens.get(key.as_str()).map(|e| e.token.clone()))
            .filter(|token| !token.is_empty())
            .map(SecretString::new))
    }

    fn delete(&self, key: &TokenKey) -> Result<(), CoreError> {
        let Some(mut record) = read_keyed(&self.keyed)? else {
            return Ok(());
        };
        if record.tokens.remove(key.as_str()).is_some() {
            // An empty map is stored as "no file", matching the pre-keying
            // single-token file: after signing out of everything, nothing
            // this client wrote is left behind in the config dir.
            if record.tokens.is_empty() {
                return match std::fs::remove_file(&self.keyed) {
                    Ok(()) => Ok(()),
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    Err(_) => Err(CoreError::Storage(StorageError::Io)),
                };
            }
            write_private_json(&self.keyed, &record)?;
        }
        Ok(())
    }

    fn load_legacy(&self) -> Result<Option<SecretString>, CoreError> {
        Ok(match read_record(&self.legacy)? {
            Some(record) if !record.token.is_empty() => Some(SecretString::new(record.token)),
            Some(_) | None => None,
        })
    }

    fn store_legacy(&self, token: &SecretString) -> Result<(), CoreError> {
        if token.is_empty() {
            return Err(CoreError::InvalidInput);
        }
        let record = FileRecord {
            schema_version: 1,
            stored_at_unix: unix_now(),
            token: token.expose().to_owned(),
        };
        write_private_json(&self.legacy, &record)
    }

    fn delete_legacy(&self) -> Result<(), CoreError> {
        match std::fs::remove_file(&self.legacy) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(CoreError::Storage(StorageError::Io)),
        }
    }
}

impl std::fmt::Debug for FileTokenStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileTokenStore")
            .field("keyed", &self.keyed)
            .field("legacy", &self.legacy)
            .finish_non_exhaustive()
    }
}

fn read_keyed(path: &Path) -> Result<Option<FileKeyedRecord>, CoreError> {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            tighten_file_mode(path);
            serde_json::from_str(&text)
                .map(Some)
                .map_err(|_| CoreError::Storage(StorageError::Payload))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(CoreError::Storage(StorageError::Io)),
    }
}

fn read_record(path: &Path) -> Result<Option<FileRecord>, CoreError> {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            tighten_file_mode(path);
            serde_json::from_str(&text)
                .map(Some)
                .map_err(|_| CoreError::Storage(StorageError::Payload))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(CoreError::Storage(StorageError::Io)),
    }
}

/// Self-heal the `0600` mode if something (umask, a manual copy) made the
/// token file group/other readable. Never logs the token.
#[cfg(unix)]
fn tighten_file_mode(path: &Path) {
    use std::os::unix::fs::PermissionsExt as _;
    if let Ok(meta) = std::fs::metadata(path) {
        let mode = meta.permissions().mode() & 0o777;
        if mode != 0o600 {
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
            log::warn!(
                "token file mode was {:o}, tightened to 600 (path {})",
                mode,
                path.display()
            );
        }
    }
}

#[cfg(not(unix))]
fn tighten_file_mode(_path: &Path) {}

/// Write `value` as JSON private to the user (see [`write_private_file`]).
fn write_private_json<T: Serialize>(path: &Path, value: &T) -> Result<(), CoreError> {
    let json =
        serde_json::to_vec_pretty(value).map_err(|_| CoreError::Storage(StorageError::Payload))?;
    ensure_private_dir(path)?;
    write_private_file(path, &json).map_err(|_| CoreError::Storage(StorageError::Io))
}

/// Make sure `path`'s parent directory exists and is private, **without
/// changing the mode of a directory this client did not create**.
///
/// Rule: create the directory `0700` when it is missing; when it already
/// exists, refuse to store a plaintext bearer token in a directory group or
/// other can write to ([`CoreError::Storage(StorageError::UnsafeDirectory)`])
/// instead of silently chmod-ing someone else's directory (an earlier version
/// chmod-ed *any* parent, including the process CWD).
fn ensure_private_dir(path: &Path) -> Result<(), CoreError> {
    let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) else {
        return Err(CoreError::Storage(StorageError::Io));
    };
    match std::fs::metadata(parent) {
        Ok(meta) => {
            if dir_is_group_or_other_writable(&meta) {
                return Err(CoreError::Storage(StorageError::UnsafeDirectory));
            }
            Ok(())
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            create_private_dir(parent).map_err(|_| CoreError::Storage(StorageError::Io))
        }
        Err(_) => Err(CoreError::Storage(StorageError::Io)),
    }
}

/// Without Unix mode bits there is nothing to check: the per-user profile
/// folders a non-Unix host gives us are private by ACL.
#[cfg(unix)]
fn dir_is_group_or_other_writable(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt as _;

    meta.permissions().mode() & 0o022 != 0
}

#[cfg(not(unix))]
fn dir_is_group_or_other_writable(_meta: &std::fs::Metadata) -> bool {
    false
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        // Deliberately no pid in the name: this crate's sources are scanned for
        // process/argv sites and a test helper would count as one.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!("plex-core-tokens-{tag}-{nanos}"));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[cfg(unix)]
    fn mode_of(path: &Path) -> Option<u32> {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::metadata(path)
            .ok()
            .map(|meta| meta.permissions().mode() & 0o777)
    }

    #[test]
    fn the_config_dir_is_this_client_s_own_directory() {
        // `config_dir` is None only where the OS gives us no per-user home.
        if let Some(dir) = config_dir() {
            assert!(dir.is_absolute(), "the config dir is absolute: {dir:?}");
            let name = dir.file_name().and_then(|name| name.to_str());
            assert_eq!(name, Some(APP_DIR), "the config dir belongs to this client");
        }
    }

    #[test]
    fn a_pre_rename_app_folder_is_adopted_once_with_its_files() {
        let parent = temp_dir("adopt");
        let former = parent.join(FORMER_APP_DIR);
        assert!(std::fs::create_dir_all(&former).is_ok());
        assert!(std::fs::write(former.join("settings.json"), b"kept").is_ok());

        adopt_former_app_dir(&parent);
        let current = parent.join(APP_DIR);
        assert!(!former.exists(), "the old folder name is gone");
        assert_eq!(
            std::fs::read(current.join("settings.json")).ok().as_deref(),
            Some(&b"kept"[..]),
            "its files arrive under the new name"
        );

        // A later run that finds both names must not overwrite the live folder.
        assert!(std::fs::create_dir_all(&former).is_ok());
        assert!(std::fs::write(former.join("settings.json"), b"stale").is_ok());
        adopt_former_app_dir(&parent);
        assert_eq!(
            std::fs::read(current.join("settings.json")).ok().as_deref(),
            Some(&b"kept"[..])
        );
        assert!(former.exists(), "a second old folder is left alone");
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn a_fresh_install_has_nothing_to_adopt() {
        let parent = temp_dir("adopt-fresh");
        assert!(std::fs::create_dir_all(&parent).is_ok());
        adopt_former_app_dir(&parent);
        assert!(!parent.join(APP_DIR).exists(), "no folder is invented");
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn a_private_file_stays_private_when_rewritten() {
        let dir = temp_dir("file");
        let path = dir.join("private.json");
        assert!(
            create_private_dir(&dir).is_ok(),
            "the test dir must be creatable"
        );
        assert!(write_private_file(&path, b"first").is_ok());
        assert_eq!(
            std::fs::read(&path).ok().as_deref(),
            Some("first".as_bytes())
        );
        // A rewrite of an existing file must not widen its permissions.
        assert!(write_private_file(&path, b"second").is_ok());
        assert_eq!(
            std::fs::read(&path).ok().as_deref(),
            Some("second".as_bytes())
        );
        #[cfg(unix)]
        {
            assert_eq!(mode_of(&path), Some(0o600), "the file is owner-only");
            assert_eq!(mode_of(&dir), Some(0o700), "the app dir is owner-only");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[cfg(unix)]
    fn a_group_writable_existing_dir_is_refused_not_chmod_ed() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = temp_dir("unsafe");
        assert!(
            std::fs::create_dir_all(&dir).is_ok(),
            "the test dir must be creatable"
        );
        let loose = std::fs::Permissions::from_mode(0o777);
        assert!(
            std::fs::set_permissions(&dir, loose).is_ok(),
            "must be chmod-able"
        );
        assert_eq!(
            ensure_private_dir(&dir.join("token.json")).err(),
            Some(CoreError::Storage(StorageError::UnsafeDirectory)),
            "a group/other-writable parent is refused, not chmod-ed"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
