//! **Mutation guard** - the source-scan half.
//!
//! Four kinds of assertion, all executed by `cargo test --all-targets`.
//!
//! 1. **A scanner self-test** ([`the_scanner_counts_strings_and_ignores_comments'])
//!    showing that [`code_only`] counts verb text inside strings, byte strings,
//!    raw strings and char literals and ignores it inside line and block
//!    comments.
//! 2. **A pinned source scan** over `crates/{plex-core,plex-player,plex-app,
//!    plex-cli}/src/**/*.rs`: after stripping comments (strings are kept), every
//!    HTTP write-verb call site and verb literal in product code appears as an
//!    exact `(file, needle, count)` row of [`PINNED_WRITE_ROWS`]. Both sides are
//!    put in canonical `(file, needle, count)` order by [`format_rows`] and then
//!    compared elementwise, so the comparison is an exact multiset comparison
//!    that is insensitive to table declaration order. Adding a write call site
//!    anywhere in product code - or removing one - fails the gate instead of
//!    being noticed in review.
//! 3. **Pinned negatives.** [`PINNED_ZERO_ROWS`] is 13 explicit `(file, needle)`
//!    pairs that must count zero (`progress.rs` reaches no verb wrapper and names
//!    no verb literal; `Server` performs no `POST`/`DELETE` and names neither the
//!    verbs nor the progress paths; `api.rs` has no `.post(`/no `"POST"`; `auth.rs`
//!    names no `"DELETE"`). The workspace-wide `PUT`/`PATCH` ban is a separate
//!    needle list ([`FORBIDDEN_NEEDLES`], call form **and** literal form, total
//!    over the whole tree), **not** a [`PINNED_ZERO_ROWS`] row.
//! 4. **A structural guard**
//!    ([`the_only_state_changes_reachable_in_product_code_are_the_progress_calls`])
//!    on the two allowed state changes: `Server` exposes no write wrapper at all
//!    (no `pub fn post|put|patch|delete`, no crate-internal `post_candidate_write`
//!    left over from the earlier candidate shape), `progress.rs` is the only file
//!    that names the two endpoint paths, `progress.rs` consults the opt-in
//!    **before** its first request, and `enabled = true` appears in exactly one
//!    product file.
//!
//! The **mock-execution** half - a default `plex_core::progress::
//! ProgressUpdate` producing **zero** requests, an opted-in one producing
//! **exactly one** `GET /:/timeline` on the mock PMS, and `mark_watched`
//! producing one `GET /:/scrobble`, with the credential only in `X-Plex-Token` -
//! runs in `tests/library_read_only.rs`
//! (`progress_reporting_is_off_by_default_and_is_the_only_write`), which already
//! owns the mock PMS + mock plex.tv harness, `M6B_PROGRESS_ROUTES` and the
//! request-record helpers. This target deliberately does not duplicate them.
//!
//! # What this target does NOT prove (read before quoting it)
//!
//! * It is **not** a Plex integration test. `/:/timeline` and `/:/scrobble` are
//!   the endpoints the published Plex libraries and server request logs show
//!   (`src/progress.rs` cites them), but the mock answers them
//!   because we wrote the mock: what a real PMS does with them is a
//!   user-verified gate (`docs/CAPABILITIES.md`).
//! * The scan proves *reachability in this source tree*, not a security
//!   property. `plex_core::http::Http::post`/`::delete` stay public API, so a
//!   **new** crate could call them directly; what is pinned is that no product
//!   crate does, that `Server` exposes no write method at all, and that the two
//!   state-changing paths exist in exactly one file (`plex-core/src/progress.rs`).
//! * `store.delete()` calls in `src/api.rs` delete the **local** token file, not
//!   a server resource. The pinned row `plex-core/src/api.rs .delete( = 5`
//!   therefore mixes 1 HTTP signout (`http.delete(SIGNOUT_PATH`) with 4 local
//!   `TokenStore::delete()` calls; `crates/plex-core/tests/token_storage.rs` and
//!   `tests/logout.rs` are what show which is which.
//! * The sign-in writes (`POST /api/v2/pins`, `DELETE /api/v2/users/signout`) are
//!   auth flows accepted in M2, not media mutations.
//!   `crates/plex-core/tests/library_read_only.rs` pins the same verb table from
//!   the mock's side (`M6B_PROGRESS_ROUTES`,
//!   `progress_reporting_is_off_by_default_and_is_the_only_write`).
//! * The scanner is a text scanner with documented limits (see [`code_only`]).
//!   It is a regression tripwire for *accidental* writes, not a proof about
//!   compiled behaviour.

