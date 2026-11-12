//! The sources file: what it holds, what it must not hold, and how it survives
//! being written and read again.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::*;

/// A throwaway config dir for one test, taken away again when that test ends.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.subsec_nanos());
        let path = std::env::temp_dir().join(format!(
            "chewvy-sources-{name}-{}-{nanos}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a temp config dir");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn server(id: &str, name: &str, added_at: i64) -> JellyfinServer {
    JellyfinServer {
        server_id: id.to_owned(),
        name: name.to_owned(),
        base_url: format!("http://{id}:8096"),
        user_id: Some(format!("user-of-{id}")),
        user_name: Some(id.to_owned()),
        added_at,
    }
}

#[test]
fn a_missing_file_answers_with_an_empty_list() {
    let empty = super::load_from(Path::new("/nonexistent-plex-rust-dir"));
    assert_eq!(empty, Sources::new());
    assert!(!empty.has_sign_in());
}

#[test]
fn a_garbage_or_half_written_file_answers_the_same_way() {
    let temp = TempDir::new("garbage");
    std::fs::write(temp.path().join(FILE), "{\"plex\": {\"account_id\"").expect("write garbage");
    assert_eq!(super::load_from(temp.path()), Sources::new());
}

#[test]
fn a_round_trip_keeps_both_kinds_of_source() -> std::io::Result<()> {
    let temp = TempDir::new("round-trip");
    let mut sources = Sources::new();
    sources.record_plex(Some("account-uuid".to_owned()));
    sources.set_jellyfin(server("server-a", "Living Room", 1_700_000_000));
    sources.set_jellyfin(server("server-b", "Cabin", 1_700_000_600));

    save_to(temp.path(), &sources)?;
    assert_eq!(super::load_from(temp.path()), sources);
    Ok(())
}

#[test]
fn the_file_holds_no_token_and_stays_private() -> std::io::Result<()> {
    let temp = TempDir::new("private");
    let mut sources = Sources::new();
    sources.set_jellyfin(server("server-a", "Living Room", 1_700_000_000));
    let token = plex_core::secret::SecretString::new("shhh-9f3a7c21be");
    let key = jellyfin_token_key(sources.jellyfin.first().expect("the row just added"));

    save_to(temp.path(), &sources)?;
    let text = std::fs::read_to_string(temp.path().join(FILE))?;
    assert!(
        !text.contains(token.expose()),
        "the token belongs in the keyed store, not in {FILE}"
    );
    assert!(text.contains("server-a"), "the id keys that slot: {text}");
    assert!(key.is_some_and(|key| key.as_str().contains("server-a")));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(temp.path().join(FILE))?
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "{FILE} is readable by its owner only");
    }
    Ok(())
}

#[test]
fn re_signing_in_to_a_server_replaces_its_row() {
    let mut sources = Sources::new();
    sources.set_jellyfin(server("server-a", "Living Room", 1_700_000_000));
    let mut renamed = server("server-a", "Den", 1_700_000_900);
    renamed.user_name = Some("someone-else".to_owned());
    sources.set_jellyfin(renamed.clone());

    assert_eq!(sources.jellyfin.len(), 1, "one row per server id");
    assert_eq!(sources.jellyfin("server-a"), Some(&renamed));
}

#[test]
fn removing_a_server_leaves_the_plex_account_alone() {
    let mut sources = Sources::new();
    sources.record_plex(Some("account-uuid".to_owned()));
    sources.set_jellyfin(server("server-a", "Living Room", 1_700_000_000));
    sources.set_jellyfin(server("server-b", "Cabin", 1_700_000_600));

    assert!(sources.remove_jellyfin("server-a").is_some());
    assert!(sources.remove_jellyfin("server-a").is_none());
    assert_eq!(sources.jellyfin.len(), 1);
    assert!(sources.has_sign_in());

    sources.remove_jellyfin("server-b");
    assert!(sources.has_sign_in(), "the Plex account is its own source");
    assert!(sources.remove_jellyfin("never added").is_none());
}

#[test]
fn the_plex_slot_follows_the_sign_in() {
    let mut sources = Sources::new();
    assert_eq!(sources.plex, None);

    // A sign-in whose account plex.tv has not named yet still owns the source.
    sources.record_plex(None);
    assert!(sources.has_sign_in());
    let linked_at = sources.plex.clone().map_or(0, |held| held.added_at);

    // Learning the account id re-keys the same source; it does not re-add it.
    sources.record_plex(Some("account-uuid".to_owned()));
    assert_eq!(
        sources.plex.as_ref().map(|held| held.account_id.as_deref()),
        Some(Some("account-uuid")),
        "the row follows the account it is linked to"
    );
    assert!(
        sources
            .plex
            .as_ref()
            .is_none_or(|held| held.added_at >= linked_at),
        "a different account is a different sign-in, never an older one"
    );
    let re_recorded = sources.plex.clone();
    sources.record_plex(Some("account-uuid".to_owned()));
    assert_eq!(sources.plex, re_recorded, "the same account keeps its date");

    sources.clear_plex();
    assert!(!sources.has_sign_in());
}

#[test]
fn summary_names_what_is_signed_in() {
    let mut sources = Sources::new();
    assert_eq!(sources.summary(), "Nothing signed in yet");

    sources.record_plex(Some("account-uuid".to_owned()));
    assert_eq!(sources.summary(), "Plex account");

    sources.set_jellyfin(server("server-a", "Living Room", 1_700_000_000));
    assert_eq!(sources.summary(), "Plex account and 1 Jellyfin server");

    sources.clear_plex();
    sources.set_jellyfin(server("server-b", "Cabin", 1_700_000_600));
    assert_eq!(sources.summary(), "2 Jellyfin servers");
}
