//! Shared scaffolding for the sign-in behaviour tests.
//!
//! Not a test target of its own (a directory without `main.rs` is not compiled
//! by cargo); each `tests/*.rs` file pulls it in with `mod common;`. Everything
//! here is test-only: a self-cleaning private temporary directory, and helpers
//! that build the real product types (identity file, `ClientInfo`, `Http`) so a
//! test exercises the same code paths the UI will.
//!
//! `tempfile` is deliberately **not** used: it is not a direct dependency of
//! this workspace and the lock file is `--locked`, so the helper is written
//! here instead of adding a dependency.

#![allow(dead_code)] // each test binary uses a subset

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _; // required for Permissions::from_mode / mode()
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use plex_core::client_info::ClientInfo;
use plex_core::http::Http;

/// Version string handed to `ClientInfo` (mirrors the workspace version).
pub const TEST_VERSION: &str = "0.1.0";

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn unique_name(tag: &str) -> String {
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("plex-core-{tag}-{}-{id}", nanos % 1_000_000_000)
}

/// A private (`0700`) temporary directory removed when the value is dropped.
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    /// Create one under the system temp dir.
    pub fn new(tag: &str) -> Self {
        Self::with_name(&unique_name(tag))
    }

    /// Create one with an explicit name (for tests that need a known path).
    pub fn with_name(name: &str) -> Self {
        let path = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path)
            .unwrap_or_else(|e| panic!("test temp dir {} must be creatable: {e}", path.display()));
        #[cfg(unix)]
        {
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
                .unwrap_or_else(|e| panic!("test temp dir must be 0700: {e}"));
            let mode = std::fs::metadata(&path)
                .unwrap_or_else(|e| panic!("test temp dir must be stat-able: {e}"))
                .permissions()
                .mode();
            assert_eq!(
                mode & 0o777,
                0o700,
                "test temp dir must be private before a token is written into it"
            );
        }
        Self { path }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// A path inside this directory (parent exists, so stores can write there).
    #[must_use]
    pub fn join(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }

    /// Create a subdirectory with an explicit mode (for unsafe-directory cases).
    pub fn subdir(&self, name: &str, mode: u32) -> PathBuf {
        let path = self.path.join(name);
        std::fs::create_dir_all(&path)
            .unwrap_or_else(|e| panic!("test subdir must be creatable: {e}"));
        #[cfg(unix)]
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode))
            .unwrap_or_else(|e| panic!("test subdir mode must be settable: {e}"));
        #[cfg(not(unix))]
        let _ = mode;
        path
    }

    /// Mode of a path (`0` when it does not exist).
    #[cfg(unix)]
    #[must_use]
    pub fn mode_of(&self, path: &Path) -> u32 {
        std::fs::metadata(path)
            .map(|meta| meta.permissions().mode() & 0o777)
            .unwrap_or(0)
    }

    /// Mode of a path: `0` everywhere on a host without Unix mode bits.
    #[cfg(not(unix))]
    #[must_use]
    pub fn mode_of(&self, _path: &Path) -> u32 {
        0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Load (creating on first use) the real per-install identity file.
pub fn client(dir: &TempDir) -> ClientInfo {
    ClientInfo::load_or_create(&dir.join("identity.json"), TEST_VERSION)
        .expect("client identity must load or be created")
}

/// A transport pointed at `base` (usually the mock Plex server) with the
/// product identity headers.
pub fn http_at(base: &str, client: &ClientInfo) -> Http {
    Http::new(base, client).expect("test transport must build")
}