// clippy::indexing_slicing is a deliberate trade-off here: the scanner walks a
// character-position index and `c[i + 1]` reads are clearer as index arithmetic
// than as iterator plumbing.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::path::{Path, PathBuf};

/// Every HTTP write-verb call site, verb literal and progress endpoint the scan
/// counts. The `:/…` needles are path names, not verbs: both progress calls are
/// `GET`, so the only way to pin them in source is the path they address.
const WRITE_NEEDLES: [&str; 10] = [
    ".post(",
    ".put(",
    ".patch(",
    ".delete(",
    "\"POST\"",
    "\"PUT\"",
    "\"PATCH\"",
    "\"DELETE\"",
    ":/timeline",
    ":/scrobble",
];
/// Verbs this client must never use at all, anywhere in `crates/*/src`.
const FORBIDDEN_NEEDLES: [&str; 4] = [".put(", ".patch(", "\"PUT\"", "\"PATCH\""];

/// The crates whose `src/` the scan covers.
const CRATES: [&str; 5] = [
    "plex-core",
    "jellyfin-core",
    "plex-player",
    "plex-app",
    "plex-cli",
];

/// Pinned output of [`scanned_tree`] + [`needle_counts`]: the complete set of nonzero
/// `(file relative to crates/, needle, count)` rows. The test compares the
/// observed set against this table exactly, so:
///
/// * a **new** write call site in any product crate fails the gate;
/// * a **removed** one fails it too (the mutation surface has to be changed on
///   purpose, never by accident);
/// * any file absent here must have **zero** write-verb needles - which is why
///   `plex-player` and `plex-cli` contribute no rows, and `plex-app` contributes
///   exactly one: the local token-store delete that forgets a source.
///
/// Meaning of each row:
///
/// * `http.rs` - the transport's own `pub fn post` / `pub fn delete` wrappers
///   calling `agent.post` / `agent.delete`, plus the `"POST"` / `"DELETE"` verb
///   literals they pass down. This is the allow-listed public write surface
///   named in this file's "what this does NOT prove" section.
/// * `api.rs` - 1 `.delete(`: the HTTP `http.delete(SIGNOUT_PATH` (M2
///   signout). Local token deletion moved to the single `delete_plex_token`
///   helper, so the row count matches the HTTP surface one-to-one.
/// * `tokens.rs` - 1 `.delete(`: `delete_plex_token` calling
///   `TokenStore::delete` on the **local** record (keyring/file), not a server.
/// * `auth.rs` - 1 `.post(`: the M2 pin creation (`http.post(PINS_PATH`).
/// * `progress.rs` - the two endpoint paths, named nowhere else. Both calls are
///   `GET`, so no verb row exists for them.
/// * `jellyfin-core/src/http.rs` - the transport's `post` and `delete`
///   wrappers and the verb literals they pass down.
/// * `jellyfin-core/src/auth.rs` - 2 `.post(`: password sign-in and sign-out.
/// * `jellyfin-core/src/browse.rs` - 2 `.post(`: PlaybackInfo, which reads the
///   item's media sources and changes nothing, and the `/Sessions/Playing*`
///   reports, which only run while the Settings progress-sync switch is on;
///   and 1 `.delete(`: `/Videos/ActiveEncodings`, which ends the encode behind
///   a stream this client itself opened and touches no library data.
/// * `jellyfin-core/src/quickconnect.rs` - 2 `.post(`: initiate and authenticate.
/// * `plex-app/src/sources/worker.rs` - 1 `.delete(`: forgetting a source deletes
///   its token slot on this device, which is the only write this screen does.
const PINNED_WRITE_ROWS: [(&str, &str, usize); 18] = [
    ("plex-core/src/api.rs", ".delete(", 1),
    ("plex-core/src/tokens.rs", ".delete(", 1),
    ("plex-core/src/auth.rs", ".post(", 1),
    ("plex-core/src/http.rs", ".post(", 1),
    ("plex-core/src/http.rs", ".delete(", 1),
    ("plex-core/src/http.rs", "\"POST\"", 2),
    ("plex-core/src/http.rs", "\"DELETE\"", 2),
    ("plex-core/src/progress.rs", ":/timeline", 1),
    ("plex-core/src/progress.rs", ":/scrobble", 1),
    ("jellyfin-core/src/auth.rs", ".post(", 2),
    ("jellyfin-core/src/browse.rs", ".post(", 2),
    ("jellyfin-core/src/browse.rs", ".delete(", 1),
    ("jellyfin-core/src/http.rs", ".post(", 1),
    ("jellyfin-core/src/http.rs", ".delete(", 1),
    ("jellyfin-core/src/http.rs", "\"POST\"", 2),
    ("jellyfin-core/src/http.rs", "\"DELETE\"", 2),
    ("jellyfin-core/src/quickconnect.rs", ".post(", 2),
    ("plex-app/src/sources/worker.rs", ".delete(", 1),
];

