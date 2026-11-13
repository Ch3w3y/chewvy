//! Which server the app opens on start: the remembered one, the only one, or
//! the picker.
//!
//! The remembered choice is the connected server's `clientIdentifier` written
//! as one line in the app config dir. That id is the server's own stable
//! identifier from the plex.tv resource listing, not a credential, so it is
//! safe to keep next to the token store (and is never sent anywhere). A stale
//! or unknown id falls back to the picker rather than guessing a server.

use std::io::ErrorKind;
use std::path::Path;

use plex_core::resources::ServerResource;

/// File holding the remembered `clientIdentifier`, under the app config dir.
pub const LAST_SERVER: &str = "last-server";

/// File holding the plex.tv account id the stored token belongs to — the id
/// [`plex_core::tokens::TokenKey::plex`] keys the Plex token slot by. An id,
/// not a credential, so it lives in plain text like [`LAST_SERVER`].
pub const PLEX_ACCOUNT: &str = "plex-account-id";

/// Longest id accepted; longer values are treated as a corrupt file.
const MAX_ID: usize = 128;

/// The remembered server id, when the file exists and holds one plausible id.
pub fn read_last_server(config_dir: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(config_dir.join(LAST_SERVER)).ok()?;
    sanitize(raw.trim()).map(str::to_owned)
}

/// The remembered plex.tv account id, when the file exists and holds one the
/// token slot can be keyed by.
pub fn read_plex_account(config_dir: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(config_dir.join(PLEX_ACCOUNT)).ok()?;
    let uuid = raw.trim();
    keys_a_slot(uuid).then(|| uuid.to_owned())
}

/// Whether the token store can key a slot by `account`. plex.tv calls the id
/// a uuid but does not send the dashed form, so the slot's own rule is the
/// test: an id that keys a slot must also be remembered, or the next start
/// cannot find the token it was stored under.
fn keys_a_slot(account: &str) -> bool {
    plex_core::tokens::TokenKey::plex(account).is_ok()
}

/// Remember which plex.tv account the stored token belongs to.
///
/// # Errors
///
/// Io errors from the config dir, or an id the token slot cannot be keyed by.
pub fn remember_plex_account(config_dir: &Path, uuid: &str) -> std::io::Result<()> {
    let uuid = uuid.trim();
    if !keys_a_slot(uuid) {
        return Err(std::io::Error::new(
            ErrorKind::InvalidInput,
            "account uuid cannot be remembered verbatim",
        ));
    }
    plex_core::tokens::create_private_dir(config_dir)?;
    let path = config_dir.join(PLEX_ACCOUNT);
    plex_core::tokens::write_private_file(&path, format!("{uuid}\n").as_bytes())
}

/// Store `client_identifier` as the server to reconnect to.
///
/// # Errors
///
/// Io errors from the config dir, or an id that cannot be stored verbatim.
pub fn remember_last_server(config_dir: &Path, client_identifier: &str) -> std::io::Result<()> {
    let Some(id) = sanitize(client_identifier) else {
        return Err(std::io::Error::new(
            ErrorKind::InvalidInput,
            "clientIdentifier cannot be remembered verbatim",
        ));
    };
    plex_core::tokens::create_private_dir(config_dir)?;
    let path = config_dir.join(LAST_SERVER);
    plex_core::tokens::write_private_file(&path, format!("{id}\n").as_bytes())
}

/// The index into `servers` to connect to without asking, or `None` to show the
/// server picker.
///
/// Exactly one server connects on its own. Otherwise the remembered
/// `clientIdentifier` must still be listed, in which case that server is
/// reconnected to. Anything else (several servers, a remembered id that is gone
/// or renamed) leaves the user to choose.
#[must_use]
pub fn auto_connect(servers: &[ServerResource], remembered: Option<&str>) -> Option<usize> {
    if servers.len() == 1 {
        return Some(0);
    }
    let wanted = sanitize(remembered?)?;
    servers
        .iter()
        .position(|s| sanitize(s.client_identifier()) == Some(wanted))
}

