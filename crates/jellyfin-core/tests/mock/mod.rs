//! A tiny in-process HTTP mock for the Jellyfin endpoints (same tiny_http
//! approach as the plex-core mock; kept deliberately small).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::collections::{HashMap, VecDeque};
use std::io::Cursor;
use std::sync::{Arc, Mutex};

use tiny_http::{Header, Method, Request, Response};

/// One recorded request, as the assertions need it.
#[derive(Debug, Clone)]
pub struct Recorded {
    /// HTTP method as text.
    pub method: String,
    /// Path and query string as sent (`/QuickConnect/Connect?secret=…`).
    pub path_and_query: String,
    /// The `Authorization` header value, if present.
    pub authorization: Option<String>,
    /// The request body.
    pub body: String,
}

/// The next canned answer for one route.
#[derive(Debug, Clone)]
pub struct Answer {
    /// HTTP status to send.
    pub status: u16,
    /// `Content-Type` header value.
    pub content_type: String,
    /// Body bytes.
    pub body: Vec<u8>,
}

impl Answer {
    /// A JSON answer.
    pub fn json(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            content_type: "application/json".to_owned(),
            body: body.into().into_bytes(),
        }
    }
}

#[derive(Clone)]
pub struct MockJfServer {
    port: u16,
    routes: Arc<Mutex<HashMap<String, VecDeque<Answer>>>>,
    requests: Arc<Mutex<Vec<Recorded>>>,
}

impl MockJfServer {
    /// Bind an ephemeral port and start serving.
    pub fn start() -> Self {
        let server = tiny_http::Server::http("127.0.0.1:0").expect("mock server must bind");
        let port = server
            .server_addr()
            .to_ip()
            .expect("mock must have an ip addr")
            .port();
        let mock = Self {
            port,
            routes: Arc::new(Mutex::new(HashMap::new())),
            requests: Arc::new(Mutex::new(Vec::new())),
        };
        let routes = Arc::clone(&mock.routes);
        let requests = Arc::clone(&mock.requests);
        std::thread::spawn(move || {
            for request in server.incoming_requests() {
                handle(request, &routes, &requests);
            }
        });
        mock
    }

    /// Script the next answer(s) for `"GET /path"` or `"POST /path"`.
    pub fn route(&self, method_path: &str, answer: Answer) {
        self.routes
            .lock()
            .expect("routes lock")
            .entry(method_path.to_owned())
            .or_default()
            .push_back(answer);
    }

    /// The base URL for `jellyfin_core::Http::new`.
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{port}", port = self.port)
    }

    /// Recorded requests so far.
    pub fn requests(&self) -> Vec<Recorded> {
        self.requests.lock().expect("requests lock").clone()
    }
}

fn handle(
    mut request: Request,
    routes: &Mutex<HashMap<String, VecDeque<Answer>>>,
    requests: &Mutex<Vec<Recorded>>,
) {
    let method = match request.method() {
        Method::Get => "GET",
        Method::Post => "POST",
        Method::Delete => "DELETE",
        other => {
            let _ = other;
            let _ = request.respond(unsupported());
            return;
        }
    };
    let url = request.url().to_owned();
    let (path, query) = match url.split_once('?') {
        Some((path, query)) => (path.to_owned(), query.to_owned()),
        None => (url.clone(), String::new()),
    };
    let authorization = request
        .headers()
        .iter()
        .find(|header| header.field.equiv("Authorization"))
        .map(|header| header.value.as_str().to_owned());
    let mut body = String::new();
    request.as_reader().read_to_string(&mut body).ok();
    let key = format!("{method} {path}");
    let answer = routes
        .lock()
        .expect("routes lock")
        .get_mut(&key)
        .and_then(VecDeque::pop_front);
    {
        let mut recorded = requests.lock().expect("requests lock");
        recorded.push(Recorded {
            method: method.to_owned(),
            path_and_query: if query.is_empty() {
                path.clone()
            } else {
                format!("{path}?{query}")
            },
            authorization,
            body,
        });
    }
    let answer = answer.unwrap_or_else(|| Answer {
        status: 404,
        content_type: "application/json".to_owned(),
        body: "{}".as_bytes().to_vec(),
    });
    let content_type = Header::from_bytes(&b"Content-Type"[..], answer.content_type.as_bytes())
        .expect("valid content type header");
    let response = Response::new(
        tiny_http::StatusCode(answer.status),
        vec![content_type],
        Cursor::new(answer.body),
        None,
        None,
    );
    let _ = request.respond(response);
}

fn unsupported() -> tiny_http::Response<Cursor<Vec<u8>>> {
    Response::from_data(Vec::new()).with_status_code(405)
}