/// The **explicit zero** rows: `(file, needle)` pairs that must be observed as
/// zero. These are deliberately separate from [`PINNED_WRITE_ROWS`] so a "this
/// file has no verb X" claim is never encoded as a zero-count row inside a
/// table of nonzero rows (that representation was rejected in review: it
/// contradicts the table's own "absent means zero" rule and makes an
/// observed==pinned comparison ambiguous).
const PINNED_ZERO_ROWS: [(&str, &str); 13] = [
    // The progress calls go through `Server::get`: they reach no write verb
    // wrapper and name no verb literal at all.
    ("plex-core/src/progress.rs", ".post("),
    ("plex-core/src/progress.rs", ".delete("),
    ("plex-core/src/progress.rs", "\"POST\""),
    ("plex-core/src/progress.rs", "\"DELETE\""),
    ("plex-core/src/progress.rs", "\"GET\""),
    // `Server` performs no non-`GET` request and does not name the endpoints it
    // is used to reach.
    ("plex-core/src/server.rs", ".post("),
    ("plex-core/src/server.rs", ".delete("),
    ("plex-core/src/server.rs", "\"POST\""),
    ("plex-core/src/server.rs", ":/timeline"),
    ("plex-core/src/server.rs", ":/scrobble"),
    // The pin POST lives in `auth.rs`; the sign-out DELETE lives in `api.rs`.
    ("plex-core/src/api.rs", ".post("),
    ("plex-core/src/api.rs", "\"POST\""),
    ("plex-core/src/auth.rs", "\"DELETE\""),
];

// ---------------------------------------------------------------- source scan

/// Byte at `i`, if the slice is that long.
fn at(c: &[char], i: usize) -> Option<char> {
    c.get(i).copied()
}

/// True when a raw-string closer (`"` followed by `hashes` `#`) starts at `i`.
fn closes_raw(c: &[char], i: usize, hashes: usize) -> bool {
    c.get(i) == Some(&'"')
        && c.len() >= i + 1 + hashes
        && c[i + 1..i + 1 + hashes].iter().all(|x| *x == '#')
}

/// Length (in chars) of a raw/byte-raw string opener starting at `c[i]`, i.e.
/// `r"`, `r#"`, `br"`, `br##"` …; returns `(hash run, index after the quote)`.
fn raw_opener(c: &[char], i: usize) -> Option<(usize, usize)> {
    let mut j = i;
    if c[j] == 'b' {
        j += 1;
        if j >= c.len() {
            return None;
        }
    }
    if c[j] != 'r' {
        return None;
    }
    j += 1;
    let mut hashes = 0usize;
    while j < c.len() && c[j] == '#' {
        hashes += 1;
        j += 1;
    }
    if c[j] == '"' {
        Some((hashes, j + 1))
    } else {
        None
    }
}

