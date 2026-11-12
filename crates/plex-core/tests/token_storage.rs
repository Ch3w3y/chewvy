//! Behaviour tests for the token store: the consent gate, the `0600`/`0700`
//! file rules, the recorded storage choice, and the refusal to touch a
//! directory this client did not create.
//!
//! Every assertion here runs against a real private temporary directory on the
//! real filesystem (`common::TempDir`, mode `0700`, removed on drop); nothing
//! is mocked except the XDG Secret Service, which is not reachable in the
//! offline test environment. The secret-service backend itself stays
//! **compile-verified only** and its live behaviour is an M8 gate.
//!
//! The token literal is `mock::MOCK_TOKEN`, the same distinctive string the
//! redaction test watches for in logs (`tests/redaction.rs`).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)] // product code keeps these lints on

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _; // required for Permissions::from_mode / mode()
use std::path::{Path, PathBuf};

use plex_core::error::{CoreError, StorageError};
use plex_core::secret::SecretString;
use plex_core::tokens::{
    attribute_safe, delete_plex_token, load_choice, load_plex_token, open_store, record_choice,
    save_plex_token, Backend, Consent, FileTokenStore, SecretServiceStore, StoreChoice, StorePaths,
    TokenKey, TokenStore, CONSENT_HEADLINE, CONSENT_TEXT,
};

mod common;
mod mock;

use common::TempDir;
use mock::MOCK_TOKEN;

fn token() -> SecretString {
    SecretString::new(MOCK_TOKEN)
}

/// The Plex slot tests exercise; any well-formed account uuid will do.
fn key() -> TokenKey {
    TokenKey::plex("deadbeef-dead-4ead-beef-deadbeefdead").expect("valid test uuid")
}

/// A second, distinct slot (a Jellyfin server) for coexistence tests.
fn other_key() -> TokenKey {
    TokenKey::jellyfin("1111111111114111a111111111111111").expect("valid test system uuid")
}

/// The pre-keying file that belongs with a given keyed file.
fn legacy_path(path: &std::path::Path) -> PathBuf {
    path.with_file_name("legacy-token.json")
}

/// A private config dir inside the test temp dir (a store may write there).
fn config_dir(dir: &TempDir, tag: &str) -> PathBuf {
    dir.subdir(tag, 0o700)
}

#[test]
fn a_claimed_token_is_stored_and_reloadable_then_deleted() {
    let dir = TempDir::new("store-roundtrip");
    let path = config_dir(&dir, "cfg").join("token.json");
    let store = FileTokenStore::with_consent(&path, &legacy_path(&path), Consent::granted())
        .expect("file store needs consent, which this test gives");

    assert_eq!(store.backend(), Backend::File);
    assert_eq!(store.path(), path);
    assert_eq!(store.path().file_name().unwrap(), "token.json");

    // Nothing stored yet: `load` is `None`, `delete` of an absent token is ok.
    assert!(store.load(&key()).expect("empty store must load").is_none());
    store
        .delete(&key())
        .expect("deleting an absent token is not an error");

    store
        .store(&key(), &token())
        .expect("store must accept a token");
    let reloaded = store
        .load(&key())
        .expect("token must reload")
        .expect("Some(token)");
    assert!(
        reloaded.matches(MOCK_TOKEN),
        "the reloaded token must be the stored one"
    );
    assert_eq!(reloaded.len(), MOCK_TOKEN.len());
    assert!(store
        .stored_at(&key())
        .expect("stored_at must read")
        .is_some());

    // The on-disk file is the plaintext the consent text promised (0600), and
    // it is the only place the value lives.
    #[cfg(unix)]
    {
        let mode = dir.mode_of(&path);
        assert_eq!(
            mode & 0o777,
            0o600,
            "the token file must be 0600, got {mode:o}"
        );
    }
    let on_disk = std::fs::read_to_string(&path).expect("token file must be readable");
    assert!(
        on_disk.contains(MOCK_TOKEN),
        "the fallback store is plaintext at rest, by design and by consent"
    );

    // Overwriting replaces the value instead of appending or splitting files.
    store
        .store(&key(), &SecretString::new("replacement-token"))
        .expect("re-store");
    let again = store.load(&key()).expect("reload").expect("Some(token)");
    assert!(again.matches("replacement-token"));
    assert!(!again.matches(MOCK_TOKEN));

    // Logout-style local deletion: file gone, `load` back to None.
    store
        .delete(&key())
        .expect("delete must remove the token file");
    assert!(!path.exists(), "delete must remove the file, not empty it");
    assert!(store.load(&key()).expect("load after delete").is_none());
    assert_eq!(dir.mode_of(&path), 0, "the file must no longer exist");
}

