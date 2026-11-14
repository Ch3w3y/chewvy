//! The SyncPlay requests against a mock 12.4 server: the path, the body and
//! the header of each, and how the answers are read.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::time::Duration;

use jellyfin_core::browse::Client;
use jellyfin_core::syncplay::clock::UtcTime;
use jellyfin_core::syncplay::{GroupState, PlayerReport};
use jellyfin_core::{DeviceInfo, Http, JfError, SecretString};

mod mock;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/live-12.4/");
const USER_ID: &str = "70eb2746aca443c098159c5d6e66d886";
const TOKEN: &str = "c8f0a1d2e3b4456789abcdef01234567";
const GROUP_ID: &str = "a69bbb19e2fd498595de88161eba1fbb";
const PLAYLIST_ITEM_ID: &str = "a7c11b921e6f4ca9959c05e714f6485f";
const F01: &str = "4966f0f90dd1fa4c2539dd7f319b0f57";

struct Server {
    mock: mock::MockJfServer,
    http: Http,
    device: DeviceInfo,
    token: SecretString,
}

impl Server {
    fn new() -> Self {
        let mock = mock::MockJfServer::start();
        let http = Http::with_timeouts(&mock.url(), Duration::from_secs(5), Duration::from_secs(5))
            .expect("mock base must normalise");
        Self {
            mock,
            http,
            device: DeviceInfo {
                app: "Chewvy".to_owned(),
                app_version: "0.1.0".to_owned(),
                device: "test-machine".to_owned(),
                device_id: "0a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d".to_owned(),
            },
            token: SecretString::new(TOKEN),
        }
    }

    fn client(&self) -> Client<'_> {
        Client::new(&self.http, &self.device, &self.token, USER_ID).expect("a usable user id")
    }

    fn answer(&self, method_path: &str, status: u16, body: &str) {
        self.mock
            .route(method_path, mock::Answer::json(status, body));
    }

    fn answer_fixture(&self, method_path: &str, name: &str) {
        let body = std::fs::read_to_string(format!("{FIXTURES}{name}")).expect("fixture");
        self.answer(method_path, 200, &body);
    }

    /// The one request made: `METHOD /path`, its body as JSON (`null` when
    /// empty), with the token checked to be in the header and nowhere else.
    fn only_request(&self) -> (String, serde_json::Value) {
        let requests = self.mock.requests();
        assert_eq!(requests.len(), 1, "{requests:?}");
        let request = &requests[0];
        let authorization = request.authorization.as_deref().unwrap_or_default();
        assert!(authorization.contains(&format!("Token=\"{TOKEN}\"")));
        assert!(!request.path_and_query.contains(TOKEN));
        assert!(!request.body.contains(TOKEN));
        let body = if request.body.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_str(&request.body).expect("a JSON body")
        };
        (
            format!("{} {}", request.method, request.path_and_query),
            body,
        )
    }
}

#[test]
fn listing_and_reading_groups() {
    let server = Server::new();
    server.answer_fixture("GET /SyncPlay/List", "syncplay_list.json");
    let groups = server.client().syncplay_groups().expect("groups");
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].group_id, GROUP_ID);
    assert_eq!(groups[0].group_name, "Movie night");
    assert_eq!(groups[0].participants, vec!["peer", "guest"]);
    assert_eq!(server.only_request().0, "GET /SyncPlay/List");

    let server = Server::new();
    server.answer_fixture(&format!("GET /SyncPlay/{GROUP_ID}"), "syncplay_group.json");
    let group = server.client().syncplay_group(GROUP_ID).expect("group");
    assert_eq!(group.state, GroupState::Waiting);
    assert_eq!(
        group.last_updated_at,
        UtcTime::parse("2026-11-12T22:05:10.5765549Z").unwrap()
    );
}

#[test]
fn a_new_group_answers_itself_or_nothing() {
    let server = Server::new();
    server.answer_fixture("POST /SyncPlay/New", "syncplay_new.json");
    let group = server
        .client()
        .syncplay_new_group("Movie night")
        .expect("created");
    assert_eq!(group.map(|group| group.state), Some(GroupState::Idle));
    assert_eq!(
        server.only_request(),
        (
            "POST /SyncPlay/New".to_owned(),
            serde_json::json!({"GroupName": "Movie night"})
        )
    );

    let server = Server::new();
    server.answer("POST /SyncPlay/New", 204, "");
    assert_eq!(server.client().syncplay_new_group("x"), Ok(None));
}