/// An id that can be written and read back unchanged.
fn sanitize(id: &str) -> Option<&str> {
    let id = id.trim();
    if id.is_empty() || id.len() > MAX_ID || id.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return None;
    }
    Some(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use plex_core::resources::{parse_resources, ServerList};

    /// Servers built the way the app builds them: from a plex.tv-shaped row list
    /// (the `api/v2/resources` renderings `parse_resources` accepts).
    fn servers(ids: &[&str]) -> ServerList {
        let rows = ids
            .iter()
            .map(|id| {
                serde_json::json!({
                    "clientIdentifier": id,
                    "name": format!("Server {id}"),
                    "device": format!("{id}-device"),
                    "provides": "server",
                    "accessToken": "mock-not-a-real-token",
                    "presence": 1,
                    "owned": 1,
                    "httpsRequired": 0,
                    "dnsRebindingProtection": 1,
                    "natLoopbackSupported": 1,
                    "relay": 0,
                    "connections": [{
                        "address": "192.0.2.10",
                        "port": 32400,
                        "protocol": "http",
                        "uri": "http://192.0.2.10:32400",
                        "httpuri": "http://192.0.2.10:32400",
                        "local": true,
                        "relay": false,
                        "IPv6": false
                    }]
                })
            })
            .collect::<Vec<_>>();
        let Ok(text) = serde_json::to_string(&rows) else {
            unreachable!("the mock rows must serialize");
        };
        let Ok(list) = parse_resources(text.as_bytes(), "this-client") else {
            unreachable!("mock resource rows must parse");
        };
        list
    }

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("plex-app-connect-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            std::fs::create_dir_all(&dir).is_ok(),
            "the test config directory must be creatable"
        );
        dir
    }

    #[test]
    fn one_server_connects_without_a_choice() {
        let list = servers(&["aaaa1111"]);
        assert_eq!(list.servers().len(), 1);
        assert_eq!(auto_connect(list.servers(), None), Some(0));
        // A remembered id that does not match the single server is still fine.
        assert_eq!(auto_connect(list.servers(), Some("zzzz9999")), Some(0));
    }

    #[test]
    fn a_remembered_server_is_picked_out_of_several() {
        let list = servers(&["aaaa1111", "bbbb2222"]);
        assert_eq!(auto_connect(list.servers(), Some("bbbb2222")), Some(1));
        assert_eq!(auto_connect(list.servers(), Some("cccc3333")), None);
        assert_eq!(auto_connect(list.servers(), Some("  ")), None);
        assert_eq!(auto_connect(list.servers(), None), None);
    }

    #[test]
    fn no_servers_means_no_auto_connect() {
        let list = servers(&[]);
        assert_eq!(auto_connect(list.servers(), Some("aaaa1111")), None);
    }

    #[test]
    fn remembered_id_survives_a_write_and_read_round_trip() {
        let dir = temp_dir("roundtrip");
        assert!(
            remember_last_server(&dir, "aaaa1111").is_ok(),
            "a plausible id must be storeable"
        );
        assert_eq!(read_last_server(&dir).as_deref(), Some("aaaa1111"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                std::fs::metadata(dir.join(LAST_SERVER))
                    .map(|meta| meta.permissions().mode() & 0o777)
                    .ok(),
                Some(0o600),
                "the remembered id is readable only by this user"
            );
        }
        assert_eq!(
            remember_last_server(&dir, "").err().map(|err| err.kind()),
            Some(ErrorKind::InvalidInput),
            "an empty id is refused instead of silently dropping the memory"
        );
        assert_eq!(read_last_server(&dir).as_deref(), Some("aaaa1111"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_or_mangled_file_remembers_nothing() {
        let dir = temp_dir("missing");
        assert_eq!(read_last_server(&dir), None);
        assert!(
            std::fs::write(dir.join(LAST_SERVER), "two words\n").is_ok(),
            "the test file must be writable"
        );
        assert_eq!(read_last_server(&dir), None);
        assert!(
            std::fs::write(dir.join(LAST_SERVER), "x".repeat(MAX_ID + 1)).is_ok(),
            "the test file must be writable"
        );
        assert_eq!(read_last_server(&dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// plex.tv sends the account id as sixteen hex digits, not a dashed uuid.
    /// It keys the token slot, so it has to survive to the next start.
    #[test]
    fn the_account_id_plex_tv_sends_is_remembered() {
        let dir = temp_dir("account");
        for id in ["0123456789abcdef", "01234567-89ab-cdef-0123-456789abcdef"] {
            assert!(remember_plex_account(&dir, id).is_ok(), "{id} is kept");
            assert_eq!(read_plex_account(&dir).as_deref(), Some(id));
        }
        assert_eq!(
            remember_plex_account(&dir, "two words")
                .err()
                .map(|err| err.kind()),
            Some(ErrorKind::InvalidInput),
            "an id no token slot can be keyed by is refused"
        );
        assert!(
            std::fs::write(dir.join(PLEX_ACCOUNT), "two words\n").is_ok(),
            "the test file must be writable"
        );
        assert_eq!(read_plex_account(&dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
