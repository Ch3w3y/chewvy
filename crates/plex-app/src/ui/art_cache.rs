//! The artwork disk cache: finished `/photo/:/transcode` answers written under
//! `$XDG_CACHE_HOME/chewvy/art` as `<key>.img`, bounded in total size
//! and age so a long-lived browse session cannot grow the cache forever.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use plex_core::tokens::{adopt_former_app_dir, APP_DIR};

/// Disk-cache subdirectory under `$XDG_CACHE_HOME/chewvy`.
pub(crate) const CACHE_SUBDIR: &str = "art";
/// Total bytes of cached artwork held before the oldest answers are pruned.
pub(crate) const MAX_BYTES: u64 = 256 * 1024 * 1024;
/// Cached answers older than this are dropped when the next answer is written.
pub(crate) const MAX_AGE: Duration = Duration::from_secs(30 * 24 * 60 * 60);
/// Suffix of one cache slot.
const SUFFIX: &str = ".img";

/// The cache slots of one artwork loader, `None` when the run has no cache dir.
pub(crate) struct DiskCache {
    dir: Option<PathBuf>,
}

impl DiskCache {
    /// Cache slots under `dir` (`None` disables the disk cache).
    pub(crate) fn new(dir: Option<PathBuf>) -> Self {
        Self { dir }
    }

    /// The bytes of a finished answer stored under `key`, if any.
    pub(crate) fn read(&self, key: &str) -> Option<Vec<u8>> {
        self.dir
            .as_ref()
            .and_then(|dir| std::fs::read(dir.join(file_name(key))).ok())
    }

    /// Store one answer, creating the directory, then keep the cache inside its
    /// size and age bounds. A failed write is not an artwork failure.
    pub(crate) fn write(&self, key: &str, bytes: &[u8]) {
        let Some(dir) = self.dir.as_ref() else {
            return;
        };
        if std::fs::create_dir_all(dir).is_err()
            || std::fs::write(dir.join(file_name(key)), bytes).is_err()
        {
            log::debug!("artwork cache write failed (the cache stays a cache)");
            return;
        }
        prune(dir, MAX_BYTES, SystemTime::now());
    }

    /// Total bytes of cache slots under `dir` (test view).
    #[cfg(test)]
    pub(crate) fn total(&self) -> u64 {
        self.dir
            .as_ref()
            .and_then(|dir| std::fs::read_dir(dir).ok())
            .map(|entries| {
                entries
                    .filter_map(|entry| entry.ok())
                    .filter(|entry| entry.file_name().to_string_lossy().ends_with(SUFFIX))
                    .filter_map(|entry| entry.metadata().ok())
                    .filter(|meta| meta.is_file())
                    .map(|meta| meta.len())
                    .sum()
            })
            .unwrap_or(0)
    }
}

/// The cache directory one app run shares, created when the OS gives us one.
///
/// Linux follows the XDG specification through `xdg` (a Unix-only crate); the
/// other desktops use their documented per-user cache root - `%LOCALAPPDATA%` on
/// Windows, `~/Library/Caches` on macOS - because that is where the OS expects
/// an application's own cache to live.
#[cfg(all(unix, not(target_os = "macos")))]
#[must_use]
pub(crate) fn env_dir() -> Option<PathBuf> {
    let dirs = xdg::BaseDirectories::with_prefix(APP_DIR);
    if let Some(parent) = dirs.get_cache_home().as_deref().and_then(Path::parent) {
        adopt_former_app_dir(parent);
    }
    dirs.create_cache_directory(CACHE_SUBDIR).ok()
}

/// Windows [`env_dir`].
#[cfg(not(unix))]
#[must_use]
pub(crate) fn env_dir() -> Option<PathBuf> {
    let root = std::env::var_os("LOCALAPPDATA")?;
    owned_cache_dir(Path::new(&root))
}

/// macOS [`env_dir`].
#[cfg(target_os = "macos")]
#[must_use]
pub(crate) fn env_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    owned_cache_dir(Path::new(&home).join("Library/Caches").as_path())
}