/// Length (in chars) of a simple char / byte-char literal starting at `c[i]`
/// (`'a'`, `'\n'`, `'\''`, `b'\\'`), or `None` when `c[i]` is a lifetime or
/// something this scanner cannot decide.
fn char_literal_len(c: &[char], i: usize) -> Option<usize> {
    let quote = if c[i] == 'b' { i + 1 } else { i };
    if c[quote] != '\'' {
        return None;
    }
    let mut j = quote + 1;
    if j >= c.len() {
        return None;
    }
    if c[j] == '\\' {
        j += 2;
    } else {
        j += 1;
    }
    if j < c.len() && c[j] == '\'' {
        Some(j + 1 - i)
    } else {
        None
    }
}

/// Comment-free text of `source`: comments are removed (block comments replaced
/// by newlines so lines stay comparable) while string, byte-string, raw-string
/// and byte-char contents are **kept verbatim**, so a verb literal inside a
/// string is counted and a verb mention inside a comment is not.
///
/// Documented limits (this is a tripwire, not a Rust parser):
///
/// * `\xNN` / `\u{…}` escapes inside char literals are not parsed: only
///   `\` + one char is consumed, and an unexpected continuation falls back to
///   treating the `'` as a lifetime. No such literal exists in `crates/*/src`
///   today; a source change that introduces one must be reviewed by a human.
/// * A `'` that is neither a simple char literal nor a lifetime (e.g. a `\'`
///   inside a string that our string scan already consumed) is passed through.
/// * Raw-string closers inside a raw string's own content cannot be
///   distinguished, which is correct Rust behaviour and costs nothing here.
/// * Output is character-indexed, so byte offsets in the original file are not
///   preserved.
fn code_only(source: &str) -> String {
    let c: Vec<char> = source.chars().collect();
    let mut out = String::with_capacity(source.len());
    let mut i = 0usize;
    while i < c.len() {
        // Block comment, nesting-aware; each removed line leaves a newline.
        if c[i] == '/' && at(&c, i + 1) == Some('*') {
            i += 2;
            let mut depth = 1usize;
            while i < c.len() && depth > 0 {
                if c[i] == '/' && at(&c, i + 1) == Some('*') {
                    depth += 1;
                    i += 2;
                } else if c[i] == '*' && at(&c, i + 1) == Some('/') {
                    depth -= 1;
                    i += 2;
                } else {
                    if c[i] == '\n' {
                        out.push('\n');
                    }
                    i += 1;
                }
            }
            continue;
        }
        // Line comment (also `//!` / `///`): dropped up to the newline.
        if c[i] == '/' && at(&c, i + 1) == Some('/') {
            while i < c.len() && c[i] != '\n' {
                i += 1;
            }
            continue;
        }
        // Raw / byte-raw string: kept verbatim, including its delimiters.
        // `raw_opener` decides whether this `r`/`b` really opens a raw string, so
        // identifiers ending in `r` or `b` fall through to the normal branch.
        if c[i] == 'r' || c[i] == 'b' {
            if let Some((hashes, content_start)) = raw_opener(&c, i) {
                let mut j = content_start;
                while j < c.len() && !closes_raw(&c, j, hashes) {
                    j += 1;
                }
                let stop = if j < c.len() { j + 1 + hashes } else { c.len() };
                out.extend(c[i..stop].iter());
                i = stop;
                continue;
            }
        }
        // Char / byte-char literal: kept verbatim so its quote cannot be
        // mistaken for the start of a string.
        if c[i] == '\'' || (c[i] == 'b' && at(&c, i + 1) == Some('\'')) {
            if let Some(len) = char_literal_len(&c, i) {
                out.extend(c[i..i + len].iter());
                i += len;
                continue;
            }
        }
        // String / byte string: kept verbatim including the surrounding quotes,
        // honouring backslash escapes (`"\"POST\""` must not open a phantom
        // string).
        if c[i] == '"' || (c[i] == 'b' && at(&c, i + 1) == Some('"')) {
            if c[i] == 'b' {
                out.push('b');
                i += 1;
            }
            out.push('"');
            i += 1;
            while i < c.len() {
                if c[i] == '\\' {
                    out.push('\\');
                    i += 1;
                    if i < c.len() {
                        out.push(c[i]);
                        i += 1;
                    }
                    continue;
                }
                if c[i] == '"' {
                    out.push('"');
                    i += 1;
                    break;
                }
                out.push(c[i]);
                i += 1;
            }
            continue;
        }
        out.push(c[i]);
        i += 1;
    }
    out
}