#[test]
fn every_request_has_its_documented_body() {
    let report = PlayerReport {
        when: UtcTime::parse("2026-11-12T22:05:10.5924683Z").unwrap(),
        position: Duration::from_millis(31_500),
        is_playing: false,
        playlist_item_id: PLAYLIST_ITEM_ID,
    };
    let report_body = serde_json::json!({
        "When": "2026-11-12T22:05:10.5924683Z",
        "PositionTicks": 315_000_000_i64,
        "IsPlaying": false,
        "PlaylistItemId": PLAYLIST_ITEM_ID,
    });
    type Call = fn(&Client<'_>, &PlayerReport<'_>) -> Result<(), JfError>;
    let cases: Vec<(&str, Call, serde_json::Value)> = vec![
        (
            "Join",
            |client, _| client.syncplay_join(GROUP_ID),
            serde_json::json!({"GroupId": GROUP_ID}),
        ),
        (
            "Leave",
            |client, _| client.syncplay_leave(),
            serde_json::Value::Null,
        ),
        (
            "SetNewQueue",
            |client, _| client.syncplay_set_new_queue(&[F01], 0, Duration::from_secs(5)),
            serde_json::json!({"PlayingQueue": [F01], "PlayingItemPosition": 0, "StartPositionTicks": 50_000_000}),
        ),
        (
            "Unpause",
            |client, _| client.syncplay_unpause(),
            serde_json::Value::Null,
        ),
        (
            "Pause",
            |client, _| client.syncplay_pause(),
            serde_json::Value::Null,
        ),
        (
            "Stop",
            |client, _| client.syncplay_stop(),
            serde_json::Value::Null,
        ),
        (
            "Seek",
            |client, _| client.syncplay_seek(Duration::from_secs(30)),
            serde_json::json!({"PositionTicks": 300_000_000}),
        ),
        (
            "Buffering",
            |client, report| client.syncplay_buffering(report),
            report_body.clone(),
        ),
        (
            "Ready",
            |client, report| client.syncplay_ready(report),
            report_body.clone(),
        ),
        (
            "Ping",
            |client, _| client.syncplay_ping(Duration::from_micros(12_900)),
            serde_json::json!({"Ping": 12}),
        ),
        (
            "SetIgnoreWait",
            |client, _| client.syncplay_set_ignore_wait(true),
            serde_json::json!({"IgnoreWait": true}),
        ),
    ];
    for (path, call, body) in cases {
        let server = Server::new();
        server.answer(&format!("POST /SyncPlay/{path}"), 204, "");
        call(&server.client(), &report).unwrap_or_else(|error| panic!("{path}: {error}"));
        assert_eq!(
            server.only_request(),
            (format!("POST /SyncPlay/{path}"), body),
            "{path}"
        );
    }
}

#[test]
fn a_refusal_is_a_403_rejection() {
    // No SyncPlay access, or no group to act on: the server says only 403.
    let server = Server::new();
    server.answer("POST /SyncPlay/Pause", 403, "");
    assert_eq!(
        server.client().syncplay_pause(),
        Err(JfError::AuthRejected { status: 403 })
    );
}

#[test]
fn ids_that_could_change_the_route_are_refused_before_sending() {
    let server = Server::new();
    let client = server.client();
    assert_eq!(client.syncplay_join("../Users"), Err(JfError::InvalidInput));
    assert_eq!(
        client.syncplay_group("a/b").map(drop),
        Err(JfError::InvalidInput)
    );
    assert_eq!(
        client.syncplay_set_new_queue(&[], 0, Duration::ZERO),
        Err(JfError::InvalidInput)
    );
    assert_eq!(
        client.syncplay_set_new_queue(&[F01, "x y"], 0, Duration::ZERO),
        Err(JfError::InvalidInput)
    );
    let report = PlayerReport {
        when: UtcTime(0),
        position: Duration::ZERO,
        is_playing: false,
        playlist_item_id: "",
    };
    assert_eq!(client.syncplay_ready(&report), Err(JfError::InvalidInput));
    assert!(server.mock.requests().is_empty());
}

#[test]
fn the_servers_clock_is_read_and_sampled() {
    let server = Server::new();
    server.answer_fixture("GET /GetUtcTime", "get_utc_time.json");
    let answer = server.client().utc_time().expect("time");
    let received = UtcTime::parse("2026-11-12T22:05:16.9871534Z").unwrap();
    assert_eq!(answer.request_reception_time, received);
    assert_eq!(answer.response_transmission_time, received);

    // The fixture's clock is in the past, so this device reads as ahead.
    server.answer_fixture("GET /GetUtcTime", "get_utc_time.json");
    let sample = server.client().clock_sample().expect("sample");
    assert!(sample.offset_ticks < 0);
    assert!(sample.round_trip() < Duration::from_secs(5));
}
