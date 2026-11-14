//! The Watch Together room calls and the people lists against a mock HTTP
//! server: each request's method, path, body and headers, how each answer
//! reads, and how refusals come back.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use plex_core::client_info::{ClientInfo, Identity, TOKEN_HEADER};
use plex_core::error::CoreError;
use plex_core::http::Http;
use plex_core::secret::SecretString;
use plex_core::together::people::{
    friends, home_users, COMMUNITY_PATH, FRIENDS_QUERY, HOME_USERS_PATH,
};
use plex_core::together::room::{
    create_room, invite, leave_room, list_rooms, room, NewRoom, ROOMS_PATH,
};
use serde_json::Value;

const TOKEN: &str = "MOCKTOKEN-TOGETHER-5d1e";
const SOURCE: &str = "server://FAKEMACHINE0001/com.plexapp.plugins.library/library/metadata/1";

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/together/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

/// One request as the mock saw it.
#[derive(Debug, Clone)]
struct Seen {
    method: String,
    url: String,
    body: String,
    headers: Vec<(String, String)>,
}

impl Seen {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// Answer each request with the next `(status, body)`, recording it.
fn serve(answers: Vec<(u16, String)>) -> (String, Arc<Mutex<Vec<Seen>>>, JoinHandle<()>) {
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let base = format!("http://{}", server.server_addr().to_ip().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let record = Arc::clone(&seen);
    let handle = std::thread::spawn(move || {
        for (status, body) in answers {
            let mut request = server.recv().unwrap();
            let mut text = String::new();
            request.as_reader().read_to_string(&mut text).unwrap();
            record.lock().unwrap().push(Seen {
                method: request.method().to_string(),
                url: request.url().to_owned(),
                body: text,
                headers: request
                    .headers()
                    .iter()
                    .map(|h| (h.field.to_string(), h.value.to_string()))
                    .collect(),
            });
            let content_type =
                tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..])
                    .unwrap();
            let response = tiny_http::Response::from_string(body)
                .with_status_code(status)
                .with_header(content_type);
            request.respond(response).unwrap();
        }
    });
    (base, seen, handle)
}

fn http(base: &str) -> Http {
    let identity = Identity {
        client_identifier: "0a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d".to_owned(),
    };
    Http::new(base, &ClientInfo::with_identity(&identity, "0.1.0")).unwrap()
}

fn token() -> SecretString {
    SecretString::new(TOKEN)
}

fn check_headers(seen: &Seen) {
    assert_eq!(seen.header(TOKEN_HEADER), Some(TOKEN));
    assert_eq!(
        seen.header("X-Plex-Client-Identifier"),
        Some("0a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d")
    );
    assert!(seen.header("X-Plex-Product").is_some());
    assert!(!seen.url.contains(TOKEN), "the token stays out of the URL");
    assert!(!seen.body.contains(TOKEN), "and out of the body");
}

#[test]
fn rooms_are_listed_read_created_invited_to_and_left() {
    let (base, seen, server) = serve(vec![
        (200, fixture("rooms.json")),
        (200, fixture("room.json")),
        (201, fixture("room.json")),
        (201, fixture("room.json")),
        (200, String::new()),
        (204, String::new()),
    ]);
    let http = http(&base);
    let token = token();

    let rooms = list_rooms(&http, &token).unwrap();
    assert_eq!(rooms.len(), 2);
    let full = &rooms[0];
    assert_eq!(full.id, "fake-room-0001");
    assert_eq!(full.title, "Film night");
    assert_eq!(full.source_uri, SOURCE);
    assert_eq!(full.created_by, Some(1_000_001));
    assert_eq!(full.starts_at, Some(1_700_000_000));
    assert_eq!(full.ends_at, Some(1_700_010_800));
    assert_eq!(full.updated_at, Some(1_700_000_000));
    assert_eq!(
        full.syncplay_url().as_deref(),
        Some("wss://sync.example.invalid:7776/ws")
    );
    assert_eq!(full.users.len(), 1);
    assert_eq!(full.users[0].id, 1_000_002);
    assert_eq!(full.users[0].uuid, "fakeuuid00000002");
    assert_eq!(full.users[0].title, "Guest");
    assert!(full.users[0].thumb.is_some());
    assert_eq!(full.users[0].username, "", "a managed user has none: null");
    let bare = &rooms[1];
    assert_eq!(bare.id, "fake-room-0002");
    assert!(bare.users.is_empty());
    assert_eq!(bare.title, "", "null reads as empty");
    assert_eq!(bare.syncplay_url(), None, "no server, no socket");

    assert_eq!(room(&http, &token, "fake-room-0001").unwrap(), *full);

    let created = create_room(
        &http,
        &token,
        &NewRoom {
            source_uri: SOURCE,
            title: "Film night",
            users: Some(&[1_000_002]),
        },
    )
    .unwrap();
    assert_eq!(created.id, "fake-room-0001");
    create_room(
        &http,
        &token,
        &NewRoom {
            source_uri: SOURCE,
            title: "Alone",
            users: None,
        },
    )
    .unwrap();
    invite(&http, &token, "fake-room-0001", &[1_000_002, 1_000_003]).unwrap();
    leave_room(&http, &token, "fake-room-0001").unwrap();
    server.join().unwrap();

    let seen = seen.lock().unwrap().clone();
    let lines: Vec<(&str, &str)> = seen
        .iter()
        .map(|s| (s.method.as_str(), s.url.as_str()))
        .collect();
    assert_eq!(
        lines,
        vec![
            ("GET", ROOMS_PATH),
            ("GET", "/rooms/fake-room-0001"),
            ("POST", ROOMS_PATH),
            ("POST", ROOMS_PATH),
            ("POST", "/rooms/fake-room-0001/invite"),
            ("DELETE", "/rooms/fake-room-0001"),
        ]
    );
    for request in &seen {
        check_headers(request);
    }
    let body = |i: usize| serde_json::from_str::<Value>(&seen[i].body).unwrap();
    assert_eq!(
        body(2),
        serde_json::json!({"sourceUri": SOURCE, "title": "Film night", "users": [1_000_002]})
    );
    assert_eq!(
        body(3),
        serde_json::json!({"sourceUri": SOURCE, "title": "Alone", "users": null})
    );
    assert_eq!(
        body(4),
        serde_json::json!({"users": [1_000_002, 1_000_003]})
    );
    assert_eq!(seen[2].header("Content-Type"), Some("application/json"));
}

