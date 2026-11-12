//! Static redaction scan: every `.expose()` in the jellyfin-core sources is
//! a place where a secret leaves its wrapper, so each one must be a known
//! call site (the `MediaBrowser` header value, the Quick Connect query
//! parameter and body). Anything new has to be justified here first, just
//! like in plex-core's redaction test.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::path::{Path, PathBuf};

/// `(file, expected number of .expose() calls)`.
const EXPOSE_ALLOWLIST: [(&str, usize); 2] = [
    // media_browser_header puts the token into the quoted header value.
    ("header.rs", 1),
    // The Quick Connect secret as the poll query parameter and as the JSON
    // body field of the exchange.
    ("quickconnect.rs", 2),
];

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn every_secret_unwrap_is_a_known_call_site() {
    let src = crate_root().join("src");
    let mut found = Vec::new();
    for entry in std::fs::read_dir(&src).expect("src dir") {
        let path: PathBuf = entry.expect("entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("readable source");
        let hits = text.matches(".expose()").count();
        if hits > 0 {
            found.push((path.file_name().unwrap().to_str().unwrap().to_owned(), hits));
        }
    }
    found.sort();
    let expected: Vec<(String, usize)> = EXPOSE_ALLOWLIST
        .iter()
        .map(|(file, hits)| ((*file).to_owned(), *hits))
        .collect();
    assert_eq!(found, expected, "unexpected .expose() call sites");
}

#[test]
fn no_source_file_names_a_real_token_shape() {
    // Structural guard for the convention: real Jellyfin access tokens are
    // base64-ish blobs; the sources and tests must only ever contain the
    // mock-prefixed literals.
    let dir = crate_root().join("src");
    for entry in std::fs::read_dir(&dir).expect("src dir") {
        let path = entry.expect("entry").path();
        let text = std::fs::read_to_string(&path).expect("readable source");
        assert!(
            !text.contains("jfn-") && !text.contains("JFN_"),
            "{} looks like it hard-codes a real token",
            path.file_name().unwrap().to_str().unwrap()
        );
    }
}

#[test]
fn a_secret_string_stays_inert_when_rendered() {
    let token = jellyfin_core::SecretString::new("mockjf-0123456789abcdef");
    assert!(!format!("{token:?}").contains("0123456789"));
    assert!(!token.to_string().contains("0123456789"));
    // The errors carry no payload that could hold a secret at all.
    let error = jellyfin_core::JfError::HttpStatus { status: 503 };
    assert_eq!(format!("{error}"), "HTTP status 503");
}

/// Compile-time-ish sanity: the crate root must never opt into `unsafe`.
#[test]
fn the_crate_stays_free_of_unsafe() {
    let lib = std::fs::read_to_string(Path::new(&crate_root().join("src/lib.rs"))).expect("lib.rs");
    assert!(
        !lib.contains("unsafe"),
        "jellyfin-core is display- and unsafe-free"
    );
}