#[test]
fn an_empty_token_is_refused_and_never_written() {
    let dir = TempDir::new("store-empty");
    let path = config_dir(&dir, "cfg").join("token.json");
    let store = FileTokenStore::with_consent(&path, &legacy_path(&path), Consent::granted())
        .expect("consent given in this test");
    let error = store
        .store(&key(), &SecretString::new(""))
        .expect_err("an empty token must be refused");
    assert_eq!(error, CoreError::InvalidInput);
    assert!(
        !path.exists(),
        "a refused store must not create a token file"
    );
}

#[test]
fn a_missing_parent_directory_is_created_private_0700() {
    let dir = TempDir::new("store-mkdir");
    // Two missing levels: the store must create them and make them private.
    let path = dir.join("deep").join("chewvy").join("token.json");
    assert!(!path.parent().unwrap().exists());

    let store = FileTokenStore::with_consent(&path, &legacy_path(&path), Consent::granted())
        .expect("the store must create its own private parent");
    store
        .store(&key(), &token())
        .expect("store into the created directory");

    #[cfg(unix)]
    {
        let parent_mode = dir.mode_of(path.parent().unwrap());
        assert_eq!(
            parent_mode & 0o777,
            0o700,
            "parent dir must be 0700, got {parent_mode:o}"
        );
        let file_mode = dir.mode_of(&path);
        assert_eq!(
            file_mode & 0o777,
            0o600,
            "token file must be 0600, got {file_mode:o}"
        );
    }
}

#[test]
#[cfg(unix)] // the whole test is about Unix mode bits and who may write a dir
fn a_world_or_group_writable_parent_is_refused_instead_of_chmodded() {
    let dir = TempDir::new("store-unsafe");
    for (tag, mode) in [("world-writable", 0o777u32), ("group-writable", 0o720u32)] {
        let parent = dir.subdir(tag, mode);
        let path = parent.join("token.json");
        let error = FileTokenStore::with_consent(&path, &legacy_path(&path), Consent::granted())
            .expect_err("a directory group or other can write to, so storing is refused");
        assert!(
            matches!(error, CoreError::Storage(StorageError::UnsafeDirectory)),
            "{tag}: expected UnsafeDirectory, got {error:?}"
        );
        assert_eq!(error.code(), "storage");
        assert!(
            !path.exists(),
            "{tag}: a refused store must not write a token file"
        );
        // The refusal must not "fix" someone else's directory.
        assert_eq!(
            dir.mode_of(&parent) & 0o777,
            mode,
            "{tag}: the client must never chmod a directory it did not create"
        );
    }
}

#[test]
fn an_existing_private_parent_is_left_alone_and_written_into() {
    let dir = TempDir::new("store-no-chmod");
    // Private but not created by us: 0750 (readable by the user's group, not
    // writable). The rule only refuses *writable-by-group/other* parents.
    let parent = dir.subdir("private", 0o750);
    let path = parent.join("token.json");
    let store = FileTokenStore::with_consent(&path, &legacy_path(&path), Consent::granted())
        .expect("0750 is not writable by group/other, so storing is allowed");
    store
        .store(&key(), &token())
        .expect("write into the existing private dir");
    #[cfg(unix)]
    {
        assert_eq!(
            dir.mode_of(&parent) & 0o777,
            0o750,
            "the client must not chmod a directory it did not create"
        );
        assert_eq!(dir.mode_of(&path) & 0o777, 0o600);
    }
}

#[test]
#[cfg(unix)] // tightening a loose mode is a Unix mode-bit rule
fn a_loose_token_file_mode_is_tightened_on_read() {
    let dir = TempDir::new("store-selfheal");
    let path = config_dir(&dir, "cfg").join("token.json");
    let store = FileTokenStore::with_consent(&path, &legacy_path(&path), Consent::granted())
        .expect("consent given in this test");
    store.store(&key(), &token()).expect("store");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
        .expect("test must be able to loosen the mode");

    let reloaded = store
        .load(&key())
        .expect("load must still work")
        .expect("Some(token)");
    assert!(reloaded.matches(MOCK_TOKEN));
    assert_eq!(
        dir.mode_of(&path) & 0o777,
        0o600,
        "a group/other-readable token file must be tightened back to 0600"
    );
}