/// `root/chewvy/art`, created on demand; `None` without a `root`.
#[cfg(not(all(unix, not(target_os = "macos"))))]
#[must_use]
fn owned_cache_dir(root: &Path) -> Option<PathBuf> {
    adopt_former_app_dir(root);
    let dir = root.join(APP_DIR).join(CACHE_SUBDIR);
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// The cache slot name for one artwork key.
#[must_use]
pub(crate) fn file_name(key: &str) -> String {
    format!("{key}{SUFFIX}")
}

/// Drop answers older than [`MAX_AGE`], then the oldest answers until the
/// directory holds at most `budget` bytes. Returns the bytes freed.
///
/// Modification time is the usage record: writing an answer refreshes it, and
/// the answers a browse session asked for last are the ones kept.
pub(crate) fn prune(dir: &Path, budget: u64, now: SystemTime) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut slots: Vec<(PathBuf, u64, SystemTime)> = Vec::new();
    for entry in entries.filter_map(|entry| entry.ok()) {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if !name.ends_with(SUFFIX) {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        slots.push((entry.path(), meta.len(), modified));
    }
    let mut freed = 0u64;
    let mut live: Vec<(PathBuf, u64, SystemTime)> = Vec::new();
    for (path, len, modified) in slots {
        if now.duration_since(modified).unwrap_or_default() > MAX_AGE {
            if std::fs::remove_file(&path).is_ok() {
                freed += len;
            }
        } else {
            live.push((path, len, modified));
        }
    }
    let mut total: u64 = live.iter().map(|(_, len, _)| *len).sum();
    if total <= budget {
        return freed;
    }
    live.sort_by_key(|(_, _, modified)| *modified);
    for (path, len, _) in live {
        if total <= budget {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total -= len;
            freed += len;
        }
    }
    freed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("chewvy-cache-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            std::fs::create_dir_all(&dir).is_ok(),
            "the test cache directory must be creatable"
        );
        dir
    }

    fn touch(dir: &Path, key: &str, len: usize, age: Duration, now: SystemTime) {
        let path = dir.join(file_name(key));
        assert!(
            std::fs::write(&path, vec![7u8; len]).is_ok(),
            "a cache slot must be writable"
        );
        let modified = now - age;
        let opened = std::fs::File::options().write(true).open(&path);
        assert!(opened.is_ok(), "a slot must be openable");
        if let Ok(file) = opened {
            assert!(
                file.set_times(std::fs::FileTimes::new().set_modified(modified))
                    .is_ok(),
                "a slot mtime must be settable"
            );
        }
    }

    #[test]
    fn a_write_creates_the_directory_it_writes_into() {
        let dir = temp_dir("write").join(CACHE_SUBDIR);
        let cache = DiskCache::new(Some(dir.clone()));
        cache.write("probe", b"not-an-image");
        assert!(dir.is_dir(), "the cache directory must be created");
        assert_eq!(cache.read("probe"), Some(b"not-an-image".to_vec()));
        assert_eq!(cache.read("absent"), None);
    }

    #[test]
    fn pruning_drops_the_oldest_slots_until_the_cap_holds() {
        let now = SystemTime::now();
        let dir = temp_dir("prune");
        for (index, age) in [1u64, 2, 3, 4, 5].into_iter().enumerate() {
            let age = Duration::from_secs(age * 60);
            touch(&dir, &format!("k{index}"), 1024, age, now);
        }
        assert_eq!(
            std::fs::metadata(dir.join(file_name("k4")))
                .map(|meta| meta.len())
                .ok(),
            Some(1024)
        );
        let budget = 2 * 1024;
        let freed = prune(&dir, budget, now);
        assert_eq!(freed, 3 * 1024, "the three oldest answers are pruned");
        assert!(
            !dir.join(file_name("k4")).exists(),
            "the oldest answer is dropped"
        );
        assert!(
            dir.join(file_name("k0")).exists(),
            "the newest answer is kept"
        );
        assert!(DiskCache::new(Some(dir)).total() <= budget);
    }

    #[test]
    fn an_answer_past_the_age_limit_is_dropped_first() {
        let now = SystemTime::now();
        let dir = temp_dir("stale");
        touch(&dir, "old", 1024, MAX_AGE + Duration::from_secs(3600), now);
        touch(&dir, "fresh", 1024, Duration::from_secs(60), now);
        let freed = prune(&dir, 10 * 1024, now);
        assert_eq!(freed, 1024, "only the stale answer is past the age limit");
        assert!(!dir.join(file_name("old")).exists());
        assert!(dir.join(file_name("fresh")).exists());
    }

    #[test]
    fn a_loader_without_a_cache_dir_never_reads_or_writes() {
        let cache = DiskCache::new(None);
        cache.write("k", b"bytes");
        assert_eq!(cache.read("k"), None);
        assert_eq!(cache.total(), 0);
    }
}