/// Number of non-overlapping occurrences of every needle in `code`.
fn needle_counts(code: &str) -> Vec<(&'static str, usize)> {
    WRITE_NEEDLES
        .iter()
        .map(|needle| (*needle, code.matches(needle).count()))
        .collect()
}

/// One scanned product source file.
struct ScannedFile {
    /// Path relative to `crates/`, e.g. `plex-core/src/api.rs`.
    rel: String,
    /// Comment-free, string-keeping text ([`code_only`]).
    code: String,
}

/// The `crates/` directory of this workspace, from the test target's manifest.
fn crates_dir() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let parent = manifest
        .parent()
        .expect("manifest dir must sit under crates/")
        .to_path_buf();
    assert!(parent.is_dir(), "crates dir {parent:?} must exist");
    parent
}

fn collect_rust(dir: &Path, root: &Path, out: &mut Vec<ScannedFile>) {
    for entry in std::fs::read_dir(dir).expect("scan: read_dir must succeed") {
        let entry = entry.expect("scan: directory entry");
        let path = entry.path();
        let file_type = entry.file_type().expect("scan: file_type");
        if file_type.is_dir() {
            collect_rust(&path, root, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            let rel = path
                .strip_prefix(root)
                .expect("scan: file must be under crates/")
                .to_string_lossy()
                .into_owned();
            let source = std::fs::read_to_string(&path).expect("scan: source must read");
            out.push(ScannedFile {
                rel,
                code: code_only(&source),
            });
        }
    }
}

/// Every `crates/<each of CRATES>/src/**/*.rs` file, sorted by relative path.
///
/// The lower bound is a guard against the scan silently covering nothing (a
/// renamed crate directory would otherwise turn the pinned table into a
/// passing empty comparison).
fn scanned_tree() -> Vec<ScannedFile> {
    let root = crates_dir();
    let mut out = Vec::new();
    for krate in CRATES {
        let src = root.join(krate).join("src");
        assert!(src.is_dir(), "the mutation scan must see {src:?}");
        collect_rust(&src, &root, &mut out);
    }
    out.sort_by(|a, b| a.rel.cmp(&b.rel));
    assert!(
        out.len() >= 20,
        "the mutation scan found only {} product source files",
        out.len()
    );
    out
}

/// Raw (comment-included) text of one product source file, for the structural
/// checks that read signatures rather than verb counts.
fn raw_source(rel: &str) -> String {
    let path = crates_dir().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {path:?}: {e}"))
}

/// Canonical row form used by **both** sides of the pin comparison.
///
/// Rows are sorted as `(file, needle, count)` tuples before formatting, so the
/// comparison is order-insensitive: `PINNED_WRITE_ROWS` is written in a readable
/// per-file grouping while `observed_rows` walks the tree and sorts. Comparing
/// the two in declaration order would fail on ordering alone (review 2026-11-06
/// named this comparator bug; the pinned content itself was already correct).
fn format_rows(rows: &[(&str, &str, usize)]) -> Vec<String> {
    let mut rows: Vec<(&str, &str, usize)> = rows.to_vec();
    rows.sort();
    rows.iter()
        .map(|(file, needle, count)| format!("{file}|{needle}|{count}"))
        .collect()
}

fn observed_rows(files: &[ScannedFile]) -> Vec<String> {
    let mut rows: Vec<(&str, &str, usize)> = Vec::new();
    for file in files {
        for (needle, count) in needle_counts(&file.code) {
            if count > 0 {
                rows.push((&*file.rel, needle, count));
            }
        }
    }
    rows.sort();
    format_rows(&rows)
}

fn count_needle(files: &[ScannedFile], rel: &str, needle: &str) -> usize {
    files
        .iter()
        .find(|file| file.rel == rel)
        .map(|file| file.code.matches(needle).count())
        .unwrap_or_else(|| panic!("{rel} was not scanned, so its {needle:?} count is unknowable"))
}

// ------------------------------------------------------------ scanner self-test

/// The scan must count what it claims before its numbers mean anything: this
/// sample has hand-counted expectations for every case the scanner claims to
/// handle (line/doc comments, plain and nested block comments, a string with an
/// escaped quote, a byte string, a `#`-delimited raw string, char literals whose
/// quote must not open a phantom string, and the raw-string-in-comment case).
#[test]
fn the_scanner_counts_strings_and_ignores_comments() {
    let code = code_only(SCANNER_SAMPLE);
    for needle in WRITE_NEEDLES {
        let want = SAMPLE_WANT
            .iter()
            .find(|(name, _)| *name == needle)
            .map(|(_, count)| *count)
            .expect("sample expectation table must name every needle");
        assert_eq!(
            code.matches(needle).count(),
            want,
            "needle {needle:?} miscounted; comment-free text was:\n{code}"
        );
    }
    assert!(
        !code.contains("must-not-count"),
        "a needle inside a comment leaked into the scanned text:\n{code}"
    );
    assert_eq!(
        code.lines().count(),
        SCANNER_SAMPLE.lines().count(),
        "comment removal must preserve line structure (block comments become newlines):\n{code}"
    );
}

// ------------------------------------------------- pinned mutation surface

/// **The mutation-surface pin** ("no mutation endpoint is
/// reachable except the opt-in progress setting", source-check half).
///
/// The complete set of nonzero `(file, needle, count)` rows the scanner finds in
/// `crates/*/src` must equal [`PINNED_WRITE_ROWS`] **exactly**: same rows, same
/// counts, nothing extra, nothing missing, compared in the canonical row order
/// produced by [`format_rows`] (see its comment: declaration order in the table
/// is documentation, not part of the assertion). A file absent from the table
/// must have zero write-verb needles, which is why `plex-player` and `plex-cli`
/// contribute no rows (`plex-app`'s one row is the local delete that forgets a
/// source's token).
#[test]
fn write_verb_rows_in_product_code_are_pinned() {
    let files = scanned_tree();
    let observed = observed_rows(&files);
    let pinned = format_rows(&PINNED_WRITE_ROWS);
    assert_eq!(
        observed,
        pinned,
        "the write surface of product code changed. observed rows:\n{}\npinned rows:\n{}\n(scanned {} files)",
        observed.join("\n"),
        pinned.join("\n"),
        files.len(),
    );
}

/// **The pinned negatives** (source-check half).
///
/// Every row of [`PINNED_ZERO_ROWS`] must be observed as zero, and `PUT`/`PATCH`
/// (in call form **and** as a verb literal) must appear nowhere in the scanned
/// tree. A row whose file is not scanned fails rather than silently passing.
#[test]
fn pinned_zero_rows_hold_and_put_patch_appear_nowhere() {
    let files = scanned_tree();
    for (rel, needle) in PINNED_ZERO_ROWS {
        assert_eq!(
            count_needle(&files, rel, needle),
            0,
            "{rel} must contain no {needle:?} (it would be a second way to write to a server)",
        );
    }
    for needle in FORBIDDEN_NEEDLES {
        let total: usize = files
            .iter()
            .map(|file| file.code.matches(needle).count())
            .sum();
        assert_eq!(
            total, 0,
            "{needle:?} is forbidden in every product crate, seen {total} times",
        );
    }
}

/// **The structural guard on the two allowed state changes** (M6b acceptance,
/// source-check half; the runtime half is
/// `tests/library_read_only.rs::progress_reporting_is_off_by_default_and_is_the_only_write`).
///
/// * `Server` defines no write wrapper: no `pub fn post`/`put`/`patch`/`delete`,
///   and no `post_candidate_write` left over from the earlier candidate shape, so
///   no caller outside `plex-core` can perform a non-read request through
///   `Server`.
/// * `progress.rs` consults the opt-in (`update.refusal()`) **before** its first
///   request (`server.get(`), so the default-off path cannot reach a transport.
/// * `enabled = true` - the only mutation that makes a report sendable - exists
///   in exactly one product file, and `enabled: false` (the constructor default)
///   in exactly one product file.
///
/// The order assertion is a first-occurrence text comparison, so it proves the
/// guard sits ahead of the request in this source, not that it runs first.
#[test]
fn the_only_state_changes_reachable_in_product_code_are_the_progress_calls() {
    let server = code_only(&raw_source("plex-core/src/server.rs"));
    assert_eq!(
        server.matches("post_candidate_write").count(),
        0,
        "Server must not keep the candidate-era write wrapper once the endpoints are GET",
    );
    for signature in [
        "pub fn post(",
        "pub fn put(",
        "pub fn patch(",
        "pub fn delete(",
    ] {
        assert_eq!(
            server.matches(signature).count(),
            0,
            "Server must expose no public {signature:?}: a public write wrapper would be reachable from every crate",
        );
    }

    let progress = code_only(&raw_source("plex-core/src/progress.rs"));
    let refusal_at = progress
        .find(".refusal()")
        .expect("progress.rs must consult refusal() before reporting");
    let write_at = progress
        .find("server.get(")
        .expect("progress.rs must send the progress request");
    assert!(
        refusal_at < write_at,
        "the opt-in check must precede the request in progress.rs (refusal at {refusal_at}, request at {write_at})",
    );

    let files = scanned_tree();
    let opt_in_files: Vec<&str> = files
        .iter()
        .filter(|file| file.code.contains("enabled = true"))
        .map(|file| file.rel.as_str())
        .collect();
    assert_eq!(
        opt_in_files,
        vec!["plex-core/src/progress.rs"],
        "only ProgressUpdate::opt_in may turn a report on: any other `enabled = true` in product code is a new mutation path",
    );
    let default_off_files: Vec<&str> = files
        .iter()
        .filter(|file| file.code.contains("enabled: false"))
        .map(|file| file.rel.as_str())
        .collect();
    assert_eq!(
        default_off_files,
        vec!["plex-core/src/progress.rs"],
        "ProgressUpdate::new is the only construction of the default",
    );
}

const SCANNER_SAMPLE: &str = r###"
fn sample() {
    // .post("must-not-count") in a line comment
    // doc comment: "POST" must not count either
    /* .delete("must-not-count") with a /* nested .put("must-not-count") */ inner */
    /* another
       multi-line "PATCH" must not count
    */
    let a = ".put(called-from-a-string)";
    let b = "escaped \"POST\" literal";
    let c = b"byte .delete( string";
    let d = r#"raw "DELETE" content"#;
    let e = r##"hashy ".patch( content"## ;
    let f = '"';
    let g = '\'';
    let h = b'\\';
    let i: &'static str = a_life_time;
    let j = "POST";
    let k = "/:/timeline";
    let l = "/:/scrobble";
    http.post(".post( in a string");
    http.delete("x");
}
"###;

/// Hand-counted expectations for [`SCANNER_SAMPLE`]: strings count, comments do
/// not, char literals neither open nor close a string.
/// `http.post(".post( in a string")` is deliberately counted twice: the call
/// site **and** the needle inside the string it passes.
///
/// `"POST"` is **1**, not 2: only `let j = "POST";` is that needle. In
/// `let b = "escaped \"POST\" literal";` the scanner keeps the escape
/// backslashes verbatim, so the text it sees is `\"POST\"` - a quote before
/// `POST` and a backslash after it, i.e. **no** `"POST"` substring. An earlier
/// revision of this table expected 2 here and the `test` gate failed on it
/// (review 2026-11-06); counting the escaped form would mean the scanner had
/// rewritten the string, which it does not do.
const SAMPLE_WANT: [(&str, usize); 10] = [
    (".post(", 2),
    (".put(", 1),
    (".patch(", 1),
    (".delete(", 2),
    ("\"POST\"", 1),
    ("\"PUT\"", 0),
    ("\"PATCH\"", 0),
    ("\"DELETE\"", 1),
    (":/timeline", 1),
    (":/scrobble", 1),
];
