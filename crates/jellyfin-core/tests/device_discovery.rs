//! Device identity persistence (getrandom-backed uuid in a private file) and
//! the LAN discovery answer handling.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use jellyfin_core::device::DeviceInfo;
use jellyfin_core::discovery::{self, DiscoveredServer};
use jellyfin_core::JfError;

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join("chewvy-jf-device-tests");
    std::fs::create_dir_all(&root).expect("temp root");
    let unique = format!(
        "{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    let dir = root.join(unique);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

#[test]
fn a_device_identity_is_created_once_and_reloaded() {
    let dir = temp_dir("create");
    let path = dir.join("jellyfin-device.json");
    let first = DeviceInfo::load_or_create(&path, "0.1.0", "test-machine").expect("create");
    assert!(
        plex_core::client_info::is_uuid(&first.device_id),
        "device id must be a uuid"
    );
    assert_eq!(first.app, jellyfin_core::device::APP_NAME);
    // Reload returns the very same uuid: the server's Device list stays
    // stable across restarts.
    let second = DeviceInfo::load_or_create(&path, "0.1.0", "test-machine").expect("reload");
    assert_eq!(first.device_id, second.device_id);
    // The file is private (0600) like every other identity file here.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "identity file mode");
    }
}

#[test]
fn a_corrupt_identity_file_is_refused_not_silently_replaced() {
    let dir = temp_dir("corrupt");
    let path = dir.join("jellyfin-device.json");
    std::fs::write(&path, b"not json at all").expect("write");
    // load_or_create treats an existing-but-corrupt file as Identity: the
    // caller decides (e.g. "this may orphan an old Device entry").
    assert_eq!(
        DeviceInfo::load_or_create(&path, "0.1.0", "test-machine"),
        Err(JfError::Identity)
    );
}

#[test]
fn a_discovery_answer_parses_and_bad_datagrams_are_ignored() {
    let answer =
        br#"{"Id":"srv-1","ServerName":"Living Room","Address":"http://192.168.1.5:8096"}"#;
    let found = discovery::parse_answer(answer).expect("parse");
    assert_eq!(
        found,
        DiscoveredServer {
            id: "srv-1".to_owned(),
            name: "Living Room".to_owned(),
            address: Some("http://192.168.1.5:8096".to_owned()),
        }
    );
    assert!(discovery::parse_answer(b"{{not json").is_none());
    assert!(discovery::parse_answer(br#"{"Id":"","ServerName":"x"}"#).is_none());
    // An empty Address field behaves as absent.
    let empty_address = discovery::parse_answer(br#"{"Id":"srv-2","Address":""}"#).expect("parse");
    assert_eq!(empty_address.address, None);
}

#[test]
fn answers_are_collected_until_the_read_timeout_and_deduplicated() {
    // A loopback stand-in for the broadcast exchange: pre-fill a socket with
    // answers (one duplicate id, one junk datagram) and collect.
    let socket = std::net::UdpSocket::bind("127.0.0.1:0").expect("bind");
    let addr = socket.local_addr().expect("addr");
    let sender = std::net::UdpSocket::bind("127.0.0.1:0").expect("sender bind");
    for answer in [
        br#"{"Id":"srv-1","ServerName":"A","Address":"http://a:8096"}"#.as_slice(),
        br#"{"Id":"srv-1","ServerName":"A again","Address":"http://a2:8096"}"#.as_slice(),
        br#"{"Id":"srv-2","ServerName":"B"}"#.as_slice(),
        b"junk".as_slice(),
    ] {
        sender.send_to(answer, addr).expect("send");
    }
    socket
        .set_read_timeout(Some(std::time::Duration::from_millis(120)))
        .expect("read timeout");
    let found = discovery::collect_answers(&socket, 8).expect("collect");
    let ids: Vec<&str> = found.iter().map(|server| server.id.as_str()).collect();
    assert_eq!(ids, ["srv-1", "srv-2"], "deduplicated, order of arrival");
}

#[test]
fn a_broadcast_discovery_round_never_hard_fails_on_quiet_networks() {
    // No Jellyfin server is listening here: Ok with zero answers is the
    // contract (an io error is also honest, e.g. in a fully closed sandbox).
    let outcome = discovery::discover(std::time::Duration::from_millis(150));
    match outcome {
        Ok(servers) => assert!(servers.len() < 8, "implausible answer count"),
        Err(JfError::Io) => {}
        Err(other) => panic!("unexpected discovery error: {other:?}"),
    }
}