#[test]
fn without_consent_there_is_no_file_store_and_nothing_is_written() {
    let dir = TempDir::new("store-consent");
    let paths = StorePaths::in_dir(config_dir(&dir, "cfg2"));
    let token_path = paths.token_file();

    // There is no constructor that skips the consent check.
    let error =
        FileTokenStore::with_consent(&token_path, &legacy_path(&token_path), Consent::DECLINED)
            .expect_err("DECLINED must not build a file store");
    assert_eq!(error, CoreError::ConsentRequired);
    assert_eq!(error.code(), "consent-required");

    // `open_store` with a declined consent is refused too, and writes nothing.
    let secret_service_available = SecretServiceStore::probe().is_ok();
    match open_store(&paths, Consent::DECLINED, 1_700_000_000) {
        // A real secret service needs no consent; that is the primary backend.
        Ok(store) => assert_eq!(store.backend(), Backend::SecretService),
        Err(CoreError::ConsentRequired) => {
            assert!(
                !secret_service_available,
                "consent gate reached only because the secret service is unavailable"
            );
        }
        Err(other) => panic!("unexpected open_store error: {other:?}"),
    }
    assert!(
        !token_path.exists(),
        "a refused store choice must never write a token file"
    );
    assert_eq!(dir.mode_of(&token_path), 0);
}

#[test]
fn the_granted_choice_is_recorded_with_the_consent_text_that_was_shown() {
    let dir = TempDir::new("store-choice");
    let paths = StorePaths::in_dir(config_dir(&dir, "cfg3"));

    // Nothing recorded yet.
    assert!(load_choice(&paths).expect("choice load").is_none());

    let granted = Consent::granted();
    match open_store(&paths, granted, 1_700_000_001) {
        Ok(store) if store.backend() == Backend::File => {
            let choice = load_choice(&paths)
                .expect("choice must be recorded")
                .expect("Some(choice)");
            assert_eq!(choice.backend, Backend::File.as_str());
            assert_eq!(choice.backend, "file-0600");
            assert_eq!(
                choice.consent_text, CONSENT_TEXT,
                "the exact consent text shown to the user must be persisted verbatim"
            );
            assert_eq!(choice.decided_at_unix, 1_700_000_001);
            assert!(
                CONSENT_TEXT.contains("PLAINTEXT AT REST") && CONSENT_TEXT.contains("0600"),
                "the consent text must state plaintext-at-rest and the 0600 mode"
            );
            assert!(CONSENT_HEADLINE.contains("plaintext file"));
            assert!(!CONSENT_TEXT.contains(MOCK_TOKEN));

            #[cfg(unix)]
            {
                let choice_mode = dir.mode_of(&paths.choice_file()) & 0o777;
                assert_eq!(choice_mode, 0o600, "the recorded choice file must be 0600");
            }
            let choice_body =
                std::fs::read_to_string(paths.choice_file()).expect("choice file must be readable");
            assert!(
                !choice_body.contains(MOCK_TOKEN),
                "the recorded choice must contain no token"
            );

            // The store `open_store` hands back is usable end to end.
            store
                .store(&key(), &token())
                .expect("chosen store must accept a token");
            assert!(store
                .load(&key())
                .expect("chosen store must reload")
                .expect("Some(token)")
                .matches(MOCK_TOKEN));
        }
        Ok(store) => {
            // Environment-dependent branch: this machine has a working secret
            // service, so the consent gate is not reached and no consent text
            // is recorded. Asserted rather than silently skipped.
            assert_eq!(store.backend(), Backend::SecretService);
            let choice = load_choice(&paths)
                .expect("choice must be recorded")
                .expect("Some(choice)");
            assert_eq!(choice.backend, Backend::SecretService.as_str());
            assert!(
                choice.consent_text.is_empty(),
                "secret service needs no consent text"
            );
        }
        Err(error) => panic!("open_store with granted consent must not fail: {error:?}"),
    }

    // A hand-recorded choice round-trips too (UI "what did I choose?" read).
    let manual = StoreChoice {
        backend: Backend::File.as_str().to_owned(),
        consent_text: CONSENT_TEXT.to_owned(),
        decided_at_unix: 1_700_000_002,
    };
    record_choice(&paths, &manual).expect("record_choice must write");
    let reloaded = load_choice(&paths).expect("load").expect("Some(choice)");
    assert_eq!(reloaded, manual);
}

