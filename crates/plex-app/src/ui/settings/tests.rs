//! The settings file: what the app starts on, that a written file reads back,
//! and the mode it is written with.

use std::path::PathBuf;

/// The POSIX mode bits are what "private" means here; other hosts have no mode
/// bits to assert, so these checks stay Unix like the modes themselves.
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;

use super::*;

/// A config dir of its own for one test, removed when the test ends.
struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("plex-settings-{name}"));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap_or_else(|e| panic!("settings test io: {e}"));
        Self(path)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn the_app_starts_with_progress_sync_off_and_medium_posters() {
    let settings = Settings::default();
    assert!(!settings.progress_sync);
    assert!(settings.hardware_decoding);
    assert_eq!(settings.poster_size, PosterSize::Medium);
    assert_eq!(
        PosterSize::ALL,
        [PosterSize::Small, PosterSize::Medium, PosterSize::Large]
    );
    assert_eq!(PosterSize::Small.label(), "Small");
}

#[test]
fn a_written_settings_file_reads_back_with_its_modes() {
    let dir = Dir::new("round-trip");
    let wanted = Settings {
        progress_sync: true,
        hardware_decoding: false,
        poster_size: PosterSize::Large,
        hdr_auto: true,
        hdr_fullscreen: false,
        hdr_explained: true,
        check_updates: true,
    };
    save_to(&dir.0, wanted).unwrap_or_else(|e| panic!("settings test io: {e}"));
    #[cfg(unix)]
    {
        let path = dir.0.join(FILE);
        assert_eq!(
            std::fs::metadata(&path)
                .unwrap_or_else(|e| panic!("settings test io: {e}"))
                .permissions()
                .mode()
                & 0o777,
            0o600,
            "{path:?} is readable by others"
        );
    }
    assert_eq!(load_from(&dir.0), wanted);
}

#[test]
fn a_second_write_replaces_what_the_first_wrote() {
    let dir = Dir::new("rewrite");
    save_to(&dir.0, Settings::default()).unwrap_or_else(|e| panic!("settings test io: {e}"));
    save_to(
        &dir.0,
        Settings {
            poster_size: PosterSize::Small,
            ..Settings::default()
        },
    )
    .unwrap_or_else(|e| panic!("settings test io: {e}"));
    assert_eq!(load_from(&dir.0).poster_size, PosterSize::Small);
    let text = std::fs::read_to_string(dir.0.join(FILE))
        .unwrap_or_else(|e| panic!("settings test io: {e}"));
    assert_eq!(text.matches("poster_size").count(), 1, "{text}");
}

#[test]
fn a_file_naming_only_one_field_leaves_the_others_at_their_defaults() {
    let dir = Dir::new("partial");
    std::fs::write(dir.0.join(FILE), r#"{"poster_size":"small"}"#)
        .unwrap_or_else(|e| panic!("settings test io: {e}"));
    let settings = load_from(&dir.0);
    assert_eq!(settings.poster_size, PosterSize::Small);
    assert!(!settings.progress_sync);
    assert!(settings.hardware_decoding);
}

#[test]
fn a_missing_or_unreadable_file_gives_the_defaults() {
    let dir = Dir::new("missing");
    assert_eq!(load_from(&dir.0), Settings::default());
    std::fs::write(dir.0.join(FILE), "not json")
        .unwrap_or_else(|e| panic!("settings test io: {e}"));
    assert_eq!(load_from(&dir.0), Settings::default());
}

#[test]
fn a_settings_file_written_with_no_config_dir_creates_it_0700() {
    let parent = std::env::temp_dir().join("plex-settings-nested");
    let _ = std::fs::remove_dir_all(&parent);
    let dir = parent.join("config");
    save_to(&dir, Settings::default()).unwrap_or_else(|e| panic!("settings test io: {e}"));
    #[cfg(unix)]
    {
        assert_eq!(
            std::fs::metadata(&dir)
                .unwrap_or_else(|e| panic!("settings test io: {e}"))
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(dir.join(FILE))
                .unwrap_or_else(|e| panic!("settings test io: {e}"))
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    std::fs::remove_dir_all(&parent).unwrap_or_else(|e| panic!("settings test io: {e}"));
}