#[test]
fn refusals_and_odd_answers_are_typed_errors() {
    let (base, _, server) = serve(vec![
        (401, String::new()),
        (404, r#"{"error":"not found"}"#.to_owned()),
        (200, r#"{"rooms":"nope"}"#.to_owned()),
        (500, String::new()),
    ]);
    let http = http(&base);
    let token = token();
    assert_eq!(
        list_rooms(&http, &token),
        Err(CoreError::AuthRejected { status: 401 })
    );
    assert_eq!(
        room(&http, &token, "gone").unwrap_err(),
        CoreError::HttpStatus { status: 404 }
    );
    assert_eq!(list_rooms(&http, &token), Err(CoreError::ResponseShape));
    assert_eq!(
        leave_room(&http, &token, "fake-room-0001"),
        Err(CoreError::HttpStatus { status: 500 })
    );
    server.join().unwrap();
}

#[test]
fn bad_input_never_reaches_the_service() {
    let (base, seen, server) = serve(Vec::new());
    let http = http(&base);
    let token = token();
    for id in ["", "a/b", "..", "x?y=1", "a b"] {
        assert_eq!(
            room(&http, &token, id),
            Err(CoreError::InvalidInput),
            "{id:?}"
        );
        assert_eq!(leave_room(&http, &token, id), Err(CoreError::InvalidInput));
        assert_eq!(
            invite(&http, &token, id, &[1]),
            Err(CoreError::InvalidInput)
        );
    }
    assert_eq!(
        invite(&http, &token, "fake-room-0001", &[]),
        Err(CoreError::InvalidInput)
    );
    server.join().unwrap();
    assert!(seen.lock().unwrap().is_empty());
}

#[test]
fn home_users_and_friends_read_with_their_numeric_ids() {
    let (base, seen, server) = serve(vec![
        (200, fixture("home_users.json")),
        (200, fixture("friends.json")),
        (
            200,
            r#"{"data":null,"errors":[{"message":"no"}]}"#.to_owned(),
        ),
    ]);
    let http = http(&base);
    let token = token();

    let home = home_users(&http, &token).unwrap();
    assert_eq!(home.len(), 2);
    assert_eq!(home[0].id, 1_000_001);
    assert!(home[0].admin && !home[0].restricted);
    assert_eq!(home[1].id, 1_000_002);
    assert_eq!(home[1].uuid, "fakeuuid00000002");
    assert_eq!(home[1].title, "Guest");
    assert!(home[1].restricted && !home[1].admin);

    let friends_list = friends(&http, &token).unwrap();
    assert_eq!(friends_list.len(), 2);
    assert_eq!(friends_list[0].id, 1_000_003);
    assert_eq!(friends_list[0].uuid, "fakeuuid00000003");
    assert_eq!(friends_list[0].username, "friend-one");
    assert_eq!(friends_list[0].display_name, "Friend One");
    assert!(friends_list[0].avatar.is_some());
    assert_eq!(friends_list[1].id, 1_000_004, "an id sent as text reads");
    assert_eq!(friends_list[1].avatar, None);
    assert_eq!(friends_list[1].display_name, "");

    assert_eq!(friends(&http, &token), Err(CoreError::ResponseShape));
    server.join().unwrap();

    let seen = seen.lock().unwrap().clone();
    assert_eq!(
        (seen[0].method.as_str(), seen[0].url.as_str()),
        ("GET", HOME_USERS_PATH)
    );
    assert_eq!(
        (seen[1].method.as_str(), seen[1].url.as_str()),
        ("POST", COMMUNITY_PATH)
    );
    assert_eq!(
        serde_json::from_str::<Value>(&seen[1].body).unwrap(),
        serde_json::json!({"query": FRIENDS_QUERY, "variables": {}})
    );
    for request in &seen {
        check_headers(request);
    }
}