#[test]
fn resolve_never_falls_back_to_the_current_directory() {
    // The security rule: with no `HOME`/`$XDG_CONFIG_HOME` the core writes
    // nothing. An earlier version fell back to `PathBuf::from(".")`.
    match StorePaths::resolve() {
        Ok(paths) => {
            assert!(
                paths.config_dir.is_absolute(),
                "resolved config dir must be absolute, not relative to the CWD: {:?}",
                paths.config_dir
            );
            assert_ne!(paths.config_dir, PathBuf::from("."));
            assert!(
                paths.config_dir.to_string_lossy().contains("chewvy"),
                "the app config dir must be this client's own: {:?}",
                paths.config_dir
            );
            assert_eq!(
                paths.token_file().file_name().unwrap(),
                "tokens.json",
                "the keyed fallback file name is stable"
            );
            assert_eq!(
                paths.legacy_token_file().file_name().unwrap(),
                "token.json",
                "the pre-keying fallback file name stays where old installs left it"
            );
            assert_eq!(
                paths.choice_file().file_name().unwrap(),
                "token-store-choice.json"
            );
        }
        Err(CoreError::Storage(StorageError::NoConfigHome)) => {
            // Legitimate in an environment without HOME: nothing was written.
        }
        Err(other) => panic!("unexpected StorePaths::resolve error: {other:?}"),
    }
}

#[test]
fn the_store_debug_and_error_text_never_show_the_token() {
    let dir = TempDir::new("store-debug");
    let path = config_dir(&dir, "cfg4").join("token.json");
    let store = FileTokenStore::with_consent(&path, &legacy_path(&path), Consent::granted())
        .expect("consent given in this test");
    store.store(&key(), &token()).expect("store");

    let debug = format!("{store:?}");
    assert!(
        !debug.contains(MOCK_TOKEN),
        "FileTokenStore Debug must not print the token: {debug}"
    );
    assert!(debug.contains("FileTokenStore") && debug.contains("token.json"));

    // A corrupt token file yields a fixed storage code, not the payload.
    std::fs::write(&path, b"not json at all").expect("test must be able to corrupt the file");
    let error = store
        .load(&key())
        .expect_err("a corrupt token file must be a storage error");
    assert_eq!(error, CoreError::Storage(StorageError::Payload));
    assert_eq!(error.to_string(), "token storage failure: bad-payload");
    assert!(!format!("{error:?}").contains(MOCK_TOKEN));

    // The Debug of a store path (UI display) is not a secret.
    let displayed: &Path = store.path();
    assert!(!displayed.to_string_lossy().contains(MOCK_TOKEN));
}

#[test]
fn the_secret_service_probe_is_side_effect_free_or_simply_unavailable() {
    // `probe` is documented as side-effect free (no `CreateCollection`, no
    // `Unlock`). In this offline test environment there is no session bus, so the
    // observable part is: it fails with `SecretServiceUnavailable`, it creates
    // no local bookkeeping, and it never turns the file fallback into a silent
    // default. Live D-Bus behaviour (call shapes, `(oayays)` layout) stays
    // **compile-verified only** until the M8 real-desktop gate.
    let dir = TempDir::new("store-probe");
    let paths = StorePaths::in_dir(dir.join("never-created"));

    match SecretServiceStore::probe() {
        Ok(store) => {
            let debug = format!("{store:?}");
            assert!(
                !debug.contains(MOCK_TOKEN),
                "secret service store Debug must not print a token: {debug}"
            );
        }
        Err(CoreError::SecretServiceUnavailable) => {
            // Expected here: no bus / no `org.freedesktop.secrets` name.
        }
        Err(other) => panic!("probe must fail with SecretServiceUnavailable, got {other:?}"),
    }
    assert!(
        !paths.config_dir.exists(),
        "a probe must not create the app config directory"
    );
    assert!(
        attribute_safe(&common::client(&dir).client_identifier),
        "the per-install UUID must be usable as a Secret Service attribute"
    );
    assert!(
        !attribute_safe("not-a-uuid"),
        "non-UUID attributes must be refused"
    );
}

/// Live KDE Wallet / GNOME Keyring round trip with a dummy value.
/// Run by hand on a desktop: `cargo test -p plex-core --test token_storage -- --ignored`.
#[test]
#[ignore = "needs a real desktop secrets daemon"]
fn live_secret_service_round_trip() {
    use plex_core::tokens::SecretServiceStore;
    let store = SecretServiceStore::probe().expect("secret service on the session bus");
    let dummy = plex_core::secret::SecretString::new("dummy-not-a-token");
    store.store(&key(), &dummy).expect("store");
    let loaded = store.load(&key()).expect("load").expect("item present");
    assert_eq!(loaded.expose(), "dummy-not-a-token");
    store.delete(&key()).expect("delete");
    assert!(store.load(&key()).expect("load after delete").is_none());
}

#[test]
fn a_token_key_is_restricted_to_stable_identifiers() {
    assert_eq!(
        key().as_str(),
        "chewvy:plex:deadbeef-dead-4ead-beef-deadbeefdead"
    );
    assert_ne!(key(), other_key());
    // Empty, oversized or punctuation-heavy ids could break out of a file name
    // or a D-Bus attribute value; both constructors refuse them.
    assert_eq!(TokenKey::plex(""), Err(CoreError::InvalidInput));
    assert_eq!(TokenKey::plex("two words"), Err(CoreError::InvalidInput));
    assert_eq!(TokenKey::jellyfin("bad/id"), Err(CoreError::InvalidInput));
    assert_eq!(
        TokenKey::jellyfin(&"x".repeat(97)),
        Err(CoreError::InvalidInput)
    );
    assert!(TokenKey::jellyfin("x".repeat(96).as_str()).is_ok());
}

#[test]
fn two_accounts_coexist_in_one_keyed_file_and_deleting_one_keeps_the_other() {
    let dir = TempDir::new("store-keyed");
    let path = config_dir(&dir, "cfg").join("token.json");
    let store = FileTokenStore::with_consent(&path, &legacy_path(&path), Consent::granted())
        .expect("consent given in this test");
    store.store(&key(), &token()).expect("plex slot");
    store
        .store(&other_key(), &SecretString::new("jf-token"))
        .expect("jellyfin slot");

    assert!(store
        .load(&key())
        .expect("plex load")
        .expect("plex token")
        .matches(MOCK_TOKEN));
    assert!(store
        .load(&other_key())
        .expect("jellyfin load")
        .expect("jellyfin token")
        .matches("jf-token"));

    // Deleting one slot keeps the file and the other token.
    store.delete(&key()).expect("delete plex slot");
    assert!(path.exists(), "the other account still needs the file");
    assert!(store.load(&key()).expect("plex load").is_none());
    assert!(store
        .load(&other_key())
        .expect("jellyfin reload")
        .is_some_and(|t| t.matches("jf-token")));

    // The last slot leaving takes the file with it.
    store.delete(&other_key()).expect("delete jellyfin slot");
    assert!(!path.exists(), "an empty key map is stored as no file");
}

#[test]
fn a_pre_keying_token_migrates_into_the_keyed_slot_exactly_once() {
    let dir = TempDir::new("store-migrate");
    let path = config_dir(&dir, "cfg").join("token.json");
    let legacy = config_dir(&dir, "cfg").join("legacy-token.json");
    let store =
        FileTokenStore::with_consent(&path, &legacy, Consent::granted()).expect("consent given");

    // An install from before keying: token in the legacy slot, keyed empty.
    store.store_legacy(&token()).expect("legacy store");
    assert!(legacy.exists());
    assert!(store.load(&key()).expect("keyed load").is_none());

    // The first keyed read adopts it: keyed filled, legacy cleared.
    let adopted = load_plex_token(&store, Some(&key()))
        .expect("read")
        .expect("the legacy token must be adopted");
    assert!(adopted.matches(MOCK_TOKEN));
    assert!(!legacy.exists(), "the legacy slot must be cleared");
    assert!(path.exists(), "the keyed slot must now hold the token");

    // Later reads come from the keyed slot only.
    store
        .store(&key(), &SecretString::new("replacement"))
        .expect("re-store");
    assert!(load_plex_token(&store, Some(&key()))
        .expect("second read")
        .expect("Some(token)")
        .matches("replacement"));

    // Without an account id there is nothing to migrate into: the legacy slot
    // is read as-is and left where it is.
    store.store_legacy(&token()).expect("legacy store again");
    assert!(load_plex_token(&store, None)
        .expect("legacy read")
        .expect("Some(token)")
        .matches(MOCK_TOKEN));
    assert!(legacy.exists(), "a keyless read must not move the token");

    // Saving without an id keeps using the legacy slot; signing out clears
    // whichever slots exist.
    save_plex_token(&store, None, &token()).expect("legacy save");
    store.store(&key(), &token()).expect("keyed save");
    delete_plex_token(&store, Some(&key())).expect("sign-out clear");
    assert!(store.load(&key()).expect("keyed load").is_none());
    assert!(store.load_legacy().expect("legacy load").is_none());
    assert!(!legacy.exists() && !path.exists());
}
